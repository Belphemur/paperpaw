//! A book opened only for its details (the library's walk, #98): its
//! metadata, chapters and cover read from the archive with nothing
//! written; the package goes into the cache the first time the book is
//! read, beside a cover made for the library before.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{Recording, blocks};
use paperpaw_plugin_format_epub::EpubDocument;
use paperpaw_sdk::document::Document;
use paperpaw_sdk::{MemoryStorage, Storage};

const KAFKA: &[u8] = include_bytes!("../../../samples/library/books/die-verwandlung.epub");
const ALICE: &[u8] = include_bytes!("../../../samples/library/books/alice-in-wonderland.epub");

/// Where format-epub keeps the package of a book.
fn package_of(storage: &dyn Storage) -> Vec<String> {
    let mut found = Vec::new();
    files(storage, ".paperpaw/plugins/format-epub", &mut found);
    found.retain(|path| path.ends_with("/package.ini"));
    found
}

fn files(storage: &dyn Storage, dir: &str, out: &mut Vec<String>) {
    let mut entries = Vec::new();
    let _ = storage.list(dir, &mut |entry| {
        entries.push((entry.name.to_string(), entry.is_dir));
    });
    for (name, is_dir) in entries {
        let path = format!("{dir}/{name}");
        if is_dir {
            files(storage, &path, out);
        } else {
            out.push(path);
        }
    }
}

#[test]
fn opened_for_its_details_nothing_is_written() {
    let storage = Recording::new(MemoryStorage::new().with("books/kafka.epub", KAFKA));
    let doc = EpubDocument::open(&storage, "books/kafka.epub").unwrap();
    let metadata = doc.metadata();
    assert_eq!(metadata.title.as_deref(), Some("Die Verwandlung"));
    assert_eq!(metadata.authors, ["Franz Kafka"]);
    assert!(doc.sections() > 0);
    assert_eq!(*storage.written.borrow(), Vec::<String>::new());
    assert!(package_of(&storage).is_empty());
}

/// Read, the package goes into the cache, with the contents; opened
/// again, it is read from there and nothing is written.
#[test]
fn the_first_read_puts_the_package_in_the_cache() {
    let storage = Recording::new(MemoryStorage::new().with("books/kafka.epub", KAFKA));
    let doc = EpubDocument::open(&storage, "books/kafka.epub").unwrap();
    let first = blocks(&doc, &storage);
    assert!(!first.is_empty());
    assert_eq!(package_of(&storage).len(), 1);
    let toc = doc.toc_len(&storage);
    assert!(toc > 0);
    storage.written.borrow_mut().clear();
    let again = EpubDocument::open(&storage, "books/kafka.epub").unwrap();
    assert_eq!(again.metadata(), doc.metadata());
    assert_eq!(again.toc_len(&storage), toc);
    assert_eq!(blocks(&again, &storage), first);
    assert_eq!(*storage.written.borrow(), Vec::<String>::new());
}

/// The contents, wanted first (before any chapter), are there.
#[test]
fn the_contents_are_there_before_any_chapter() {
    let storage = MemoryStorage::new().with("books/alice.epub", ALICE);
    let doc = EpubDocument::open(&storage, "books/alice.epub").unwrap();
    let len = doc.toc_len(&storage);
    assert!(len > 0);
    let first = doc.toc_entry(&storage, 0).unwrap();
    assert!(!first.title.is_empty());
    assert!(doc.toc_target(&storage, len - 1).is_some());
}

/// A cover made for the library (before the book was read) stays when
/// the book is read: the library's details point at it.
#[test]
fn a_cover_made_for_the_library_stays_when_the_book_is_read() {
    let storage = MemoryStorage::new().with("books/alice.epub", ALICE);
    let doc = EpubDocument::open(&storage, "books/alice.epub").unwrap();
    let cover = doc.cover(&storage).unwrap().unwrap();
    assert!(package_of(&storage).is_empty());
    let read = EpubDocument::open(&storage, "books/alice.epub").unwrap();
    assert!(!blocks(&read, &storage).is_empty());
    assert!(storage.size(&cover.path).is_ok());
    let again = EpubDocument::open(&storage, "books/alice.epub").unwrap();
    assert_eq!(again.cover(&storage).unwrap(), Some(cover));
}

/// Another book copied over one whose details were read (its cover made,
/// its package never written): its own metadata and cover, and its own
/// chapters once read.
#[test]
fn a_book_replaced_after_its_details_were_read_is_read_anew() {
    let storage = MemoryStorage::new().with("books/book.epub", ALICE);
    let alice = EpubDocument::open(&storage, "books/book.epub").unwrap();
    let alice_cover = alice.cover(&storage).unwrap().unwrap();
    let alice_picture = storage.size(&alice_cover.path).unwrap();
    drop(alice);
    storage.write("books/book.epub", KAFKA).unwrap();
    let kafka = EpubDocument::open(&storage, "books/book.epub").unwrap();
    assert_eq!(kafka.metadata().title.as_deref(), Some("Die Verwandlung"));
    let kafka_cover = kafka.cover(&storage).unwrap().unwrap();
    let kafka_picture = storage.size(&kafka_cover.path).unwrap();
    assert!(
        (kafka_cover.width, kafka_cover.height, kafka_picture)
            != (alice_cover.width, alice_cover.height, alice_picture),
        "the old book's cover stays: {kafka_cover:?}"
    );
    let text = blocks(&kafka, &storage);
    assert!(
        text.iter()
            .any(|(_, block)| block.text.contains("Gregor") || block.text.contains("Samsa"))
    );
    // Read, then replaced again: the package and chapters go.
    storage.write("books/book.epub", ALICE).unwrap();
    let alice = EpubDocument::open(&storage, "books/book.epub").unwrap();
    assert!(package_of(&storage).is_empty());
    assert!(
        blocks(&alice, &storage)
            .iter()
            .all(|(_, block)| !block.text.contains("Samsa"))
    );
}
