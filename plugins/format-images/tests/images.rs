//! Pictures as books: the pages of a folder, in order, from the picture
//! opened; pictures decoded on their pages; and what is done with files
//! that are damaged, too large, or no pictures at all.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use paperpaw_plugin_format_images::ImagesDocument;
use paperpaw_sdk::document::{Block, BlockKind, Document, DocumentError, Locator};
use paperpaw_sdk::image::Fit;
use paperpaw_sdk::{MemoryStorage, OpenOptions, Storage};
use proptest::prelude::*;

const PNG: &[u8] = include_bytes!("../../../crates/paperpaw-image/tests/data/gray.png");
const JPG: &[u8] = include_bytes!("../../../crates/paperpaw-image/tests/data/alice-cover.jpg");
const GIF: &[u8] = include_bytes!("../../../crates/paperpaw-image/tests/data/alice-cover.gif");

fn open(storage: &MemoryStorage, path: &str) -> Result<ImagesDocument, DocumentError> {
    ImagesDocument::open(storage, path, &OpenOptions::default())
}

fn page(doc: &ImagesDocument, storage: &MemoryStorage, index: u32) -> Block {
    doc.block_at(storage, Locator::new(index, 0))
        .unwrap()
        .expect("a page")
}

/// The names of the pages that are shown as text: the pictures that are
/// not pictures.
fn named(doc: &ImagesDocument, storage: &MemoryStorage) -> Vec<String> {
    (0..doc.sections())
        .map(|i| page(doc, storage, i).text)
        .collect()
}

#[test]
fn a_picture_is_a_book_of_one_page() {
    let storage = MemoryStorage::new().with("books/a b.png", PNG.to_vec());
    let doc = open(&storage, "books/a b.png").unwrap();
    assert_eq!(doc.metadata().title.as_deref(), Some("a b"));
    assert_eq!(doc.sections(), 1);
    assert!(doc.whole_pages());
    let block = page(&doc, &storage, 0);
    assert_eq!(block.kind, BlockKind::Image);
    let image = block.image.expect("a picture");
    assert!(image.width > 0 && image.height > 0);
    assert!(
        doc.block_at(&storage, Locator::new(1, 0))
            .unwrap()
            .is_none()
    );
    assert!(
        doc.block_at(&storage, Locator::new(0, 1))
            .unwrap()
            .is_none()
    );
    assert!(doc.cover(&storage).unwrap().is_some());
}

#[test]
fn every_kind_of_picture_is_shown() {
    let storage = MemoryStorage::new()
        .with("p/1.png", PNG.to_vec())
        .with("p/2.jpg", JPG.to_vec())
        .with("p/3.gif", GIF.to_vec());
    let doc = open(&storage, "p/1.png").unwrap();
    assert_eq!(doc.sections(), 3);
    for i in 0..3 {
        assert_eq!(page(&doc, &storage, i).kind, BlockKind::Image, "page {i}");
    }
}

#[test]
fn a_picture_is_fit_to_the_screen() {
    let storage = MemoryStorage::new().with("a.jpg", JPG.to_vec());
    let options = OpenOptions::default().image_fit(Fit::side(64));
    let doc = ImagesDocument::open(&storage, "a.jpg", &options).unwrap();
    let image = page(&doc, &storage, 0).image.unwrap();
    assert!(image.width.max(image.height) <= 64, "{image:?}");
}

#[test]
fn the_pages_are_the_pictures_from_the_opened_one_on_in_natural_order() {
    let mut storage = MemoryStorage::new();
    for name in [
        "10.gif",
        "2.png",
        "1.jpg",
        "02b.jpg",
        "x/9.png",
        "other/3.png",
    ] {
        storage = storage.with(&format!("p/{name}"), b"not a picture".to_vec());
    }
    storage = storage
        .with("p/notes.txt", b"text".to_vec())
        .with("p/.hidden.jpg", b"x".to_vec())
        .with("p/._1.jpg", b"x".to_vec());
    let all = open(&storage, "p/1.jpg").unwrap();
    assert_eq!(
        named(&all, &storage),
        ["1.jpg", "2.png", "02b.jpg", "10.gif"]
    );
    // Opened in the middle, it starts there.
    let later = open(&storage, "p/2.png").unwrap();
    assert_eq!(named(&later, &storage), ["2.png", "02b.jpg", "10.gif"]);
    let last = open(&storage, "p/10.gif").unwrap();
    assert_eq!(last.sections(), 1);
}

#[test]
fn the_folder_of_a_picture_in_the_library_folder_is_the_library_folder() {
    let storage = MemoryStorage::new()
        .with("1.png", b"x".to_vec())
        .with("2.png", b"x".to_vec())
        .with("books/3.png", b"x".to_vec());
    assert_eq!(open(&storage, "1.png").unwrap().sections(), 2);
}

#[test]
fn what_is_no_picture_is_refused() {
    let storage = MemoryStorage::new()
        .with("a.txt", b"x".to_vec())
        .with("noextension", b"x".to_vec());
    assert!(matches!(
        open(&storage, "a.txt"),
        Err(DocumentError::Malformed(_))
    ));
    assert!(open(&storage, "noextension").is_err());
    assert!(open(&storage, "missing.png").is_err());
}

#[test]
fn a_file_that_is_no_picture_is_a_page_with_its_name() {
    let storage = MemoryStorage::new().with("p/a.jpg", b"definitely not a jpeg".to_vec());
    let doc = open(&storage, "p/a.jpg").unwrap();
    let block = page(&doc, &storage, 0);
    assert_eq!(block.kind, BlockKind::Paragraph);
    assert_eq!(block.text, "a.jpg");
    assert!(block.image.is_none());
    assert!(doc.cover(&storage).unwrap().is_none());
    // It is marked, and not read again.
    let marks = storage_files(&storage, ".bad");
    assert_eq!(marks, 2, "the page and the cover");
}

fn storage_files(storage: &MemoryStorage, suffix: &str) -> usize {
    let mut count = 0;
    let mut dirs = vec![String::new()];
    while let Some(dir) = dirs.pop() {
        storage
            .list(&dir, &mut |entry| {
                let path = if dir.is_empty() {
                    entry.name.to_string()
                } else {
                    format!("{dir}/{}", entry.name)
                };
                if entry.is_dir {
                    dirs.push(path);
                } else if path.ends_with(suffix) {
                    count += 1;
                }
            })
            .unwrap();
    }
    count
}

#[test]
fn a_file_too_large_is_named_not_decoded() {
    let mut big = PNG.to_vec();
    big.resize(25 << 20, 0);
    let storage = MemoryStorage::new().with("a.png", big);
    let doc = open(&storage, "a.png").unwrap();
    let block = page(&doc, &storage, 0);
    assert_eq!(block.text, "a.png");
    assert_eq!(block.kind, BlockKind::Paragraph);
}

#[test]
fn a_folder_of_a_thousand_pictures_has_the_first_three_hundred() {
    let mut storage = MemoryStorage::new();
    for n in (0..1000).rev() {
        storage = storage.with(&format!("p/{n:04}.png"), b"x".to_vec());
    }
    let doc = open(&storage, "p/0000.png").unwrap();
    assert_eq!(doc.sections(), 300);
    assert_eq!(page(&doc, &storage, 299).text, "0299.png");
    // From the middle: the next three hundred.
    let later = open(&storage, "p/0500.png").unwrap();
    assert_eq!(later.sections(), 300);
    assert_eq!(page(&later, &storage, 0).text, "0500.png");
    assert_eq!(page(&later, &storage, 299).text, "0799.png");
}

#[test]
fn the_picture_is_decoded_once() {
    let storage = MemoryStorage::new().with("a.png", PNG.to_vec());
    let doc = open(&storage, "a.png").unwrap();
    let first = page(&doc, &storage, 0).image.unwrap();
    // The cache has it: the file may change under the same name and size
    // and the page is the same, from the cache.
    storage.write("a.png", &vec![0u8; PNG.len()]).unwrap();
    let doc = open(&storage, "a.png").unwrap();
    assert_eq!(page(&doc, &storage, 0).image, Some(first));
    // Another size is another picture.
    storage.write("a.png", b"short").unwrap();
    let doc = open(&storage, "a.png").unwrap();
    assert_eq!(page(&doc, &storage, 0).text, "a.png");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn damaged_pictures_do_no_harm(
        which in 0usize..3,
        hits in proptest::collection::vec((0usize..40_000, any::<u8>()), 1..24),
        cut in proptest::option::of(0usize..40_000),
    ) {
        let mut bytes = [PNG, JPG, GIF][which].to_vec();
        for (at, value) in hits {
            let at = at % bytes.len();
            bytes[at] = value;
        }
        if let Some(cut) = cut {
            bytes.truncate(cut % bytes.len());
        }
        let name = ["a.png", "a.jpg", "a.gif"][which];
        let storage = MemoryStorage::new().with(name, bytes);
        let doc = open(&storage, name).unwrap();
        let block = page(&doc, &storage, 0);
        prop_assert!(block.kind == BlockKind::Image || block.text == name);
        let _ = doc.cover(&storage);
    }

    #[test]
    fn random_bytes_are_named(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let storage = MemoryStorage::new().with("a.png", bytes);
        let doc = open(&storage, "a.png").unwrap();
        let block = page(&doc, &storage, 0);
        prop_assert!(block.kind == BlockKind::Image || block.text == "a.png");
    }
}

/// The CRC-32 of PNG chunks.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn a_picture_claiming_a_huge_size_is_no_harm() {
    // A PNG of 100,000 x 100,000 pixels in the header of a tiny one, with
    // the header's checksum made right.
    let mut png = PNG.to_vec();
    png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    // A JPEG of 65,535 x 65,535.
    let mut jpg = JPG.to_vec();
    let sof = jpg
        .windows(2)
        .position(|w| w == [0xff, 0xc0] || w == [0xff, 0xc2])
        .expect("a frame header");
    jpg[sof + 5..sof + 9].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
    for (name, bytes) in [("a.png", png), ("a.jpg", jpg)] {
        let storage = MemoryStorage::new().with(name, bytes);
        let doc = open(&storage, name).unwrap();
        let block = page(&doc, &storage, 0);
        // Shown at the size the screen takes, or named: never a crash.
        if let Some(image) = block.image {
            assert!(image.width <= 1600 && image.height <= 1600, "{image:?}");
        } else {
            assert_eq!(block.text, name);
        }
    }
}
