//! Anki's `.apkg` decks from the card (`/Flashcards/deck.apkg`): a ZIP with
//! an SQLite database of the notes, read here without Anki.
//!
//! The deck's database (`collection.anki21`, or `collection.anki2` of an
//! export for older Anki) is inflated to a file of the plugin's folder, a
//! piece at a time; its `notes` table is read page by page; each note's
//! first three fields (front, back, example) are written as plain text to a
//! tab separated file, which is then merged as every other file of the
//! folder is. Nothing of the database is kept in memory but one page and
//! one note, so the heap is the same as for a text file. Scheduling, media
//! and the other note types' templates are left out (ticket #26, Anki's own
//! sync, is not this).

use alloc::string::String;

use paperpaw_sdk::{Storage, StorageError};
use paperpaw_sqlite::{Database, Error as SqlError, Value};
use paperpaw_zip::{ZipArchive, ZipError};

use crate::{cards, table};

/// The deck's database inflated: at most this large. A deck exported
/// without scheduling is far smaller than one with.
pub(crate) const MAX_DB: u32 = 16 * 1024 * 1024;
/// A ZIP of at most this many files (the media of a deck are files too):
/// its index takes 8 bytes of memory for each.
const MAX_FILES: u16 = 4000;
/// The notes of a deck, as many as the cards a deck holds.
const MAX_NOTES: usize = cards::MAX_CARDS;

/// Why a deck could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fault {
    /// Not an Anki deck: no ZIP, no database, no notes.
    NoNotes,
    /// Made by a newer Anki, which keeps its notes where this cannot read.
    NewerAnki,
    /// More files, a larger database or more notes than are read.
    TooLarge,
    /// No room for it in memory.
    Memory,
    /// The card could not be read or written.
    Storage,
}

impl From<StorageError> for Fault {
    fn from(_: StorageError) -> Self {
        Fault::Storage
    }
}

impl From<ZipError> for Fault {
    fn from(e: ZipError) -> Self {
        match e {
            ZipError::OutOfMemory => Fault::Memory,
            ZipError::TooLarge => Fault::TooLarge,
            ZipError::Storage(_) => Fault::Storage,
            _ => Fault::NoNotes,
        }
    }
}

impl From<SqlError> for Fault {
    fn from(e: SqlError) -> Self {
        match e {
            SqlError::OutOfMemory => Fault::Memory,
            SqlError::Storage(_) => Fault::Storage,
            _ => Fault::NoNotes,
        }
    }
}

/// Whether `name` is a deck of Anki's.
pub(crate) fn is_apkg(name: &str) -> bool {
    name.len() > 5
        && name.is_char_boundary(name.len() - 5)
        && name[name.len() - 5..].eq_ignore_ascii_case(".apkg")
}

/// Where the database is inflated to.
fn database_path() -> String {
    cards::path("apkg.db")
}

/// Where the notes are written to as text, to be read as a file of the
/// folder (until the job is done with it).
pub(crate) fn text_path() -> String {
    cards::path("apkg.tsv")
}

/// Removes what a deck left in the plugin's folder.
pub(crate) fn tidy(storage: &dyn Storage) {
    for path in [database_path(), text_path()] {
        if storage.exists(&path) {
            // A leftover is overwritten by the next deck; nothing to tell.
            let _ = storage.remove(&path);
        }
    }
}

/// How many files the ZIP at `path` holds, from its end record (no
/// comment); `None` where it is not found there.
fn file_count(storage: &dyn Storage, path: &str) -> Result<Option<u16>, StorageError> {
    let size = storage.size(path)?;
    let Some(at) = size.checked_sub(22) else {
        return Ok(None);
    };
    let mut record = [0u8; 22];
    if storage.read_at(path, at, &mut record)? != record.len() || record[..4] != [0x50, 0x4b, 5, 6]
    {
        return Ok(None);
    }
    Ok(Some(u16::from_le_bytes([record[10], record[11]])))
}

/// The deck at `path` written as text to [`text_path`]; how many notes.
pub(crate) fn convert(storage: &dyn Storage, path: &str) -> Result<usize, Fault> {
    tidy(storage);
    let result = read(storage, path);
    // The database is only needed while it is read.
    let db = database_path();
    if storage.exists(&db) {
        let _ = storage.remove(&db);
    }
    result
}

fn read(storage: &dyn Storage, path: &str) -> Result<usize, Fault> {
    if file_count(storage, path)?.is_some_and(|n| n > MAX_FILES) {
        return Err(Fault::TooLarge);
    }
    let zip = ZipArchive::open(storage, path)?;
    let named = |name: &str| zip.entry(storage, name);
    // Newer Anki puts its notes in `collection.anki21b` (zstd) and leaves
    // an old one with a note that says so: only a deck exported for older
    // versions is read.
    let entry = match (named("collection.anki21")?, named("collection.anki21b")?) {
        (Some(entry), _) => entry,
        (None, Some(_)) => return Err(Fault::NewerAnki),
        (None, None) => named("collection.anki2")?.ok_or(Fault::NoNotes)?,
    };
    if entry.size > MAX_DB {
        return Err(Fault::TooLarge);
    }
    let db = database_path();
    storage.write(&db, &[])?;
    let mut failed = None;
    zip.stream(
        storage,
        &entry,
        &mut |chunk| match storage.append(&db, chunk) {
            Ok(()) => true,
            Err(e) => {
                failed = Some(e);
                false
            }
        },
    )?;
    if let Some(e) = failed {
        return Err(e.into());
    }
    notes(storage, &db)
}

/// The notes table of the database at `db` written as text.
fn notes(storage: &dyn Storage, db: &str) -> Result<usize, Fault> {
    let database = Database::open(storage, db)?;
    let (columns, root) = database.table("notes").map_err(|_| Fault::NoNotes)?;
    let flds = columns
        .iter()
        .position(|c| c.eq_ignore_ascii_case("flds"))
        .ok_or(Fault::NoNotes)?;
    let out = text_path();
    storage.write(&out, b"#separator:tab\n")?;
    let (mut count, mut buffer, mut failed) = (0usize, String::new(), None);
    let mut put = |buffer: &mut String| match storage.append(&out, buffer.as_bytes()) {
        Ok(()) => {
            buffer.clear();
            true
        }
        Err(e) => {
            failed = Some(e);
            false
        }
    };
    let mut too_many = false;
    database.scan(root, &mut |_, values: &[Value]| {
        let Some(Value::Text(text)) = values.get(flds) else {
            return true;
        };
        let mut fields = text.split('\x1f').map(note_text);
        let front = fields.next().unwrap_or_default();
        if front.is_empty() {
            return true;
        }
        if count >= MAX_NOTES {
            too_many = true;
            return false;
        }
        count += 1;
        let back = fields.next().unwrap_or_default();
        let example = fields.next().unwrap_or_default();
        for (i, cell) in [front, back, example].iter().enumerate() {
            if i > 0 {
                buffer.push('\t');
            }
            quote(&mut buffer, cell);
        }
        buffer.push('\n');
        buffer.len() < 2048 || put(&mut buffer)
    })?;
    if too_many {
        return Err(Fault::TooLarge);
    }
    if !buffer.is_empty() {
        put(&mut buffer);
    }
    match failed {
        Some(e) => Err(e.into()),
        None if count == 0 => Err(Fault::NoNotes),
        None => Ok(count),
    }
}

/// A field of Anki's as plain text: markup and sounds out.
fn note_text(field: &str) -> String {
    let mut text = String::with_capacity(field.len());
    let mut rest = field;
    while let Some(at) = rest.find("[sound:") {
        text.push_str(&rest[..at]);
        match rest[at..].find(']') {
            Some(end) => rest = &rest[at + end + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    text.push_str(rest);
    table::plain(&text)
}

/// `cell` as a field of a tab separated file, quoted where it has to be.
fn quote(out: &mut String, cell: &str) {
    if cell.contains(['\t', '"', '\n', '\r']) {
        out.push('"');
        out.push_str(&cell.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(cell);
    }
}

#[cfg(test)]
mod tests;
