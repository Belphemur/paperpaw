//! The shelves of a large library take a bounded part of the heap on a
//! device of little memory (the X4): 10,000 books of 5,000 authors in
//! 2,500 series and 300 folders, the list's shelves and the authors' and
//! series' screens shown, each measured by how far the heap grows (on
//! the host: about twice what the X4's 32-bit heap takes).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::*;
use paperpaw_sdk::Params;
use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::event::EventData;
use paperpaw_sdk::library::{
    Book,
    catalog::{self, Record},
};

/// Counts the bytes allocated now, and the most since the peak was reset.
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

/// The tests take turns: the heap is counted for the whole program.
static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn turn() -> std::sync::MutexGuard<'static, ()> {
    TURN.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// How far the heap grows while `run` runs (at most), and how much more
/// it holds after.
fn growth(run: impl FnOnce()) -> (usize, isize) {
    let before = NOW.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    run();
    let after = NOW.load(Ordering::Relaxed);
    (
        PEAK.load(Ordering::Relaxed) - before,
        after as isize - before as isize,
    )
}

fn path(n: usize) -> String {
    format!(
        "books/Folder {:03}/A book of the library {n:05}.txt",
        n % 300
    )
}

/// The library of `books` books on the X4 (two of each author, four of
/// each series), its catalog made: every book's file is there and of
/// the size the catalog says, so the walk after the start takes all
/// from it.
fn library(books: usize) -> App {
    library_named(books, "")
}

/// [`library`], each author's and series' name ending in `long`.
fn library_named(books: usize, long: &str) -> App {
    let paths: Vec<String> = (0..books).map(path).collect();
    let files: Vec<(&str, &[u8])> = paths.iter().map(|p| (p.as_str(), &b"t"[..])).collect();
    let storage = Shared::memory("", &files);
    let mut records: Vec<Record> = paths
        .iter()
        .enumerate()
        .map(|(n, path)| {
            let book = Book::default()
                .title(Some(format!("A book of the library {n:05}")))
                .author(Some(format!("Somebody Writing {:04}{long}", n / 2)))
                .series(Some(format!("A series of books {:04}{long}", n / 4)))
                .series_index(Some(format!("{}", n % 4 + 1)))
                .size(1);
            Record::of(path, &book)
        })
        .collect();
    // In the order a walk meets them (the folders in the order found), as
    // the catalog a walk made is: the walk after the start takes each
    // from it.
    records.sort_by(|a, b| {
        let folder = |r: &Record| r.path.rsplit_once('/').map(|(f, _)| f.to_string());
        folder(a).cmp(&folder(b)).then_with(|| a.path.cmp(&b.path))
    });
    storage.create_dir(".paperpaw").unwrap();
    storage.create_dir(".paperpaw/library").unwrap();
    catalog::write(&storage, &records).unwrap();
    let mut app = App::new(X4, storage);
    app.run(actions::LIBRARY);
    app.wait(61_000);
    app
}

/// The list of every book (the X4 listed 2,000 at most before): opened
/// and paged to its end it holds a few kilobytes, whatever the
/// library's size.
#[test]
fn the_list_of_a_large_library_takes_a_few_kilobytes() {
    let _turn = turn();
    let mut app = library(10_000);
    let first = app.focused().unwrap_or_default();
    app.run(actions::BACK);
    let (opened, held) = growth(|| {
        app.run(actions::LIBRARY);
        app.render();
    });
    let (paged, _) = growth(|| {
        for _ in 0..40 {
            app.run(actions::NEXT_PAGE);
        }
        app.render();
    });
    eprintln!("list of 10,000: opened peak {opened} B, held {held} B; paged peak {paged} B");
    // Paged on, past what the X4 held before.
    let later = app.focused().unwrap_or_default();
    assert_ne!(later, first, "{}", app.described());
    // Before, the X4 held an index of 2,000 books at most: 48 KB.
    assert!(held <= 16 * 1024, "the list holds {held} B");
    assert!(opened <= 32 * 1024, "the list took {opened} B");
    assert!(paged <= 32 * 1024, "paging took {paged} B");
}

#[test]
fn the_shelves_of_a_large_library_take_a_bounded_part_of_the_heap() {
    let _turn = turn();
    let mut app = library(10_000);
    let mut count = 0;
    catalog::each(&app.storage, &mut |_| count += 1);
    assert_eq!(count, 10_000);
    let shelves = paperpaw_sdk::ui::choice::Picked::new(String::new(), String::from("shelves"));
    let (list, list_held) = growth(|| {
        app.send(
            paperpaw_sdk::Id::new_static("files.list.shown"),
            shelves.to_params(),
        );
        app.render();
    });
    // The first 200 series and folders, and rows of all of them: the
    // last row (up from the first), and the one before the folders.
    let mut focused = String::new();
    for _ in 0..3 {
        app.run(actions::UP);
        focused = app.focused().unwrap_or_default();
        if focused.starts_with("All folders") {
            break;
        }
    }
    assert!(focused.starts_with("All folders"), "{focused}");
    app.repeat(actions::UP, 201);
    let focused = app.focused().unwrap_or_default();
    assert!(focused.starts_with("All series"), "{focused}");
    let mut screens = Vec::new();
    for mode in ["authors", "series"] {
        let open = Params::new().with(params::MODE, mode);
        screens.push(growth(|| {
            app.open_screen(paperpaw_sdk::Id::new_static("files.shelf"), open);
            app.render();
        }));
        app.run(actions::BACK);
    }
    eprintln!(
        "shelves in the list: peak {list} B, held {list_held} B; authors: {:?}; series: {:?}",
        screens[0], screens[1]
    );
    // All the folders: by their names.
    let open = Params::new()
        .with(params::MODE, "folders")
        .with(params::PATH, "books");
    app.open_screen(paperpaw_sdk::Id::new_static("files.shelf"), open);
    let focused = app.focused().unwrap_or_default();
    assert!(focused.starts_with("Folder 000"), "{focused}");
    // Before the bound: 1,258,550 B, 760,570 held; the screens 214,137
    // and 217,824 B.
    assert!(list <= 64 * 1024, "the list's shelves took {list} B");
    assert!(
        list_held <= 32 * 1024,
        "the list's shelves hold {list_held} B"
    );
    for (peak, _) in screens {
        assert!(peak <= 32 * 1024, "a shelf took {peak} B");
    }
}

/// Names of a kilobyte each (as long as a catalog keeps them): the
/// shelves hold no more bytes of them than their budget (16 KB on the
/// X4), however few names that is.
#[test]
fn the_shelves_of_long_names_take_a_bounded_part_of_the_heap() {
    let _turn = turn();
    let long = " and others".repeat(92);
    let mut app = library_named(2_000, &long);
    let shelves = paperpaw_sdk::ui::choice::Picked::new(String::new(), String::from("shelves"));
    let (list, list_held) = growth(|| {
        app.send(
            paperpaw_sdk::Id::new_static("files.list.shown"),
            shelves.to_params(),
        );
        app.render();
    });
    let mut screens = Vec::new();
    for mode in ["authors", "series"] {
        let open = Params::new().with(params::MODE, mode);
        screens.push(growth(|| {
            app.open_screen(paperpaw_sdk::Id::new_static("files.shelf"), open);
            app.render();
        }));
        let focused = app.focused().unwrap_or_default();
        assert!(focused.contains(" 0000 and"), "{focused}");
        app.run(actions::BACK);
    }
    eprintln!(
        "shelves of long names in the list: peak {list} B, held {list_held} B; authors: {:?}; series: {:?}",
        screens[0], screens[1]
    );
    // Before the budget: 200 names of a kilobyte a window.
    assert!(list <= 64 * 1024, "the list's shelves took {list} B");
    for (peak, _) in screens {
        assert!(peak <= 48 * 1024, "a shelf took {peak} B");
    }
}

/// The authors' screen holds a window of them (200 on the X4): moving on
/// past its ends takes the next, so every author is reached, in order,
/// and back.
#[test]
fn every_name_is_reached_through_the_windows() {
    let _turn = turn();
    let mut app = library(900);
    let open = Params::new().with(params::MODE, "authors");
    app.open_screen(paperpaw_sdk::Id::new_static("files.shelf"), open);
    let step = |app: &mut App, action, met: &mut Vec<String>| {
        app.run(action);
        let focused = app.focused().unwrap_or_default();
        let author = focused.split(", ").next().unwrap_or_default().to_string();
        if met.last() != Some(&author) {
            met.push(author);
        }
    };
    let all: Vec<String> = (0..450)
        .map(|n| format!("Somebody Writing {n:04}"))
        .collect();
    // Down from the first to the last, and on round to the first.
    let mut met: Vec<String> = Vec::new();
    for _ in 0..449 {
        step(&mut app, actions::DOWN, &mut met);
    }
    assert_eq!(met[..], all[1..]);
    met.clear();
    step(&mut app, actions::DOWN, &mut met);
    assert_eq!(met, all[..1]);
    // Up round to the last, and on to the first.
    met.clear();
    for _ in 0..450 {
        step(&mut app, actions::UP, &mut met);
    }
    let back: Vec<String> = all.iter().rev().cloned().collect();
    assert_eq!(met[..], back[..]);
}
