//! The notes of all books in the form of a Kindle's `My Clippings.txt`,
//! which notes apps and importers read (Readwise, Goodreads, Calibre
//! plugins, clipping managers):
//!
//! ```text
//! <BOM>Title (Author)
//! - Your Highlight at location 42-42 | Added on Sunday, 1 November 2026 10:05:00
//!
//! The text that was highlighted
//! ==========
//! ```
//!
//! UTF-8 with a byte order mark and CR LF line ends, as the Kindle writes
//! it; the weekday and the month in English whatever the language is, as
//! importers expect them. A book has no Kindle locations or printed pages
//! here: *location* is how far in the book the note is, in percent. A note
//! on a highlight is an entry of its own (*Your Note*) after it, as the
//! Kindle makes it; a bookmark has no text.
//!
//! The file is made anew each time, a book at a time, written to a
//! `.part` file that replaces the old one only when it is whole.

use alloc::format;
use alloc::string::String;
use core::fmt::Write as _;

use paperpaw_sdk::annotations::{self, Annotation, Kind};
use paperpaw_sdk::time;
use paperpaw_sdk::{Env, Storage, StorageError};

use crate::scan;

/// The folder beside the library that exports go to.
pub(crate) const FOLDER: &str = "Exports";
/// The file, as the Kindle names it.
pub(crate) const NAME: &str = "My Clippings.txt";

/// What an export made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Exported {
    pub notes: u32,
    pub books: u32,
}

const BOM: &str = "\u{feff}";
const SEPARATOR: &str = "==========";
const LINE: &str = "\r\n";
/// Written out when the text held is this long, so a book with many notes
/// does not fill the memory.
const FLUSH: usize = 6 * 1024;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Where the export goes: `Exports` beside the library folder.
pub(crate) fn folder(env: &Env<'_>) -> String {
    let library = paperpaw_sdk::library::folder(env);
    let parent = paperpaw_sdk::files::parent(library);
    if parent.is_empty() {
        String::from(FOLDER)
    } else {
        format!("{parent}/{FOLDER}")
    }
}

/// `seconds` since 1970 (UTC) as the Kindle says it, in local time:
/// `Sunday, 1 November 2026 10:05:00`. A time not known (0) is the
/// start of 1970.
fn date(env: &Env<'_>, seconds: i64) -> String {
    civil_text(if seconds > 0 {
        time::local_seconds(env, seconds)
    } else {
        0
    })
}

/// `local` seconds since 1970 as `Sunday, 1 November 2026 10:05:00`.
fn civil_text(local: i64) -> String {
    let days = local.div_euclid(86_400);
    let (year, month, day) = time::civil_from_days(days);
    let in_day = local.rem_euclid(86_400);
    format!(
        "{}, {day} {} {year} {:02}:{:02}:{:02}",
        WEEKDAYS[usize::from(time::weekday_of(days)) % 7],
        MONTHS[usize::from(month.saturating_sub(1)) % 12],
        in_day / 3600,
        in_day % 3600 / 60,
        in_day % 60,
    )
}

/// `text` in one line, as the file has lines of its own.
fn one_line(text: &str) -> String {
    let mut line = String::with_capacity(text.len());
    for word in text.split_whitespace() {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    line
}

/// The first line of an entry: the book as the Kindle names it.
fn book_line(title: &str, author: &str) -> String {
    let title = one_line(title);
    let author = one_line(author);
    if author.is_empty() {
        title
    } else {
        format!("{title} ({author})")
    }
}

/// One entry: what it is, where, when, and its text (none for a bookmark).
fn entry(out: &mut String, book: &str, what: &str, a: &Annotation, text: &str, when: &str) {
    // The Kindle gives a highlight a range of locations, the rest one.
    let place = if what == "Highlight" {
        format!("{p}-{p}", p = a.percent)
    } else {
        format!("{}", a.percent)
    };
    // Writing to a string cannot fail.
    write!(
        out,
        "{book}{LINE}- Your {what} at location {place} | Added on {when}{LINE}{LINE}{}{LINE}{SEPARATOR}{LINE}",
        one_line(text),
    )
    .ok();
}

/// The entries of the notes of one book, by position.
pub(crate) fn entries(
    when_text: &dyn Fn(i64) -> String,
    title: &str,
    author: &str,
    all: &[Annotation],
) -> (String, u32) {
    let book = book_line(title, author);
    let mut out = String::new();
    let mut count = 0;
    for a in all {
        let when = when_text(if a.created > 0 { a.created } else { a.updated });
        match a.mark.kind {
            Kind::Highlight => {
                entry(&mut out, &book, "Highlight", a, &a.text, &when);
                count += 1;
                if !a.note.is_empty() {
                    entry(&mut out, &book, "Note", a, &a.note, &when);
                    count += 1;
                }
            }
            Kind::Bookmark => {
                entry(&mut out, &book, "Bookmark", a, "", &when);
                count += 1;
            }
            _ => {}
        }
    }
    (out, count)
}

/// Writes the notes of every book to `Exports/My Clippings.txt`, replacing
/// the file there. An error where the file could not be written; nothing
/// is changed then.
pub(crate) fn export(env: &Env<'_>) -> Result<Exported, StorageError> {
    let storage: &dyn Storage = env.storage();
    let dir = folder(env);
    storage.create_dir(&dir)?;
    let file = format!("{dir}/{NAME}");
    let part = format!("{file}.part");
    storage.write(&part, BOM.as_bytes())?;
    let mut result = Exported { notes: 0, books: 0 };
    let mut failed = None;
    let mut buffer = String::new();
    scan::each_noted(env, &mut |record| {
        if failed.is_some() {
            return;
        }
        let all = annotations::load(storage, &record.path);
        let (text, count) = entries(&|at| date(env, at), &record.title, &record.author, &all);
        if count == 0 {
            return;
        }
        result.notes += count;
        result.books += 1;
        buffer.push_str(&text);
        if buffer.len() >= FLUSH {
            failed = storage.append(&part, buffer.as_bytes()).err();
            buffer.clear();
        }
    });
    if failed.is_none() && !buffer.is_empty() {
        failed = storage.append(&part, buffer.as_bytes()).err();
    }
    if let Some(error) = failed {
        storage.remove(&part).ok();
        return Err(error);
    }
    if result.notes == 0 {
        storage.remove(&part).ok();
        return Ok(result);
    }
    storage.rename(&part, &file).inspect_err(|_| {
        storage.remove(&part).ok();
    })?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::annotations::Mark;
    use paperpaw_sdk::document::Locator;

    fn note(kind: Kind, percent: u8, created: i64, text: &str, note: &str) -> Annotation {
        let at = Locator::new(1, 10);
        Annotation::new(
            Mark::new(kind, at, at, !note.is_empty()),
            percent,
            created,
            created,
            text,
            note,
        )
    }

    #[test]
    fn dates_are_as_the_kindle_says_them() {
        assert_eq!(
            civil_text(1_793_527_500),
            "Sunday, 1 November 2026 10:05:00"
        );
        assert_eq!(civil_text(0), "Thursday, 1 January 1970 00:00:00");
        assert_eq!(civil_text(951_782_399), "Monday, 28 February 2000 23:59:59");
        assert_eq!(
            civil_text(951_782_400),
            "Tuesday, 29 February 2000 00:00:00"
        );
    }

    #[test]
    fn a_highlight_with_a_note_is_two_entries_a_bookmark_has_no_text() {
        let all = [
            note(
                Kind::Highlight,
                42,
                1_793_527_500,
                "The wind\ncame  over",
                "so true",
            ),
            note(Kind::Bookmark, 50, 0, "p. 12", ""),
            note(Kind::Other(120), 60, 0, "x", ""),
        ];
        let (text, count) = entries(&civil_text, "A Quiet Year", "Ann Reed", &all);
        assert_eq!(count, 3);
        assert_eq!(
            text,
            "A Quiet Year (Ann Reed)\r\n\
             - Your Highlight at location 42-42 | Added on Sunday, 1 November 2026 10:05:00\r\n\
             \r\n\
             The wind came over\r\n\
             ==========\r\n\
             A Quiet Year (Ann Reed)\r\n\
             - Your Note at location 42 | Added on Sunday, 1 November 2026 10:05:00\r\n\
             \r\n\
             so true\r\n\
             ==========\r\n\
             A Quiet Year (Ann Reed)\r\n\
             - Your Bookmark at location 50 | Added on Thursday, 1 January 1970 00:00:00\r\n\
             \r\n\
             \r\n\
             ==========\r\n"
        );
    }

    #[test]
    fn a_book_without_an_author_is_named_by_its_title() {
        assert_eq!(book_line("  Two\nlines ", ""), "Two lines");
        assert_eq!(book_line("T", " A \n B"), "T (A B)");
    }
}
