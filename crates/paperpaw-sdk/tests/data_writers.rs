//! Every SDK data file of `docs/data-files.md` has its version 1 in
//! `testdata` (favourites too), and what the current writers make of it
//! the readers read as before: version 1 read, written again and read
//! once more gives the same.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use paperpaw_sdk::builtin::events::Reading;
use paperpaw_sdk::secrets::{FILE, Secrets};
use paperpaw_sdk::{MemoryStorage, annotations, library};

const BOOK_RECORD: &str = ".paperpaw/library/details/0c/0c6afbf2778c9bbf.ini";

#[test]
fn favourites_of_version_1_are_read() {
    let storage = MemoryStorage::new().with(
        ".paperpaw/library/favourites.txt",
        include_str!("../testdata/favourites.txt.v1"),
    );
    assert_eq!(
        library::favourites(&storage),
        [
            "books/Ångström & Co.epub",
            "books/b.epub",
            "notes/ with spaces .txt"
        ]
    );
    // Written again: the same paths, a line each.
    library::set_favourite(&storage, "books/b.epub", true);
    let text = storage.get(".paperpaw/library/favourites.txt").unwrap();
    assert_eq!(
        String::from_utf8(text).unwrap(),
        "books/b.epub\nbooks/Ångström & Co.epub\nnotes/ with spaces .txt"
    );
}

#[test]
fn opened_books_of_version_1_are_written_as_read() {
    let mut h = common::Harness::new();
    h.storage = MemoryStorage::new().with(
        ".paperpaw/library/opened.txt",
        include_str!("../testdata/opened.txt.v1"),
    );
    let env = h.env();
    let reading = Reading::new(
        "books/c.epub".into(),
        String::new(),
        0.0,
        0,
        0,
        String::new(),
    );
    library::record(&env, &reading, true);
    assert_eq!(
        library::opened_books(&h.storage),
        ["books/c.epub", "books/b.epub", "books/a b.txt"]
    );
}

#[test]
fn a_book_record_of_version_1_is_written_and_read_again() {
    let storage = MemoryStorage::new()
        .with("books/a.epub", b"12345".to_vec())
        .with(BOOK_RECORD, include_str!("../testdata/library-book.ini.v1"));
    let first = library::book(&storage, "books/a.epub").unwrap();
    library::save(&storage, "books/a.epub", &first).unwrap();
    let again = library::book(&storage, "books/a.epub").unwrap();
    assert_eq!(again, first);
    // The quoted series keeps its spaces, the plugins' fields their
    // section: as version 1 wrote them.
    let text = String::from_utf8(storage.get(BOOK_RECORD).unwrap()).unwrap();
    assert!(text.contains("series = \"  Classics \""), "{text}");
    assert!(text.contains("[field]\ncalibre.rating = 4"), "{text}");
    for key in [
        "size = 5",
        "tag.1 = Novel",
        "tag.2 = Russia",
        "cover-width = 120",
        "progress = 42",
    ] {
        assert!(text.contains(key), "{key} in {text}");
    }
}

#[test]
fn annotations_of_version_1_are_written_and_read_again() {
    let path = annotations::file("books/a.epub");
    let storage = MemoryStorage::new().with(&path, include_str!("../testdata/annotations.txt.v1"));
    let first = annotations::load(&storage, "books/a.epub");
    assert_eq!(first.len(), 2);
    annotations::save(&storage, "books/a.epub", &first).unwrap();
    assert_eq!(annotations::load(&storage, "books/a.epub"), first);
    // Lines without times are written with them now: 10 fields.
    let text = String::from_utf8(storage.get(&path).unwrap()).unwrap();
    assert!(
        text.lines().all(|line| line.split('\t').count() == 10),
        "{text}"
    );
}

#[test]
fn secrets_of_version_1_are_written_as_they_were() {
    let v1 = include_str!("../testdata/secrets.txt.v1");
    assert_eq!(Secrets::decode(v1).encode(), v1);
    assert_eq!(FILE, ".paperpaw/secrets.txt");
}
