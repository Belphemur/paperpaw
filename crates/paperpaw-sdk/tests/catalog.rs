//! Making the library's catalog as the kernel does: a walk takes the
//! books whose files did not change from the old catalog, the log noted
//! before goes with a new catalog, a book removed and sent again starts
//! afresh, catching up takes books that came without walking, and an
//! error leaves the old catalog as it was.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::borrow::Cow;
use std::cell::Cell;

use paperpaw_sdk::builtin::events::Reading;
use paperpaw_sdk::document::DocumentError;
use paperpaw_sdk::library::catalog::{self, Rewrite, Status, Walk};
use paperpaw_sdk::library::{self, index::Index};
use paperpaw_sdk::{DirEntry, Env, FormatSpec, Formats, Id, MemoryStorage, Storage, StorageError};

fn harness(storage: MemoryStorage) -> common::Harness {
    let mut h = common::Harness::new();
    let mut formats = Formats::new();
    formats
        .add(FormatSpec::new(
            Id::new_static("test.txt"),
            "TXT",
            vec![Cow::Borrowed("txt")],
            Box::new(|_, _, _| Err(DocumentError::UnsupportedFormat)),
        ))
        .unwrap();
    h.formats = formats;
    h.storage = storage;
    h
}

fn walk(env: &Env<'_>) {
    let mut walk = Walk::start(env, Vec::new()).unwrap();
    while !walk.step(env).unwrap() {}
}

fn catch_up(env: &Env<'_>, fresh: &[&str]) {
    let fresh = fresh.iter().map(|p| p.to_string()).collect();
    let mut rewrite = Rewrite::start(env, fresh).unwrap();
    while !rewrite.step(env).unwrap() {}
}

/// The books of the catalog: path and how far read, by path.
fn books(storage: &dyn Storage) -> Vec<(String, Option<u8>)> {
    let mut books = Vec::new();
    catalog::each(storage, &mut |r| books.push((r.path, r.status.progress)));
    books.sort();
    books
}

fn details(path: &str) -> String {
    let key = library::key(path);
    format!(".paperpaw/library/details/{}/{key}.ini", &key[..2])
}

fn reading(path: &str, progress: f32) -> Reading {
    Reading::new(path.into(), String::new(), progress, 0, 0, String::new())
}

#[test]
fn a_walk_takes_books_whose_files_did_not_change_from_the_old_catalog() {
    let h = harness(
        MemoryStorage::new()
            .with("books/a.txt", "one")
            .with("books/Sub/b.txt", "two")
            .with("books/.hidden.txt", "no")
            .with("books/c.pdf", "not a book here"),
    );
    let env = h.env();
    walk(&env);
    let found = books(&h.storage);
    assert_eq!(
        found,
        [
            ("books/Sub/b.txt".to_string(), None),
            ("books/a.txt".to_string(), None)
        ]
    );
    assert!(h.storage.exists(&details("books/a.txt")), "indexed");
    let first = catalog::generation(&h.storage).unwrap();
    // The details gone, the files as they were: taken from the catalog,
    // nothing read or opened.
    h.storage.remove(&details("books/a.txt")).unwrap();
    walk(&env);
    assert_eq!(books(&h.storage), found);
    assert!(!h.storage.exists(&details("books/a.txt")));
    assert_eq!(catalog::generation(&h.storage), Some(first + 1));
    // A file of another size is read anew; the listing kept goes.
    h.storage.write("books/a.txt", b"one more").unwrap();
    walk(&env);
    assert!(h.storage.exists(&details("books/a.txt")));
    assert!(!h.storage.exists(".paperpaw/library/walk.list"));
    assert!(!h.storage.exists(".paperpaw/library/catalog.tmp"));
}

#[test]
fn what_was_noted_before_a_walk_goes_with_it() {
    let h = harness(
        MemoryStorage::new()
            .with("books/a.txt", "one")
            .with("books/b.txt", "two"),
    );
    let env = h.env();
    walk(&env);
    // Opened at 10 % (noted), read on to 45 % without closing (the
    // details only), b finished (noted).
    library::record(&env, &reading("books/a.txt", 10.0), true);
    library::record(&env, &reading("books/a.txt", 45.0), false);
    library::set_finished(&h.storage, "books/b.txt", true);
    assert_eq!(books(&h.storage)[0].1, Some(10));
    walk(&env);
    // The details' 45 %, not the 10 % noted before; b as noted; nothing
    // left in the log to override them.
    assert_eq!(
        books(&h.storage),
        [
            ("books/a.txt".to_string(), Some(45)),
            ("books/b.txt".to_string(), Some(100))
        ]
    );
    assert!(catalog::changes(&h.storage).is_empty());
}

#[test]
fn a_book_removed_and_sent_again_starts_afresh() {
    let h = harness(
        MemoryStorage::new()
            .with("books/a.txt", "one")
            .with("books/b.txt", "two"),
    );
    let env = h.env();
    walk(&env);
    library::record(&env, &reading("books/b.txt", 80.0), true);
    assert_eq!(books(&h.storage)[1].1, Some(80));
    // Removed, as the kernel does it: left out at once.
    h.storage.remove("books/b.txt").unwrap();
    library::removed(&h.storage, "books/b.txt");
    catalog::forget(&h.storage, "books/b.txt").unwrap();
    assert_eq!(books(&h.storage).len(), 1);
    // Sent again: caught up, without its old progress.
    h.storage.write("books/b.txt", b"two").unwrap();
    let before = catalog::generation(&h.storage).unwrap();
    catch_up(&env, &["books/b.txt"]);
    assert_eq!(
        books(&h.storage),
        [
            ("books/a.txt".to_string(), None),
            ("books/b.txt".to_string(), None)
        ]
    );
    assert_eq!(catalog::generation(&h.storage), Some(before + 1));
    assert!(catalog::changes(&h.storage).is_empty());
}

#[test]
fn catching_up_folds_the_log_in_and_takes_books_that_came() {
    let h = harness(MemoryStorage::new().with("books/a.txt", "one"));
    let env = h.env();
    walk(&env);
    let index = Index::build(&h.storage, 100, "");
    library::set_finished(&h.storage, "books/a.txt", true);
    h.storage.write("books/new.txt", b"new").unwrap();
    catch_up(&env, &["books/new.txt", "books/gone.txt"]);
    assert_eq!(
        books(&h.storage),
        [
            ("books/a.txt".to_string(), Some(100)),
            ("books/new.txt".to_string(), None)
        ]
    );
    // A new catalog: an index of the old one no longer holds.
    let service = library::service(&env);
    assert!(!service.is_current(&env, &index));
    assert!(service.is_current(&env, &service.index(&env, 100, "")));
}

/// A storage whose appends fail after `left` of them, as a full card's
/// do, and whose reads of `unreadable` fail.
struct Failing<'a> {
    inner: &'a MemoryStorage,
    left: Cell<usize>,
    unreadable: &'static str,
}

impl Storage for Failing<'_> {
    fn read_at(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        if path == self.unreadable {
            return Err(StorageError::Io);
        }
        self.inner.read_at(path, offset, buf)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.inner.size(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.inner.write(path, data)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        if self.left.get() == 0 {
            return Err(StorageError::Full);
        }
        self.left.set(self.left.get() - 1);
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

#[test]
fn an_error_leaves_the_old_catalog_as_it_was() {
    let mut storage = MemoryStorage::new();
    for n in 0..200 {
        storage = storage.with(&format!("books/{n:03}.txt"), "a book");
    }
    let h = harness(storage);
    walk(&h.env());
    let before = h.storage.get(".paperpaw/library/catalog.txt").unwrap();
    // The card fills while the next is made: the old one stays, whole.
    h.storage.write("books/zzz.txt", b"one more book").unwrap();
    let failing = Failing {
        inner: &h.storage,
        left: Cell::new(1),
        unreadable: "",
    };
    let env = h.env_on(&failing);
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    let result = loop {
        match walk.step(&env) {
            Ok(true) => break Ok(()),
            Ok(false) => {}
            Err(e) => break Err(e),
        }
    };
    assert_eq!(result, Err(StorageError::Full));
    assert_eq!(
        h.storage.get(".paperpaw/library/catalog.txt").unwrap(),
        before
    );
    assert!(!h.storage.exists(".paperpaw/library/catalog.tmp"));
    // The catalog unreadable: no walk begins, nothing is changed.
    let failing = Failing {
        inner: &h.storage,
        left: Cell::new(usize::MAX),
        unreadable: ".paperpaw/library/catalog.txt",
    };
    let env = h.env_on(&failing);
    assert!(Walk::start(&env, Vec::new()).is_err());
    assert!(Rewrite::start(&env, Vec::new()).is_ok_and(|r| {
        r.abort(&failing);
        true
    }));
    assert_eq!(
        h.storage.get(".paperpaw/library/catalog.txt").unwrap(),
        before
    );
}

#[test]
fn a_status_that_cannot_be_read_is_not_noted() {
    let h = harness(MemoryStorage::new().with("books/a.txt", "one"));
    walk(&h.env());
    let failing = Failing {
        inner: &h.storage,
        left: Cell::new(usize::MAX),
        unreadable: ".paperpaw/library/favourites.txt",
    };
    h.storage
        .write(".paperpaw/library/favourites.txt", b"books/a.txt\n")
        .unwrap();
    library::note_status(&failing, "books/a.txt");
    assert!(catalog::changes(&h.storage).is_empty(), "nothing noted");
    // Readable: noted as the lists say.
    library::note_status(&h.storage, "books/a.txt");
    let noted = catalog::changes(&h.storage);
    let status = noted.status(paperpaw_sdk::digest::fnv1a64("books/a.txt"));
    assert_eq!(status, Some(Status::new(None, true, false)));
}

/// The old catalog has the books of a folder Y before those of X, and
/// `removed` of Y's are removed since: a walk finds its place again past
/// them, and takes every book of X from the old catalog.
fn taken_after_removed(removed: usize) -> (usize, usize) {
    let storage = MemoryStorage::new();
    storage.create_dir("books").unwrap();
    for (dir, n) in [("X", 40), ("Y", removed)] {
        storage.create_dir(&format!("books/{dir}")).unwrap();
        for i in 0..n {
            storage
                .write(&format!("books/{dir}/b{i:03}.txt"), b"text")
                .unwrap();
        }
    }
    let h = harness(MemoryStorage::new());
    let env = h.env_on(&storage);
    walk(&env);
    // Marked, to tell what the next walk takes from the old catalog.
    let mut records = Vec::new();
    catalog::each(&storage, &mut |r| records.push(r));
    for record in &mut records {
        record.title = String::from("OLD");
    }
    catalog::write(&storage, &records).unwrap();
    for i in 0..removed {
        storage.remove(&format!("books/Y/b{i:03}.txt")).unwrap();
    }
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    while !walk.step(&env).unwrap() {}
    let (mut old, mut all) = (0, 0);
    catalog::each(&storage, &mut |r| {
        all += 1;
        old += usize::from(r.title == "OLD");
    });
    assert_eq!(walk.from_old(), old);
    (old, all)
}

#[test]
fn a_walk_finds_its_place_again_past_books_removed() {
    for removed in [10, 31, 33, 100] {
        assert_eq!(taken_after_removed(removed), (40, 40), "{removed} removed");
    }
}

/// Paused after every step (letting go of what it read), a walk makes
/// the same catalog.
#[test]
fn a_walk_paused_goes_on_where_it_was() {
    let mut storage = MemoryStorage::new();
    for n in 0..30 {
        storage = storage.with(&format!("books/Shelf {}/b{n:02}.txt", n % 3), "text");
    }
    let h = harness(storage);
    let env = h.env();
    walk(&env);
    let made = books(&h.storage);
    library::set_finished(&h.storage, "books/Shelf 1/b04.txt", true);
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    loop {
        walk.pause();
        if walk.step(&env).unwrap() {
            break;
        }
    }
    assert_eq!(walk.from_old(), 30);
    let again = books(&h.storage);
    assert_eq!(again.len(), made.len());
    assert!(again.contains(&("books/Shelf 1/b04.txt".to_string(), Some(100))));
}

#[test]
fn a_walk_paused_after_a_new_book_still_takes_the_others_from_the_old_catalog() {
    // A book new to the library anywhere among nine known, the walk paused
    // after every step: the nine still come from the old catalog.
    for new in ["a-new.txt", "b05-new.txt", "z-new.txt"] {
        let storage = MemoryStorage::new();
        storage.create_dir("books").unwrap();
        for n in 0..9 {
            storage
                .write(&format!("books/b{n:02}.txt"), b"text")
                .unwrap();
        }
        let h = harness(MemoryStorage::new());
        let env = h.env_on(&storage);
        walk(&env);
        storage.write(&format!("books/{new}"), b"text").unwrap();
        let mut walk = Walk::start(&env, Vec::new()).unwrap();
        loop {
            walk.pause();
            if walk.step(&env).unwrap() {
                break;
            }
        }
        assert_eq!(walk.from_old(), 9, "{new}");
        assert_eq!(walk.found(), 10, "{new}");
    }
}

#[test]
fn a_line_too_long_in_the_old_catalog_is_passed_over() {
    let storage = MemoryStorage::new();
    storage.create_dir("books").unwrap();
    for n in 0..5 {
        storage
            .write(&format!("books/b{n:02}.txt"), b"text")
            .unwrap();
    }
    let h = harness(MemoryStorage::new());
    let env = h.env_on(&storage);
    walk(&env);
    // A line of 100 KB (not one the catalog writes) before the third book.
    let path = ".paperpaw/library/catalog.txt";
    let text = paperpaw_sdk::read_file(&storage, path, 1 << 20).unwrap();
    let text = String::from_utf8(text).unwrap();
    let at = text.find("books/b02.txt").unwrap();
    let long = format!("{}{}\n{}", &text[..at], "x".repeat(100 * 1024), &text[at..]);
    storage.write(path, long.as_bytes()).unwrap();
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    while !walk.step(&env).unwrap() {}
    assert_eq!(walk.from_old(), 5);
    assert_eq!(walk.found(), 5);
}

#[test]
fn the_kernels_library_deletes_books_and_says_which_are_gone() {
    let storage = MemoryStorage::new()
        .with("books/a.txt", "a")
        .with("books/b.txt", "b");
    let h = harness(MemoryStorage::new());
    let env = h.env_on(&storage);
    let library = library::service(&env);
    assert!(library.can_delete(&env, "books/a.txt"));
    assert!(!library.can_delete(&env, "books/gone.txt"));
    let deleted = library.delete(&env, &["books/a.txt", "books/gone.txt", "books/b.txt"]);
    assert!(!storage.exists("books/a.txt") && !storage.exists("books/b.txt"));
    // One gone already is forgotten as well; none failed.
    assert!(deleted.failed.is_empty());
    // Each one gone said so: the kernel then forgets it and what is kept.
    let removed: Vec<String> = deleted
        .commands
        .iter()
        .filter_map(|command| match command {
            paperpaw_sdk::Command::Emit(id, params) => paperpaw_sdk::builtin::events::BOOK_REMOVED
                .read(id, params)
                .map(|r| r.path),
            _ => None,
        })
        .collect();
    assert_eq!(removed, ["books/a.txt", "books/gone.txt", "books/b.txt"]);
}

/// A card that cannot be written: what is there reads, nothing goes.
struct ReadOnly(MemoryStorage);

impl Storage for ReadOnly {
    fn read_at(&self, path: &str, at: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        self.0.read_at(path, at, buf)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.0.size(path)
    }
    fn write(&self, _: &str, _: &[u8]) -> Result<(), StorageError> {
        Err(StorageError::Unsupported)
    }
    fn append(&self, _: &str, _: &[u8]) -> Result<(), StorageError> {
        Err(StorageError::Unsupported)
    }
    fn list(&self, dir: &str, visit: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), StorageError> {
        self.0.list(dir, visit)
    }
    fn remove(&self, _: &str) -> Result<(), StorageError> {
        Err(StorageError::Unsupported)
    }
}

#[test]
fn only_books_are_deleted_and_what_fails_says_why() {
    let storage = MemoryStorage::new()
        .with("books/a.txt", "a")
        .with("books/notes.md", "not of a format known")
        .with("paperpaw.ini", "[core]")
        .with(".paperpaw/library/x.txt", "kept");
    let h = harness(MemoryStorage::new());
    let env = h.env_on(&storage);
    let library = library::service(&env);
    // Not a book: of no format known, in PaperPaw's own folder (also by
    // another spelling, or climbing up to it).
    for path in [
        "books/notes.md",
        "paperpaw.ini",
        ".paperpaw/library/x.txt",
        ".PaperPaw/library/x.txt",
        "books/../.paperpaw/library/x.txt",
    ] {
        assert!(!library.can_delete(&env, path), "{path}");
        let deleted = library.delete(&env, &[path]);
        assert!(
            deleted.commands.is_empty() && deleted.failed.is_empty(),
            "{path}"
        );
        assert_eq!(deleted.refused, [path]);
    }
    assert!(storage.exists("books/notes.md") && storage.exists(".paperpaw/library/x.txt"));
    // A card that cannot be written: the book stays, and why is said.
    let locked = ReadOnly(MemoryStorage::new().with("books/a.txt", "a"));
    let env = h.env_on(&locked);
    let deleted = library::service(&env).delete(&env, &["books/a.txt"]);
    assert!(deleted.commands.is_empty());
    assert_eq!(
        deleted.failed,
        [("books/a.txt".to_string(), StorageError::Unsupported)]
    );
    assert!(locked.exists("books/a.txt"));
}

#[test]
fn a_book_without_an_extension_known_by_its_first_bytes_is_deleted() {
    let storage = MemoryStorage::new()
        .with("books/Story", "%STORY and the rest of it")
        .with("books/README", "plain words, of no format");
    let mut h = harness(MemoryStorage::new());
    let mut formats = Formats::new();
    formats
        .add(
            FormatSpec::new(
                Id::new_static("test.story"),
                "Story",
                vec![Cow::Borrowed("story")],
                Box::new(|_, _, _| Err(DocumentError::UnsupportedFormat)),
            )
            .sniff(|bytes| bytes.starts_with(b"%STORY")),
        )
        .unwrap();
    h.formats = formats;
    let env = h.env_on(&storage);
    let library = library::service(&env);
    assert!(library.can_delete(&env, "books/Story"));
    assert!(!library.can_delete(&env, "books/README"));
    let deleted = library.delete(&env, &["books/Story", "books/README"]);
    assert!(!storage.exists("books/Story"));
    assert!(storage.exists("books/README"));
    assert_eq!(deleted.refused, ["books/README"]);
    assert_eq!(deleted.commands.len(), 1);
}
