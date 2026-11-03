//! Bringing CrossPoint's reading data over, with the real kernel: a card
//! with CrossPoint's folder on it, the sync screen's row for it, and
//! what the statistics hold afterwards. The records themselves are made
//! here byte for byte, as CrossPoint writes them (see the plugin's own
//! tests for the decoding).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{App, DIR, X4};
use paperpaw_sdk::{MemoryStorage, Params, Storage};

/// 14 October 2026 (a Wednesday), 10:41 in Berlin (UTC+2).
const NOW: i64 = 1_791_967_260_000;
/// That day, in days since 1970.
const TODAY: i64 = 20_740;

/// The paths hashed the way CrossPoint named its folders by, as
/// `libstdc++`'s `std::hash<std::string>` does on a 32-bit machine
/// (pinned in the plugin's own tests too).
const ANNA_HASH: u32 = 735_186_958; // "/books/anna.epub"
const XTC_HASH: u32 = 1_619_920_147; // "/books/serial.xtc"
const GONE_HASH: u32 = 208_911_342; // "/elsewhere/gone.epub"

/// A per-book record of version 8: two sessions of half an hour, read
/// halfway through, its words a minute and its sessions recorded.
fn book_record(percent: u8) -> Vec<u8> {
    let mut b = vec![0u8; 135];
    b[0] = 8;
    b[1..3].copy_from_slice(&2u16.to_le_bytes());
    b[3..7].copy_from_slice(&3_600u32.to_le_bytes());
    b[7..11].copy_from_slice(&150u32.to_le_bytes());
    b[108] = percent;
    for i in 0..15u16 {
        b[77 + 2 * i as usize..79 + 2 * i as usize].copy_from_slice(&(200 + i).to_le_bytes());
    }
    b[75..77].copy_from_slice(&15u16.to_le_bytes());
    b[107] = 15;
    b[109..111].copy_from_slice(&1_500u16.to_le_bytes());
    b[111..113].copy_from_slice(&4u16.to_le_bytes());
    for i in 0..4u16 {
        b[113 + 2 * i as usize..115 + 2 * i as usize].copy_from_slice(&(1_400 + i).to_le_bytes());
    }
    b[133] = 4;
    b
}

/// The global record of version 6: half an hour read on the day `TODAY`
/// and a quarter of an hour the day before.
fn global_record() -> Vec<u8> {
    let mut b = vec![0u8; 407];
    b[0] = 6;
    b[61..65].copy_from_slice(&(TODAY as u32).to_le_bytes());
    b[225..227].copy_from_slice(&30u16.to_le_bytes());
    b[227..229].copy_from_slice(&15u16.to_le_bytes());
    b
}

/// The index of finished books, with the one book finished.
fn finished_record(path: &str) -> Vec<u8> {
    let hash: u64 = 14_695_981_039_346_656_037;
    let mut key = hash;
    for byte in path.bytes() {
        key ^= u64::from(byte);
        key = key.wrapping_mul(1_099_511_628_211);
    }
    let mut b = vec![];
    b.extend_from_slice(b"CPFB");
    b.push(3);
    b.push(1);
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&key.to_le_bytes());
    b.extend_from_slice(&1_800u32.to_le_bytes());
    b.extend_from_slice(&2026u16.to_le_bytes());
    b.push(10);
    b.push(13);
    b.extend_from_slice(&2026u16.to_le_bytes());
    b.push(10);
    b.push(14);
    b.extend_from_slice(&5u16.to_le_bytes());
    b.extend_from_slice(b"Anna ");
    b.extend_from_slice(&0u16.to_le_bytes());
    b
}

/// Where the EPUB book was left: the second file of the book, its third
/// page of twelve.
fn progress() -> Vec<u8> {
    let mut b = vec![0u8; 10];
    b[0..2].copy_from_slice(&2u16.to_le_bytes());
    b[2..4].copy_from_slice(&3u16.to_le_bytes());
    b[4..6].copy_from_slice(&12u16.to_le_bytes());
    b
}

/// A card with CrossPoint's data on it and the books themselves: one
/// EPUB read halfway, one XTC book at page 41, and one book CrossPoint
/// kept no folder of.
fn crosspoint_card() -> MemoryStorage {
    let storage = MemoryStorage::new()
        .with("books/anna.epub", vec![0u8; 4096])
        .with("books/serial.xtc", vec![7u8; 4096])
        .with("books/other.epub", vec![9u8; 4096]);
    let anna = format!(".crosspoint/epub_{ANNA_HASH}");
    storage
        .write(&format!("{anna}/stats_v8.bin"), &book_record(45))
        .unwrap();
    storage
        .write(&format!("{anna}/progress.bin"), &progress())
        .unwrap();
    // The book's own words of itself, for its title and authors.
    let mut meta = vec![10u8];
    meta.extend_from_slice(&100u32.to_le_bytes());
    meta.extend_from_slice(&17u16.to_le_bytes());
    meta.extend_from_slice(&2u16.to_le_bytes());
    for text in ["Anna Karenina", "Leo Tolstoy", "ru"] {
        meta.extend_from_slice(&(text.len() as u32).to_le_bytes());
        meta.extend_from_slice(text.as_bytes());
    }
    storage.write(&format!("{anna}/book.bin"), &meta).unwrap();
    let xtc = format!(".crosspoint/xtc_{XTC_HASH}");
    storage
        .write(&format!("{xtc}/stats_v8.bin"), &book_record(42))
        .unwrap();
    storage
        .write(&format!("{xtc}/progress.bin"), &41u32.to_le_bytes())
        .unwrap();
    // A folder whose book is not on the card.
    let gone = format!(".crosspoint/epub_{GONE_HASH}");
    storage
        .write(&format!("{gone}/stats_v8.bin"), &book_record(0))
        .unwrap();
    storage
        .write(".crosspoint/global_stats.bin", &global_record())
        .unwrap();
    storage
        .write(
            ".crosspoint/finished_books.bin",
            &finished_record("/books/anna.epub"),
        )
        .unwrap();
    storage.write(
        ".crosspoint/recent.json",
        br#"{"books":[{"path":"/books/anna.epub","title":"Anna Karenina","author":"Leo Tolstoy","coverBmpPath":""}]}"#,
    ).unwrap();
    storage
}

/// The books after the import, from `books.ini`.
fn books_of(storage: &dyn paperpaw_sdk::Storage) -> Vec<(String, String, i64)> {
    let data = paperpaw_sdk::read_file(storage, &format!("{DIR}/books.ini"), 1 << 20).unwrap();
    let text = String::from_utf8(data).unwrap();
    let mut out = Vec::new();
    let mut title = String::new();
    let mut path = String::new();
    let mut finished = 0;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(" = ") else {
            continue;
        };
        match key {
            "title" => title = value.to_string(),
            "path" => path = value.to_string(),
            "finished" => finished = value.parse().unwrap_or(0),
            _ => {}
        }
        if key == "finished" {
            out.push((
                core::mem::take(&mut title),
                core::mem::take(&mut path),
                finished,
            ));
        }
    }
    out
}

#[test]
fn the_sync_screen_takes_crosspoint_s_reading_over() {
    let storage = crosspoint_card();
    let mut app = App::new(X4, storage, Some(NOW), "", false);
    app.open("stats.sync", Params::new());
    // The row is there, for CrossPoint's data is on the card.
    let shown = app.shown();
    assert!(shown.contains("Take over from CrossPoint"), "{shown}");
    app.tap_on("Take over from CrossPoint");
    let shown = app.shown();
    assert!(shown.contains("Taken over:"), "{shown}");
    // The books came over: the EPUB with its title and authors, marked
    // finished; the XTC book beside it.
    let books = books_of(&app.storage);
    assert_eq!(books.len(), 2, "{books:?}");
    assert!(
        books
            .iter()
            .any(|(title, path, finished)| title == "Anna Karenina"
                && path == "books/anna.epub"
                && *finished > 0)
    );
    assert!(books.iter().any(|(title, _, _)| title.contains("serial")));
    // Where the EPUB was left: the second file of the book, 45 percent
    // in. The XTC book at its 42nd page (from 0).
    let anna = paperpaw_sdk::positions::load(&app.storage, "books/anna.epub").unwrap();
    assert_eq!(anna.at.section, 2);
    assert_eq!(anna.percent, 45.0);
    let serial = paperpaw_sdk::positions::load(&app.storage, "books/serial.xtc").unwrap();
    assert_eq!(serial.at.section, 41);
    // Nothing of CrossPoint's was changed.
    assert!(
        app.storage
            .0
            .get(".crosspoint/global_stats.bin")
            .is_some()
    );
    // And the row is there again, bringing nothing twice.
    app.open("stats.sync", Params::new());
    app.tap_on("Take over from CrossPoint");
    assert!(app.shown().contains("already"), "{}", app.shown());
}

#[test]
fn the_row_is_nowhere_without_crosspoint_s_data() {
    let mut app = App::new(common::X4, MemoryStorage::new(), Some(NOW), "", false);
    app.open("stats.sync", Params::new());
    assert!(
        !app.shown().contains("Take over from CrossPoint"),
        "{}",
        app.shown()
    );
}
