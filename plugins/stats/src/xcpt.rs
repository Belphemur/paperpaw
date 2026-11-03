//! CrossPoint's reading data, read and brought over.
//!
//! A card that a CrossPoint reader has written keeps its reading data in
//! `/.crosspoint`: a global record, an index of finished books, and, in a
//! folder per book, a record of that book's reading and where it was
//! left. The folders are named after the book's path, hashed the way
//! C++'s standard library does it, so a folder is matched to a book by
//! hashing the paths of the books on the card again (see [`std_hash`]).
//!
//! Every file is decoded with the constants of its own version, and a
//! file of a version this does not know is left alone: it is never read
//! as data of an older layout, and nothing is written over it. What is
//! brought over is what CrossPoint knew for certain: the minutes read per
//! day, the books with their titles and when they were finished, and
//! where each book was left. What would have to be made up is left out:
//! PaperPaw's log of page stays is never written to (those rows are
//! KOReader's, and CrossPoint kept none), and a position's xpointer,
//! which only a reader of the book itself can name, is not made up
//! either.

use alloc::string::String;
use alloc::vec::Vec;

/// A u16 at `at`, little-endian.
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

/// A u32 at `at`, little-endian.
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// A day, month and year, as the records keep them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Date {
    fn at(b: &[u8], at: usize) -> Date {
        let date = Date {
            year: u16_at(b, at),
            month: b[at + 2],
            day: b[at + 3],
        };
        if date.is_valid() {
            date
        } else {
            Date::default()
        }
    }

    /// Whether it is a day there can have been.
    fn is_valid(&self) -> bool {
        (1..=12).contains(&self.month) && (1..=31).contains(&self.day) && self.year > 0
    }

    /// The Unix seconds of that day at noon (the middle of the day, so
    /// that the date comes back in every time zone), 0 where the date is
    /// not one.
    pub(crate) fn unix_noon(&self) -> i64 {
        if !self.is_valid() {
            return 0;
        }
        // Days from 1970-01-01 to the first of the month, by the calendar.
        let year = i64::from(self.year);
        let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let mut days = 365 * (year - 1970)
            + i64::try_from((1970..year).filter(|y| leap(*y)).count()).unwrap_or(0);
        for month in 1..i64::from(self.month) {
            days += match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                _ if leap(year) => 29,
                _ => 28,
            };
        }
        (days + i64::from(self.day) - 1) * 86_400 + 12 * 3600
    }
}

// The per-book record, `stats_v8.bin` in a book's folder. Each version
// keeps the fields of the one before and adds its own at the end, so the
// file's size says which fields a record has; every version is checked
// against its own size and version, never against those of the newest.
//
//   0    1   version (5..8)
//   1    2   sessions read              u16
//   3    4   seconds read               u32
//   7    4   pages turned               u32
//   11   1   whether it was read to the end
//   12   4   (unused since version 6)
//   16   1   flags: whether the dates were set by hand
//   17   4   when reading began (year, month, day)
//   21   4   when it was finished
//   25  16   seconds read in the morning, afternoon, evening and night
//   41  28   seconds read on each day of the week
//   69   4   seconds left to read, 0 when unknown
//   73   2   words a minute, the average   (version 6 and later)
//   75   2   how many samples it is of
//   77  30   the samples themselves
//  107   1   where the next sample goes
//  108   1   how far in the book, in percent (255: unknown)
//  109   2   how long a session, the average (version 7 and later)
//  111   2   how many sessions it is of
//  113  20   the sessions themselves
//  133   1   where the next session goes
//  134   1   flags of the finishing (version 8)
pub(crate) const BOOK_V8: u8 = 8;
pub(crate) const BOOK_V7: u8 = 7;
pub(crate) const BOOK_V6: u8 = 6;
pub(crate) const BOOK_V5: u8 = 5;
const BOOK_SIZE_V8: usize = 135;
const BOOK_SIZE_V7: usize = 134;
const BOOK_SIZE_V6: usize = 109;
const BOOK_SIZE_V5: usize = 73;

/// What came of reading a per-book record: `Forward` when the file is of
/// a version this does not know, which is left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BookRecord {
    Ok(Book),
    Forward,
    Invalid,
}

/// A book's reading, as a record of version 5 to 8 keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Book {
    pub sessions: u32,
    pub seconds: u64,
    pub pages_turned: u64,
    pub completed: bool,
    pub start: Date,
    pub finished: Date,
    /// How far in the book, in percent; 255 where unknown.
    pub percent: u8,
    /// The words a minute of the last pages, as the record kept them.
    pub wpm: super::pace::WpmWindow,
    /// How long the last sessions were, as the record kept them.
    pub session: super::pace::SessionWindow,
}

/// Reads one of the per-book records, told by the file's size: every
/// version is checked against its own size and version, so a record cut
/// short is never read as a smaller one.
pub(crate) fn decode_book(b: &[u8]) -> BookRecord {
    // A version later than this knows is left alone before anything
    // else is said of the file — but only where the file is as long as
    // the smallest record this knows, for a cut-off write's bytes mean
    // nothing.
    if b.len() >= BOOK_SIZE_V5 && b[0] > BOOK_V8 {
        return BookRecord::Forward;
    }
    let version = match b.len() {
        BOOK_SIZE_V8 => BOOK_V8,
        BOOK_SIZE_V7 => BOOK_V7,
        BOOK_SIZE_V6 => BOOK_V6,
        BOOK_SIZE_V5 => BOOK_V5,
        _ => return BookRecord::Invalid,
    };
    if b[0] != version {
        return BookRecord::Invalid;
    }
    let mut book = Book {
        sessions: u32::from(u16_at(b, 1)),
        seconds: u64::from(u32_at(b, 3)),
        pages_turned: u64::from(u32_at(b, 7)),
        completed: b[11] != 0,
        start: Date::at(b, 17),
        finished: Date::at(b, 21),
        // The percent is of version 6 and later; a record without it
        // says nothing of how far in the book it was.
        percent: 255,
        ..Book::default()
    };
    if version >= BOOK_V6 {
        book.wpm =
            super::pace::WpmWindow::from_wire(u16_at(b, 73), u16_at(b, 75), &b[77..107], b[107]);
        book.percent = if b[108] <= 100 { b[108] } else { 255 };
    }
    if version >= BOOK_V7 {
        book.session = super::pace::SessionWindow::from_wire(
            u16_at(b, 109),
            u16_at(b, 111),
            &b[113..133],
            b[133],
        );
    }
    BookRecord::Ok(book)
}

// The global record, `global_stats.bin` at `/.crosspoint`. Like the
// per-book ones it grows by version, and each version is read with its
// own constants.
//
//   0    1   version (3..6)
//   1    4   sessions read              u32
//   5    4   seconds read               u32
//   9    4   pages turned               u32
//   13   4   books read to the end      u32
//   17  16   seconds read by time of day
//   33  28   seconds read by day of week
//   61   4   the day the history begins, in days since 1970
//   65  92   a bit a day, 730 days, set where the day was read
//  157   2   the most days read in a row
//  159   2   words a minute, the average   (version 4 and later)
//  161   2   how many samples it is of
//  163  30   the samples themselves
//  193   1   where the next sample goes
//  194   1   (unused)
//  195   2   how long a session, the average (version 5 and later)
//  197   2   how many sessions it is of
//  199  20   the sessions themselves
//  224   1   where the next session goes (219-223 unused)
//  225 182   minutes read on each of the last 91 days (version 6), the
//             first the day the history begins, then ever older
pub(crate) const GLOBAL_V6: u8 = 6;
pub(crate) const GLOBAL_V5: u8 = 5;
pub(crate) const GLOBAL_V4: u8 = 4;
pub(crate) const GLOBAL_V3: u8 = 3;
const GLOBAL_SIZE_V6: usize = 407;
const GLOBAL_SIZE_V5: usize = 225;
const GLOBAL_SIZE_V4: usize = 195;
const GLOBAL_SIZE_V3: usize = 159;
/// Where the minutes of the last days are, in version 6.
pub(crate) const MINUTES_AT: usize = 225;
/// How many days the minutes cover.
pub(crate) const MINUTES_DAYS: usize = 91;
/// The bytes of the read days, a bit a day for 730 days.
const HISTORY_BYTES: usize = 92;
/// Minutes there are in a day: more than that in a record is a damaged
/// field, not a long day.
const MINUTES_A_DAY: u32 = 24 * 60;

/// What came of reading the global record.
#[expect(
    clippy::large_enum_variant,
    reason = "the record is read whole and passed on whole; a box would cost a dereference on every field"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GlobalRecord {
    Ok(Global),
    Forward,
    Invalid,
}

/// All reading, as the global record keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Global {
    pub sessions: u32,
    pub seconds: u64,
    pub pages_turned: u64,
    pub completed_books: u32,
    /// The day the history begins, in days since 1970.
    pub anchor: u32,
    /// A bit a day back from the anchor, set where the day was read.
    pub history: [u8; HISTORY_BYTES],
    /// Minutes read on each of the last days, the first the anchor day,
    /// then ever older; 0 where the record has none.
    pub minutes: [u16; MINUTES_DAYS],
    /// The most days read in a row, as the record kept it.
    pub longest_streak: u16,
    pub wpm: super::pace::WpmWindow,
    pub session: super::pace::SessionWindow,
}

impl Default for Global {
    fn default() -> Self {
        Self {
            sessions: 0,
            seconds: 0,
            pages_turned: 0,
            completed_books: 0,
            anchor: 0,
            history: [0; HISTORY_BYTES],
            minutes: [0; MINUTES_DAYS],
            longest_streak: 0,
            wpm: super::pace::WpmWindow::default(),
            session: super::pace::SessionWindow::default(),
        }
    }
}

impl Global {
    /// Whether the day `days` before the anchor was read, from the
    /// history's bits.
    fn was_read(&self, days: usize) -> bool {
        let (byte, bit) = (days / 8, days % 8);
        byte < HISTORY_BYTES && self.history[byte] & (1 << bit) != 0
    }

    /// The minutes read on each of the last days, as the record has
    /// them: where it is of version 5 or before, which has no minutes,
    /// every read day counts a minute, as CrossPoint showed those too.
    pub(crate) fn minutes_per_day(&self, version: u8) -> [u16; MINUTES_DAYS] {
        let mut minutes = self.minutes;
        if version < GLOBAL_V6 {
            for (day, minutes) in minutes.iter_mut().enumerate() {
                *minutes = u16::from(self.was_read(day));
            }
        }
        minutes
    }
}

/// Reads one of the global records, told by the size and checked against
/// its own constants.
pub(crate) fn decode_global(b: &[u8]) -> GlobalRecord {
    // A version later than this knows is left alone before anything
    // else is said of the file, from the smallest record's size on.
    if b.len() >= GLOBAL_SIZE_V3 && b[0] > GLOBAL_V6 {
        return GlobalRecord::Forward;
    }
    let version = match b.len() {
        GLOBAL_SIZE_V6 => GLOBAL_V6,
        GLOBAL_SIZE_V5 => GLOBAL_V5,
        GLOBAL_SIZE_V4 => GLOBAL_V4,
        GLOBAL_SIZE_V3 => GLOBAL_V3,
        _ => return GlobalRecord::Invalid,
    };
    if b[0] != version {
        return GlobalRecord::Invalid;
    }
    let mut global = Global {
        sessions: u32_at(b, 1),
        seconds: u64::from(u32_at(b, 5)),
        pages_turned: u64::from(u32_at(b, 9)),
        completed_books: u32_at(b, 13),
        anchor: u32_at(b, 61),
        history: b[65..65 + HISTORY_BYTES]
            .try_into()
            .unwrap_or([0; HISTORY_BYTES]),
        longest_streak: u16_at(b, 157),
        ..Global::default()
    };
    if version >= GLOBAL_V4 {
        global.wpm =
            super::pace::WpmWindow::from_wire(u16_at(b, 159), u16_at(b, 161), &b[163..193], b[193]);
    }
    if version >= GLOBAL_V5 {
        // The sessions sit at 199, and where the next goes at 224, with
        // five unused bytes between: as CrossPoint kept them.
        global.session = super::pace::SessionWindow::from_wire(
            u16_at(b, 195),
            u16_at(b, 197),
            &b[199..219],
            b[224],
        );
    }
    if version >= GLOBAL_V6 {
        for (day, minutes) in global.minutes.iter_mut().enumerate() {
            *minutes = u32::from(u16_at(b, MINUTES_AT + day * 2)).min(MINUTES_A_DAY) as u16;
        }
    }
    GlobalRecord::Ok(global)
}

// The index of finished books, `finished_books.bin` at `/.crosspoint`:
// the letters `CPFB`, a version, how many and two empty bytes, then that
// many entries. An entry: the path hashed with FNV-1a (u64), the seconds
// read (u32), when reading began and when the book was finished (year,
// month, day each), then the title and the author, each as their length
// (u16) and that many bytes.
pub(crate) const FINISHED_MAGIC: &[u8; 4] = b"CPFB";
pub(crate) const FINISHED_V3: u8 = 3;
/// How many books the index keeps at most.
pub(crate) const FINISHED_MAX: usize = 32;
const TITLE_MAX: usize = 160;
const AUTHOR_MAX: usize = 120;

/// What came of reading the index of finished books.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FinishedRecord {
    Ok(Vec<FinishedEntry>),
    Forward,
    Invalid,
}

/// A finished book, as the index keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinishedEntry {
    /// The path hashed with FNV-1a, as CrossPoint named the book by.
    pub path_key: u64,
    pub seconds: u32,
    pub start: Date,
    pub finished: Date,
    pub title: String,
    pub author: String,
}

/// Reads the index of finished books. Entries without a title are left
/// out, as CrossPoint itself does (it could never show them).
pub(crate) fn decode_finished(b: &[u8]) -> FinishedRecord {
    if b.len() < 8 || &b[0..4] != FINISHED_MAGIC {
        return FinishedRecord::Invalid;
    }
    if b[4] > FINISHED_V3 {
        return FinishedRecord::Forward;
    }
    let count = usize::from(b[5]).min(FINISHED_MAX);
    let mut at = 8;
    let mut entries = Vec::new();
    for _ in 0..count {
        if b.len() < at + 20 {
            return FinishedRecord::Invalid;
        }
        let entry = FinishedEntry {
            path_key: u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default()),
            seconds: u32_at(b, at + 8),
            start: Date::at(b, at + 12),
            finished: Date::at(b, at + 16),
            title: String::new(),
            author: String::new(),
        };
        at += 20;
        let Some((title, next)) = read_text(b, at, TITLE_MAX) else {
            return FinishedRecord::Invalid;
        };
        at = next;
        let Some((author, next)) = read_text(b, at, AUTHOR_MAX) else {
            return FinishedRecord::Invalid;
        };
        at = next;
        if title.is_empty() {
            continue;
        }
        entries.push(FinishedEntry {
            title,
            author,
            ..entry
        });
    }
    // The entries end where the file does: a file cut short is no
    // index, and neither is one whose count says more than there is.
    if at != b.len() {
        return FinishedRecord::Invalid;
    }
    FinishedRecord::Ok(entries)
}

/// A text of `limit` bytes at most, its length (u16) before it. `None`
/// where the length or the text runs past the file, for a file cut
/// short is no index, read in no parts.
fn read_text(b: &[u8], at: usize, limit: usize) -> Option<(String, usize)> {
    if b.len() < at + 2 {
        return None;
    }
    let len = usize::from(u16_at(b, at));
    // A length past what the index keeps, or past the file: no index.
    if len > limit || b.len() < at + 2 + len {
        return None;
    }
    Some((
        String::from_utf8_lossy(&b[at + 2..at + 2 + len]).into_owned(),
        at + 2 + len,
    ))
}

// Where a book was left, `progress.bin` in a book's folder. A record of
// an EPUB (6, 10 or 16 bytes, each shape keeping the fields of the one
// before): the section (which file of the book, from 0), the page in it
// and how many pages the section has, then where on the page the text
// shown began, then, in the newest shape, where in the section's text
// the page began and which layout that was under. A record of an XTC
// book is the page of the book alone (4 bytes), from 0.
/// What came of reading where a book was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressRecord {
    /// A page of an XTC book, from 0.
    Xtc(u32),
    /// An EPUB: the section (from 0), the page in it (from 0), how many
    /// pages it has, and where in the section the page begins, where the
    /// record says.
    Epub {
        section: u16,
        page: u16,
        pages: u16,
        offset: u32,
    },
    /// No record that can be read.
    None,
}

/// Reads where a book was left, by the record's size, as CrossPoint's
/// ProgressManager does: an unknown size is no record at all.
pub(crate) fn decode_progress(b: &[u8]) -> ProgressRecord {
    match b.len() {
        4 => ProgressRecord::Xtc(u32_at(b, 0)),
        6 | 10 | 16 => ProgressRecord::Epub {
            section: u16_at(b, 0),
            page: u16_at(b, 2),
            pages: u16_at(b, 4),
            offset: if b.len() >= 10 { u32_at(b, 6) } else { 0 },
        },
        _ => ProgressRecord::None,
    }
}

/// What a book's cache keeps of the book itself (`book.bin`): the version
/// (10), where its tables begin (u32), how many files the book has and
/// how many entries its table of contents (u16 each), then the title, the
/// author, the language and two places in the book, each as their length
/// (u32) and that many bytes.
pub(crate) const BOOK_BIN_VERSION: u8 = 10;

/// What `book.bin` says of the book.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct BookMeta {
    pub title: String,
    pub author: String,
    pub language: String,
    /// How many files the book is made of (its spine), 0 where unknown.
    pub sections: u32,
}

/// Reads what `book.bin` says of the book: its title and author, and how
/// many files it has. `None` where it is not a `book.bin` this knows.
pub(crate) fn decode_book_meta(b: &[u8]) -> Option<BookMeta> {
    if b.len() < 9 || b[0] != BOOK_BIN_VERSION {
        return None;
    }
    let mut meta = BookMeta {
        sections: u32::from(u16_at(b, 5)),
        ..BookMeta::default()
    };
    let mut at = 9;
    for text in 0..3 {
        let (string, next) = read_text32(b, at)?;
        at = next;
        match text {
            0 => meta.title = string,
            1 => meta.author = string,
            2 => meta.language = string,
            _ => {}
        }
    }
    Some(meta)
}

/// A text as `book.bin` keeps it: its length (u32) before it. `None`
/// where the text runs past the file, for a file cut short is not a
/// `book.bin` to be read in parts.
fn read_text32(b: &[u8], at: usize) -> Option<(String, usize)> {
    if b.len() < at + 4 {
        return None;
    }
    let len = u32_at(b, at) as usize;
    // A length this size is no length.
    if len > 4096 || b.len() < at + 4 + len {
        return None;
    }
    let next = at + 4 + len;
    Some((String::from_utf8_lossy(&b[at + 4..next]).into_owned(), next))
}

/// How C++'s standard library hashed the paths CrossPoint named its book
/// folders by: Murmur, the way libstdc++ does it for 32-bit sizes, with
/// its own seed, of the path as the card names it (`/books/a.epub`).
///
/// The folders are matched to books by hashing the paths of the books on
/// the card again — CrossPoint's records say nothing of which book they
/// belong to but the folder's name.
pub(crate) fn std_hash(path: &str) -> u32 {
    const M: u32 = 0x5bd1e995;
    const SEED: u32 = 0xc70f6907;
    let bytes = path.as_bytes();
    let mut hash = SEED ^ (bytes.len() as u32);
    let (chunks, rest) = bytes.as_chunks::<4>();
    for chunk in chunks {
        let mut k = u32::from_le_bytes(*chunk);
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        hash = hash.wrapping_mul(M);
        hash ^= k;
    }
    // The last one, two or three bytes, as the switch of the original
    // falls through: the second and third byte in from the end, then the
    // first, and only then the mix.
    if rest.len() >= 2 {
        hash ^= u32::from(rest[1]) << 8;
    }
    if rest.len() == 3 {
        hash ^= u32::from(rest[2]) << 16;
    }
    if !rest.is_empty() {
        hash ^= u32::from(rest[0]);
        hash = hash.wrapping_mul(M);
    }
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(M);
    hash ^= hash >> 15;
    hash
}

/// Which kind of book a folder is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Epub,
    Xtc,
}

impl Kind {
    /// From a path's ending, as CrossPoint told them apart.
    pub(crate) fn of(path: &str) -> Option<Kind> {
        let lower = path.to_ascii_lowercase();
        if lower.ends_with(".epub") {
            Some(Kind::Epub)
        } else if lower.ends_with(".xtc") {
            Some(Kind::Xtc)
        } else {
            None
        }
    }

    /// As the folders are named.
    fn name(self) -> &'static str {
        match self {
            Kind::Epub => "epub",
            Kind::Xtc => "xtc",
        }
    }
}

impl core::fmt::Display for Kind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

/// The name of a book's folder: `epub_` or `xtc_` and the path hashed,
/// as a number, as `std::to_string` writes it.
pub(crate) fn cache_dir(kind: Kind, path: &str) -> String {
    alloc::format!("{kind}_{}", std_hash(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A per-book record of version 8, byte by byte, from the layout
    /// above. Fields the tests set are given, the rest zero.
    fn book_v8(
        version: u8,
        sessions: u32,
        seconds: u32,
        wpm: &[u16],
        session: &[u16],
        percent: u8,
    ) -> alloc::vec::Vec<u8> {
        let mut b = alloc::vec![0u8; 135];
        b[0] = version;
        b[1..3].copy_from_slice(&(sessions as u16).to_le_bytes());
        b[3..7].copy_from_slice(&seconds.to_le_bytes());
        b[7..11].copy_from_slice(&42u32.to_le_bytes());
        b[11] = 1;
        b[17..19].copy_from_slice(&2024u16.to_le_bytes());
        b[19] = 3;
        b[20] = 5;
        b[21..23].copy_from_slice(&2025u16.to_le_bytes());
        b[23] = 6;
        b[24] = 7;
        b[25..29].copy_from_slice(&100u32.to_le_bytes());
        b[33..37].copy_from_slice(&200u32.to_le_bytes());
        b[37..41].copy_from_slice(&300u32.to_le_bytes());
        b[41..45].copy_from_slice(&10u32.to_le_bytes());
        b[41 + 24..45 + 24].copy_from_slice(&70u32.to_le_bytes());
        b[69..73].copy_from_slice(&3600u32.to_le_bytes());
        b[73..75].copy_from_slice(&0u16.to_le_bytes());
        for (i, sample) in wpm.iter().enumerate() {
            b[77 + i * 2..79 + i * 2].copy_from_slice(&sample.to_le_bytes());
        }
        b[75..77].copy_from_slice(&(wpm.len() as u16).to_le_bytes());
        b[107] = wpm.len() as u8;
        b[108] = percent;
        for (i, sample) in session.iter().enumerate() {
            b[113 + i * 2..115 + i * 2].copy_from_slice(&sample.to_le_bytes());
        }
        b[111..113].copy_from_slice(&(session.len() as u16).to_le_bytes());
        b[133] = session.len() as u8;
        b[134] = 0b10;
        b
    }

    #[test]
    fn the_book_record_is_read() {
        let bytes = book_v8(8, 3, 7200, &[250, 260], &[1200, 1300], 45);
        let BookRecord::Ok(book) = decode_book(&bytes) else {
            panic!("not read");
        };
        assert_eq!(
            (
                book.sessions,
                book.seconds,
                book.pages_turned,
                book.completed
            ),
            (3, 7200, 42, true)
        );
        assert_eq!(
            book.start,
            Date {
                year: 2024,
                month: 3,
                day: 5
            }
        );
        assert_eq!(
            book.finished,
            Date {
                year: 2025,
                month: 6,
                day: 7
            }
        );
        assert_eq!(book.percent, 45);
        assert_eq!(book.wpm.avg(), 255);
        assert_eq!(book.wpm.count(), 2);
        assert_eq!(book.session.avg(), 1250);
    }

    #[test]
    fn every_version_is_read_with_its_own_constants() {
        // A v7 record: 134 bytes, without the finishing byte.
        let mut v7 = book_v8(7, 1, 60, &[300], &[900], 10);
        v7.truncate(134);
        let BookRecord::Ok(book) = decode_book(&v7) else {
            panic!("not read");
        };
        assert_eq!((book.sessions, book.percent), (1, 10));
        // A v6 record: 109 bytes, without the session window.
        let mut v6 = v7.clone();
        v6.truncate(109);
        v6[0] = 6;
        let BookRecord::Ok(book) = decode_book(&v6) else {
            panic!("not read");
        };
        assert_eq!(book.wpm.avg(), 300);
        assert_eq!(book.session.count(), 0);
        // A v5 record: 73 bytes, without the words a minute.
        let mut v5 = v6.clone();
        v5.truncate(73);
        v5[0] = 5;
        let BookRecord::Ok(book) = decode_book(&v5) else {
            panic!("not read");
        };
        assert_eq!((book.seconds, book.wpm.count()), (60, 0));
        assert_eq!(book.percent, 255);
        // A version later than this knows is left alone, whole as it is.
        let mut v9 = book_v8(8, 1, 1, &[], &[], 0);
        v9[0] = 9;
        assert_eq!(decode_book(&v9), BookRecord::Forward);
        // A cut record is no record.
        v9[0] = 8;
        v9.truncate(50);
        assert_eq!(decode_book(&v9), BookRecord::Invalid);
        // A size with the wrong version byte in it is not read.
        let mut wrong = book_v8(8, 1, 1, &[], &[], 0);
        wrong[0] = 7;
        assert_eq!(decode_book(&wrong), BookRecord::Invalid);
    }

    /// A corrupt window is normalized, never taken as it is.
    #[test]
    fn a_corrupt_window_is_normalized() {
        let mut bytes = book_v8(
            8,
            1,
            60,
            &(150..165)
                .map(|w| w as u16)
                .collect::<alloc::vec::Vec<u16>>(),
            &[50; 10],
            0,
        );
        // A count larger than the window holds, a cursor past it, and an
        // average that is not the samples'.
        bytes[75..77].copy_from_slice(&20u16.to_le_bytes());
        bytes[107] = 9;
        bytes[73..75].copy_from_slice(&999u16.to_le_bytes());
        let BookRecord::Ok(book) = decode_book(&bytes) else {
            panic!("not read");
        };
        assert_eq!(book.wpm.count(), 15);
        // The cursor points inside the window: it stays there.
        assert_eq!(book.wpm.pos(), 9);
        // The odd ones of the fifteen are trimmed: 157 on average.
        assert_eq!(book.wpm.avg(), 157);
        bytes[111..113].copy_from_slice(&99u16.to_le_bytes());
        bytes[133] = 77;
        let BookRecord::Ok(book) = decode_book(&bytes) else {
            panic!("not read");
        };
        assert_eq!(book.session.count(), 10);
        assert_eq!(book.session.avg(), 50);
        // A percent past 100 is unknown, never a made-up one.
        bytes[108] = 200;
        let BookRecord::Ok(book) = decode_book(&bytes) else {
            panic!("not read");
        };
        assert_eq!(book.percent, 255);
    }

    /// The global record of every version, and what the older ones say
    /// of the minutes read.
    #[test]
    fn the_global_record_is_read() {
        // Version 6: 407 bytes, with 91 days of minutes.
        let mut v6 = alloc::vec![0u8; 407];
        v6[0] = 6;
        v6[1..5].copy_from_slice(&12u32.to_le_bytes());
        v6[5..9].copy_from_slice(&36000u32.to_le_bytes());
        v6[9..13].copy_from_slice(&700u32.to_le_bytes());
        v6[13..17].copy_from_slice(&4u32.to_le_bytes());
        v6[61..65].copy_from_slice(&20_000u32.to_le_bytes());
        v6[65] = 0b11; // The anchor day and the one before were read.
        v6[157..159].copy_from_slice(&5u16.to_le_bytes());
        v6[159..161].copy_from_slice(&250u16.to_le_bytes());
        v6[161..163].copy_from_slice(&2u16.to_le_bytes());
        v6[163..165].copy_from_slice(&240u16.to_le_bytes());
        v6[165..167].copy_from_slice(&260u16.to_le_bytes());
        v6[193] = 2;
        v6[195..197].copy_from_slice(&1500u16.to_le_bytes());
        v6[197..199].copy_from_slice(&1u16.to_le_bytes());
        v6[199..201].copy_from_slice(&1500u16.to_le_bytes());
        v6[224] = 1;
        v6[MINUTES_AT..MINUTES_AT + 2].copy_from_slice(&45u16.to_le_bytes());
        v6[MINUTES_AT + 2..MINUTES_AT + 4].copy_from_slice(&90u16.to_le_bytes());
        v6[MINUTES_AT + 4..MINUTES_AT + 6].copy_from_slice(&2000u16.to_le_bytes());
        let GlobalRecord::Ok(global) = decode_global(&v6) else {
            panic!("not read");
        };
        assert_eq!(
            (
                global.sessions,
                global.seconds,
                global.pages_turned,
                global.completed_books
            ),
            (12, 36000, 700, 4)
        );
        assert_eq!(global.anchor, 20_000);
        assert_eq!(global.wpm.avg(), 250);
        assert_eq!(global.session.avg(), 1500);
        assert_eq!(global.longest_streak, 5);
        assert_eq!(global.minutes[0], 45);
        assert_eq!(global.minutes[1], 90);
        // More minutes than a day has are a damaged field, not a long day.
        assert_eq!(global.minutes[2], 1440);
        assert_eq!(global.minutes_per_day(6)[0], 45);
        // Version 5: 225 bytes; its read days are a minute each.
        let mut v5 = v6[..225].to_vec();
        v5[0] = 5;
        let GlobalRecord::Ok(global) = decode_global(&v5) else {
            panic!("not read");
        };
        assert_eq!(global.session.avg(), 1500);
        assert_eq!(global.minutes_per_day(5)[0], 1);
        assert_eq!(global.minutes_per_day(5)[2], 0);
        // Version 4: 195 bytes, without the session window.
        let mut v4 = v5[..195].to_vec();
        v4[0] = 4;
        let GlobalRecord::Ok(global) = decode_global(&v4) else {
            panic!("not read");
        };
        assert_eq!(global.wpm.avg(), 250);
        assert_eq!(global.session.count(), 0);
        assert_eq!(global.minutes_per_day(4)[0], 1);
        // Version 3: 159 bytes, without the words a minute.
        let mut v3 = v4[..159].to_vec();
        v3[0] = 3;
        let GlobalRecord::Ok(global) = decode_global(&v3) else {
            panic!("not read");
        };
        assert_eq!(global.wpm.count(), 0);
        assert_eq!(global.longest_streak, 5);
        // Later than this knows: left alone.
        let mut v7 = v6.clone();
        v7[0] = 7;
        assert_eq!(decode_global(&v7), GlobalRecord::Forward);
        // Cut short, or the wrong version in the size's shape: no record.
        v7[0] = 6;
        v7.truncate(300);
        assert_eq!(decode_global(&v7), GlobalRecord::Invalid);
        let mut wrong = v6.clone();
        wrong[0] = 5;
        assert_eq!(decode_global(&wrong), GlobalRecord::Invalid);
    }

    /// The index of finished books, whole and cut, and one of a newer
    /// version.
    #[test]
    fn the_finished_index_is_read() {
        let mut b = alloc::vec::Vec::new();
        b.extend_from_slice(FINISHED_MAGIC);
        b.push(3);
        b.push(2);
        b.extend_from_slice(&0u16.to_le_bytes());
        // First entry: the path hashed, seconds read, dates, title, author.
        b.extend_from_slice(&0x1234_5678_9abc_def0u64.to_le_bytes());
        b.extend_from_slice(&7200u32.to_le_bytes());
        b.extend_from_slice(&2024u16.to_le_bytes());
        b.push(1);
        b.push(2);
        b.extend_from_slice(&2025u16.to_le_bytes());
        b.push(3);
        b.push(4);
        b.extend_from_slice(&6u16.to_le_bytes());
        b.extend_from_slice(b"Anna K");
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(b"Leo");
        // Second entry without a title: not read back, as CrossPoint does.
        b.extend_from_slice(&0u64.to_le_bytes());
        b.extend_from_slice(&0u32.to_le_bytes());
        b.extend_from_slice(&[0u8; 8]);
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&0u16.to_le_bytes());
        let FinishedRecord::Ok(entries) = decode_finished(&b) else {
            panic!("not read");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path_key, 0x1234_5678_9abc_def0);
        assert_eq!(entries[0].seconds, 7200);
        assert_eq!(
            entries[0].start,
            Date {
                year: 2024,
                month: 1,
                day: 2
            }
        );
        assert_eq!(
            entries[0].finished,
            Date {
                year: 2025,
                month: 3,
                day: 4
            }
        );
        assert_eq!(entries[0].title, "Anna K");
        assert_eq!(entries[0].author, "Leo");
        // A version later than this knows is left alone.
        b[4] = 4;
        assert_eq!(decode_finished(&b), FinishedRecord::Forward);
        // Cut short, wrong magic, or too many books: no index.
        b[4] = 3;
        assert_eq!(decode_finished(&b[..b.len() - 4]), FinishedRecord::Invalid);
        b[0] = b'X';
        assert_eq!(decode_finished(&b), FinishedRecord::Invalid);
        b[0] = b'C';
        b[5] = 200;
        assert_eq!(decode_finished(&b), FinishedRecord::Invalid);
    }

    #[test]
    fn where_a_book_was_left_is_read() {
        let mut epub = [0u8; 10];
        epub[0..2].copy_from_slice(&3u16.to_le_bytes());
        epub[2..4].copy_from_slice(&7u16.to_le_bytes());
        epub[4..6].copy_from_slice(&12u16.to_le_bytes());
        epub[6..10].copy_from_slice(&900u32.to_le_bytes());
        assert_eq!(
            decode_progress(&epub),
            ProgressRecord::Epub {
                section: 3,
                page: 7,
                pages: 12,
                offset: 900
            }
        );
        // Without the offset, and in the oldest shape.
        assert_eq!(
            decode_progress(&epub[..6]),
            ProgressRecord::Epub {
                section: 3,
                page: 7,
                pages: 12,
                offset: 0
            }
        );
        // An XTC book's page, from 0.
        assert_eq!(
            decode_progress(&999u32.to_le_bytes()),
            ProgressRecord::Xtc(999)
        );
        // Nothing that can be read.
        assert_eq!(decode_progress(&[]), ProgressRecord::None);
        assert_eq!(decode_progress(&[0; 8]), ProgressRecord::None);
    }

    #[test]
    fn the_book_s_metadata_is_read() {
        let mut b = alloc::vec::Vec::new();
        b.push(10);
        b.extend_from_slice(&100u32.to_le_bytes());
        b.extend_from_slice(&7u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        for text in ["Anna Karenina", "Leo Tolstoy", "ru"] {
            b.extend_from_slice(&(text.len() as u32).to_le_bytes());
            b.extend_from_slice(text.as_bytes());
        }
        let meta = decode_book_meta(&b).unwrap();
        assert_eq!(meta.title, "Anna Karenina");
        assert_eq!(meta.author, "Leo Tolstoy");
        assert_eq!(meta.language, "ru");
        assert_eq!(meta.sections, 7);
        // A text cut short: not a `book.bin`, read in no parts.
        b.truncate(b.len() - 2);
        assert_eq!(decode_book_meta(&b), None);
        assert_eq!(decode_book_meta(&[9, 0, 0, 0, 0, 0, 0, 0, 0]), None);
    }

    /// The paths hashed the way C++'s standard library does it: the same
    /// as libstdc++'s `std::hash<std::string>` on a 32-bit machine,
    /// which is what CrossPoint runs on. The values pinned here come
    /// from that algorithm, checked against its source in GCC.
    #[test]
    fn paths_are_hashed_as_libstdcxx_hashes_them() {
        for (path, want) in [
            ("", 3_990_065_800),
            ("a", 2_167_009_006),
            ("hello", 3_305_111_549),
            ("/books/Anna Karenina.epub", 1_071_988_964),
            ("books/x.epub", 250_798_361),
            ("a longer path with spaces in it.epub", 3_332_617_438),
        ] {
            assert_eq!(std_hash(path), want, "{path}");
        }
        assert_eq!(cache_dir(Kind::Epub, "/books/x.epub"), "epub_1473393747");
        assert_eq!(
            cache_dir(Kind::Xtc, "/books/x.epub"),
            alloc::format!("xtc_{}", std_hash("/books/x.epub"))
        );
        assert_eq!(Kind::of("/books/A.EPUB"), Some(Kind::Epub));
        assert_eq!(Kind::of("/books/b.xtc"), Some(Kind::Xtc));
        assert_eq!(Kind::of("/books/c.pdf"), None);
    }

    /// The days a date is, worked out from the calendar alone.
    #[test]
    fn dates_are_days() {
        assert_eq!(
            Date {
                year: 1970,
                month: 1,
                day: 1
            }
            .unix_noon(),
            43_200
        );
        assert_eq!(
            Date {
                year: 2000,
                month: 3,
                day: 1
            }
            .unix_noon(),
            951_912_000
        );
        assert_eq!(
            Date {
                year: 2024,
                month: 2,
                day: 29
            }
            .unix_noon(),
            1_709_208_000
        );
        assert_eq!(
            Date {
                year: 2025,
                month: 0,
                day: 4
            }
            .unix_noon(),
            0
        );
        assert_eq!(
            Date {
                year: 2025,
                month: 13,
                day: 4
            }
            .unix_noon(),
            0
        );
    }
}
