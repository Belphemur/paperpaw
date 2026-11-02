//! The library's shelves with the real kernel and the standard app, on a
//! buttons-only X4 and a touch-only device: by author (made from the
//! books' own details in the background, an author's books listed when
//! chosen, books without one under "Unknown author"), the favourites,
//! and an empty library.
//!
//! The catalog's freshness is known for the whole program, as on a device,
//! where a new library means a new start. The tests here, each with a
//! library of its own, take turns and tell the shelves their library is
//! new (as the library does when books come or go).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_sdk::builtin::events;

const SHELF: &str = "files.shelf";

/// A Project Gutenberg text with its title and author in its header.
fn gutenberg(title: &str, author: Option<&str>) -> Vec<u8> {
    let author = author.map_or(String::new(), |a| format!("Author: {a}\r\n"));
    format!(
        "The Project Gutenberg eBook of {title}\r\n\r\nTitle: {title}\r\n{author}\
         Language: English\r\n\r\n*** START OF THE PROJECT GUTENBERG EBOOK ***\r\n\r\n\
         Some text of the book.\r\n\r\n*** END OF THE PROJECT GUTENBERG EBOOK ***\r\n"
    )
    .into_bytes()
}

/// The app on `storage`, its catalog to be made anew for that library.
fn app_on(device: DeviceInfo, storage: Shared) -> App {
    let mut app = App::new(device, storage);
    app.send(events::LIBRARY_CHANGED.id().clone(), Params::new());
    app
}

fn library() -> Shared {
    let books = [
        ("books/a.txt", gutenberg("Alice", Some("Lewis Carroll"))),
        (
            "books/b.txt",
            gutenberg("Sylvie and Bruno", Some("Lewis Carroll")),
        ),
        (
            "books/c.txt",
            gutenberg("Die Verwandlung", Some("Franz Kafka")),
        ),
        (
            "books/d.txt",
            gutenberg("Ödön's Tales", Some("Ödön von Horváth")),
        ),
        ("books/e.txt", gutenberg("Nobody's", None)),
        (
            "books/sub/f.txt",
            gutenberg("Der Process", Some("Franz Kafka")),
        ),
    ];
    let files: Vec<(&str, &[u8])> = books.iter().map(|(p, d)| (*p, d.as_slice())).collect();
    Shared::memory("", &files)
}

fn rows(app: &mut App) -> Vec<String> {
    app.parts()
        .into_iter()
        .filter(|p| p.role == Role::Row)
        .map(|p| p.label)
        .collect()
}

/// Lets the catalog be made in the background.
fn wait(app: &mut App) {
    for _ in 0..20 {
        app.now += 1000;
        let effects = app.kernel.tick(Instant(app.now));
        app.settle(effects);
    }
}

/// Chooses the row that begins with `label`.
fn choose(app: &mut App, label: &str) {
    if app.kernel.device().input.touch {
        app.tap_on(label);
        return;
    }
    for _ in 0..40 {
        if app.focused().is_some_and(|f| f.starts_with(label)) {
            app.run(actions::CONFIRM);
            return;
        }
        app.run(actions::DOWN);
    }
    panic!("no row {label}:\n{}", app.described());
}

#[test]
fn books_by_author_listed_and_opened() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = app_on(device, library());
        app.run(actions::LIBRARY);
        app.run(Id::new_static("files.by-author"));
        assert_eq!(app.screen(), SHELF);
        wait(&mut app);
        let authors = rows(&mut app);
        let names: Vec<&str> = authors
            .iter()
            .map(|r| r.split(", ").next().unwrap())
            .collect();
        for author in [
            "Lewis Carroll",
            "Franz Kafka",
            "Ödön von Horváth",
            "Unknown author",
        ] {
            assert!(names.contains(&author), "{author}: {authors:?}");
        }
        // Books in folders below count too.
        let kafka = authors
            .iter()
            .find(|r| r.starts_with("Franz Kafka"))
            .unwrap();
        assert!(kafka.contains("2 books"), "{kafka}");
        choose(&mut app, "Franz Kafka");
        let books = rows(&mut app);
        assert_eq!(books.len(), 2, "{books:?}");
        assert!(
            books.iter().any(|b| b.starts_with("Der Process")),
            "{books:?}"
        );
        choose(&mut app, "Die Verwandlung");
        assert_eq!(app.screen(), "reader.screen");
        // Back: the author's books, the authors, the library.
        app.run(actions::BACK);
        assert_eq!(app.screen(), "files.list.shelf");
        app.run(actions::BACK);
        assert_eq!(app.screen(), SHELF);
        app.run(actions::BACK);
        assert_eq!(app.screen(), "files.browser");
    }
}

#[test]
fn the_shelves_of_an_empty_library_say_so() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = app_on(device, Shared::memory("", &[]));
        for shelf in ["files.by-author", "files.by-series", "files.by-tag"] {
            app.run(Id::new_static(shelf));
            assert_eq!(app.screen(), SHELF, "{shelf}");
            wait(&mut app);
            assert!(rows(&mut app).is_empty(), "{shelf}: {}", app.described());
            app.run(actions::BACK);
        }
        app.run(Id::new_static("files.favourites"));
        assert!(app.shows("No favourites yet"), "{}", app.described());
    }
}

#[test]
fn books_without_series_or_tags_say_so_on_those_shelves() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = app_on(device, library());
        app.run(Id::new_static("files.by-series"));
        wait(&mut app);
        assert!(
            app.shows("None of the books names a series."),
            "{}",
            app.described()
        );
        app.run(actions::BACK);
        app.run(Id::new_static("files.by-tag"));
        wait(&mut app);
        assert!(
            app.shows("None of the books has tags."),
            "{}",
            app.described()
        );
    }
}

#[test]
fn the_shelves_menu_offers_every_shelf() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = app_on(device, library());
        app.run(Id::new_static("files.shelves"));
        assert_eq!(app.screen(), "menu.list");
        let shown: Vec<String> = rows(&mut app);
        assert_eq!(shown, ["Favourites", "By author", "By series", "By tag"]);
        choose(&mut app, "By series");
        assert_eq!(app.screen(), SHELF);
    }
}

#[test]
fn turned_the_shelves_keep_their_focus() {
    let _turn = turn();
    let mut app = app_on(X4, library());
    app.run(actions::LIBRARY);
    app.run(Id::new_static("files.by-author"));
    wait(&mut app);
    app.run(actions::DOWN);
    let name = app.focused();
    assert!(name.is_some(), "a name has the focus");
    app.resize(800, 480);
    assert_eq!(app.screen(), SHELF);
    assert_eq!(app.focused(), name, "landscape");
    app.resize(480, 800);
    assert_eq!(app.focused(), name, "upright again");
}
