//! The kernel's catalog of the library: never walked at a start, walked
//! once a view asks after one (and when asked anew), nothing done while a
//! book is open, books moved, removed or changed caught up without a
//! walk, and a walk that fails told to the views.

use super::*;
use alloc::rc::Rc;
use core::cell::Cell;
use paperpaw_platform::{DirEntry, Storage, StorageError};
use paperpaw_sdk::builtin::{events, params};
use paperpaw_sdk::document::DocumentError;
use paperpaw_sdk::event::EventData;
use paperpaw_sdk::library::{self, catalog};
use paperpaw_sdk::{FormatSpec, MemoryStorage, Params};

/// Reads text books (as names only) and keeps a rating in the catalog;
/// logs every new catalog.
struct LibraryPlugin {
    log: Log,
}

impl Plugin for LibraryPlugin {
    fn manifest(&self) -> Manifest {
        Manifest::new(Id::new_static("lib"), "Library", Version::new(1, 0, 0))
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        r.add_format(FormatSpec::new(
            Id::new_static("lib.txt"),
            "TXT",
            alloc::vec![Cow::Borrowed("txt")],
            Box::new(|_, _, _| Err(DocumentError::UnsupportedFormat)),
        ))?;
        r.add_book_field(
            library::BookField::new(Id::new_static("lib.rating"), "lib.rating").in_catalog(),
        )?;
        // A reader, as the reader's slot shows it.
        r.add_screen(ScreenSpec::new(
            Id::new_static("lib.reader"),
            "reader",
            Box::new(|_, _| Box::new(Page)),
        ))?;
        r.bind_slot(
            slots::READER,
            Id::new_static("lib.reader"),
            Priority(i16::MAX),
        )?;
        // A library's list, as the library's slot shows it.
        r.add_screen(ScreenSpec::new(
            Id::new_static("lib.list"),
            "list",
            Box::new(|_, _| Box::new(Page)),
        ))?;
        r.bind_slot(
            slots::LIBRARY,
            Id::new_static("lib.list"),
            Priority(i16::MAX),
        )?;
        let log = self.log.clone();
        r.subscribe(
            events::CATALOG_FAILED.id().clone(),
            Box::new(move |_, _, _| log.borrow_mut().push(String::from("failed"))),
        )?;
        let log = self.log.clone();
        r.subscribe(
            events::CATALOG_CHANGED.id().clone(),
            Box::new(move |_, _, _| log.borrow_mut().push(String::from("catalog"))),
        )
    }
}

/// A book's page.
struct Page;

impl Screen for Page {
    fn draw(&mut self, _: &mut Canvas<'_>, _: &Env<'_>) {}

    fn on_action(&mut self, _: &Id, _: &mut Cx<'_, '_>) -> Handled {
        Handled::No
    }
}

fn library(storage: MemoryStorage) -> (Kernel, Log, u64) {
    library_on(DEVICE, storage)
}

fn library_on(device: DeviceInfo, storage: MemoryStorage) -> (Kernel, Log, u64) {
    let log = log();
    let mut kernel = Kernel::builder(device)
        .plugin(LibraryPlugin { log: log.clone() })
        .storage(storage)
        .build()
        .unwrap();
    let effects = kernel.start(Instant(0));
    let now = run(&mut kernel, 0, effects);
    (kernel, log, now)
}

/// Runs what is due within a minute and more, as time goes on.
fn run(kernel: &mut Kernel, mut now: u64, mut effects: crate::Effects) -> u64 {
    for _ in 0..2000 {
        match effects.wake_at() {
            Some(at) if at.0 <= now + 61_000 => {
                now = now.max(at.0);
                effects = kernel.tick(Instant(now));
            }
            _ => break,
        }
    }
    now
}

fn send(kernel: &mut Kernel, now: u64, event: &Id, params: Params) -> u64 {
    let effects = kernel.send_event(Instant(now), event.clone(), params);
    run(kernel, now, effects)
}

fn want(kernel: &mut Kernel, now: u64, anew: bool) -> u64 {
    let wanted = events::CatalogWanted::new(anew).to_params();
    send(kernel, now, events::CATALOG_WANTED.id(), wanted)
}

fn paths(kernel: &Kernel) -> Vec<String> {
    let mut paths = Vec::new();
    catalog::each(kernel.storage(), &mut |r| paths.push(r.path));
    paths.sort();
    paths
}

fn two_books() -> MemoryStorage {
    MemoryStorage::new()
        .with("books/a.txt", "one")
        .with("books/b.txt", "two")
}

#[test]
fn the_catalog_is_walked_when_a_view_asks_never_at_a_start() {
    let (mut kernel, log, now) = library(two_books());
    // A start (on the X4 every wake): nothing walked.
    assert!(!kernel.storage().exists(".paperpaw/library/catalog.txt"));
    // A view shows: walked, and said.
    let now = want(&mut kernel, now, false);
    assert_eq!(paths(&kernel), ["books/a.txt", "books/b.txt"]);
    assert_eq!(*log.borrow(), ["catalog"]);
    // Shown again: not again since the start; asked anew, it is.
    let now = want(&mut kernel, now, false);
    assert_eq!(log.borrow().len(), 1);
    want(&mut kernel, now, true);
    assert_eq!(log.borrow().len(), 2);
}

#[test]
fn nothing_is_done_while_a_book_is_read() {
    let (mut kernel, log, now) = library(two_books());
    let path = Params::new().with(params::PATH, "books/a.txt");
    let effects = kernel.open_slot(Instant(now), slots::READER, path);
    let now = run(&mut kernel, now, effects);
    let now = want(&mut kernel, now, true);
    assert!(log.borrow().is_empty(), "walked while reading");
    assert!(!kernel.storage().exists(".paperpaw/library/catalog.txt"));
    // Another screen on top (the book left, or the library over it):
    // done then.
    let effects = kernel.run_action(Instant(now), actions::BACK);
    run(&mut kernel, now, effects);
    assert_eq!(*log.borrow(), ["catalog"]);
    assert_eq!(paths(&kernel), ["books/a.txt", "books/b.txt"]);
}

/// On a device of little memory (the X4) the catalog waits while the
/// library shows its books; on others it is made beside them.
#[test]
fn on_small_devices_nothing_is_done_while_the_library_shows() {
    for (memory, beside) in [
        (paperpaw_platform::MemoryClass::Small, false),
        (paperpaw_platform::MemoryClass::Large, true),
    ] {
        let device = DEVICE.with_memory(memory);
        let (mut kernel, log, now) = library_on(device, two_books());
        let now = want(&mut kernel, now, false);
        log.borrow_mut().clear();
        let effects = kernel.open_slot(Instant(now), slots::LIBRARY, Params::new());
        let now = run(&mut kernel, now, effects);
        let now = want(&mut kernel, now, true);
        assert_eq!(log.borrow().len(), usize::from(beside), "{memory:?}");
        let effects = kernel.run_action(Instant(now), actions::BACK);
        run(&mut kernel, now, effects);
        assert_eq!(log.borrow().len(), 1, "{memory:?}");
    }
}

#[test]
fn books_moved_removed_or_changed_are_caught_up() {
    let (mut kernel, log, now) = library(two_books());
    let now = want(&mut kernel, now, false);
    log.borrow_mut().clear();
    // Moved: where it is now, not where it was.
    kernel
        .storage()
        .rename("books/a.txt", "books/Kept/a.txt")
        .unwrap();
    let moved = events::BookMoved::new("books/a.txt".into(), "books/Kept/a.txt".into());
    let now = send(&mut kernel, now, events::BOOK_MOVED.id(), moved.to_params());
    assert_eq!(paths(&kernel), ["books/Kept/a.txt", "books/b.txt"]);
    assert_eq!(*log.borrow(), ["catalog"]);
    // Removed: left out at once, before any catalog is made.
    kernel.storage().remove("books/b.txt").unwrap();
    let removed = events::BookRemoved::new("books/b.txt".into()).to_params();
    let effects = kernel.send_event(Instant(now), events::BOOK_REMOVED.id().clone(), removed);
    assert_eq!(paths(&kernel), ["books/Kept/a.txt"]);
    let now = run(&mut kernel, now, effects);
    // A field kept in the catalog changed: caught up with it.
    library::set_field(kernel.storage(), "books/Kept/a.txt", "lib.rating", "5");
    let changed = events::BookChanged::new("books/Kept/a.txt".into(), library::what::FIELDS);
    send(
        &mut kernel,
        now,
        events::BOOK_CHANGED.id(),
        changed.to_params(),
    );
    let mut rating = None;
    catalog::each(kernel.storage(), &mut |r| {
        rating = r.field("lib.rating").map(String::from)
    });
    assert_eq!(rating.as_deref(), Some("5"));
}

/// A catalog without books holds no index: on a small device the walk
/// runs beside the library (books copied onto the card meanwhile show at
/// once), where one with books, or with entries in its log, waits until
/// the library is closed.
#[test]
fn an_empty_catalog_is_walked_beside_the_library_on_small_devices() {
    let device = DEVICE.with_memory(paperpaw_platform::MemoryClass::Small);
    let empty = || two_books().with(".paperpaw/library/catalog.txt", "\tcatalog\t2\t1\n");
    let (mut kernel, log, now) = library_on(device, empty());
    let effects = kernel.open_slot(Instant(now), slots::LIBRARY, Params::new());
    let now = run(&mut kernel, now, effects);
    assert!(paths(&kernel).is_empty());
    want(&mut kernel, now, true);
    assert_eq!(*log.borrow(), ["catalog"]);
    assert_eq!(paths(&kernel), ["books/a.txt", "books/b.txt"]);
    // Once it lists books, a walk asked while the library shows waits.
    let (mut kernel, log, now) = library_on(device, empty());
    let now = want(&mut kernel, now, true);
    log.borrow_mut().clear();
    let effects = kernel.open_slot(Instant(now), slots::LIBRARY, Params::new());
    let now = run(&mut kernel, now, effects);
    want(&mut kernel, now, true);
    assert!(
        log.borrow().is_empty(),
        "walked beside a library with books"
    );
    // No book in the catalog, one in its log (waiting to be folded in):
    // a view holds an index, so the walk waits.
    let noted = empty().with(".paperpaw/library/status.log", "1\t2\n");
    let (mut kernel, log, now) = library_on(device, noted);
    let effects = kernel.open_slot(Instant(now), slots::LIBRARY, Params::new());
    let now = run(&mut kernel, now, effects);
    want(&mut kernel, now, true);
    assert!(log.borrow().is_empty(), "walked beside a log with entries");
    assert!(paths(&kernel).is_empty());
    let effects = kernel.run_action(Instant(now), actions::BACK);
    run(&mut kernel, now, effects);
    assert_eq!(paths(&kernel), ["books/a.txt", "books/b.txt"]);
}

/// A library with nothing in it is walked once per start: asked again, the
/// view is told (so that it stops saying books are being collected), but
/// no new catalog is made.
#[test]
fn an_empty_library_is_not_walked_again_each_time_a_view_asks() {
    let (mut kernel, log, now) = library(MemoryStorage::new());
    let now = want(&mut kernel, now, false);
    assert_eq!(*log.borrow(), ["catalog"]);
    let generation = catalog::generation(kernel.storage());
    assert!(generation.is_some());
    // Shown again, twice: told, not walked (the generation stays).
    let now = want(&mut kernel, now, false);
    want(&mut kernel, now, false);
    assert_eq!(log.borrow().len(), 3);
    assert_eq!(catalog::generation(kernel.storage()), generation);
    // Books copied in meanwhile are found where one asks anew.
    kernel.storage().write("books/a.txt", b"one").unwrap();
    want(&mut kernel, now, true);
    assert_eq!(paths(&kernel), ["books/a.txt"]);
}

/// A card whose books folder cannot be listed while `broken` is set.
struct BrokenFolder {
    files: MemoryStorage,
    broken: Rc<core::cell::Cell<bool>>,
}

impl paperpaw_platform::Storage for BrokenFolder {
    fn read_at(
        &self,
        path: &str,
        offset: u64,
        buf: &mut [u8],
    ) -> Result<usize, paperpaw_platform::StorageError> {
        self.files.read_at(path, offset, buf)
    }

    fn size(&self, path: &str) -> Result<u64, paperpaw_platform::StorageError> {
        self.files.size(path)
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<(), paperpaw_platform::StorageError> {
        self.files.write(path, data)
    }

    fn append(&self, path: &str, data: &[u8]) -> Result<(), paperpaw_platform::StorageError> {
        self.files.append(path, data)
    }

    fn list(
        &self,
        dir: &str,
        visit: &mut dyn FnMut(paperpaw_platform::DirEntry<'_>),
    ) -> Result<(), paperpaw_platform::StorageError> {
        if self.broken.get() && dir.trim_matches('/').starts_with("books") {
            return Err(paperpaw_platform::StorageError::Io);
        }
        self.files.list(dir, visit)
    }

    fn remove(&self, path: &str) -> Result<(), paperpaw_platform::StorageError> {
        self.files.remove(path)
    }
}

#[test]
fn a_walk_that_fails_is_told_and_a_later_one_tells_the_catalog() {
    let broken = Rc::new(core::cell::Cell::new(true));
    let log = log();
    let mut kernel = Kernel::builder(DEVICE)
        .plugin(LibraryPlugin { log: log.clone() })
        .storage(BrokenFolder {
            files: two_books(),
            broken: Rc::clone(&broken),
        })
        .build()
        .unwrap();
    let effects = kernel.start(Instant(0));
    let now = run(&mut kernel, 0, effects);
    let now = want(&mut kernel, now, false);
    // Told that it failed, not that a catalog was made; none is there.
    assert_eq!(log.borrow().first().map(String::as_str), Some("failed"));
    assert!(!log.borrow().iter().any(|l| l == "catalog"));
    assert!(paths(&kernel).is_empty());
    // The card can be read again: asked anew, the books are there.
    broken.set(false);
    log.borrow_mut().clear();
    want(&mut kernel, now + 600_000, true);
    assert!(
        log.borrow().iter().any(|l| l == "catalog"),
        "{:?}",
        log.borrow()
    );
    assert_eq!(paths(&kernel), ["books/a.txt", "books/b.txt"]);
}

/// A card whose writes to the library's folder fail while `full` is set,
/// as a full card's do.
struct Card {
    inner: MemoryStorage,
    full: Rc<Cell<bool>>,
}

impl Card {
    fn check(&self, path: &str) -> Result<(), StorageError> {
        if self.full.get() && path.starts_with(".paperpaw/library") {
            Err(StorageError::Io)
        } else {
            Ok(())
        }
    }
}

impl Storage for Card {
    fn read_at(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        self.inner.read_at(path, offset, buf)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.inner.size(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.check(path)?;
        self.inner.write(path, data)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.check(path)?;
        self.inner.append(path, data)
    }
    fn list(&self, dir: &str, visit: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), StorageError> {
        self.inner.list(dir, visit)
    }
    fn remove(&self, path: &str) -> Result<(), StorageError> {
        self.inner.remove(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        self.check(to)?;
        self.inner.rename(from, to)
    }
    fn create_dir(&self, path: &str) -> Result<(), StorageError> {
        self.check(path)?;
        self.inner.create_dir(path)
    }
}

/// A walk that fails (the card is full) tells the views that it failed, so
/// that none keeps saying the books are being collected; it is tried again
/// after a while, and tells once more when it works.
#[test]
fn a_walk_that_cannot_write_is_told_and_tried_again() {
    let full = Rc::new(Cell::new(true));
    let log = log();
    let card = Card {
        inner: two_books(),
        full: full.clone(),
    };
    let mut kernel = Kernel::builder(DEVICE)
        .plugin(LibraryPlugin { log: log.clone() })
        .storage(card)
        .build()
        .unwrap();
    let effects = kernel.start(Instant(0));
    let now = run(&mut kernel, 0, effects);
    let now = want(&mut kernel, now, false);
    assert!(paths(&kernel).is_empty());
    assert_eq!(log.borrow().first().map(String::as_str), Some("failed"));
    // The card has room again: the next attempt (after the wait) makes
    // the catalog, and tells once more.
    full.set(false);
    let told = log.borrow().len();
    let mut now = now;
    for _ in 0..40 {
        if !paths(&kernel).is_empty() {
            break;
        }
        now += 60_000;
        let effects = kernel.tick(Instant(now));
        now = run(&mut kernel, now, effects);
    }
    assert_eq!(paths(&kernel), ["books/a.txt", "books/b.txt"]);
    assert!(log.borrow().len() > told);
}
