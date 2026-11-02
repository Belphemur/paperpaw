//! What plugins add to the library, with the real kernel and the
//! standard app: a badge on books' rows, a field on a book's
//! information, an action in the book menu, told for which book.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::borrow::Cow;
use std::cell::RefCell;

use common::*;
use paperpaw_sdk::builtin::{events, menus};
use paperpaw_sdk::library::{self, Badge, BookBadge, BookField};
use paperpaw_sdk::{
    ActionSpec, Commands, Manifest, MenuEntry, Plugin, RegisterError, Registrar, Version,
};

const TEXT: &[u8] = b"Some text of a book.\n\nAnd more of it.\n";
const SEND: Id = Id::new_static("adds.send");

thread_local! {
    /// The books the action was chosen for, and that it ran.
    static CHOSEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Adds a badge ("Loaned" on the second book), a field (a rating) and
/// an action to the book menu.
struct Adds;

impl Plugin for Adds {
    fn manifest(&self) -> Manifest {
        Manifest::new(Id::new_static("adds"), "Adds", Version::new(1, 0, 0))
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        r.add_translations(Cow::Borrowed(
            "[meta]\nlanguage = en\n[messages]\nadds.loaned = Loaned\nadds.rating = Rating\nadds.send = Send away\n",
        ))?;
        r.add_book_badge(BookBadge::new(
            Id::new_static("adds.loaned"),
            "adds.loaned",
            |_, record| {
                record
                    .path
                    .contains("02")
                    .then(|| Badge::Text(String::from("Loaned")))
            },
        ))?;
        r.add_book_field(BookField::new(Id::new_static("adds.rating"), "adds.rating"))?;
        r.add_action(
            ActionSpec::new(SEND, "adds.send").handler(|_, _, _: &mut Commands| {
                CHOSEN.with(|c| c.borrow_mut().push(String::from("ran")));
            }),
        )?;
        r.add_menu_entry(MenuEntry::new(menus::BOOK, SEND))?;
        r.on(events::BOOK_CHOSEN, |_, chosen, _| {
            CHOSEN.with(|c| c.borrow_mut().push(chosen.path.clone()));
        })?;
        Ok(())
    }
}

fn library(device: DeviceInfo) -> App {
    let names: Vec<String> = (1..=3).map(|n| format!("books/Book {n:02}.txt")).collect();
    let files: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), TEXT)).collect();
    let config = "[files]\nsort = files.sort.name\n";
    let mut app = App::with(device, Shared::memory(config, &files), Adds);
    app.run(actions::LIBRARY);
    assert_eq!(app.screen(), "files.browser");
    app
}

#[test]
fn badges_show_on_the_rows_of_their_books() {
    let _turn = turn();
    for device in DEVICES {
        let mut app = library(device);
        let rows: Vec<String> = app
            .parts()
            .into_iter()
            .filter(|p| p.role == Role::Row)
            .map(|p| p.label)
            .collect();
        let loaned: Vec<&String> = rows.iter().filter(|r| r.contains("Loaned")).collect();
        assert_eq!(loaned.len(), 1, "{rows:?}");
        assert!(loaned[0].starts_with("Book 02"), "{rows:?}");
    }
}

#[test]
fn book_fields_show_on_a_books_information() {
    let _turn = turn();
    let mut app = library(X4);
    let path = "books/Book 01.txt";
    // As a plugin does, through the kernel's storage.
    library::set_field(app.kernel.storage(), path, "adds.rating", "4 of 5");
    app.open_slot(slots::BOOK_INFO, Params::new().with(params::PATH, path));
    assert!(app.shows("Rating"), "{}", app.described());
    assert!(app.shows("4 of 5"), "{}", app.described());
}

#[test]
fn actions_of_the_book_menu_are_told_their_book() {
    let _turn = turn();
    for device in DEVICES {
        CHOSEN.with(|c| c.borrow_mut().clear());
        let mut app = library(device);
        if device.input.touch {
            app.long_press_on("Book 03");
        } else {
            app.repeat(actions::DOWN, 2);
            app.run(actions::MENU);
        }
        assert!(app.shows("Send away"), "{}", app.described());
        if device.input.touch {
            app.tap_on("Send away");
        } else {
            while app.focused().is_none_or(|f| !f.contains("Send away")) {
                app.run(actions::DOWN);
            }
            app.run(actions::CONFIRM);
        }
        let chosen = CHOSEN.with(|c| c.borrow().clone());
        assert_eq!(chosen, ["books/Book 03.txt", "ran"]);
    }
}
