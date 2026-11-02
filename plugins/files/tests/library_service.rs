//! The library as a service with the real kernel and the standard app: a
//! library of one's own feeds every view (the list, the letters, the
//! shelves) without a catalog on the card, and the kernel's catalog
//! folded while the list shows keeps it right.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_sdk::builtin::events;
use paperpaw_sdk::event::EventData;
use paperpaw_sdk::library::catalog::{Record, State};
use paperpaw_sdk::library::{self, Book, Library};
use paperpaw_sdk::{Env, Manifest, Plugin, RegisterError, Registrar, Version};

/// A library kept elsewhere (a server's, say): three books of a series,
/// none of them on the card.
struct Remote;

impl Remote {
    fn books() -> Vec<Record> {
        ["Ash", "Birch", "Cedar"]
            .iter()
            .enumerate()
            .map(|(n, title)| {
                let book = Book::default()
                    .title(Some(String::from(*title)))
                    .author(Some(String::from("Ann Remote")))
                    .series(Some(String::from("Trees")))
                    .series_index(Some(format!("{}", n + 1)));
                Record::of(&format!("remote/{title}.txt"), &book)
            })
            .collect()
    }
}

impl Library for Remote {
    fn catalog_state(&self, _: &Env<'_>) -> State {
        State::Ready { stamp: 1 }
    }

    fn each(&self, _: &Env<'_>, f: &mut dyn FnMut(u64, Record)) {
        for (key, record) in Self::books().into_iter().enumerate() {
            f(key as u64 * 7, record);
        }
    }

    fn records(&self, _: &Env<'_>, keys: &[u64]) -> Vec<Option<Record>> {
        let books = Self::books();
        keys.iter()
            .map(|key| books.get((key / 7) as usize).cloned())
            .collect()
    }

    fn version(&self, _: &Env<'_>) -> Option<u64> {
        Some(1)
    }
}

struct RemotePlugin;

impl Plugin for RemotePlugin {
    fn manifest(&self) -> Manifest {
        Manifest::new(Id::new_static("remote"), "Remote", Version::new(1, 0, 0))
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let library: Box<dyn Library> = Box::new(Remote);
        r.provide(
            Id::new_static(library::service::SERVICE),
            library::service::LibraryService(library),
            library::service::VERSION,
        )
    }
}

fn rows(app: &mut App) -> Vec<String> {
    app.parts()
        .into_iter()
        .filter(|p| p.role == Role::Row)
        .map(|p| p.label.split(", ").next().unwrap_or_default().to_string())
        .collect()
}

#[test]
fn a_library_of_ones_own_feeds_every_view() {
    let _turn = turn();
    for device in DEVICES {
        let config = "[files]\nsort = files.sort.name\n";
        let mut app = App::with(device, Shared::memory(config, &[]), RemotePlugin);
        let warnings = app.kernel.take_warnings();
        assert!(warnings.is_empty(), "{warnings:?}");
        app.run(actions::LIBRARY);
        assert_eq!(
            rows(&mut app),
            ["Ash", "Birch", "Cedar"],
            "{}",
            app.described()
        );
        // The letters: of its books.
        app.open_screen(Id::new_static("files.list.find"), Params::new());
        assert!(app.shows("A"), "{}", app.described());
        app.run(actions::BACK);
        // A series' shelf: its books, by number.
        app.open_screen(
            Id::new_static("files.list.shelf"),
            Params::new().with("mode", "series").with("name", "Trees"),
        );
        assert_eq!(
            rows(&mut app),
            ["Ash", "Birch", "Cedar"],
            "{}",
            app.described()
        );
        app.run(actions::BACK);
        // The authors' shelf.
        app.run(Id::new_static("files.by-author"));
        assert!(app.shows("Ann Remote"), "{}", app.described());
    }
}

/// The library of 4 books on `device`, its log made long by books read
/// and closed many times (the first two finished).
fn read_often(device: DeviceInfo) -> (App, u64) {
    let names: Vec<String> = (1..=4).map(|n| format!("books/Book {n:02}.txt")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b"text"[..])).collect();
    let config = "[files]\nsort = files.sort.name\n";
    let mut app = App::new(device, Shared::memory(config, &files));
    app.run(actions::LIBRARY);
    assert_eq!(rows(&mut app), ["Book 01", "Book 02", "Book 03", "Book 04"]);
    let before = library::catalog::generation(app.storage.0.as_ref()).unwrap();
    for n in 0..300 {
        let path = &names[n % 2];
        let reading = events::Reading::new(path.clone(), String::new(), 100.0, 0, 0, String::new());
        app.send(events::BOOK_CLOSED.id().clone(), reading.to_params());
    }
    (app, before)
}

/// The rows the list shows, all, then those finished.
fn shown_and_finished(app: &mut App) {
    assert_eq!(rows(app), ["Book 01", "Book 02", "Book 03", "Book 04"]);
    let finished = paperpaw_sdk::ui::choice::Picked::new(String::new(), String::from("finished"));
    app.send(Id::new_static("files.list.shown"), finished.to_params());
    assert_eq!(rows(app), ["Book 01", "Book 02"], "{}", app.described());
}

#[test]
fn the_catalog_folded_while_the_list_shows_keeps_it_right() {
    let _turn = turn();
    // A device of much memory: folded at a calm moment beside the list,
    // whose lines move; the list follows.
    let (mut app, before) = read_often(LARGE);
    app.wait(61_000);
    let after = library::catalog::generation(app.storage.0.as_ref()).unwrap();
    assert!(after > before, "not folded");
    shown_and_finished(&mut app);
}

#[test]
fn on_the_x4_the_catalog_is_caught_up_beside_the_list() {
    let _turn = turn();
    // Catching up takes a few kilobytes: there is room for it beside the
    // list's index, and the list follows.
    let (mut app, before) = read_often(X4);
    app.wait(61_000);
    let after = library::catalog::generation(app.storage.0.as_ref()).unwrap();
    assert!(after > before, "not folded beside the list");
    shown_and_finished(&mut app);
}

/// On the X4 a walk goes on beside the list (which holds a few
/// kilobytes of the sorted books, not an index of them all), and the
/// list shows the catalog made anew.
#[test]
fn on_the_x4_a_walk_goes_on_beside_the_list() {
    let _turn = turn();
    let (mut app, _) = read_often(X4);
    app.wait(61_000);
    let generation = |app: &App| library::catalog::generation(app.storage.0.as_ref()).unwrap();
    let before = generation(&app);
    // A favourite chosen without a word: a walk makes the catalog anew
    // (one that finds nothing changed keeps it, and its generation).
    app.storage
        .0
        .write(".paperpaw/library/favourites.txt", b"books/Book 04.txt\n")
        .unwrap();
    let anew = events::CatalogWanted::new(true);
    app.send(events::CATALOG_WANTED.id().clone(), anew.to_params());
    app.wait(61_000);
    assert!(generation(&app) > before, "not walked beside the list");
    shown_and_finished(&mut app);
}
