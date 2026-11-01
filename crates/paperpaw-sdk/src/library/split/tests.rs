#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use super::*;
use crate::MemoryStorage;
use crate::library::{
    self, Book, BookData, BookDataKind, clean_up_unused, key, move_data, moved, remove_data,
    removed,
};

/// Where the details of the book at `path` are kept now.
fn details(path: &str) -> String {
    let key = key(path);
    format!(".paperpaw/library/details/{}/{key}.ini", &key[..2])
}

/// Where earlier versions kept them.
fn old_details(path: &str) -> String {
    format!(".paperpaw/library/books/{}.ini", key(path))
}

/// Where earlier versions kept the cover of the book at `path` made
/// small, of `variant` (`.row` or none).
fn old_thumb(path: &str, variant: &str) -> String {
    format!(".paperpaw/library/covers/{}{variant}.thumb", key(path))
}

fn thumb(path: &str, variant: &str) -> String {
    let key = key(path);
    format!(
        ".paperpaw/library/thumbs/{}/{key}{variant}.thumb",
        &key[..2]
    )
}

/// Details of a book of `size` bytes titled `title`.
fn titled(title: &str, size: u64) -> Book {
    Book {
        size,
        title: Some(title.to_string()),
        ..Book::default()
    }
}

/// Every step of `split` until it is done; how many it took.
fn finish(split: &mut Split, storage: &dyn Storage) -> usize {
    let mut steps = 0;
    while !split.step(storage, &[]).unwrap() {
        steps += 1;
        assert!(steps < 10_000, "the moving does not end");
    }
    steps
}

#[test]
fn details_and_covers_are_kept_in_the_part_of_their_key() {
    let storage = MemoryStorage::new().with("books/a.epub", "12345");
    library::save(&storage, "books/a.epub", &titled("Alice", 5)).unwrap();
    assert!(storage.exists(&details("books/a.epub")));
    assert!(!storage.exists(&old_details("books/a.epub")));
    assert_eq!(
        library::book(&storage, "books/a.epub").and_then(|b| b.title),
        Some("Alice".into())
    );
    assert_eq!(library::split("x", "0123456789abcdef"), "x/01");
    // A key too short for a part (none is) still has one.
    assert_eq!(library::split("x", "1"), "x/00");
}

#[test]
fn details_where_earlier_versions_kept_them_are_read_until_moved() {
    let path = "books/a.epub";
    let storage = MemoryStorage::new().with(path, "12345");
    library::save(&storage, path, &titled("Alice", 5)).unwrap();
    let text = storage.get(&details(path)).unwrap();
    storage.remove(&details(path)).unwrap();
    storage.write(&old_details(path), &text).unwrap();
    assert_eq!(
        library::book(&storage, path).and_then(|b| b.title),
        Some("Alice".into())
    );
    // Changed: written where they go now, the old ones left to the move,
    // which finds them out of date.
    library::save(&storage, path, &titled("Alice II", 5)).unwrap();
    assert!(storage.exists(&details(path)));
    assert_eq!(
        library::book(&storage, path).and_then(|b| b.title),
        Some("Alice II".into())
    );
    assert!(wanted(&storage));
    finish(&mut Split::new(), &storage);
    assert!(!wanted(&storage));
    assert!(!storage.exists(&old_details(path)));
    assert_eq!(
        library::book(&storage, path).and_then(|b| b.title),
        Some("Alice II".into())
    );
}

#[test]
fn a_book_moved_or_removed_takes_what_earlier_versions_kept_along() {
    let (a, b, c) = ("books/a.epub", "books/b.epub", "books/c.epub");
    let storage = MemoryStorage::new().with(a, "12345").with(c, "123");
    library::save(&storage, a, &titled("Alice", 5)).unwrap();
    library::save(&storage, c, &titled("Carol", 3)).unwrap();
    for path in [a, c] {
        let text = storage.get(&details(path)).unwrap();
        storage.remove(&details(path)).unwrap();
        storage.write(&old_details(path), &text).unwrap();
        storage.write(&old_thumb(path, ""), b"a cover").unwrap();
    }
    storage.rename(a, b).unwrap();
    moved(&storage, a, b);
    assert!(storage.exists(&details(b)));
    assert!(storage.exists(&thumb(b, "")));
    assert!(!storage.exists(&old_details(a)) && !storage.exists(&old_thumb(a, "")));
    assert_eq!(
        library::book(&storage, b).and_then(|b| b.title),
        Some("Alice".into())
    );
    removed(&storage, c);
    assert!(!storage.exists(&old_details(c)) && !storage.exists(&old_thumb(c, "")));
    // Nothing is moved back to where the books were.
    finish(&mut Split::new(), &storage);
    assert!(!storage.exists(&details(a)) && !storage.exists(&details(c)));
}

#[test]
fn everything_is_moved_into_the_parts_of_its_keys_and_the_old_folders_go() {
    let storage = MemoryStorage::new();
    let paths: Vec<String> = (0..700).map(|i| format!("books/{i}.epub")).collect();
    for (i, path) in paths.iter().enumerate() {
        storage
            .write(&old_details(path), format!("n={i}").as_bytes())
            .unwrap();
        if i % 3 == 0 {
            storage
                .write(&old_thumb(path, ""), format!("c{i}").as_bytes())
                .unwrap();
            storage
                .write(&old_thumb(path, ".row"), format!("r{i}").as_bytes())
                .unwrap();
        }
    }
    // Not a book's: taken away with the folder, the library's own.
    storage
        .write(".paperpaw/library/books/notes.txt", b"x")
        .unwrap();
    storage
        .write(".paperpaw/library/books/0123.ini", b"x")
        .unwrap();
    let mut split = Split::new();
    let steps = finish(&mut split, &storage);
    // Listed 256 at a time, moved 16 a step.
    assert!(steps >= 700 / 16, "{steps}");
    assert_eq!(split.moved(), 700 + 2 * 234);
    for (i, path) in paths.iter().enumerate() {
        assert_eq!(
            storage.get(&details(path)),
            Some(format!("n={i}").into_bytes())
        );
        if i % 3 == 0 {
            assert_eq!(
                storage.get(&thumb(path, "")),
                Some(format!("c{i}").into_bytes())
            );
            assert_eq!(
                storage.get(&thumb(path, ".row")),
                Some(format!("r{i}").into_bytes())
            );
        }
    }
    assert!(!files::is_dir(&storage, ".paperpaw/library/books"));
    assert!(!files::is_dir(&storage, ".paperpaw/library/covers"));
    assert!(!wanted(&storage));
    // Done, a new one has nothing to do.
    assert_eq!(finish(&mut Split::new(), &storage), 0);
}

#[test]
fn a_move_stopped_anywhere_goes_on_from_there() {
    let storage = MemoryStorage::new();
    let paths: Vec<String> = (0..300).map(|i| format!("books/{i}.epub")).collect();
    for (i, path) in paths.iter().enumerate() {
        storage
            .write(&old_details(path), format!("n={i}").as_bytes())
            .unwrap();
    }
    // A few steps, then PaperPaw stops (asleep, switched off): the next
    // start begins anew with what is left.
    for _ in 0..2 {
        let mut split = Split::new();
        for _ in 0..7 {
            if split.step(&storage, &[]).unwrap() {
                break;
            }
        }
    }
    let moved = paths.iter().filter(|p| storage.exists(&details(p))).count();
    assert!(moved > 0 && moved < paths.len(), "{moved}");
    finish(&mut Split::new(), &storage);
    for (i, path) in paths.iter().enumerate() {
        assert_eq!(
            storage.get(&details(path)),
            Some(format!("n={i}").into_bytes())
        );
        assert!(!storage.exists(&old_details(path)));
    }
}

#[test]
fn where_both_are_there_the_newer_stays() {
    let path = "books/a.epub";
    let storage = MemoryStorage::new()
        .with(&old_details(path), "old")
        .with(&details(path), "new");
    finish(&mut Split::new(), &storage);
    assert_eq!(storage.get(&details(path)), Some(b"new".to_vec()));
    assert!(!storage.exists(&old_details(path)));
}

#[test]
fn nothing_to_move_is_nothing_to_do() {
    let storage = MemoryStorage::new();
    assert!(!wanted(&storage));
    assert!(Split::new().step(&storage, &[]).unwrap());
    // An old folder of nothing to move goes.
    storage
        .write(".paperpaw/library/books/notes.txt", b"x")
        .unwrap();
    storage
        .write(".paperpaw/library/covers/notes.txt", b"x")
        .unwrap();
    assert!(wanted(&storage));
    finish(&mut Split::new(), &storage);
    assert!(!wanted(&storage));
}

/// A storage whose moves of names containing `refused` fail.
struct Refusing<'a> {
    inner: &'a MemoryStorage,
    refused: &'a str,
}

impl Storage for Refusing<'_> {
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
        self.inner.append(path, data)
    }
    fn list(
        &self,
        dir: &str,
        visit: &mut dyn FnMut(paperpaw_platform::DirEntry<'_>),
    ) -> Result<(), StorageError> {
        self.inner.list(dir, visit)
    }
    fn remove(&self, path: &str) -> Result<(), StorageError> {
        self.inner.remove(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        match from.contains(self.refused) {
            true => Err(StorageError::Io),
            false => self.inner.rename(from, to),
        }
    }
    fn remove_dir_all(&self, path: &str) -> Result<(), StorageError> {
        self.inner.remove_dir_all(path)
    }
}

/// A book moved while its details are where earlier versions kept them,
/// and they cannot be moved (a full card, say): they stay, read there.
#[test]
fn details_that_cannot_go_with_their_book_are_kept() {
    let (from, to) = ("books/a.epub", "books/b.epub");
    let inner = MemoryStorage::new().with(from, "12345");
    library::save(&inner, from, &titled("Alice", 5)).unwrap();
    inner.rename(&details(from), &old_details(from)).unwrap();
    let refused = key(from);
    let storage = Refusing {
        inner: &inner,
        refused: &refused,
    };
    moved(&storage, from, to);
    assert!(inner.exists(&old_details(from)));
    assert_eq!(
        library::book(&inner, from).and_then(|b| b.title),
        Some("Alice".into())
    );
    // Moved where it can be: none left behind.
    inner.rename(from, to).unwrap();
    moved(&inner, from, to);
    assert!(!inner.exists(&old_details(from)));
    assert_eq!(
        library::book(&inner, to).and_then(|b| b.title),
        Some("Alice".into())
    );
}

#[test]
fn what_cannot_be_moved_stays_where_it_is_read() {
    let inner = MemoryStorage::new();
    let paths: Vec<String> = (0..40).map(|i| format!("books/{i}.epub")).collect();
    for path in &paths {
        inner.write(&old_details(path), b"x").unwrap();
    }
    let stuck = key(&paths[7]);
    let storage = Refusing {
        inner: &inner,
        refused: &stuck,
    };
    finish(&mut Split::new(), &storage);
    assert!(inner.exists(&old_details(&paths[7])));
    for path in paths.iter().filter(|p| **p != paths[7]) {
        assert!(inner.exists(&details(path)), "{path}");
    }
    // Its folder stays, tried again at the next start.
    assert!(wanted(&inner));
    finish(&mut Split::new(), &storage);
    assert!(inner.exists(&old_details(&paths[7])));
}

#[test]
fn book_data_is_found_in_its_folder_and_in_the_parts_of_its_keys() {
    let (a, b, gone) = ("books/a.epub", "books/b.epub", "books/gone.epub");
    let (ka, kb, kg) = (key(a), key(b), key(gone));
    let split = |k: &str| format!("cache/{}/{k}", &k[..2]);
    let storage = MemoryStorage::new()
        .with(a, "book")
        // As earlier plugins keep it, and split.
        .with(&format!("cache/{ka}.ini"), "flat")
        .with(&format!("{}/0.txt", split(&ka)), "split")
        .with(&format!("{}.row", split(&kg)), "gone")
        .with(&format!("cache/{kg}"), "gone too")
        // Not of a key, in a part: stays.
        .with("cache/ab/notes.txt", "x");
    let data = [BookData::new("cache", "", BookDataKind::Cache)];
    move_data(&storage, &data, a, b);
    assert_eq!(
        storage.get(&format!("cache/{kb}.ini")),
        Some(b"flat".to_vec())
    );
    assert_eq!(
        storage.get(&format!("{}/0.txt", split(&kb))),
        Some(b"split".to_vec())
    );
    assert!(!storage.exists(&format!("cache/{ka}.ini")));
    assert!(!storage.exists(&format!("{}/0.txt", split(&ka))));
    // Cleaned up: of books not there, flat and split.
    storage.rename(a, b).unwrap();
    assert_eq!(
        clean_up_unused(&storage, &data, "books", BookDataKind::Cache),
        Ok(2)
    );
    assert!(storage.exists(&format!("{}/0.txt", split(&kb))));
    assert!(storage.exists("cache/ab/notes.txt"));
    // Removed with its book.
    remove_data(&storage, &data, b);
    assert!(!storage.exists(&format!("cache/{kb}.ini")));
    assert!(!storage.exists(&format!("{}/0.txt", split(&kb))));
}

#[test]
fn caches_kept_in_their_folder_itself_go_those_in_parts_stay() {
    let path = "books/a.epub";
    let k = key(path);
    let storage = MemoryStorage::new()
        .with(&old_details(path), "details")
        .with(&format!("cache/{k}/0.txt"), "old chapter")
        .with(&format!("cache/cbz-{k}"), "old pictures")
        .with(&library::kept_path("cache", "", path), "new")
        .with("cache/settings.ini", "not a book's")
        .with(&format!("notes/{k}"), "a note");
    let data = [
        BookData::new("cache", "", BookDataKind::Cache),
        BookData::new("cache", "cbz-", BookDataKind::Cache),
        // The reader's own: never cleared.
        BookData::new("notes", "", BookDataKind::UserData),
    ];
    let mut split = Split::new();
    while !split.step(&storage, &data).unwrap() {}
    assert!(!storage.exists(&format!("cache/{k}/0.txt")));
    assert!(!storage.exists(&format!("cache/cbz-{k}")));
    assert_eq!(
        storage.get(&library::kept_path("cache", "", path)),
        Some(b"new".to_vec())
    );
    assert!(storage.exists("cache/settings.ini"));
    assert!(storage.exists(&format!("notes/{k}")));
    assert_eq!(
        library::kept_path("cache/", "md-", path),
        format!("cache/{}/md-{k}", &k[..2])
    );
}

#[test]
fn only_names_of_keys_are_moved() {
    for (name, kept) in [
        ("0123456789abcdef.ini", true),
        ("0123456789abcdef.row.thumb", true),
        ("0123456789abcdef", false),
        ("0123456789ABCDEF.ini", false),
        ("0123456789abcdeg.ini", false),
        ("0123456789abcdef.", false),
        ("é123456789abcdef.ini", false),
        ("", false),
    ] {
        assert_eq!(is_kept_name(name), kept, "{name}");
    }
}
