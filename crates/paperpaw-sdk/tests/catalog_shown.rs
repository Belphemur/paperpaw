//! A first walk shows the books it found so far (#108): where there is no
//! catalog yet, the books found are put into one at checkpoints (20, then
//! each time twice as many, or every two minutes); the catalog it ends
//! with is the one it would have put in place, byte for byte; cut off at
//! any step, the catalog left holds books found, and the next walk makes
//! it whole.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::borrow::Cow;

use paperpaw_sdk::document::DocumentError;
use paperpaw_sdk::library;
use paperpaw_sdk::library::catalog::{self, Walk};
use paperpaw_sdk::{Env, FormatSpec, Formats, Id, Instant, MemoryStorage, Params, Storage};

const CATALOG: &str = ".paperpaw/library/catalog.txt";
const BUILDING: &str = ".paperpaw/library/catalog.tmp";

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

/// A card of `n` books in a few folders.
fn library(n: usize) -> MemoryStorage {
    (0..n).fold(MemoryStorage::new(), |storage, i| {
        storage.with(
            &format!("books/f{}/b{i:04}.txt", i % 5),
            format!("book {i}"),
        )
    })
}

/// The books of the catalog, by path.
fn books(storage: &dyn Storage) -> Vec<String> {
    let mut books = Vec::new();
    catalog::each(storage, &mut |r| books.push(r.path));
    books.sort();
    books
}

/// Every file of `storage`, copied: the card as it is at a cut.
fn snapshot(storage: &MemoryStorage) -> MemoryStorage {
    fn copy(from: &MemoryStorage, to: MemoryStorage, dir: &str) -> MemoryStorage {
        let mut entries = Vec::new();
        from.list(dir, &mut |e| entries.push((e.name.to_string(), e.is_dir)))
            .unwrap();
        entries.into_iter().fold(to, |to, (name, is_dir)| {
            let path = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            if is_dir {
                copy(from, to, &path)
            } else {
                to.with(&path, from.get(&path).unwrap())
            }
        })
    }
    copy(storage, MemoryStorage::new(), "")
}

fn walk(env: &Env<'_>) {
    let mut walk = Walk::start(env, Vec::new()).unwrap();
    while !walk.step(env).unwrap() {}
}

fn walking(env: &Env<'_>) -> bool {
    let key = Id::new_static("core.library-walking");
    library::service(env)
        .extended(env, &key, &Params::new())
        .is_some_and(|p| p.get("walking") == Some("true"))
}

/// Walked step by step: at each checkpoint the catalog lists exactly the
/// books found until then (20, then twice as many each time); it ends as
/// the building file was, byte for byte (what a walk that never shows
/// its books puts in place); while it goes on, the library says so.
#[test]
fn a_first_walk_shows_the_books_found_so_far() {
    let h = harness(library(200));
    let env = h.env();
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    let (mut shown, mut building) = (Vec::new(), None);
    loop {
        let done = walk.step(&env).unwrap();
        if let Some(bytes) = h.storage.get(BUILDING) {
            building = Some(bytes);
        }
        if walk.published() {
            let listed = books(&h.storage);
            let text = String::from_utf8(h.storage.get(CATALOG).unwrap()).unwrap();
            assert!(text.ends_with('\n'), "whole lines");
            assert!(listed.len() <= walk.found());
            shown.push(listed.len());
        }
        if done {
            break;
        }
        assert!(walking(&env) || !h.storage.exists(BUILDING));
    }
    assert!(!walking(&env));
    assert_eq!(shown.len(), 4, "{shown:?}");
    assert!((20..36).contains(&shown[0]), "{shown:?}");
    for pair in shown.windows(2) {
        assert!(pair[1] >= 2 * pair[0], "{shown:?}");
    }
    assert_eq!(h.storage.get(CATALOG), building, "byte for byte");
    assert!(!h.storage.exists(BUILDING));
    assert_eq!(books(&h.storage).len(), 200);
}

/// Few books: no checkpoint, the catalog put in place at the end as
/// always; a walk with a catalog there shows nothing before its end, nor
/// says its books are those found so far.
#[test]
fn only_a_first_walk_of_enough_books_shows_them_before_its_end() {
    let h = harness(library(19));
    let env = h.env();
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    while !walk.step(&env).unwrap() {
        assert!(!walk.published());
        assert!(!h.storage.exists(CATALOG));
    }
    assert_eq!(books(&h.storage).len(), 19);
    // More books, the catalog there: compared with it, none shown early.
    for i in 19..120 {
        h.storage
            .write(&format!("books/g/c{i}.txt"), b"more")
            .unwrap();
    }
    let mut walk = Walk::start(&env, Vec::new()).unwrap();
    while !walk.step(&env).unwrap() {
        assert!(!walk.published());
        assert!(!walking(&env));
        assert_eq!(books(&h.storage).len(), 19);
    }
    assert_eq!(books(&h.storage).len(), 120);
}

/// Every two minutes at least, once 50 books more were found.
#[test]
fn a_slow_walk_shows_its_books_every_two_minutes() {
    let mut h = harness(library(150));
    let mut walk = Walk::start(&h.env(), Vec::new()).unwrap();
    let mut shown = Vec::new();
    loop {
        // A book every two seconds.
        h.now = Instant(h.now.0 + 2_000);
        let env = h.env();
        let done = walk.step(&env).unwrap();
        if walk.published() {
            shown.push(books(&h.storage).len());
        }
        if done {
            break;
        }
    }
    // 20, 40, 80, then 130 or so after two minutes (not 160: 150 books).
    assert!(shown.len() >= 4, "{shown:?}");
    assert!(shown[3] < 160, "{shown:?}");
}

/// Cut off after any step (the power gone), a piece of a checkpoint
/// maybe half written: the catalog left lists books of the library and
/// no other, and the next walk makes it whole, its books said to be
/// those found so far until it ends.
#[test]
fn a_cut_leaves_books_found_and_the_next_walk_makes_them_all() {
    let h = harness(library(100));
    let env = h.env();
    let all = {
        let whole = harness(library(100));
        walk(&whole.env());
        books(&whole.storage)
    };
    let mut walk_on = Walk::start(&env, Vec::new()).unwrap();
    let mut cuts = 0;
    loop {
        let done = walk_on.step(&env).unwrap();
        let _ = walk_on.published();
        let Some(catalog) = h.storage.get(CATALOG) else {
            if done {
                break;
            }
            continue;
        };
        // Cut off as it was, and with its last line cut short.
        for cut in [catalog.len(), catalog.len().saturating_sub(7)] {
            let card = snapshot(&h.storage);
            card.write(CATALOG, &catalog[..cut]).unwrap();
            let after = harness(card);
            let left = books(&after.storage);
            assert!(left.iter().all(|b| all.contains(b)), "{cut}: {left:?}");
            let env = after.env();
            let mut next = Walk::start(&env, Vec::new()).unwrap();
            while !next.step(&env).unwrap() {
                let under_way = after.storage.exists(BUILDING);
                assert_eq!(walking(&env), !done && under_way, "{cut}");
            }
            assert!(!walking(&env), "{cut}");
            assert_eq!(books(&after.storage), all, "{cut}");
            cuts += 1;
        }
        if done {
            break;
        }
    }
    assert!(cuts > 10, "{cuts}");
}
