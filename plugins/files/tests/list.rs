//! The library as a list with the real kernel and the standard app: books
//! moved elsewhere (by the file manager, to the archive), settings
//! changed while it shows, and where the focus is when it shows again.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_sdk::builtin::events;
use paperpaw_sdk::event::EventData;

const TEXT: &[u8] = b"Some text of a book.\n\nAnd more of it.\n";

/// The library on `device` with `count` books, `Book 01.txt` and on.
fn library(device: DeviceInfo, config: &str, count: usize) -> App {
    let names: Vec<String> = (1..=count)
        .map(|n| format!("books/Book {n:02}.txt"))
        .collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), TEXT)).collect();
    let mut app = App::new(device, Shared::memory(config, &files));
    app.run(actions::LIBRARY);
    assert_eq!(app.screen(), "files.browser");
    app
}

/// The rows shown now, by their first part (a book's title).
fn rows(app: &mut App) -> Vec<String> {
    app.parts()
        .into_iter()
        .filter(|p| p.role == Role::Row)
        .map(|p| p.label.split(", ").next().unwrap_or_default().to_string())
        .collect()
}

fn focused(app: &mut App) -> String {
    let label = app.focused().unwrap_or_default();
    label.split(", ").next().unwrap_or_default().to_string()
}

#[test]
fn a_book_moved_elsewhere_is_listed_where_it_went() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = library(device, "[files]\nsort = files.sort.name\n", 3);
        assert_eq!(rows(&mut app), ["Book 01", "Book 02", "Book 03"]);
        // The file manager moves a book into a folder and says so.
        let (from, to) = ("books/Book 02.txt", "books/Kept/Book 02.txt");
        app.storage.rename(from, to).unwrap();
        let moved = events::BookMoved::new(from.into(), to.into());
        app.send(events::BOOK_MOVED.id().clone(), moved.to_params());
        // Listed again where it is now (on the X4 once the catalog caught
        // up, away from the list), and it opens from there.
        app.let_catalog_work();
        assert_eq!(rows(&mut app), ["Book 01", "Book 02", "Book 03"]);
        if device.input.touch {
            app.tap_on("Book 02");
        } else {
            app.run(actions::DOWN);
            assert_eq!(focused(&mut app), "Book 02");
            app.run(actions::CONFIRM);
        }
        assert_eq!(app.screen(), "reader.screen", "{}", app.described());
    }
}

#[test]
fn an_archived_book_goes_from_the_list_at_once() {
    let _turn = turn();
    let mut app = library(X4, "[files]\nsort = files.sort.name\n", 3);
    app.run(actions::DOWN);
    app.run(actions::MENU);
    assert_eq!(app.screen(), "files.list.sheet");
    for _ in 0..10 {
        if app
            .focused()
            .is_some_and(|f| f.starts_with("Move to the archive"))
        {
            break;
        }
        app.run(actions::DOWN);
    }
    app.run(actions::CONFIRM);
    assert_eq!(app.screen(), "files.browser");
    assert!(app.storage.exists("books/Archive/Book 02.txt"));
    assert_eq!(rows(&mut app), ["Book 01", "Book 03"]);
    // The focus at its place, on the next book; nothing left that would
    // open a missing file.
    assert_eq!(focused(&mut app), "Book 03");
    app.run(actions::CONFIRM);
    assert_eq!(app.screen(), "reader.screen", "{}", app.described());
}

#[test]
fn showing_books_as_covers_applies_while_the_library_shows() {
    let _turn = turn();
    let mut app = library(X4, "[files]\nsort = files.sort.name\n", 12);
    app.run(actions::DOWN);
    app.run(actions::DOWN);
    assert_eq!(focused(&mut app), "Book 03");
    let listed = rows(&mut app).len();
    app.set("files.view", "files.view.covers");
    // Covers at once, more of them a page, the focus on its book.
    let covers = rows(&mut app).len();
    assert!(covers > listed, "{covers} covers, {listed} rows");
    assert_eq!(focused(&mut app), "Book 03");
    app.set("files.view", "files.view.list");
    assert_eq!(rows(&mut app).len(), listed);
    assert_eq!(focused(&mut app), "Book 03");
}

#[test]
fn the_focus_stays_on_its_book_with_the_order_by_recent_reading() {
    let _turn = turn();
    let mut app = library(X4, "", 8);
    for _ in 0..4 {
        app.run(actions::DOWN);
    }
    assert_eq!(focused(&mut app), "Book 05");
    // A full screen above and back: where it was.
    let info = Params::new().with(params::PATH, "books/Book 05.txt");
    app.open_screen(paperpaw_plugin_files::INFO_SCREEN, info);
    app.run(actions::BACK);
    assert_eq!(focused(&mut app), "Book 05");
    // Read, then back: the book read last comes first, the focus with it.
    app.run(actions::CONFIRM);
    assert_eq!(app.screen(), "reader.screen");
    app.run(actions::BACK);
    assert_eq!(app.screen(), "files.browser");
    assert_eq!(focused(&mut app), "Book 05");
    assert_eq!(rows(&mut app).first().map(String::as_str), Some("Book 05"));
}

#[test]
fn while_a_book_covers_it_the_library_keeps_nothing_and_finds_its_place_again() {
    let _turn = turn();
    // An index an earlier version kept: removed, nothing kept instead
    // (the books sorted on the card are read again).
    let mut app = library(X4, "", 5);
    app.kernel
        .storage()
        .write(".paperpaw/plugins/files/index.bin", b"PPI3")
        .unwrap();
    app.run(actions::BACK);
    app.run(actions::LIBRARY);
    assert!(!app.storage.exists(".paperpaw/plugins/files/index.bin"));
    app.run(actions::DOWN);
    app.run(actions::CONFIRM);
    assert_eq!(app.screen(), "reader.screen");
    app.run(actions::BACK);
    assert_eq!(rows(&mut app).len(), 4);
    assert_eq!(focused(&mut app), "Book 02");
}

#[test]
fn turned_the_library_keeps_the_focus_on_its_book() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = library(device, "[files]\nsort = files.sort.name\n", 8);
        // The focus shows on touch screens once a button moved it.
        app.repeat(actions::DOWN, 4);
        let book = focused(&mut app);
        assert!(book.starts_with("Book 0"), "{book}");
        app.resize(800, 480);
        assert_eq!(app.screen(), "files.browser");
        assert_eq!(focused(&mut app), book, "landscape");
        for part in app.parts() {
            let area = part.area;
            assert!(
                u32::from(area.x) + u32::from(area.width) <= 800
                    && u32::from(area.y) + u32::from(area.height) <= 480,
                "{} is off the screen: {area:?}",
                part.label
            );
        }
        app.resize(480, 800);
        assert_eq!(focused(&mut app), book, "upright again");
    }
}
