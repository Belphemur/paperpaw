//! Searching a large library on the X4 (#111): by title, author and tag,
//! the heap it takes when every book matches, and where the Find screen's
//! focus starts.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use paperpaw_sdk::Id;
use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::library::{
    Book,
    catalog::{self, Record},
};

struct Counting;
static NOW: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

// SAFETY: passes every call on to the system's allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as the caller asked.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let now = NOW.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(now, Ordering::Relaxed);
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as the caller asked.
        unsafe { System.dealloc(ptr, layout) };
        NOW.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn growth(run: impl FnOnce()) -> usize {
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    run();
    PEAK.load(Ordering::Relaxed) - before
}

const SEARCH: Id = Id::new_static("files.search");
const FIND: Id = Id::new_static("files.list.find");

fn path(n: usize) -> String {
    format!(
        "books/Folder {:03}/A book of the library {n:05}.txt",
        n % 300
    )
}

/// `books` books, every 100th tagged `scifi`; with a catalog made when
/// `catalog` is set (else the walk after the start has to make it).
fn library(books: usize, with_catalog: bool) -> App {
    let paths: Vec<String> = (0..books).map(path).collect();
    let files: Vec<(&str, &[u8])> = paths.iter().map(|p| (p.as_str(), &b"t"[..])).collect();
    let storage = Shared::memory("", &files);
    if with_catalog {
        let mut records: Vec<Record> = paths
            .iter()
            .enumerate()
            .map(|(n, path)| {
                let mut book = Book::default()
                    .title(Some(format!("A book of the library {n:05}")))
                    .author(Some(format!("Somebody Writing {:04}", n / 2)))
                    .size(1);
                if n % 100 == 0 {
                    book = book.tags(vec!["scifi".to_string()]);
                }
                Record::of(path, &book)
            })
            .collect();
        records.sort_by(|a, b| a.path.cmp(&b.path));
        storage.create_dir(".paperpaw").unwrap();
        storage.create_dir(".paperpaw/library").unwrap();
        catalog::write(&storage, &records).unwrap();
    }
    App::new(X4, storage)
}

fn searched(app: &mut App, words: &str) {
    app.will_type(words);
    app.run(SEARCH);
    app.wait(2_000);
}

#[test]
fn a_search_finds_one_book_of_three_thousand_by_title_and_by_author() {
    let _turn = turn();
    let mut app = library(3_000, true);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    searched(&mut app, "library 02999");
    assert_eq!(app.screen(), "files.list.shelf", "{}", app.described());
    assert!(
        app.shows("A book of the library 02999"),
        "{:?}",
        app.labels()
    );
    app.run(actions::BACK);
    searched(&mut app, "somebody 1499");
    assert!(
        app.shows("library 02998") && app.shows("library 02999"),
        "{:?}",
        app.labels()
    );
}

#[test]
fn a_search_finds_a_book_by_its_tag_as_the_question_says() {
    let _turn = turn();
    let mut app = library(300, true);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    // "Title, author, series or tag".
    searched(&mut app, "scifi");
    assert!(
        app.shows("A book of the library 00100"),
        "{:?}",
        app.labels()
    );
    assert!(app.shows("3 books"), "{:?}", app.labels());
    app.run(actions::BACK);
    searched(&mut app, "scifi 200");
    assert!(
        app.shows("A book of the library 00200"),
        "{:?}",
        app.labels()
    );
    assert!(
        !app.shows("A book of the library 00100"),
        "{:?}",
        app.labels()
    );
}

#[test]
fn a_search_that_matches_every_book_lists_the_first_by_title() {
    let _turn = turn();
    let mut app = library(3_000, true);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    searched(&mut app, "book");
    let labels = app.labels();
    assert!(
        labels
            .iter()
            .any(|l| l == "First 500 of 3000 books · narrow the search"),
        "{labels:?}"
    );
    // The first by title, not the first the catalog held.
    assert!(app.shows("A book of the library 00000"), "{labels:?}");
    // A narrower search lists what it finds, with no note.
    app.run(actions::BACK);
    searched(&mut app, "library 01230");
    let labels = app.labels();
    assert!(labels.iter().any(|l| l == "1 book"), "{labels:?}");
}

/// The titles on the page a search shows, in order.
fn titles(app: &mut App) -> Vec<String> {
    app.labels()
        .into_iter()
        .filter(|l| l.starts_with("A book of the library"))
        .collect()
}

#[test]
fn a_search_that_matches_every_book_of_a_small_library_lists_them_all_in_the_same_order() {
    let _turn = turn();
    let mut small = library(300, true);
    small.run(actions::LIBRARY);
    small.wait(61_000);
    searched(&mut small, "book");
    let labels = small.labels();
    // Every book matches and fits: no note, all of them counted.
    assert!(labels.iter().any(|l| l == "300 books"), "{labels:?}");
    assert!(!labels.iter().any(|l| l.starts_with("First")), "{labels:?}");
    // In catalog order, which equal title keys (the first 8 letters) fall to.
    let small_first = titles(&mut small);
    assert!(
        small_first[0].starts_with("A book of the library 00000"),
        "{small_first:?}"
    );
    assert!(
        small_first[1].starts_with("A book of the library 00001"),
        "{small_first:?}"
    );
    // Where the most holds, the same order: the first books held.
    let mut large = library(3_000, true);
    large.run(actions::LIBRARY);
    large.wait(61_000);
    searched(&mut large, "book");
    let large_first = titles(&mut large);
    assert!(
        large_first[0].starts_with("A book of the library 00000"),
        "{large_first:?}"
    );
    assert!(
        large_first[1].starts_with("A book of the library 00300"),
        "{large_first:?}"
    );
    // And a library of exactly the most: no note either.
    let mut exact = library(500, true);
    exact.run(actions::LIBRARY);
    exact.wait(61_000);
    searched(&mut exact, "book");
    let labels = exact.labels();
    assert!(labels.iter().any(|l| l == "500 books"), "{labels:?}");
    assert!(!labels.iter().any(|l| l.starts_with("First")), "{labels:?}");
}

#[test]
fn a_search_that_matches_every_book_takes_a_bounded_part_of_the_heap() {
    let _turn = turn();
    let mut app = library(3_000, true);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    let peak = growth(|| {
        searched(&mut app, "book");
        app.render();
    });
    eprintln!("search matching 3,000 of 3,000: peak {peak} B on the host");
    // 500 matches of 20 B, and what the page needs; not 3,000 matches
    // (123 KB before the cap).
    assert!(peak <= 40 * 1024, "the search took {peak} B");
}

#[test]
fn the_find_screen_starts_on_the_search_and_up_from_the_letters_returns_to_it() {
    let _turn = turn();
    let mut app = library(300, true);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    app.run(FIND);
    assert_eq!(app.screen(), "files.list.find");
    assert_eq!(app.focused().as_deref(), Some("Search title or author"));
    // Right goes to the first letter, Up from the first row back to the
    // search (not round to the last letter).
    app.run(actions::RIGHT);
    assert_eq!(app.focused().as_deref(), Some("B"));
    app.run(actions::UP);
    assert_eq!(app.focused().as_deref(), Some("Search title or author"));
    app.run(actions::RIGHT);
    app.run(actions::PREV_PAGE);
    assert_eq!(app.focused().as_deref(), Some("Search title or author"));
    // Confirm on the search asks for the words.
    app.will_type("library 00123");
    app.run(actions::CONFIRM);
    app.wait(2_000);
    assert!(
        app.shows("A book of the library 00123"),
        "{:?}",
        app.labels()
    );
}

/// Books whose titles run the other way from the catalog's order (the
/// last book held is the first by title), `books` of them.
fn reversed(books: usize) -> App {
    let paths: Vec<String> = (0..books).map(|n| format!("books/{n:05}.txt")).collect();
    let files: Vec<(&str, &[u8])> = paths.iter().map(|p| (p.as_str(), &b"t"[..])).collect();
    let storage = Shared::memory("", &files);
    let mut records: Vec<Record> = paths
        .iter()
        .enumerate()
        .map(|(n, path)| {
            let book = Book::default()
                .title(Some(format!("{:05} Title", books - 1 - n)))
                .size(1);
            Record::of(path, &book)
        })
        .collect();
    records.sort_by(|a, b| a.path.cmp(&b.path));
    storage.create_dir(".paperpaw").unwrap();
    storage.create_dir(".paperpaw/library").unwrap();
    catalog::write(&storage, &records).unwrap();
    App::new(X4, storage)
}

#[test]
fn over_the_limit_the_books_kept_are_the_first_by_title_not_the_first_held() {
    let _turn = turn();
    let mut app = reversed(600);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    searched(&mut app, "title");
    let labels = app.labels();
    assert!(
        labels
            .iter()
            .any(|l| l == "First 500 of 600 books · narrow the search"),
        "{labels:?}"
    );
    // The first held is last by title and not listed; the list starts
    // with the first by title.
    assert!(
        labels.iter().any(|l| l.starts_with("00000 Title")),
        "{labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("00599 Title")),
        "{labels:?}"
    );
    // Up from the first book wraps to the last: the 500th by title.
    app.run(actions::UP);
    let labels = app.labels();
    assert!(
        labels.iter().any(|l| l.starts_with("00499 Title")),
        "{labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l.starts_with("00500 Title")),
        "{labels:?}"
    );
}
