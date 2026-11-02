//! The library while the kernel walks it for the first time (#108): the
//! books found so far are listed as they come, counted as such, and can
//! be read; the focus stays on its book as more come. An empty library
//! offers to look for new books (#109), as its menu does.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_sdk::library::catalog;

const BUILDING: &str = ".paperpaw/library/catalog.tmp";

/// `n` small books in a few folders.
fn books(n: usize) -> Vec<(String, Vec<u8>)> {
    (0..n)
        .map(|i| {
            let path = format!("books/f{}/Book {i:03}.txt", i % 4);
            (path, format!("The text of book {i}.").into_bytes())
        })
        .collect()
}

fn app(device: DeviceInfo, n: usize) -> App {
    let files = books(n);
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    App::new(device, Shared::memory("", &files))
}

/// Books in the catalog.
fn listed(app: &App) -> usize {
    let mut n = 0;
    catalog::each(&app.storage, &mut |_| n += 1);
    n
}

/// One kernel tick at a time, a few milliseconds apart, until `until`
/// holds (at most 5,000 ticks): whether it did.
fn tick_until(app: &mut App, mut until: impl FnMut(&mut App) -> bool) -> bool {
    for _ in 0..5_000 {
        if until(app) {
            return true;
        }
        app.now += 5;
        let _ = app.kernel.tick(Instant(app.now));
    }
    false
}

/// While the first walk goes on, the library lists the books found so
/// far and says so in its count; they can be opened and read (the walk
/// waits meanwhile), and at its end all are listed, counted as usual.
#[test]
fn the_library_lists_the_books_found_while_the_first_walk_goes_on() {
    let _turn = turn();
    for device in [X4, LARGE] {
        let mut app = app(device, 120);
        app.now += 100;
        let _ = app.kernel.run_action(Instant(app.now), actions::LIBRARY);
        let shown = tick_until(&mut app, |app| {
            app.storage.exists(BUILDING) && listed(app) > 0 && app.shows("Book 0")
        });
        assert!(shown, "{}", app.described());
        let found = listed(&app);
        assert!((20..120).contains(&found), "{found}");
        // A book opened and read: the walk waits.
        app.run(actions::CONFIRM);
        assert_eq!(app.screen(), "reader.screen", "{}", app.described());
        app.wait(2_000);
        assert!(app.storage.exists(BUILDING), "the walk waits under a book");
        app.run(actions::BACK);
        assert_eq!(app.screen(), "files.browser");
        let done = tick_until(&mut app, |app| !app.storage.exists(BUILDING));
        assert!(done, "{}", app.described());
        app.wait(1_000);
        assert_eq!(listed(&app), 120);
        assert!(app.shows("Book 0"), "{}", app.described());
    }
}

/// More books come in while the focus is on one: it stays on it.
#[test]
fn the_focus_stays_on_its_book_as_more_are_found() {
    let _turn = turn();
    let mut app = app(X4, 200);
    app.now += 100;
    let _ = app.kernel.run_action(Instant(app.now), actions::LIBRARY);
    assert!(tick_until(&mut app, |app| listed(app) > 0 && app.shows("Book 0")));
    let first = listed(&app);
    app.run(actions::DOWN);
    app.run(actions::DOWN);
    let focused = app.focused().expect("a book focused");
    assert!(tick_until(&mut app, |app| listed(app) > first));
    app.wait(500);
    assert_eq!(app.focused(), Some(focused), "{}", app.described());
}

/// An empty library offers to look for new books: copied onto the card
/// with the library walked, they are found by it (and by its menu).
#[test]
fn an_empty_library_looks_for_new_books() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = App::new(device, Shared::memory("", &[]));
        app.storage.create_dir("books").unwrap();
        app.run(actions::LIBRARY);
        app.wait(1_000);
        assert!(app.shows("No books here yet"), "{}", app.described());
        assert!(app.shows("Look for new books"), "{}", app.described());
        for (path, data) in books(3) {
            app.storage.write(&path, &data).unwrap();
        }
        // Walked since the start: not again by itself.
        app.home();
        app.run(actions::LIBRARY);
        app.wait(1_000);
        assert!(app.shows("No books here yet"), "{}", app.described());
        if device.input.touch {
            app.tap_on("Look for new books");
        } else {
            app.run(actions::CONFIRM);
        }
        app.wait(1_000);
        app.let_catalog_work();
        app.wait(1_000);
        assert!(app.shows("Book 002"), "{}", app.described());
        assert!(!app.shows("Look for new books"));
    }
}

/// The library's menu looks for new books too.
#[test]
fn the_library_menu_looks_for_new_books() {
    let _turn = turn();
    let mut app = app(TOUCH, 2);
    app.run(actions::LIBRARY);
    app.wait(1_000);
    app.let_catalog_work();
    app.wait(1_000);
    assert!(app.shows("Book 001"), "{}", app.described());
    app.storage
        .write("books/f9/New.txt", b"The text of a new book.")
        .unwrap();
    app.run(actions::MENU);
    app.tap_on("Look for new books");
    app.wait(1_000);
    app.let_catalog_work();
    app.wait(1_000);
    assert!(app.shows("New"), "{}", app.described());
}

/// Started again midway (books came without a word): at every point of
/// the first walk, also while what it found is sorted, the walk under
/// way gives up what it began, the books found stay listed, and the next
/// walk lists them all.
#[test]
fn a_first_walk_started_again_midway_lists_all_books() {
    let _turn = turn();
    for wait in [0, 15, 40, 90] {
        let mut app = app(X4, 120);
        app.now += 100;
        let _ = app.kernel.run_action(Instant(app.now), actions::LIBRARY);
        assert!(tick_until(&mut app, |app| listed(app) > 0));
        for _ in 0..wait {
            app.now += 5;
            let _ = app.kernel.tick(Instant(app.now));
        }
        app.send(
            paperpaw_sdk::builtin::events::LIBRARY_CHANGED,
            Params::new(),
        );
        assert!(listed(&app) > 0);
        let done = tick_until(&mut app, |app| {
            !app.storage.exists(BUILDING) && listed(app) == 120
        });
        assert!(done, "{wait}: {}", listed(&app));
        app.wait(1_000);
        for left in [
            "sorted.tmp",
            "sorted.runs",
            "walk.list",
            "walk.dirs",
            "walk.first",
        ] {
            let path = format!(".paperpaw/library/{left}");
            assert!(!app.storage.exists(&path), "{wait}: {path}");
        }
        assert!(app.shows("Book 0"), "{}", app.described());
    }
}

/// Books copied onto the card of an empty library, walked already (#109):
/// once it looks for them, it says it collects them, not that there are
/// none, lists them as they are found (as a first walk does), and a
/// second press goes on with the walk under way rather than starting
/// over.
#[test]
fn an_empty_library_collects_new_books_as_a_first_walk() {
    let _turn = turn();
    for device in [X4, TOUCH] {
        let mut app = App::new(device, Shared::memory("", &[]));
        app.storage.create_dir("books").unwrap();
        app.run(actions::LIBRARY);
        app.wait(1_000);
        assert!(app.shows("No books here yet"), "{}", app.described());
        for (path, data) in books(200) {
            app.storage.write(&path, &data).unwrap();
        }
        // Pressed, before the kernel walks a step (the button by a key,
        // or its action, as a tap and the menu run it).
        app.now += 100;
        let press = if device.input.touch {
            Id::new_static("files.look-again")
        } else {
            actions::CONFIRM
        };
        let _ = app.kernel.run_action(Instant(app.now), press);
        assert!(app.shows("Collecting the books"), "{}", app.described());
        assert!(!app.shows("No books here yet"), "{}", app.described());
        assert!(!app.shows("Look for new books"), "{}", app.described());
        // Shown before the walk's end, the books found so far.
        let shown = tick_until(&mut app, |app| {
            app.storage.exists(BUILDING) && listed(app) > 0 && app.shows("Book 0")
        });
        assert!(shown, "{}", app.described());
        assert!(listed(&app) < 200);
        // Looking again meanwhile (the menu's action): the walk goes on.
        let before = app.storage.size(BUILDING).unwrap_or(0);
        app.now += 5;
        let _ = (app.kernel).run_action(Instant(app.now), Id::new_static("files.look-again"));
        app.now += 5;
        let _ = app.kernel.tick(Instant(app.now));
        let after = app.storage.size(BUILDING).unwrap_or(0);
        assert!(after >= before, "started over: {before} then {after}");
        let done = tick_until(&mut app, |app| !app.storage.exists(BUILDING));
        assert!(done, "{}", app.described());
        app.wait(1_000);
        assert_eq!(listed(&app), 200);
        assert!(app.shows("Book 0"), "{}", app.described());
    }
}
