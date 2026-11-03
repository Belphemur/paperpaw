//! Brings CrossPoint's reading data over (see [`xcpt`] for the files
//! themselves): what is on the card is read, matched to the books there,
//! and brought into the statistics — the minutes read per day into the
//! days, the books into the books with where they were left and when
//! they were finished — and what the pace is seeded with. Nothing of
//! CrossPoint's is changed or removed, what was brought over is marked
//! as done so that running this again brings nothing twice, and a file
//! of a newer CrossPoint than this knows is left alone whole.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use paperpaw_sdk::digest::{fnv1a64, md5};
use paperpaw_sdk::document::Locator;
use paperpaw_sdk::json;
use paperpaw_sdk::{Arg, Env, Storage, positions};

use crate::pace::{self, SharedPace};

use crate::store::{self, Book};
use crate::xcpt::{self, Kind};

/// Where CrossPoint kept its data, at the root of the card.
const DIR: &str = ".crosspoint";
/// What was brought over already, kept in the plugin's folder.
const MARKER: &str = "xerif.ini";
/// How deep into folders the card is walked for books: the card itself
/// and five levels of folders below it, as CrossPoint walked it for its
/// library.
const DEPTH: u8 = 6;

/// What one run brought over.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Report {
    /// Books added to the books.
    pub books: usize,
    /// Books marked as finished.
    pub finished: usize,
    /// Days the minutes read were brought into.
    pub days: usize,
    /// Where a book was left, kept.
    pub positions: usize,
    /// Files already brought over (and so left as they were).
    pub skipped: usize,
    /// Files of a newer CrossPoint than this knows: left alone.
    pub forward: usize,
    /// Book folders that match no book on the card: not brought over.
    pub unmatched: usize,
    /// The books' records, to seed the pace with, by the book's path as
    /// the card names it.
    pub seeded: Vec<(String, xcpt::Book)>,
    /// The global record, to seed the pace with.
    pub global: Option<xcpt::Global>,
}

/// Why there was nothing to bring over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum No {
    /// CrossPoint kept no data on this card.
    None,
    /// There is data, but the books cannot be written to take it.
    Unwritable,
}

/// The path of a file of CrossPoint's.
fn at(name: &str) -> String {
    alloc::format!("{DIR}/{name}")
}

/// Reads a file whole, if it is there and not too large.
fn read(storage: &dyn Storage, path: &str, limit: usize) -> Option<Vec<u8>> {
    paperpaw_sdk::read_file(storage, path, limit).ok()
}

/// The books' paths on the card, as the card names them (`/books/a.epub`),
/// best first: those CrossPoint itself remembered, then those its library
/// index knows, then everything the card holds that is a book.
fn book_paths(storage: &dyn Storage) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    let mut push = |path: String| {
        if Kind::of(&path).is_some() && !paths.contains(&path) {
            paths.push(path);
        }
    };
    for (path, _, _) in recents(storage) {
        push(path);
    }
    for (path, _, _) in library_index(storage) {
        push(path);
    }
    walk(storage, "", DEPTH, &mut |path| push(path));
    paths
}

/// The books CrossPoint remembered last reading (`recent.json`): their
/// paths, titles and authors.
fn recents(storage: &dyn Storage) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut path = String::new();
    let mut title = String::new();
    let mut author = String::new();
    json::stream::read_file(storage, &at("recent.json"), &mut |event| match event {
        json::stream::Event::Value(keys, value) if keys.len() == 3 && keys[0] == "books" => {
            match keys[2].as_str() {
                "path" => path = value.to_string(),
                "title" => title = value.to_string(),
                "author" => author = value.to_string(),
                _ => {}
            }
        }
        json::stream::Event::Close(keys)
            if keys.len() == 2 && keys[0] == "books" && !path.is_empty() =>
        {
            out.push((
                core::mem::take(&mut path),
                core::mem::take(&mut title),
                core::mem::take(&mut author),
            ));
        }
        _ => {}
    });
    out
}

/// The books of CrossPoint's library index (`library.idx`), whole paths
/// with their titles and authors. Its own format, told by the size: a
/// header, its folders, a record a book, and, at the end, the names.
/// Read in place, a little at a time, for an index of a full card is
/// more than a device holds at once.
#[expect(
    clippy::too_many_lines,
    reason = "one reading of the index, its offsets beside the bytes they are of; splitting it would pass them all through three functions' hands"
)]
fn library_index(storage: &dyn Storage) -> Vec<(String, String, String)> {
    const MAGIC: &[u8; 4] = b"CLX1";
    const VERSION: u8 = 2;
    const RECORD_LEN: u64 = 128;
    const MOST_BOOKS: u64 = 4096;
    const FOLDERS_ROOM: usize = 32 * 1024;
    let path = at("library.idx");
    let size = match storage.size(&path) {
        Ok(size) => size,
        Err(_) => return Vec::new(),
    };
    let mut head = [0u8; 64];
    if storage.read_at(&path, 0, &mut head).ok() != Some(64) {
        return Vec::new();
    }
    if head[0..4] != *MAGIC || head[4] != VERSION {
        return Vec::new();
    }
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes([b[at], b[at + 1]]);
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let books = u64::from(u16_at(&head, 8));
    // The size the header says the file has: a build cut short does not
    // pass, and nor does a count of books the format cannot hold.
    if books > MOST_BOOKS || u32_at(&head, 40) != size as u32 {
        return Vec::new();
    }
    let (folder_start, record_start, name_start) = (
        u64::from(u32_at(&head, 16)),
        u64::from(u32_at(&head, 24)),
        u64::from(u32_at(&head, 32)),
    );
    // The sections in their places, each within the file: anything else
    // is no index this reads.
    let Some(perm_end) = perm_end(record_start, books, RECORD_LEN) else {
        return Vec::new();
    };
    if !(512..size).contains(&folder_start)
        || record_start <= folder_start
        || record_start > size
        || name_start < perm_end
        || name_start > size
    {
        return Vec::new();
    }
    // The folders, in the order of their ids: a path a folder. Only so
    // much of them is read as fits a bound; more folders than that and
    // the paths come from the card alone.
    let mut folders: Vec<String> = Vec::new();
    let mut piece = [0u8; 256];
    let mut at = folder_start;
    let room = name_start.min(folder_start + FOLDERS_ROOM as u64);
    while at < room {
        if storage.read_at(&path, at, &mut piece[..1]).ok() != Some(1) {
            break;
        }
        let len = usize::from(piece[0]);
        if len == 0 || at + 1 + len as u64 > room {
            break;
        }
        if storage.read_at(&path, at + 1, &mut piece[..len]).ok() != Some(len) {
            break;
        }
        folders.push(String::from_utf8_lossy(&piece[..len]).into_owned());
        at += 1 + len as u64;
    }
    if folders.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut record = [0u8; RECORD_LEN as usize];
    for book in 0..books {
        let at = record_start + book * RECORD_LEN;
        if storage.read_at(&path, at, &mut record).ok() != Some(record.len()) {
            break;
        }
        let (name_at, folder) = (
            u64::from(u32_at(&record, 0)),
            usize::from(u16_at(&record, 10)),
        );
        let name_len = usize::from(record[12]);
        // The names sit where the record says, the hash of the whole
        // path before the file's name; past the file is no name.
        let Some(name_at) = name_start
            .checked_add(name_at)
            .and_then(|at| at.checked_add(8))
        else {
            continue;
        };
        if name_at + name_len as u64 > size {
            continue;
        }
        let mut name = vec![0u8; name_len];
        if storage.read_at(&path, name_at, &mut name).ok() != Some(name_len) {
            continue;
        }
        // And after it the author, the title and the author as the book
        // itself spells it, each as their length (a byte) and the text;
        // read where they fit, and left out where they do not.
        let mut at = name_at + name_len as u64;
        let read_text = |storage: &dyn Storage, at: &mut u64| -> String {
            let mut len = [0u8; 1];
            if storage.read_at(&path, *at, &mut len).ok() != Some(1) {
                *at = size;
                return String::new();
            }
            let len = usize::from(len[0]);
            let text = if len == 0 || *at + 1 + len as u64 > size {
                Vec::new()
            } else {
                let mut text = vec![0u8; len];
                let read = storage.read_at(&path, *at + 1, &mut text).ok();
                if read != Some(len) {
                    *at = size;
                    return String::new();
                }
                text
            };
            *at += 1 + len as u64;
            String::from_utf8_lossy(&text).into_owned()
        };
        let author = read_text(storage, &mut at);
        let title = read_text(storage, &mut at);
        let Some(folder) = folders.get(folder) else {
            continue;
        };
        let path = if name.is_empty() {
            String::new()
        } else if folder.ends_with('/') {
            alloc::format!("{folder}{}", String::from_utf8_lossy(&name))
        } else {
            alloc::format!("{folder}/{}", String::from_utf8_lossy(&name))
        };
        if !path.is_empty() {
            out.push((path, title, author));
        }
    }
    out
}

/// Where the names begin, after the records and the two orders of them
/// (an order of a number a book, two bytes each).
fn perm_end(record_start: u64, books: u64, record_len: u64) -> Option<u64> {
    record_start
        .checked_add(books.checked_mul(record_len)?)
        .and_then(|at| at.checked_add(books.checked_mul(4)?))
}

/// Walks the card for books, at most `depth` folders deep, leaving
/// hidden folders out (CrossPoint's own data among them); calls `visit`
/// with each book's path as the card names it.
fn walk(storage: &dyn Storage, dir: &str, depth: u8, visit: &mut impl FnMut(String)) {
    if depth == 0 {
        return;
    }
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a folder that cannot be listed holds no books this finds elsewhere"
    )]
    let _ = storage.list(dir, &mut |entry| {
        if entry.name == "." || entry.name == ".." || entry.name.starts_with('.') {
            return;
        }
        // The card names its paths with a slash before them; the
        // folders are walked as the storage names them, without.
        let card = if dir.is_empty() {
            alloc::format!("/{}", entry.name)
        } else {
            alloc::format!("/{}/{}", dir, entry.name)
        };
        if entry.is_dir {
            let storage_path = card.trim_start_matches('/').to_string();
            walk(storage, &storage_path, depth - 1, visit);
        } else if Kind::of(&card).is_some() {
            visit(card);
        }
    });
}

/// How much of a file was brought over already: its size and a hash of
/// what is in it, as text.
fn fingerprint(data: &[u8]) -> String {
    alloc::format!("{}:{:016x}", data.len(), fnv1a64(data))
}

/// What was brought over, by name. `Err` where the marks are there but
/// cannot be read: they are not to be written over blind, for they say
/// what was done already.
fn marker(storage: &dyn Storage) -> Result<BTreeMap<String, String>, ()> {
    let path = alloc::format!(
        "{}/{}",
        paperpaw_sdk::Env::plugin_dir(crate::PLUGIN.as_str()),
        MARKER
    );
    let data = match storage.size(&path) {
        Ok(size) if size > 64 * 1024 => return Err(()),
        Ok(_) => {
            let data = paperpaw_sdk::read_file(storage, &path, 64 * 1024).map_err(|_| ())?;
            if data.is_empty() {
                return Err(());
            }
            data
        }
        Err(_) => return Ok(BTreeMap::new()),
    };
    let text = core::str::from_utf8(&data).map_err(|_| ())?;
    let mut out = BTreeMap::new();
    for entry in paperpaw_sdk::config::parse(text) {
        let Ok(entry) = entry else { return Err(()) };
        if entry.section == "imported" {
            out.insert(entry.key.to_string(), entry.value.as_str().into_owned());
        }
    }
    Ok(out)
}

/// One of CrossPoint's book folders: where it is, which kind of book it
/// holds, and the hash its name is of.
type Folder = (String, Kind, u32);

/// Whether CrossPoint kept data on this card, to be brought over.
pub(crate) fn present(storage: &dyn Storage) -> bool {
    storage.exists(&at("global_stats.bin"))
        || storage.exists(&at("finished_books.bin"))
        || storage.exists(&at("recent.json"))
        || has_book_folders(storage)
}

/// Brings CrossPoint's data over, and says what came of it: what the
/// import brought for the pace is put into it here, where the books it
/// belongs to are known.
pub(crate) fn bring(env: &Env<'_>, pace: &SharedPace) -> String {
    let report = match import(env.storage(), |unix| crate::fmt::day_of(env, unix)) {
        Ok(report) => report,
        Err(No::None) => return env.i18n().tr("stats.xerif.none").to_string(),
        Err(No::Unwritable) => return env.i18n().tr("stats.xerif.unwritable").to_string(),
    };
    let books = crate::store::books(env.storage());
    let of = |record: &xcpt::Book| pace::FromRecord {
        wpm: record.wpm,
        session: record.session,
        seconds: record.seconds,
        sessions: record.sessions,
    };
    let seeded: Vec<(String, pace::FromRecord)> = report
        .seeded
        .iter()
        .map(|(path, record)| (path.clone(), of(record)))
        .collect();
    let global = report.global.as_ref().map(|global| pace::FromRecord {
        wpm: global.wpm,
        session: global.session,
        seconds: global.seconds,
        sessions: global.sessions,
    });
    pace.borrow_mut().seed(&seeded, global.as_ref(), |path| {
        let path = path.trim_start_matches('/');
        books.iter().find(|b| b.path == path).map(|b| b.id)
    });
    message(env, &report)
}

/// What came of it, in words.
fn message(env: &Env<'_>, report: &Report) -> String {
    let i18n = env.i18n();
    let mut text = i18n.format(
        "stats.xerif.done",
        &[
            (
                "books",
                Arg::Str(&i18n.plural(
                    "stats.books-count",
                    i64::try_from(report.books).unwrap_or(i64::MAX),
                    &[],
                )),
            ),
            (
                "days",
                Arg::Str(&i18n.plural(
                    "stats.reading-days",
                    i64::try_from(report.days).unwrap_or(i64::MAX),
                    &[],
                )),
            ),
            (
                "positions",
                Arg::Str(&i18n.plural(
                    "stats.positions",
                    i64::try_from(report.positions).unwrap_or(i64::MAX),
                    &[],
                )),
            ),
        ],
    );
    if report.skipped > 0 {
        text.push(' ');
        text.push_str(&i18n.plural(
            "stats.xerif.skipped",
            i64::try_from(report.skipped).unwrap_or(i64::MAX),
            &[],
        ));
    }
    if report.forward > 0 {
        text.push(' ');
        text.push_str(&i18n.plural(
            "stats.xerif.forward",
            i64::try_from(report.forward).unwrap_or(i64::MAX),
            &[],
        ));
    }
    if report.unmatched > 0 {
        text.push(' ');
        text.push_str(&i18n.plural(
            "stats.xerif.unmatched",
            i64::try_from(report.unmatched).unwrap_or(i64::MAX),
            &[],
        ));
    }
    text
}

/// The folders of CrossPoint's books on the card, in order of their
/// names: where each is, which kind of book it holds and the hash its
/// name is of.
fn folders(storage: &dyn Storage) -> Vec<Folder> {
    let mut folders: Vec<Folder> = Vec::new();
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a folder of CrossPoint's that cannot be listed holds nothing to bring over"
    )]
    let _ = storage.list(DIR, &mut |entry| {
        if !entry.is_dir {
            return;
        }
        let Some((kind, hash)) = entry.name.split_once('_').and_then(|(kind, name)| {
            let kind = match kind {
                "epub" => Kind::Epub,
                "xtc" => Kind::Xtc,
                _ => return None,
            };
            // The folders are named as `std::to_string` writes the hash.
            name.parse::<u32>().ok().map(|hash| (kind, hash))
        }) else {
            return;
        };
        folders.push((alloc::format!("{DIR}/{}", entry.name), kind, hash));
    });
    folders.sort_by(|a, b| a.0.cmp(&b.0));
    folders
}

/// What the folders brought: the books to write, and per folder its
/// name, the version of the record read, and where the book was left
/// (so that the mark waits for the position to be kept).
type Kept = Vec<(String, String, Option<(String, Locator, f32)>)>;

type Brought = (Vec<Book>, Kept);

/// The books of CrossPoint's folders: which records can be read and
/// which books they belong to (by the paths hashed again), what was
/// brought over already and what is to be marked as brought over.
fn bring_folders(
    storage: &dyn Storage,
    folders: &[Folder],
    by_hash: &BTreeMap<u32, String>,
    done: &BTreeMap<String, String>,
    report: &mut Report,
) -> Brought {
    let mut changed_books = Vec::new();
    let mut to_mark = Vec::new();
    for (dir, kind, hash) in folders {
        // The record of the book's reading: the newest this knows, or, as
        // a guard, the one of a newer CrossPoint, which is left alone.
        let (record, forward) = book_record(storage, dir);
        let Some((record, version)) = record else {
            if forward {
                report.forward += 1;
            }
            continue;
        };
        let Some(path) = by_hash.get(hash).cloned() else {
            // No book on the card is hashed to this folder: whose reading
            // it is cannot be told, so none of it is brought over.
            report.unmatched += 1;
            continue;
        };
        let path_storage = path.trim_start_matches('/').to_string();
        let name = dir.rsplit('/').next().unwrap_or_default();
        // The pace is seeded from every record that can be read, whether
        // it was brought over before or not.
        report.seeded.push((path.clone(), record.clone()));
        if done.get(name).map(String::as_str) == Some(version.to_string().as_str()) {
            report.skipped += 1;
            continue;
        }
        let mut left: Option<(String, Locator, f32)> = None;
        if !bring_book(
            storage,
            &mut changed_books,
            *kind,
            &path,
            &record,
            &mut |at, percent| {
                left = Some((path_storage.clone(), at, percent));
            },
        ) {
            // The book's file cannot be read: it cannot be told apart
            // from another, so nothing of it is brought over, and the
            // folder is not marked, for the next run tries again.
            continue;
        }
        // The folder is marked only once where the book was left has
        // been kept, or was there already: a failed write is tried
        // again.
        to_mark.push((String::from(name), version.to_string(), left));
    }
    (changed_books, to_mark)
}

/// The finished books of CrossPoint's index, marked in `changed_books`:
/// the index names each book by its path hashed with FNV-1a, and the
/// book is the one it is by its fingerprint — where that can be read,
/// which also reaches the books brought over before (a minimal book to
/// merge changes nothing but the finishing). `Ok(None)` where there is
/// nothing to mark; the index is marked only once every entry it names
/// was matched, so that a book whose file was not found on the card is
/// marked when it is there.
fn bring_finished_books(
    storage: &dyn Storage,
    paths: &[String],
    done: &BTreeMap<String, String>,
    changed_books: &mut Vec<Book>,
    report: &mut Report,
) -> Result<Option<(String, String)>, No> {
    // The index itself, and where it cannot be read as one, the copy
    // beside it.
    let (data, decoded) = match read(storage, &at("finished_books.bin"), 64 * 1024) {
        Some(data) => match xcpt::decode_finished(&data) {
            xcpt::FinishedRecord::Invalid => match backup_finished(storage) {
                Some(backup) => backup,
                None => (data, xcpt::FinishedRecord::Invalid),
            },
            other => (data, other),
        },
        None => match backup_finished(storage) {
            Some(backup) => backup,
            None => return Ok(None),
        },
    };
    let xcpt::FinishedRecord::Ok(entries) = decoded else {
        if matches!(decoded, xcpt::FinishedRecord::Forward) {
            report.forward += 1;
        }
        return Ok(None);
    };
    let already = done.get("finished").map(String::as_str) == Some(fingerprint(&data).as_str());
    if already {
        report.skipped += 1;
        return Ok(None);
    }
    let mut all_matched = true;
    for entry in entries {
        // The index names the book by its path hashed with FNV-1a, of
        // the path as the card names it.
        let Some(path) = paths
            .iter()
            .find(|p| fnv1a64(p.as_bytes()) == entry.path_key)
        else {
            all_matched = false;
            continue;
        };
        let finished = entry.finished.unix_noon();
        if finished <= 0 {
            continue;
        }
        // The book's fingerprint, by which it is known; where its file
        // cannot be read, the finishing waits for the next run.
        let path_storage = path.trim_start_matches('/');
        let Some(md5) = md5::partial(storage, path_storage) else {
            all_matched = false;
            continue;
        };
        changed_books.push(Book {
            md5,
            path: String::from(path_storage),
            finished,
            ..Book::default()
        });
        report.finished += 1;
    }
    if all_matched {
        Ok(Some((String::from("finished"), fingerprint(&data))))
    } else {
        Ok(None)
    }
}

/// The books to add or change, written in one pass over the books file:
/// a book is the one it is by its fingerprint, else by where it is
/// here; a new one gets an id above every book's and every stay's, so
/// that it never takes over the stays of one whose entry was lost.
fn write_books(
    storage: &dyn Storage,
    changed_books: Vec<Book>,
    report: &mut Report,
) -> Result<(), No> {
    if changed_books.is_empty() {
        return Ok(());
    }
    // A book that gets no id (the log could not be read, or the ids ran
    // out) is not brought over: the run says so, and does not mark what
    // it did not do.
    let mut unwritten: Vec<Book> = Vec::new();
    let written = store::update_books(storage, |books| {
        let mut changed = false;
        for book in changed_books {
            let there = match books
                .iter_mut()
                .find(|b| !book.md5.is_empty() && b.md5 == book.md5)
            {
                Some(there) => Some(there),
                None if !book.path.is_empty() => books.iter_mut().find(|b| b.path == book.path),
                None => None,
            };
            match there {
                Some(there) => {
                    if (there.title.is_empty() || there.title == "N/A") && !book.title.is_empty() {
                        there.title = book.title.clone();
                    }
                    if (there.authors.is_empty() || there.authors == "N/A")
                        && !book.authors.is_empty()
                    {
                        there.authors = book.authors.clone();
                    }
                    if there.finished <= 0 && book.finished > 0 {
                        there.finished = book.finished;
                    }
                    if there.pages == 0 && book.pages > 0 {
                        there.pages = book.pages;
                    }
                    if there.path.is_empty() {
                        there.path = book.path.clone();
                    }
                    changed = true;
                }
                None => match store::new_id(storage, books) {
                    Ok(id) => {
                        let mut book = book;
                        book.id = id;
                        books.push(book);
                        changed = true;
                        report.books += 1;
                    }
                    Err(_) => unwritten.push(book),
                },
            }
        }
        changed
    });
    // Where the books file cannot be read or written, nothing is, and
    // what was brought over is brought again the next time.
    written.map(|_| ()).map_err(|_| No::Unwritable)?;
    (unwritten.is_empty()).then_some(()).ok_or(No::Unwritable)
}

/// Where the books were left, kept, each only where none was kept
/// before (a position that is there but cannot be read is not written
/// over either). The marks of the folders whose position was kept come
/// back: one whose position could not be kept is left for the next run.
fn keep_positions(
    storage: &dyn Storage,
    folders: Kept,
    report: &mut Report,
) -> Vec<(String, String)> {
    let mut marks = Vec::new();
    for (name, version, left) in folders {
        let Some((path, at, percent)) = left else {
            // The record said nothing of where the book was left: the
            // folder is done all the same.
            marks.push((name, version));
            continue;
        };
        let path = path.trim_start_matches('/');
        match positions::load_for_update(storage, path) {
            Ok(None) => {}
            // A position kept already stays as it is; one that is there
            // but cannot be read is not written over either.
            _ => {
                marks.push((name, version));
                continue;
            }
        }
        let Some(md5) = md5::partial(storage, path) else {
            continue;
        };
        let size = storage.size(path).unwrap_or(0);
        let fingerprint =
            paperpaw_sdk::library::Fingerprint::parse(&alloc::format!("{size} {md5}"));
        let position = positions::Position::new(at, percent);
        if positions::save_with_book(storage, path, &position, fingerprint.as_ref()).is_ok() {
            report.positions += 1;
            marks.push((name, version));
        }
    }
    marks
}

/// The copy of the index of finished books beside it, where it can be
/// read as one: the bytes and what they say.
fn backup_finished(storage: &dyn Storage) -> Option<(Vec<u8>, xcpt::FinishedRecord)> {
    let data = read(storage, &at("finished_books.bin.bak"), 64 * 1024)?;
    let decoded = xcpt::decode_finished(&data);
    matches!(
        decoded,
        xcpt::FinishedRecord::Ok(_) | xcpt::FinishedRecord::Forward
    )
    .then_some((data, decoded))
}

/// The minutes read per day, from the global record, brought into the
/// days of the statistics. The days are brought over even where they
/// were before (the days of the log are written anew from the log and
/// the minutes together, so a log written anew, a merge with KOReader
/// say, costs nothing that another run does not bring back); what the
/// mark gates is only what is said of the run. `Ok(None)` where there
/// is nothing to mark.
fn bring_minutes(
    storage: &dyn Storage,
    day_of: impl Fn(u32) -> i32,
    done: &BTreeMap<String, String>,
    report: &mut Report,
) -> Result<Option<(String, String)>, No> {
    // The record itself, and where it cannot be read as one, the copy
    // beside it (a cut while CrossPoint wrote it is not the end of the
    // data). One of a newer CrossPoint is left alone, never replaced by
    // an older copy.
    let (data, source) = match read(storage, &at("global_stats.bin"), 4096) {
        Some(data) => match xcpt::decode_global(&data) {
            xcpt::GlobalRecord::Invalid => match backup_global(storage) {
                Some(backup) => backup,
                None => (data, xcpt::GlobalRecord::Invalid),
            },
            other => (data, other),
        },
        None => match backup_global(storage) {
            Some(backup) => backup,
            None => return Ok(None),
        },
    };
    let xcpt::GlobalRecord::Ok(global) = source else {
        if matches!(source, xcpt::GlobalRecord::Forward) {
            report.forward += 1;
        }
        return Ok(None);
    };
    report.global = Some(global.clone());
    let anchor = global.anchor;
    let minutes = global.minutes_per_day(current_version(&data));
    let mut days: Vec<(u32, u32)> = Vec::new();
    for (older, minutes) in minutes.iter().enumerate() {
        if *minutes == 0 {
            continue;
        }
        // The first minutes are of the day the history begins, then
        // ever older days follow, as the card counted the days.
        let unix_day = anchor.saturating_sub(older as u32);
        match days.binary_search_by_key(&unix_day, |(d, _)| *d) {
            Ok(i) => days[i].1 = days[i].1.saturating_add(u32::from(*minutes) * 60),
            Err(i) => days.insert(i, (unix_day, u32::from(*minutes) * 60)),
        }
    }
    store::import_days(storage, day_of, &days).map_err(|_| No::Unwritable)?;
    if done.get("global").map(String::as_str) == Some(fingerprint(&data).as_str()) {
        Ok(None)
    } else {
        report.days = days.len();
        Ok(Some((String::from("global"), fingerprint(&data))))
    }
}

/// The copy of the global record beside it, where it can be read as
/// one: the bytes and what they say.
fn backup_global(storage: &dyn Storage) -> Option<(Vec<u8>, xcpt::GlobalRecord)> {
    let data = read(storage, &at("global_stats.bin.bak"), 4096)?;
    let decoded = xcpt::decode_global(&data);
    matches!(
        decoded,
        xcpt::GlobalRecord::Ok(_) | xcpt::GlobalRecord::Forward
    )
    .then_some((data, decoded))
}

/// Brings CrossPoint's data over. `day_of` turns a moment in Unix
/// seconds into the local day it was read on (the days of the statistics
/// are local days).
pub(crate) fn import(storage: &dyn Storage, day_of: impl Fn(u32) -> i32) -> Result<Report, No> {
    if !present(storage) {
        return Err(No::None);
    }
    let mut report = Report::default();
    let paths = book_paths(storage);
    // Which folder of CrossPoint's is which book: the folder is named by
    // the path hashed, so the paths of the books are hashed again.
    let mut by_hash: BTreeMap<u32, String> = BTreeMap::new();
    for path in &paths {
        by_hash
            .entry(xcpt::std_hash(path))
            .or_insert_with(|| path.clone());
    }
    // What was brought over before, by name. Where it cannot be read,
    // nothing is brought over and nothing is written: the marks say
    // what was done, and overwriting them would make the next run
    // bring the same over again without knowing it.
    let done = marker(storage).map_err(|_| No::Unwritable)?;
    let mut to_mark: Vec<(String, String)> = Vec::new();
    let (mut changed_books, brought) =
        bring_folders(storage, &folders(storage), &by_hash, &done, &mut report);
    // Where the books were left, and the folders whose position was
    // kept (or was there already) marked only then.
    to_mark.extend(keep_positions(storage, brought, &mut report));
    if let Some(mark) =
        bring_finished_books(storage, &paths, &done, &mut changed_books, &mut report)?
    {
        to_mark.push(mark);
    }
    write_books(storage, changed_books, &mut report)?;
    if let Some(mark) = bring_minutes(storage, day_of, &done, &mut report)? {
        to_mark.push(mark);
    }
    // What was brought over, marked so that it is not brought twice.
    let mut done = done;
    for (name, value) in to_mark {
        done.insert(name, value);
    }
    write_marker(storage, &done);
    Ok(report)
}

/// Whether CrossPoint's book folders are on the card.
fn has_book_folders(storage: &dyn Storage) -> bool {
    let mut any = false;
    #[expect(
        clippy::let_underscore_must_use,
        reason = "without a listing there is nothing to bring over"
    )]
    let _ = storage.list(DIR, &mut |entry| {
        any |= entry.is_dir && (entry.name.starts_with("epub_") || entry.name.starts_with("xtc_"));
    });
    any
}

/// The record of a book's reading in its folder: the newest this knows,
/// with the version it is of; and whether a newer CrossPoint's record is
/// there (which is left alone, never read).
fn book_record(storage: &dyn Storage, dir: &str) -> (Option<(xcpt::Book, u8)>, bool) {
    // The record of a newer CrossPoint is recognized only as a guard;
    // where it is there, nothing of the folder is brought over.
    if storage.exists(&alloc::format!("{dir}/stats_v9.bin")) {
        return (None, true);
    }
    // The `.tmp` of the newest is a record a failed publish left whole,
    // and as good as the file itself.
    for (version, name) in [
        (xcpt::BOOK_V8, "stats_v8.bin"),
        (xcpt::BOOK_V8, "stats_v8.bin.tmp"),
        (xcpt::BOOK_V7, "stats_v7.bin"),
        (xcpt::BOOK_V6, "stats_v6.bin"),
        (xcpt::BOOK_V5, "stats_v5.bin"),
    ] {
        let Some(data) = read(storage, &alloc::format!("{dir}/{name}"), 4096) else {
            continue;
        };
        match xcpt::decode_book(&data) {
            xcpt::BookRecord::Ok(book) => return (Some((book, version)), false),
            xcpt::BookRecord::Forward => return (None, true),
            xcpt::BookRecord::Invalid => {}
        }
    }
    (None, false)
}

/// Which version of the global record a file is of.
fn current_version(data: &[u8]) -> u8 {
    data.first().copied().unwrap_or(xcpt::GLOBAL_V6)
}

/// Adds one book to the books to write: its title and authors as
/// CrossPoint knew them, where it is on the card, and when it was
/// finished; and says where the book was left, if its record says so and
/// it can be said how far in it that is.
fn bring_book(
    storage: &dyn Storage,
    books: &mut Vec<Book>,
    kind: Kind,
    path: &str,
    record: &xcpt::Book,
    left_at: &mut impl FnMut(Locator, f32),
) -> bool {
    // The book's own words of itself, from its folder's cache.
    let dir = xcpt::cache_dir(kind, path);
    let meta = read(storage, &alloc::format!("{DIR}/{dir}/book.bin"), 4096)
        .and_then(|data| xcpt::decode_book_meta(&data));
    let (title, author) = meta.as_ref().map_or_else(
        || (String::new(), String::new()),
        |m| (m.title.clone(), m.author.clone()),
    );
    let path_storage = path.trim_start_matches('/').to_string();
    // Which book it is, by its content; where the file cannot be read,
    // it cannot be told apart from another, so nothing of it is brought
    // over, and the folder is left for the next run.
    let Some(md5) = md5::partial(storage, &path_storage) else {
        return false;
    };
    let file = path_storage.rsplit('/').next().unwrap_or(&path_storage);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    books.push(Book {
        title: if title.is_empty() {
            String::from(stem)
        } else {
            title
        },
        authors: author,
        md5,
        pages: 0,
        language: meta
            .as_ref()
            .map_or_else(String::new, |m| m.language.clone()),
        series: String::from("N/A"),
        last_open: 0,
        path: path_storage.clone(),
        finished: record.finished.unix_noon(),
        id: 0,
    });
    // Where the book was left, if its record says so and it can be said
    // how far in it that is.
    let Some(data) = read(storage, &alloc::format!("{DIR}/{dir}/progress.bin"), 64) else {
        return true;
    };
    match xcpt::decode_progress(&data) {
        xcpt::ProgressRecord::Xtc(page) => {
            // The page of an XTC book is the place itself: exact, where
            // it can be said how far in the book it is.
            if record.percent <= 100 {
                left_at(Locator::new(page, 0), f32::from(record.percent));
            }
        }
        xcpt::ProgressRecord::Epub {
            section,
            page,
            pages,
            ..
        } => {
            // The percent is how far in the whole book: this file of the
            // book's, and the page in it, of all its files. Where the
            // record of the reading says the percent itself, that is
            // taken.
            let percent = match record.percent {
                0..=100 => f32::from(record.percent),
                // The page is counted from 0: the reading stands at
                // its start, not past it.
                _ => meta
                    .as_ref()
                    .filter(|m| m.sections > 0 && pages > 0)
                    .map(|m| {
                        (u32::from(section) as f32
                            + u32::from(page) as f32 / u32::from(pages) as f32)
                            / (m.sections as f32)
                            * 100.0
                    })
                    .unwrap_or(101.0),
            };
            if percent <= 100.0 {
                // The page begins where the file of the book does; how
                // far into it is kept in the percent, for the pages of
                // another engine do not fit here.
                left_at(Locator::new(u32::from(section), 0), percent);
            }
        }
        xcpt::ProgressRecord::None => {}
    }
    true
}

/// Writes what was brought over, as lines of a file of the plugin's.
fn write_marker(storage: &dyn Storage, done: &BTreeMap<String, String>) {
    let path = alloc::format!(
        "{}/{}",
        paperpaw_sdk::Env::plugin_dir(crate::PLUGIN.as_str()),
        MARKER
    );
    let mut text = String::from("[imported]\n");
    for (name, value) in done {
        text.push_str(&format!(
            "{} = {}\n",
            name,
            paperpaw_sdk::config::quote(value)
        ));
    }
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a mark that cannot be written only makes the next run bring the same over again, which changes nothing"
    )]
    let _ = storage.write(&path, text.as_bytes());
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::store::Day;
    use paperpaw_sdk::MemoryStorage;

    /// The local day of a moment, as the seconds since 1970 say (the
    /// tests' device is on UTC).
    fn day_of(unix: u32) -> i32 {
        (unix / 86_400) as i32
    }

    /// A per-book record of version 8, as `xcpt` reads it.
    fn book_record() -> Vec<u8> {
        let mut b = vec![0u8; 135];
        b[0] = 8;
        b[1..3].copy_from_slice(&7u16.to_le_bytes()); // sessions
        b[3..7].copy_from_slice(&7_200u32.to_le_bytes()); // seconds
        b[7..11].copy_from_slice(&300u32.to_le_bytes()); // pages turned
        b[11] = 1; // completed
        b[17..19].copy_from_slice(&2024u16.to_le_bytes());
        b[19] = 11;
        b[20] = 3;
        // The record says nothing of when the book was finished; the
        // index of finished books does.
        b[73..75].copy_from_slice(&240u16.to_le_bytes()); // wpm average
        b[75..77].copy_from_slice(&15u16.to_le_bytes()); // of 15 samples
        for i in 0..15u16 {
            b[77 + 2 * i as usize..79 + 2 * i as usize].copy_from_slice(&(200 + i).to_le_bytes());
        }
        b[107] = 15;
        b[108] = 45; // how far in the book
        b[109..111].copy_from_slice(&1_800u16.to_le_bytes()); // session average
        b[111..113].copy_from_slice(&10u16.to_le_bytes());
        for i in 0..10u16 {
            b[113 + 2 * i as usize..115 + 2 * i as usize]
                .copy_from_slice(&(1_500 + i).to_le_bytes());
        }
        b[133] = 10;
        b
    }

    /// The global record of version 6, with five minutes read on the day
    /// 20_000 and a little less the day before.
    fn global_record() -> Vec<u8> {
        let mut b = vec![0u8; 407];
        b[0] = 6;
        b[1..5].copy_from_slice(&30u32.to_le_bytes());
        b[5..9].copy_from_slice(&90_000u32.to_le_bytes());
        b[9..13].copy_from_slice(&1_200u32.to_le_bytes());
        b[13..17].copy_from_slice(&3u32.to_le_bytes());
        b[61..65].copy_from_slice(&20_000u32.to_le_bytes());
        b[159..161].copy_from_slice(&250u16.to_le_bytes());
        b[161..163].copy_from_slice(&15u16.to_le_bytes());
        for i in 0..15u16 {
            b[163 + 2 * i as usize..165 + 2 * i as usize].copy_from_slice(&(240 + i).to_le_bytes());
        }
        b[193] = 15;
        b[195..197].copy_from_slice(&1_700u16.to_le_bytes());
        b[197..199].copy_from_slice(&10u16.to_le_bytes());
        for i in 0..10u16 {
            b[199 + 2 * i as usize..201 + 2 * i as usize]
                .copy_from_slice(&(1_600 + i).to_le_bytes());
        }
        b[224] = 10;
        b[225..227].copy_from_slice(&30u16.to_le_bytes());
        b[227..229].copy_from_slice(&20u16.to_le_bytes());
        b
    }

    /// The index of finished books, with the one book finished.
    fn finished_record(path: &str) -> Vec<u8> {
        let mut b = vec![];
        b.extend_from_slice(xcpt::FINISHED_MAGIC);
        b.push(3);
        b.push(1);
        b.extend_from_slice(&0u16.to_le_bytes());
        b.extend_from_slice(&fnv1a64(path.as_bytes()).to_le_bytes());
        b.extend_from_slice(&7_200u32.to_le_bytes());
        b.extend_from_slice(&2024u16.to_le_bytes());
        b.push(11);
        b.push(3);
        b.extend_from_slice(&2025u16.to_le_bytes());
        b.push(1);
        b.push(20);
        b.extend_from_slice(&5u16.to_le_bytes());
        b.extend_from_slice(b"Anna ");
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(b"Leo");
        b
    }

    /// A card with one book at `/books/anna.epub`, its folder of
    /// CrossPoint's, and the records around it.
    fn card() -> MemoryStorage {
        let path = "/books/anna.epub";
        let storage = MemoryStorage::new().with("books/anna.epub", vec![0u8; 4096]);
        let dir = alloc::format!(".crosspoint/{}", xcpt::cache_dir(xcpt::Kind::Epub, path));
        storage
            .write(&alloc::format!("{dir}/stats_v8.bin"), &book_record())
            .unwrap();
        // Where the book was left: the third page of the second file,
        // of twelve pages in it.
        let mut progress = vec![0u8; 10];
        progress[0..2].copy_from_slice(&2u16.to_le_bytes());
        progress[2..4].copy_from_slice(&3u16.to_le_bytes());
        progress[4..6].copy_from_slice(&12u16.to_le_bytes());
        storage
            .write(&alloc::format!("{dir}/progress.bin"), &progress)
            .unwrap();
        // The book's own words of itself.
        let mut meta = vec![10u8];
        meta.extend_from_slice(&100u32.to_le_bytes());
        meta.extend_from_slice(&17u16.to_le_bytes());
        meta.extend_from_slice(&2u16.to_le_bytes());
        for text in ["Anna Karenina", "Leo Tolstoy", "ru"] {
            meta.extend_from_slice(&(text.len() as u32).to_le_bytes());
            meta.extend_from_slice(text.as_bytes());
        }
        storage
            .write(&alloc::format!("{dir}/book.bin"), &meta)
            .unwrap();
        storage
            .write(&at("global_stats.bin"), &global_record())
            .unwrap();
        storage
            .write(&at("finished_books.bin"), &finished_record(path))
            .unwrap();
        storage
            .write(&at("recent.json"), br#"{"books":[{"path":"/books/anna.epub","title":"Anna Karenina","author":"Leo Tolstoy","coverBmpPath":""}]}"#)
            .unwrap();
        storage
    }

    /// The books of the statistics after an import.
    fn books_of(storage: &dyn Storage) -> Vec<Book> {
        store::load_books(storage).unwrap()
    }

    #[test]
    fn a_card_of_crosspoint_is_brought_over() {
        let storage = card();
        let report = import(&storage, day_of).unwrap();
        assert_eq!(report.books, 1);
        assert_eq!(report.days, 2);
        assert_eq!(report.positions, 1);
        assert_eq!(report.finished, 1);
        assert_eq!(report.forward, 0);
        assert_eq!(report.unmatched, 0);
        // The book, with its title, its authors, where it is and which
        // book it is.
        let books = books_of(&storage);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Anna Karenina");
        assert_eq!(books[0].authors, "Leo Tolstoy");
        assert_eq!(books[0].path, "books/anna.epub");
        assert_eq!(books[0].language, "ru");
        assert!(!books[0].md5.is_empty());
        // It was finished on the day the index and the record say.
        assert_eq!(books[0].finished, 1_737_374_400);
        // The minutes read went into the days.
        let days = store::days(&storage, day_of);
        assert_eq!(
            days,
            [
                Day {
                    day: 19_999,
                    seconds: 20 * 60,
                    stays: 0,
                },
                Day {
                    day: 20_000,
                    seconds: 30 * 60,
                    stays: 0,
                },
            ]
        );
        // Where the book was left: the second file of the book, at its
        // start, 45 percent in.
        let position = positions::load(&storage, "books/anna.epub").unwrap();
        assert_eq!(position.at, Locator::new(2, 0));
        assert_eq!(position.percent, 45.0);
        // What the run brought over is marked, and nothing of
        // CrossPoint's was changed.
        assert!(storage.exists(&at("global_stats.bin")));
        assert!(storage.exists(&at("finished_books.bin")));
        assert!(storage.exists(&alloc::format!(
            ".crosspoint/{}/stats_v8.bin",
            xcpt::cache_dir(xcpt::Kind::Epub, "/books/anna.epub")
        )));
    }

    #[test]
    fn what_was_brought_over_is_not_brought_again() {
        let storage = card();
        let first = import(&storage, day_of).unwrap();
        let again = import(&storage, day_of).unwrap();
        assert_eq!(again.books, 0);
        assert_eq!(again.days, 0);
        assert_eq!(again.positions, 0);
        assert_eq!(again.skipped, first.skipped + 2);
        // And the books and days are as they were.
        assert_eq!(books_of(&storage).len(), 1);
        let days = store::days(&storage, day_of);
        assert_eq!(days[1].seconds, 30 * 60);
    }

    /// The days brought over survive what the log says and its own
    /// changes: they are written anew from the log and the minutes
    /// together, so reading more adds to them, not over them.
    #[test]
    fn the_days_of_the_import_and_of_the_log_add_up() {
        let storage = card();
        // Some reading of this device's own, on the day CrossPoint read
        // too and on another.
        store::append(
            &storage,
            crate::store::PageStat {
                book: 1,
                page: 1,
                start: 20_000 * 86_400 + 3600,
                duration: 600,
                pages: 100,
            },
        )
        .unwrap();
        import(&storage, day_of).unwrap();
        let days = store::days(&storage, day_of);
        assert_eq!(
            days.iter().find(|d| d.day == 20_000).map(|d| d.seconds),
            Some(30 * 60 + 600)
        );
        // More reading of this device's own, after the import.
        store::append(
            &storage,
            crate::store::PageStat {
                book: 1,
                page: 2,
                start: 20_000 * 86_400 + 7200,
                duration: 300,
                pages: 100,
            },
        )
        .unwrap();
        let days = store::days(&storage, day_of);
        assert_eq!(
            days.iter().find(|d| d.day == 20_000).map(|d| d.seconds),
            Some(30 * 60 + 900)
        );
    }

    /// A record of a newer CrossPoint than this knows is left alone,
    /// whole, and nothing of its book is brought over.
    #[test]
    fn a_record_of_a_newer_crosspoint_is_left_alone() {
        let storage = card();
        let dir = alloc::format!(
            ".crosspoint/{}",
            xcpt::cache_dir(xcpt::Kind::Epub, "/books/anna.epub")
        );
        storage
            .write(&alloc::format!("{dir}/stats_v9.bin"), &[9u8, 0, 0])
            .unwrap();
        let report = import(&storage, day_of).unwrap();
        // The record of the newer CrossPoint is left alone: nothing of
        // it is read, and so not seeded. The book itself comes over by
        // the index of finished books (a file of its own, and one this
        // knows whole); where it was left does not.
        assert_eq!(report.forward, 1);
        assert!(report.seeded.is_empty());
        assert_eq!(report.positions, 0);
        assert_eq!(report.finished, 1);
        assert!(storage.exists(&alloc::format!("{dir}/stats_v9.bin")));
    }

    /// A folder whose book cannot be told (no book on the card hashes to
    /// it) brings nothing over.
    #[test]
    fn a_folder_without_its_book_brings_nothing_over() {
        let storage = card();
        // A folder of a book that is not on the card.
        let dir = alloc::format!(
            ".crosspoint/{}",
            xcpt::cache_dir(xcpt::Kind::Epub, "/elsewhere/gone.epub")
        );
        storage
            .write(&alloc::format!("{dir}/stats_v8.bin"), &book_record())
            .unwrap();
        let report = import(&storage, day_of).unwrap();
        assert_eq!(report.unmatched, 1);
        assert_eq!(report.books, 1);
    }

    /// An XTC book's page is where the book was left, exact.
    #[test]
    fn an_xtc_book_is_brought_to_its_page() {
        let storage = card();
        let path = "/books/serial.xtc";
        storage.write("books/serial.xtc", &[0u8; 100]).unwrap();
        let dir = alloc::format!(".crosspoint/{}", xcpt::cache_dir(xcpt::Kind::Xtc, path));
        // A record of version 8, as an XTC's reader keeps it, with how
        // far in the book it was read.
        let mut record = vec![0u8; 135];
        record[0] = 8;
        record[3..7].copy_from_slice(&1_800u32.to_le_bytes());
        record[1..3].copy_from_slice(&2u16.to_le_bytes());
        record[108] = 42;
        storage
            .write(&alloc::format!("{dir}/stats_v8.bin"), &record)
            .unwrap();
        // The page of the book, from 0.
        storage
            .write(&alloc::format!("{dir}/progress.bin"), &41u32.to_le_bytes())
            .unwrap();
        let report = import(&storage, day_of).unwrap();
        assert_eq!(report.unmatched, 0, "{report:?}");
        assert_eq!(report.forward, 0, "{report:?}");
        assert_eq!(report.seeded.len(), 2, "{report:?}");
        assert_eq!(report.books, 2, "{report:?}");
        assert_eq!(report.positions, 2, "{report:?}");
        // The position file of the XTC book says its page.
        let position = positions::load(&storage, "books/serial.xtc").unwrap();
        assert_eq!(position.at, Locator::new(41, 0));
    }

    #[test]
    fn without_crosspoint_there_is_nothing_to_bring() {
        let storage = MemoryStorage::new();
        assert_eq!(import(&storage, day_of), Err(No::None));
        assert!(!present(&storage));
    }

    /// The paths hashed the way the folders are named, and the walk
    /// finds the books of a card.
    #[test]
    fn the_books_of_the_card_are_found_by_walking() {
        let storage = MemoryStorage::new()
            .with(".hidden/book.epub", vec![0u8; 16])
            .with("a/very/deeply/nested/path/book.epub", vec![0u8; 16])
            .with(".crosspoint/epub_1/stats_v8.bin", vec![0u8; 135])
            .with("not-a-book.pdf", vec![0u8; 16]);
        let paths = book_paths(&storage);
        assert!(paths.contains(&String::from("/a/very/deeply/nested/path/book.epub")));
        assert!(!paths.contains(&String::from("/.crosspoint/epub_1/stats_v8.bin")));
        assert!(!paths.iter().any(|p| p.contains(".hidden")));
        assert!(!paths.contains(&String::from("/not-a-book.pdf")));
    }

    /// The library index of CrossPoint names the books and their paths.
    #[test]
    fn the_library_index_names_the_books() {
        // A made index: a header, one folder, one record and the names.
        let mut b = vec![0u8; 64];
        b[0..4].copy_from_slice(b"CLX1");
        b[4] = 2;
        b[8..10].copy_from_slice(&1u16.to_le_bytes()); // books
        b[10..12].copy_from_slice(&1u16.to_le_bytes()); // folders
        let (folder_start, folder_bytes) = (512u32, 3u32);
        let record_start = 1024u32;
        let perm_start = record_start + 128;
        let name_start = perm_start + 8;
        b[16..20].copy_from_slice(&folder_start.to_le_bytes());
        b[20..24].copy_from_slice(&folder_bytes.to_le_bytes());
        b[24..28].copy_from_slice(&record_start.to_le_bytes());
        b[28..32].copy_from_slice(&perm_start.to_le_bytes());
        b[32..36].copy_from_slice(&name_start.to_le_bytes());
        let mut data = b;
        data.resize(name_start as usize + 100, 0);
        let size = data.len() as u32;
        data[40..44].copy_from_slice(&size.to_le_bytes());
        // The folder's path.
        data[folder_start as usize] = 2;
        data[folder_start as usize + 1] = b'/';
        data[folder_start as usize + 2] = b'b';
        // The record: the name offset, the folder, the name's length.
        data[record_start as usize..record_start as usize + 4].copy_from_slice(&0u32.to_le_bytes());
        data[record_start as usize + 10..record_start as usize + 12]
            .copy_from_slice(&0u16.to_le_bytes());
        data[record_start as usize + 12] = 6; // the name's length
        // The names: the hash of the path, the name, the author, the
        // title, the author as the book spells it.
        let mut at = name_start as usize;
        data[at..at + 8].copy_from_slice(&fnv1a64(b"/b/x.epub").to_le_bytes());
        at += 8;
        data[at..at + 6].copy_from_slice(b"x.epub");
        at += 6;
        data[at] = 0;
        at += 1;
        data[at] = 1;
        at += 1;
        data[at] = b'X';
        at += 1;
        data[at] = 0;
        let storage = MemoryStorage::new().with(".crosspoint/library.idx", data);
        let found = library_index(&storage);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "/b/x.epub");
        assert_eq!(found[0].1, "X");
    }
}
