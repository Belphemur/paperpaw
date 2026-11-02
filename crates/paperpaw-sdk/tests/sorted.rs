//! The library's books sorted on the card ([`library::sorted`]): made as
//! the kernel makes them, a view pages them as the index lists them (every
//! filter, the books read last first or by title, series stacked, the next
//! book of each series), and a file of another version, cut short or
//! damaged is never read; a making cut short leaves the file there was.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use paperpaw_sdk::library::catalog::{self, Record, Show, Status};
use paperpaw_sdk::library::index::{self, Index};
use paperpaw_sdk::library::sorted::{self, Build, Entry, Sorted, View};

use paperpaw_sdk::library::{self, Book};
use paperpaw_sdk::{DirEntry, Env, MemoryStorage, Storage, StorageError};
use std::cell::Cell;

/// The file of the sorted books for a view leaving `archive` out.
fn file_of(archive: &str) -> String {
    format!(".paperpaw/library/sorted-{:08x}.idx", index::hash(archive))
}

/// The file of the view most tests make.
fn file() -> String {
    file_of("books/Archive")
}

/// Titles in several scripts, articles and cases, alike at their start.
const TITLES: [&str; 12] = [
    "The Hound of the Baskervilles",
    "a Study in Scarlet",
    "Émile",
    "Zola",
    "كتاب الرمل",
    "한국어 책",
    "Das Schloss",
    "das Urteil",
    "1984",
    "Book",
    "Book",
    "",
];

/// A library of `n` books: series of one to five books (some without),
/// some being read, finished or favourites, in folders and the archive.
fn library(n: usize) -> MemoryStorage {
    let storage = MemoryStorage::new();
    let mut records = Vec::new();
    for i in 0..n {
        let folder = if i % 17 == 0 { "Archive" } else { "Shelf" };
        let path = format!("books/{folder}/b{i:05}.epub");
        let series = if i % 7 == 0 {
            String::new()
        } else {
            format!("Series {}", (i * 13) % (n / 3 + 1))
        };
        let title = format!("{} {}", TITLES[i % TITLES.len()], (i * 7919) % 97);
        let book = Book::default()
            .title(Some(title))
            .author(Some(format!("Author {}", i % 31)))
            .series((!series.is_empty()).then_some(series))
            .series_index(Some(format!("{}", i % 5 + 1)))
            .size(100 + i as u64);
        let progress = match i % 9 {
            0 => Some(100),
            1 | 2 => Some((i % 98 + 1) as u8),
            _ => None,
        };
        let status = Status::new(progress, i % 11 == 0, progress.is_some() || i % 13 == 0);
        records.push(Record::of(&path, &book).with_status(status));
    }
    catalog::write(&storage, &records).unwrap();
    // Read last: some of every kind.
    let opened: Vec<String> = (0..n)
        .filter(|i| i % 23 == 0 || i % 9 == 1)
        .take(60)
        .map(|i| format!("books/Shelf/b{i:05}.epub"))
        .collect();
    storage
        .write(".paperpaw/library/opened.txt", opened.join("\n").as_bytes())
        .unwrap();
    // Noted since the catalog was made.
    for i in (5..n).step_by(41) {
        let path = format!("books/Shelf/b{i:05}.epub");
        catalog::note(&storage, &path, Status::new(Some(50), true, true)).unwrap();
    }
    storage
}

/// Makes the file to its end, pausing every `pause` steps.
fn make(env: &Env<'_>, archive: &str, pause: Option<usize>) -> usize {
    let mut build = Build::start(env, archive).unwrap();
    let mut steps = 0;
    while !build.step(env).unwrap() {
        steps += 1;
        if pause.is_some_and(|p| steps % p == 0) {
            build.pause(env.storage());
        }
        assert!(steps < 100_000, "never done");
    }
    steps
}

/// Every view: the filters, by recent or title, stacked (all books).
fn views() -> Vec<View> {
    let mut views = Vec::new();
    for show in [
        Show::All,
        Show::Unread,
        Show::Reading,
        Show::Finished,
        Show::Favourites,
    ] {
        for by_recent in [true, false] {
            views.push(View::new(show, by_recent, false));
        }
    }
    views.push(View::new(Show::All, true, true));
    views.push(View::new(Show::All, false, true));
    views
}

/// What the index lists for `view`: each book's key and how many books
/// its stack holds.
fn expected(index: &Index, view: View) -> Vec<(u32, u16)> {
    let order = index.order(view.show, view.by_recent);
    if view.stacked {
        let (order, counts) = index.stacked(&order);
        return order
            .iter()
            .zip(counts)
            .map(|(&i, c)| (index.items[usize::from(i)].offset, c))
            .collect();
    }
    order
        .iter()
        .map(|&i| (index.items[usize::from(i)].offset, 1))
        .collect()
}

/// All of `view`, paged `step` at a time.
fn paged(storage: &dyn Storage, sorted: &Sorted, view: View, step: usize) -> Vec<Entry> {
    let mut all = Vec::new();
    loop {
        let page = sorted.page(storage, view, all.len(), step);
        if page.is_empty() {
            break;
        }
        all.extend(page);
    }
    all
}

#[test]
fn every_view_pages_as_the_index_lists() {
    let h = harness(library(900));
    let env = h.env();
    make(&env, "books/Archive", None);
    let sorted = Sorted::open(&env, "books/Archive").expect("made");
    let index = Index::build(&h.storage, 100_000, "books/Archive");
    assert_eq!(sorted.total(), index.items.len());
    assert_eq!(sorted.archived(), index.archived);
    for view in views() {
        let want = expected(&index, view);
        assert_eq!(sorted.len(view), want.len(), "{view:?}");
        for step in [1, 7, 64, 1000] {
            let got: Vec<(u32, u16)> = paged(&h.storage, &sorted, view, step)
                .iter()
                .map(|e| (e.item.offset, e.stack))
                .collect();
            assert_eq!(got, want, "{view:?} by {step}");
        }
        // Each found where it is listed.
        let page = sorted.page(&h.storage, view, 0, usize::MAX);
        for (at, entry) in page.iter().enumerate().step_by(13) {
            assert_eq!(
                sorted.find(&h.storage, view, entry.item.hash),
                Some(at),
                "{view:?}"
            );
        }
    }
    for by_recent in [true, false] {
        let want: Vec<u32> = index
            .next_in_series(by_recent)
            .iter()
            .map(|&i| index.items[usize::from(i)].offset)
            .collect();
        let got: Vec<u32> = sorted
            .next_in_series(&h.storage, by_recent)
            .iter()
            .map(|e| e.item.offset)
            .collect();
        assert_eq!(got, want, "next, by recent {by_recent}");
    }
}

#[test]
fn a_making_paused_makes_the_same_file() {
    let h = harness(library(700));
    let env = h.env();
    make(&env, "books/Archive", None);
    let whole = h.storage.get(&file()).unwrap();
    for pause in [1, 3, 20] {
        h.storage.remove(&file()).unwrap();
        make(&env, "books/Archive", Some(pause));
        assert_eq!(
            h.storage.get(&file()).unwrap(),
            whole,
            "paused every {pause}"
        );
    }
    assert!(!h.storage.exists(".paperpaw/library/sorted.runs"));
    assert!(!h.storage.exists(".paperpaw/library/sorted.tmp"));
}

#[test]
fn a_file_of_another_version_cut_short_or_damaged_is_not_read() {
    let h = harness(library(600));
    let env = h.env();
    make(&env, "books/Archive", None);
    let whole = h.storage.get(&file()).unwrap();
    assert!(Sorted::open(&env, "books/Archive").is_some());
    // Another archive.
    assert!(Sorted::open(&env, "books/Other").is_none());
    assert!(sorted::wanted(&env, "books/Other"));
    assert!(!sorted::wanted(&env, "books/Archive"));
    // Cut short anywhere.
    for len in [
        0,
        1,
        27,
        28,
        whole.len() / 2,
        whole.len() - 44,
        whole.len() - 1,
    ] {
        h.storage.write(&file(), &whole[..len]).unwrap();
        assert!(
            Sorted::open(&env, "books/Archive").is_none(),
            "cut at {len}"
        );
        assert!(sorted::wanted(&env, "books/Archive"));
    }
    // A byte changed anywhere: in the books, the archive's name, the end.
    for at in [
        0,
        5,
        28 * 300 + 3,
        whole.len() - 50,
        whole.len() - 30,
        whole.len() - 1,
    ] {
        let mut bad = whole.clone();
        bad[at] ^= 0x5a;
        h.storage.write(&file(), &bad).unwrap();
        assert!(
            Sorted::open(&env, "books/Archive").is_none(),
            "changed at {at}"
        );
    }
    // Of a later format, and garbage.
    let mut later = whole.clone();
    let end = later.len() - 44;
    later[end + 3] = b'2';
    h.storage.write(&file(), &later).unwrap();
    assert!(Sorted::open(&env, "books/Archive").is_none());
    let garbage: Vec<u8> = (0..5000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    h.storage.write(&file(), &garbage).unwrap();
    assert!(Sorted::open(&env, "books/Archive").is_none());
    // Of the catalog before: the catalog made anew, the file is stale.
    h.storage.write(&file(), &whole).unwrap();
    let mut records = Vec::new();
    catalog::each(&h.storage, &mut |r| records.push(r));
    records.pop();
    catalog::write(&h.storage, &records).unwrap();
    assert!(Sorted::open(&env, "books/Archive").is_none(), "stale");
    assert!(sorted::wanted(&env, "books/Archive"));
}

#[test]
fn a_making_cut_short_leaves_the_file_there_was() {
    let h = harness(library(800));
    let env = h.env();
    make(&env, "books/Archive", None);
    let before = h.storage.get(&file()).unwrap();
    // The catalog changes; the making is cut short at each step (the
    // power gone): the file there was stays whole.
    let mut records = Vec::new();
    catalog::each(&h.storage, &mut |r| records.push(r));
    records.truncate(700);
    catalog::write(&h.storage, &records).unwrap();
    for cut in [1, 5, 12, 20] {
        let mut build = Build::start(&env, "books/Archive").unwrap();
        let mut done = false;
        for _ in 0..cut {
            done = build.step(&env).unwrap();
            if done {
                break;
            }
        }
        drop(build);
        if !done {
            assert_eq!(h.storage.get(&file()).unwrap(), before, "cut after {cut}");
        }
        // What it left goes at the next start.
        sorted::clean_up(&h.storage);
        assert!(!h.storage.exists(".paperpaw/library/sorted.runs"));
        assert!(!h.storage.exists(".paperpaw/library/sorted.tmp"));
    }
    make(&env, "books/Archive", None);
    let sorted = Sorted::open(&env, "books/Archive").unwrap();
    let index = Index::build(&h.storage, 100_000, "books/Archive");
    assert_eq!(sorted.total(), index.items.len());
}

/// Two views of two archive folders (a plugin's, a widget's) each have
/// their file, its name trimmed of `/` as a view gives it; a third view's
/// takes the place of the one asked for first.
#[test]
fn each_archive_has_its_file_two_at_most() {
    let h = harness(library(300));
    let env = h.env();
    let mut archives = Vec::new();
    for archive in ["books/Archive", "books/Other/"] {
        sorted::asked(&h.storage, &mut archives, archive);
        make(&env, archive, None);
    }
    for archive in [
        "books/Archive",
        "/books/Archive/",
        "books/Other",
        "books/Other/",
    ] {
        assert!(Sorted::open(&env, archive).is_some(), "{archive}");
        assert!(!sorted::wanted(&env, archive), "{archive}");
    }
    let mut kept = sorted::kept_archives(&h.storage);
    kept.sort();
    assert_eq!(kept, ["books/Archive", "books/Other"]);
    sorted::asked(&h.storage, &mut archives, "books/Third");
    assert_eq!(archives, ["books/Third", "books/Other"]);
    assert!(!h.storage.exists(&file()));
    assert!(sorted::wanted(&env, "books/Archive"));
    assert!(Sorted::open(&env, "books/Other").is_some());
    // Asked again: first, nothing goes.
    sorted::asked(&h.storage, &mut archives, "books/Other");
    assert_eq!(archives, ["books/Other", "books/Third"]);
    assert!(h.storage.exists(&file_of("books/Other")));
}

/// A storage whose append number `fail_at` (from 1) fails, as a full
/// card's does.
struct FailingOnce<'a> {
    inner: &'a MemoryStorage,
    appends: Cell<usize>,
    fail_at: Cell<usize>,
}

impl Storage for FailingOnce<'_> {
    fn read_at(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        self.inner.read_at(path, offset, buf)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.inner.size(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.inner.write(path, data)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.appends.set(self.appends.get() + 1);
        if self.appends.get() == self.fail_at.get() {
            return Err(StorageError::Full);
        }
        self.inner.append(path, data)
    }
    fn list(&self, dir: &str, visit: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), StorageError> {
        self.inner.list(dir, visit)
    }
    fn remove(&self, path: &str) -> Result<(), StorageError> {
        self.inner.remove(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        self.inner.rename(from, to)
    }
}

/// A pause whose run is written in part only (the card full after its
/// first piece) leaves no file of books shifted: the making fails, and
/// the next one makes the file whole.
#[test]
fn a_pause_written_in_part_makes_no_file() {
    let h = harness(library(700));
    make(&h.env(), "books/Archive", None);
    let whole = h.storage.get(&file()).unwrap();
    h.storage.remove(&file()).unwrap();
    let failing = FailingOnce {
        inner: &h.storage,
        appends: Cell::new(0),
        fail_at: Cell::new(0),
    };
    let env = h.env_on(&failing);
    let mut build = Build::start(&env, "books/Archive").unwrap();
    // Three steps: some 190 books held, written in three pieces.
    for _ in 0..3 {
        assert!(!build.step(&env).unwrap());
    }
    failing.fail_at.set(failing.appends.get() + 2);
    build.pause(&failing);
    let mut failed = false;
    for _ in 0..100_000 {
        match build.step(&env) {
            Ok(true) => break,
            Ok(false) => {}
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    assert!(failed, "made from shifted runs");
    assert!(!h.storage.exists(&file()));
    assert!(!h.storage.exists(".paperpaw/library/sorted.runs"));
    make(&env, "books/Archive", None);
    assert_eq!(h.storage.get(&file()).unwrap(), whole);
}

#[test]
fn what_changed_since_is_taken_in() {
    let h = harness(library(400));
    let env = h.env();
    make(&env, "books/Archive", None);
    let mut sorted = Sorted::open(&env, "books/Archive").unwrap();
    let view = View::new(Show::Finished, false, false);
    let before = sorted.len(view);
    // A book finished, one removed, one hidden: as the index has them.
    let path = "books/Shelf/b00004.epub";
    library::set_finished(&h.storage, path, true);
    let _ = catalog::forget(&h.storage, "books/Shelf/b00009.epub");
    sorted.refresh(&h.storage);
    sorted.hide(&h.storage, index::hash("books/Shelf/b00018.epub"));
    let mut index = Index::build(&h.storage, 100_000, "books/Archive");
    index.refresh(&h.storage);
    for view in views() {
        let mut want = expected(&index, view);
        let gone = index
            .items
            .iter()
            .find(|i| i.hash == index::hash("books/Shelf/b00018.epub"))
            .map(|i| i.offset);
        want.retain(|(o, _)| Some(*o) != gone);
        if view.stacked {
            // Stacks without the hidden book: compare the books only.
            let got: Vec<u32> = paged(&h.storage, &sorted, view, 50)
                .iter()
                .map(|e| e.item.offset)
                .collect();
            assert!(got.iter().all(|o| Some(*o) != gone));
            continue;
        }
        let got: Vec<(u32, u16)> = paged(&h.storage, &sorted, view, 50)
            .iter()
            .map(|e| (e.item.offset, e.stack))
            .collect();
        assert_eq!(got, want, "{view:?}");
    }
    assert_ne!(sorted.len(view), before);
}

fn harness(storage: MemoryStorage) -> common::Harness {
    let mut h = common::Harness::new();
    h.storage = storage;
    h
}
