//! Making the catalog: walked anew from the library folder ([`Walk`]) or
//! caught up ([`Rewrite`]), written to a building file a step at a time
//! and put in the old one's place. A walk that finds the catalog as it
//! was writes nothing.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::{
    Changes, HEADER, Record, Status, building_path, catalog_path, count_lines, lines, lines_from,
    log_path, next_header, read_changes,
};
use crate::library::{self, DIR};
use crate::{Env, Storage, StorageError, files};
use paperpaw_digest::fnv1a64;

/// Where a walk keeps the listing of the folder it walks, so that it goes
/// on where it was without listing the folder again.
fn listed_path() -> String {
    format!("{DIR}/walk.list")
}

/// Entries of a folder, or books of the catalog, taken in one step.
const STEP_LINES: usize = 16;
/// Bytes of a folder's listing taken in one step: a large folder is
/// listed once a step, a piece at a time, never held whole.
const LISTED_PIECE: usize = 4 * 1024;
/// Bytes a line of a listing takes beside its name, at most (its kind,
/// its size, tabs and the line's end).
const LISTED_LINE: usize = 24;
/// Lines of the old catalog a walk looks on for a book from where it
/// found the last, before it looks through the rest.
const OLD_WINDOW: usize = 32;
/// Bytes of the old catalog a walk reads ahead.
const READ_AHEAD: usize = 1024;
/// The longest line of the old catalog a walk reads (values are at most a
/// kilobyte each: no line written whole is this long).
const OLD_LINE: usize = 8 * 1024;
/// Bytes of the new catalog written at once while catching up.
const WRITTEN_PIECE: usize = 2 * 1024;

/// The paths of a list file (the favourites, the books opened last),
/// hashed and sorted, read a line at a time; none where it is not there.
fn hashed_list(storage: &dyn Storage, path: &str) -> Result<Vec<u32>, StorageError> {
    let mut hashes = Vec::new();
    hashes
        .try_reserve_exact(count_lines(storage, path)?)
        .map_err(|_| StorageError::TooLarge)?;
    lines(storage, path, &mut |_, line| {
        let line = line.trim_start_matches('\u{feff}');
        if !line.is_empty() && !line.starts_with(crate::data::MARK) {
            hashes.push(fnv1a64(line) as u32);
        }
        true
    })?;
    hashes.sort_unstable();
    hashes.dedup();
    Ok(hashes)
}

/// The first column of a line of the catalog: the book's path.
fn path_of(line: &str) -> &str {
    line.split('\t').next().unwrap_or("")
}

/// What a new catalog is made with: written to the building file a step
/// at a time, then put in the old one's place, with only the log noted
/// since it began kept (what was noted before is in it).
///
/// A walk compares what it makes with the old catalog first and writes
/// nothing while they are the same: where they part, the building file is
/// begun with the old catalog's lines so far (copied a piece a step), and
/// where they never do the old catalog stays, its log too (all it noted is
/// in the catalog: views keep what they made of them).
#[derive(Debug, Default)]
struct Making {
    /// How long the log was when it began.
    log_from: u64,
    /// What was noted (before it began, and while it paused).
    changes: Changes,
    /// The favourites and the books opened last, their paths hashed (the
    /// low bits), by hash.
    favourites: Vec<u32>,
    opened: Vec<u32>,
    /// The ids of the fields kept in the catalog.
    fields: Vec<String>,
    /// Books written.
    found: usize,
    /// Paused: what it read of the log and the lists let go, read again
    /// when it goes on.
    paused: bool,
    /// While all it made is the old catalog's lines so far: where the old
    /// one's books start (past its header), and where the next line not
    /// compared yet starts. Nothing is written then.
    same: Option<(u64, u64)>,
    /// Where it parted from the old catalog: its lines still to copy into
    /// the building file, then the text that was not the same.
    parting: Option<Parting>,
    /// Something noted in the log is not as the catalog has it (a status
    /// other than the one written, a book removed found again): the
    /// catalog is written anew even where its lines stay, the log cut.
    noted_differs: bool,
}

/// The old catalog's lines up to where a walk found it changed, copied
/// into the building file a piece a step, and what it found then.
#[derive(Debug, Default)]
struct Parting {
    /// Where the next piece to copy starts, and where the copy ends.
    at: u64,
    until: u64,
    /// The text that was not the same, written once the copy is done.
    text: String,
}

/// Bytes of the old catalog copied in one step where a walk parts from
/// it (as many as a catch-up writes).
const COPIED_PIECE: usize = WRITTEN_PIECE;

impl Making {
    /// Begins a new catalog; `compare`: with the old one first (see
    /// [`Making`]), where there is one of this format.
    fn start(env: &Env<'_>, compare: bool) -> Result<Making, StorageError> {
        let storage = env.storage();
        let log_from = match storage.size(&log_path()) {
            Ok(size) => size,
            Err(StorageError::NotFound) => 0,
            Err(e) => return Err(e),
        };
        let mut making = Making {
            log_from,
            fields: env
                .book_fields
                .iter()
                .filter(|f| f.in_catalog)
                .map(|f| f.id.as_str().to_string())
                .collect(),
            ..Making::default()
        };
        making.read(storage)?;
        let head = if compare { header_end(storage)? } else { None };
        match head {
            Some(head) => {
                making.same = Some((head, head));
                // One left by a walk cut short (the power gone).
                if storage.exists(&building_path()) {
                    storage.remove(&building_path())?;
                }
            }
            None => storage.write(&building_path(), next_header(storage).as_bytes())?,
        }
        Ok(making)
    }

    /// Reads what was noted and the lists.
    fn read(&mut self, storage: &dyn Storage) -> Result<(), StorageError> {
        self.changes = read_changes(storage)?;
        self.favourites = hashed_list(storage, &library::favourites_file())?;
        self.opened = hashed_list(storage, &library::opened_file())?;
        self.paused = false;
        Ok(())
    }

    /// Lets go of what it read, to read it again when it goes on.
    fn pause(&mut self) {
        self.changes = Changes::default();
        self.favourites = Vec::new();
        self.opened = Vec::new();
        self.paused = true;
    }

    fn go_on(&mut self, storage: &dyn Storage) -> Result<(), StorageError> {
        if self.paused {
            self.read(storage)?;
        }
        Ok(())
    }

    /// The record of the book at `path` from its details (made, the book
    /// opened, where there are none of this file): favourite and opened
    /// as the lists say; `true` where the book was opened.
    fn of_details(&self, env: &Env<'_>, path: &str, size: u64) -> (Record, bool) {
        let cached = library::book_sized(env.storage(), path, size).filter(|b| b.indexed);
        let opened = cached.is_none();
        let book = cached.unwrap_or_else(|| library::index(env, path));
        let fields: Vec<&str> = self.fields.iter().map(String::as_str).collect();
        let record = Record::of(path, &book)
            .with_status(self.status(path, book.progress))
            .with_fields(&book, &fields);
        (record, opened)
    }

    fn status(&self, path: &str, progress: Option<u8>) -> Status {
        let hash = fnv1a64(path) as u32;
        Status::new(
            progress,
            self.favourites.binary_search(&hash).is_ok(),
            self.opened.binary_search(&hash).is_ok(),
        )
    }

    /// Notes whether what the log says of the book of `record` is not as
    /// it is written (see [`Making::noted_differs`]).
    fn check_noted(&mut self, record: &Record) {
        match self.changes.get(fnv1a64(&record.path)) {
            None => {}
            Some(Some(status)) if status == record.status => {}
            Some(_) => self.noted_differs = true,
        }
    }

    /// Writes `text`, lines of `books` books, to the new catalog; while it
    /// is the old one, only compares (see [`Making`]).
    fn write_text(
        &mut self,
        storage: &dyn Storage,
        text: String,
        books: usize,
    ) -> Result<(), StorageError> {
        if text.is_empty() {
            return Ok(());
        }
        self.found += books;
        if let Some((head, at)) = self.same {
            if old_is(storage, at, text.as_bytes())? {
                self.same = Some((head, at + text.len() as u64));
            } else {
                self.part(storage, text)?;
            }
            return Ok(());
        }
        storage.append(&building_path(), text.as_bytes())
    }

    /// Parts from the old catalog: the building file begun, its lines so
    /// far to copy into it and `text` after them ([`Making::copy`]).
    fn part(&mut self, storage: &dyn Storage, mut text: String) -> Result<(), StorageError> {
        let Some((head, at)) = self.same.take() else {
            return Ok(());
        };
        // Held while the copy takes steps: no more than it needs.
        text.shrink_to_fit();
        storage.write(&building_path(), next_header(storage).as_bytes())?;
        self.parting = Some(Parting {
            at: head,
            until: at,
            text,
        });
        Ok(())
    }

    /// A piece of the old catalog copied where it parted from it, and the
    /// text that was not the same once all is; `true` while there is
    /// more to copy (the step is taken).
    fn copy(&mut self, storage: &dyn Storage) -> Result<bool, StorageError> {
        let Some(parting) = &mut self.parting else {
            return Ok(false);
        };
        if parting.at < parting.until {
            let left = usize::try_from(parting.until - parting.at).unwrap_or(COPIED_PIECE);
            let mut piece = alloc::vec![0; left.min(COPIED_PIECE)];
            let mut read = 0;
            while read < piece.len() {
                let n = storage.read_at(
                    &catalog_path(),
                    parting.at + read as u64,
                    &mut piece[read..],
                )?;
                if n == 0 {
                    // Shorter than when it was compared: written meanwhile
                    // by none but the kernel, so never.
                    return Err(StorageError::Io);
                }
                read += n;
            }
            storage.append(&building_path(), &piece)?;
            parting.at += piece.len() as u64;
            return Ok(true);
        }
        storage.append(&building_path(), parting.text.as_bytes())?;
        self.parting = None;
        Ok(false)
    }

    /// Puts the new catalog in place and keeps the log noted since it
    /// began; the log goes where it cannot be read (the details have what
    /// it said). Where it is the old one, that stays, and its log; where
    /// the old one has more lines, or the log notes what is not in it (a
    /// book removed that is there), it parts from it: `false`, more steps
    /// to take.
    fn finish(&mut self, storage: &dyn Storage) -> Result<bool, StorageError> {
        if self.copy(storage)? {
            return Ok(false);
        }
        if let Some((_, at)) = self.same {
            if self.noted_differs || storage.size(&catalog_path())? != at {
                self.part(storage, String::new())?;
                return Ok(false);
            }
            return Ok(true);
        }
        files::move_to(storage, &building_path(), &catalog_path())?;
        let mut since = String::new();
        let read = lines_from(storage, &log_path(), self.log_from, &mut |_, line| {
            since.push_str(line);
            since.push('\n');
            true
        });
        let kept = read.is_ok()
            && !since.is_empty()
            && storage.write(&log_path(), since.as_bytes()).is_ok();
        if !kept {
            #[expect(
                clippy::let_underscore_must_use,
                reason = "the changes left in the log are in the catalog now: applied again, they change nothing"
            )]
            let _ = storage.remove(&log_path());
        }
        Ok(true)
    }

    #[expect(
        clippy::let_underscore_must_use,
        reason = "what a walk given up leaves is written anew by the next walk"
    )]
    fn abort(storage: &dyn Storage) {
        let _ = storage.remove(&building_path());
        let _ = storage.remove(&listed_path());
    }
}

/// Where the books of the catalog start, past its header; `None` without
/// one, or of an older format.
fn header_end(storage: &dyn Storage) -> Result<Option<u64>, StorageError> {
    let mut buf = [0u8; 48];
    let n = match storage.read_at(&catalog_path(), 0, &mut buf) {
        Ok(n) => n,
        Err(StorageError::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    let Some(end) = buf[..n].iter().position(|&b| b == b'\n') else {
        return Ok(None);
    };
    Ok(buf[..end]
        .starts_with(HEADER.as_bytes())
        .then_some(end as u64 + 1))
}

/// Whether the old catalog holds `text` at `at`.
fn old_is(storage: &dyn Storage, at: u64, text: &[u8]) -> Result<bool, StorageError> {
    let mut buf = [0u8; 256];
    let mut done = 0;
    while done < text.len() {
        let want = (text.len() - done).min(buf.len());
        let n = storage.read_at(&catalog_path(), at + done as u64, &mut buf[..want])?;
        if n == 0 || buf[..n] != text[done..done + n] {
            return Ok(false);
        }
        done += n;
    }
    Ok(true)
}

/// The old catalog as a walk reads it along: where the next book to look
/// for starts, and a few kilobytes read ahead from there.
#[derive(Debug, Default)]
struct OldCatalog {
    /// Where the line to look at next starts; `None` without one.
    at: Option<u64>,
    /// Bytes of it read ahead, from `from` on; `end` where they reach its
    /// end.
    ahead: Vec<u8>,
    from: u64,
    end: bool,
    /// Books looked for and not found anywhere (new to the library) since
    /// it was looked through last: books new come in a row, so it is
    /// looked through again only after a window of them.
    missed: usize,
    /// Books found in it.
    found: usize,
}

impl OldCatalog {
    fn open(storage: &dyn Storage) -> Result<OldCatalog, StorageError> {
        let mut old = OldCatalog::default();
        match storage.size(&catalog_path()) {
            Ok(_) => {}
            Err(StorageError::NotFound) => return Ok(old),
            Err(e) => return Err(e),
        }
        // Its first line, the header, is not a book.
        old.at = old.line(storage, 0)?.map(|(_, next)| next);
        Ok(old)
    }

    /// Lets go of what was read ahead (read again from where the walk is
    /// when it goes on).
    fn let_go(&mut self) {
        self.ahead = Vec::new();
        self.from = 0;
        self.end = false;
    }

    /// The line that starts at `offset` and where the next one starts;
    /// `None` at its end. Read from what was read ahead, else read ahead
    /// from there. A line longer than [`OLD_LINE`] (none is, written
    /// whole) is passed over as empty, not read.
    fn line(
        &mut self,
        storage: &dyn Storage,
        offset: u64,
    ) -> Result<Option<(String, u64)>, StorageError> {
        let mut want = READ_AHEAD;
        loop {
            let start = offset
                .checked_sub(self.from)
                .and_then(|s| usize::try_from(s).ok())
                .filter(|&s| s <= self.ahead.len());
            if let Some(start) = start {
                let rest = &self.ahead[start..];
                if let Some(end) = rest.iter().position(|&b| b == b'\n') {
                    let line = String::from_utf8_lossy(&rest[..end]).trim_end().to_string();
                    return Ok(Some((line, offset + end as u64 + 1)));
                }
                if self.end {
                    if rest.is_empty() {
                        return Ok(None);
                    }
                    let line = String::from_utf8_lossy(rest).trim_end().to_string();
                    return Ok(Some((line, offset + rest.len() as u64)));
                }
                if start == 0 {
                    // Read from here and no line's end in it (or nothing
                    // read yet): more.
                    want = (self.ahead.len() * 2).max(READ_AHEAD);
                }
            }
            if want > OLD_LINE {
                return self.pass_over(storage, offset);
            }
            // Back to its size after a long line.
            if want == READ_AHEAD && self.ahead.capacity() > 2 * READ_AHEAD {
                self.ahead = Vec::new();
            }
            self.ahead.clear();
            self.ahead.resize(want, 0);
            let mut read = 0;
            while read < want {
                let n = storage.read_at(
                    &catalog_path(),
                    offset + read as u64,
                    &mut self.ahead[read..],
                )?;
                if n == 0 {
                    break;
                }
                read += n;
            }
            self.ahead.truncate(read);
            self.from = offset;
            self.end = read < want;
        }
    }

    /// The line at `offset` passed over, too long to hold: an empty line,
    /// and where the next starts (`None` past the end).
    fn pass_over(
        &mut self,
        storage: &dyn Storage,
        offset: u64,
    ) -> Result<Option<(String, u64)>, StorageError> {
        self.let_go();
        let mut buf = [0u8; 256];
        let mut at = offset;
        loop {
            let n = storage.read_at(&catalog_path(), at, &mut buf)?;
            if n == 0 {
                return Ok(None);
            }
            if let Some(end) = buf[..n].iter().position(|&b| b == b'\n') {
                return Ok(Some((String::new(), at + end as u64 + 1)));
            }
            at += n as u64;
        }
    }

    /// The line of the book at `path`: looked for a few lines on from the
    /// last found (the walk meets the books in the order it wrote them),
    /// else through the rest, read only (past books gone since: a folder
    /// removed or renamed), the place it is found taken as the walk's.
    fn find(&mut self, storage: &dyn Storage, path: &str) -> Result<Option<Record>, StorageError> {
        let Some(mut at) = self.at else {
            return Ok(None);
        };
        for _ in 0..OLD_WINDOW {
            let Some((line, next)) = self.line(storage, at)? else {
                break;
            };
            if path_of(&line) == path {
                self.at = Some(next);
                self.missed = 0;
                self.found += 1;
                return Ok(Record::parse(&line));
            }
            at = next;
        }
        if self.missed > 0 {
            // Looked through for a book new to the library: the next few
            // (most likely new too) only by the window.
            self.missed = (self.missed + 1) % (OLD_WINDOW + 1);
            return Ok(None);
        }
        let mut found = None;
        let next = lines_from(storage, &catalog_path(), at, &mut |_, line| {
            if path_of(line) == path {
                found = Record::parse(line);
                return false;
            }
            true
        })?;
        match found {
            Some(record) => {
                self.at = Some(next);
                self.found += 1;
                Ok(Some(record))
            }
            None => {
                self.missed = 1;
                Ok(None)
            }
        }
    }
}

/// Making the catalog anew from the books under the library folder: the
/// folders still to walk, and where in the current one's listing it is.
/// A book whose file is as large as the old catalog says is taken from
/// it, read along with the walk; others from their details, opening only
/// books not seen before. It holds a few kilobytes, whatever the
/// library's size, and may pause between steps (letting go of what it
/// read) and go on where it was. The kernel walks it as a job; views hear
/// [`crate::builtin::events::CATALOG_CHANGED`].
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
#[derive(Debug, Default)]
pub struct Walk {
    making: Making,
    old: OldCatalog,
    /// Books to take from their details whatever the old catalog says
    /// (their details changed).
    fresh: Vec<String>,
    dirs: Vec<String>,
    /// The folder being walked, its listing in [`listed_path`] (or
    /// `held`), and where the next entry of it is there.
    dir: Option<String>,
    at: u64,
    /// The listing of a folder listed in one piece: held, not written.
    held: Option<String>,
    /// While its folder is listed (a piece a step): where the next piece
    /// starts (see [`Storage::list_from`]).
    listed: Option<u64>,
}

#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
impl Walk {
    /// Starts anew with the books under the library folder, the books
    /// `fresh` taken from their details; an error where what it needs
    /// cannot be read (nothing is changed then).
    pub fn start(env: &Env<'_>, fresh: Vec<String>) -> Result<Walk, StorageError> {
        let old = OldCatalog::open(env.storage())?;
        let making = Making::start(env, true)?;

        Ok(Walk {
            making,
            old,
            fresh,
            dirs: Vec::from([library::folder(env).to_string()]),
            ..Walk::default()
        })
    }

    /// Books found so far.
    pub fn found(&self) -> usize {
        self.making.found
    }

    /// Books taken from the old catalog so far.
    pub fn from_old(&self) -> usize {
        self.old.found
    }

    /// One step: a piece of a folder listed, or a few of its entries, at
    /// most one book opened; `true` when the catalog is made. On an error
    /// nothing is changed: the old catalog stays.
    pub fn step(&mut self, env: &Env<'_>) -> Result<bool, StorageError> {
        let result = self.go_on(env);
        if result.is_err() {
            Making::abort(env.storage());
        }
        result
    }

    /// Lets go of what it read (it is read again when it goes on), as
    /// while the reader or the library takes the memory.
    pub fn pause(&mut self) {
        self.making.pause();
        self.old.let_go();
    }

    /// Gives up: the old catalog stays.
    pub fn abort(self, storage: &dyn Storage) {
        Making::abort(storage);
    }

    fn go_on(&mut self, env: &Env<'_>) -> Result<bool, StorageError> {
        let storage = env.storage();
        self.making.go_on(storage)?;
        if self.making.copy(storage)? {
            // What it read ahead is read again when the walk goes on.
            self.old.let_go();
            return Ok(false);
        }
        let Some(dir) = self.dir.clone() else {
            let Some(dir) = self.dirs.pop() else {
                if storage.exists(&listed_path()) {
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "the listing of a walk that is done: the next walk writes it anew"
                    )]
                    let _ = storage.remove(&listed_path());
                }
                return self.making.finish(storage);
            };
            self.dir = Some(dir);
            self.listed = Some(0);
            self.at = 0;
            self.held = None;
            return Ok(false);
        };
        if let Some(listed) = self.listed {
            self.listed = self.list(env, &dir, listed)?;
            return Ok(false);
        }
        let (entries, next) = self.entries(storage)?;
        if entries.is_empty() {
            self.dir = None;
            self.held = None;
            return Ok(false);
        }
        // The books' lines, written together.
        let mut piece = String::with_capacity(WRITTEN_PIECE);
        let (mut taken, mut books) = (0, 0);
        for entry in &entries {
            taken += 1;
            let path = files::join(&dir, &entry.name);
            if entry.is_dir {
                self.dirs.push(path);
                continue;
            }
            books += 1;
            if let Some(record) = self.as_old(storage, &path, entry.size)? {
                self.making.check_noted(&record);
                piece.push_str(&record.text());
                continue;
            }
            let (record, opened) = self.making.of_details(env, &path, entry.size);
            self.making.check_noted(&record);
            piece.push_str(&record.text());
            // Opening a book takes a moment: the rest in the next step.
            if opened {
                break;
            }
        }
        self.making.write_text(storage, piece, books)?;
        // Where the first entry not taken starts.
        self.at = entries.get(taken).map_or(next, |entry| entry.at);
        Ok(false)
    }

    /// The next entries of the folder's listing ([`STEP_LINES`] at most),
    /// from the listing held or kept, and where the one after starts.
    fn entries(&self, storage: &dyn Storage) -> Result<(Vec<Listed>, u64), StorageError> {
        let mut entries: Vec<Listed> = Vec::new();
        let mut take = |at: u64, line: &str| {
            let mut columns = line.splitn(3, '\t');
            let kind = columns.next().unwrap_or("");
            let size = columns.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            if let Some(name) = columns.next() {
                entries.push(Listed {
                    is_dir: kind == "d",
                    size,
                    name: name.to_string(),
                    at,
                });
            }
            entries.len() < STEP_LINES
        };
        let next = match &self.held {
            Some(held) => {
                let mut at = usize::try_from(self.at).unwrap_or(usize::MAX);
                while let Some(rest) = held.get(at..).filter(|rest| !rest.is_empty()) {
                    let end = rest.find('\n').map_or(rest.len(), |end| end + 1);
                    let go_on = take(at as u64, rest[..end].trim_end());
                    at += end;
                    if !go_on {
                        break;
                    }
                }
                at as u64
            }
            None => lines_from(storage, &listed_path(), self.at, &mut take)?,
        };
        Ok((entries, next))
    }

    /// Lists a piece of folder `dir` into the listing kept, the entries
    /// from place `from` on (see [`Storage::list_from`]): its folders and
    /// the files a format plugin opens, but hidden ones; where the next
    /// piece starts, `None` once all are. The piece is written after it
    /// was listed (never while it is); a folder listed whole in one piece
    /// is held instead (a few kilobytes, kept while the walk pauses). A
    /// library folder that is not there is empty; a folder in it gone
    /// meanwhile is left out.
    fn list(&mut self, env: &Env<'_>, dir: &str, from: u64) -> Result<Option<u64>, StorageError> {
        use core::fmt::Write as _;
        let storage = env.storage();
        let mut piece = String::with_capacity(LISTED_PIECE);
        let result = storage.list_from(dir, from, &mut |entry| {
            if !library::listing::shown(env.formats(), entry.name, entry.is_dir) {
                return true;
            }
            if !piece.is_empty() && piece.len() + entry.name.len() + LISTED_LINE > LISTED_PIECE {
                return false;
            }
            let kind = if entry.is_dir { 'd' } else { 'f' };
            #[expect(
                clippy::let_underscore_must_use,
                reason = "writing into a String does not fail"
            )]
            let _ = writeln!(piece, "{kind}\t{}\t{}", entry.size, entry.name);
            true
        });
        let next = match result {
            Ok(next) => next,
            Err(StorageError::NotFound) => {
                self.held = Some(String::new());
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        if from == 0 && next.is_none() {
            // Listed whole in one piece: held (no more than it needs),
            // never written.
            piece.shrink_to_fit();
            self.held = Some(piece);
            return Ok(None);
        }
        if from == 0 {
            storage.write(&listed_path(), b"")?;
        }
        storage.append(&listed_path(), piece.as_bytes())?;
        Ok(next)
    }

    /// The book at `path` as the old catalog has it, where its file is as
    /// large as then and its details did not change since.
    fn as_old(
        &mut self,
        storage: &dyn Storage,
        path: &str,
        size: u64,
    ) -> Result<Option<Record>, StorageError> {
        if self.fresh.iter().any(|p| p == path) {
            return Ok(None);
        }
        let Some(record) = self.old.find(storage, path)? else {
            return Ok(None);
        };
        let hash = fnv1a64(path);
        // Changed since, or removed and come again: from its details.
        if record.size != Some(size) || self.making.changes.removed(hash) {
            return Ok(None);
        }
        // How far it is read: noted since, or for the books read last
        // (read further than noted, where a reading ended without its book
        // closed) as their details say.
        let noted = self
            .making
            .changes
            .status(hash)
            .map_or(record.status.progress, |s| s.progress);
        let progress = if self.making.opened.binary_search(&(hash as u32)).is_ok() {
            library::book_sized(storage, path, size).map_or(noted, |book| book.progress)
        } else {
            noted
        };
        let status = self.making.status(path, progress);
        Ok(Some(record.with_status(status)))
    }
}

/// An entry of a folder's listing, and where its line starts.
#[derive(Debug)]
struct Listed {
    is_dir: bool,
    size: u64,
    name: String,
    at: u64,
}

/// Catching the catalog up without walking the library: what was noted
/// beside it folded in, the books removed left out, and the books that
/// came or changed (`fresh`) taken from their details. The kernel does it
/// as a job, after books came or moved, or once the log is long. Its
/// lines are copied as they are, a piece at a time; it may pause between
/// steps as a walk does.
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
#[derive(Debug, Default)]
pub struct Rewrite {
    making: Making,
    fresh: Vec<String>,
    /// Where the next line of the old catalog starts; `None` once it is
    /// copied.
    at: Option<u64>,
}

#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
impl Rewrite {
    /// Starts catching up, the books `fresh` taken from their details; an
    /// error where what it needs cannot be read.
    pub fn start(env: &Env<'_>, mut fresh: Vec<String>) -> Result<Rewrite, StorageError> {
        fresh.sort_unstable();
        fresh.dedup();
        Ok(Rewrite {
            making: Making::start(env, false)?,
            fresh,
            at: Some(0),
        })
    }

    /// One step: a piece of the old catalog copied, or one book taken
    /// from its details; `true` when the catalog is made. On an error
    /// nothing is changed.
    pub fn step(&mut self, env: &Env<'_>) -> Result<bool, StorageError> {
        let result = self.go_on(env);
        if result.is_err() {
            Making::abort(env.storage());
        }
        result
    }

    /// Lets go of what it read (it is read again when it goes on).
    pub fn pause(&mut self) {
        self.making.pause();
    }

    /// Gives up: the old catalog stays.
    pub fn abort(self, storage: &dyn Storage) {
        Making::abort(storage);
    }

    fn go_on(&mut self, env: &Env<'_>) -> Result<bool, StorageError> {
        let storage = env.storage();
        self.making.go_on(storage)?;
        if let Some(at) = self.at {
            let mut piece = String::with_capacity(WRITTEN_PIECE);
            let (mut books, mut stopped) = (0, None);
            let (fresh, changes) = (&self.fresh, &self.making.changes);
            lines_from(storage, &catalog_path(), at, &mut |offset, line| {
                let path = path_of(line);
                if path.is_empty() || fresh.binary_search_by(|p| p.as_str().cmp(path)).is_ok() {
                    return true;
                }
                let changed = match changes.get(fnv1a64(path)) {
                    None => None,
                    // Removed.
                    Some(None) => return true,
                    Some(Some(status)) => match Record::parse(line) {
                        Some(record) => Some(record.with_status(status).text()),
                        None => return true,
                    },
                };
                let text = changed.as_deref().unwrap_or(line);
                // A line that does not fit begins the next piece (the
                // piece never grows past what it was given).
                if !piece.is_empty() && piece.len() + text.len() + 1 > WRITTEN_PIECE {
                    stopped = Some(offset);
                    return false;
                }
                piece.push_str(text.trim_end_matches('\n'));
                piece.push('\n');
                books += 1;
                true
            })?;
            self.making.write_text(storage, piece, books)?;
            self.at = stopped;
            return Ok(false);
        }
        let Some(path) = self.fresh.pop() else {
            return self.making.finish(storage);
        };
        match storage.size(&path) {
            Ok(size) => {
                let (record, _) = self.making.of_details(env, &path, size);
                self.making.write_text(storage, record.text(), 1)?;
            }
            // Gone again meanwhile.
            Err(StorageError::NotFound) => {}
            Err(e) => return Err(e),
        }
        Ok(false)
    }
}
