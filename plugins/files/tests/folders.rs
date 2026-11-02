//! The folder chooser (the editor of the library and archive folders)
//! with the real kernel on a folder of the computer's file system, by
//! buttons and by touch: starting in the folder set (`books`, the
//! library's default, where nothing else is set), going into folders and
//! back out, choosing one (kept in `paperpaw.ini` and used by the
//! library), and new folders whose names try to leave the folder they are
//! made in.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::path::PathBuf;
use std::rc::Rc;

use common::*;

const CHOOSER: &str = "files.choose-folder";

/// A folder of the computer's file system as the device's storage, gone
/// when the test is.
struct Card {
    root: PathBuf,
}

impl Card {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("paperpaw-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // The storage is a folder inside, so that what escapes it shows.
        std::fs::create_dir_all(root.join("card")).unwrap();
        Self { root }
    }

    fn storage(&self) -> Shared {
        let storage = Shared(Rc::new(
            paperpaw_fs::DirStorage::new(self.root.join("card")).unwrap(),
        ));
        storage.set_config("");
        storage.write(SETUP_DONE, b"").unwrap();
        for path in [
            "books/Novels/Old/a.txt",
            "books/Novels/b.txt",
            "books/Poems/c.txt",
            "Downloads/d.txt",
        ] {
            storage.write(path, b"Some text.").unwrap();
        }
        storage
    }

    /// What is next to the storage's folder: nothing may get there.
    fn beside(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.root)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn chooser(device: DeviceInfo, storage: Shared, setting: &str) -> App {
    let mut app = App::new(device, storage);
    app.open_slot(slots::CHOOSE_FOLDER, Params::new().with("id", setting));
    assert_eq!(app.screen(), CHOOSER);
    app
}

/// The rows shown, by their labels.
fn rows(app: &mut App) -> Vec<String> {
    app.parts()
        .into_iter()
        .filter(|p| p.role == Role::Row)
        .map(|p| p.label)
        .collect()
}

/// Goes down to the row that says `label` and confirms it (buttons), or
/// taps it (touch).
fn choose(app: &mut App, label: &str) {
    if app.kernel.device().input.touch {
        app.tap_on(label);
        return;
    }
    for _ in 0..30 {
        if app
            .focused()
            .is_some_and(|f| f.split(", ").next() == Some(label))
        {
            app.run(actions::CONFIRM);
            return;
        }
        app.run(actions::DOWN);
    }
    panic!("no row {label}:\n{}", app.described());
}

/// The chooser starts in the folder set; it used to ask `Storage::exists`
/// (files only) and so always started at the top.
#[test]
fn it_starts_in_the_folder_set() {
    let _turn = turn();
    let card = Card::new("start");
    for device in DEVICES {
        let mut app = chooser(device, card.storage(), "core.library-folder");
        assert!(app.shows("books"), "{}", app.described());
        assert_eq!(
            rows(&mut app),
            [
                "Use this folder",
                "Parent folder",
                "New folder",
                "Novels/",
                "Poems/"
            ]
        );
    }
}

#[test]
fn folders_open_and_parent_folder_goes_up_to_the_folder_left() {
    let _turn = turn();
    let card = Card::new("up");
    for device in DEVICES {
        let mut app = chooser(device, card.storage(), "core.library-folder");
        choose(&mut app, "Parent folder");
        assert_eq!(
            rows(&mut app),
            ["Use this folder", "New folder", "books/", "Downloads/"]
        );
        // Coming up, the folder left has the focus.
        if !device.input.touch {
            assert_eq!(app.focused().as_deref(), Some("books/"));
        }
        choose(&mut app, "Downloads/");
        assert_eq!(
            rows(&mut app),
            ["Use this folder", "Parent folder", "New folder"]
        );
        choose(&mut app, "Parent folder");
        choose(&mut app, "books/");
        assert_eq!(
            rows(&mut app),
            [
                "Use this folder",
                "Parent folder",
                "New folder",
                "Novels/",
                "Poems/"
            ]
        );
    }
}

#[test]
fn a_folder_chosen_by_buttons_or_touch_is_kept_and_the_library_shows_it() {
    let _turn = turn();
    let card = Card::new("choose");
    for device in DEVICES {
        let storage = card.storage();
        let mut app = chooser(device, storage.clone(), "core.library-folder");
        choose(&mut app, "Novels/");
        assert_eq!(
            rows(&mut app),
            ["Use this folder", "Parent folder", "New folder", "Old/"]
        );
        choose(&mut app, "Use this folder");
        assert_ne!(app.screen(), CHOOSER);
        assert_eq!(
            app.kernel.settings().text("core.library-folder"),
            "books/Novels"
        );
        assert!(
            storage.config().contains("library-folder = books/Novels"),
            "{}",
            storage.config()
        );
        // The library lists that folder now, and after a restart too.
        let mut app = app.restart();
        app.run(actions::LIBRARY);
        // Its books, those in its folder Old too.
        let books: Vec<String> = app
            .parts()
            .into_iter()
            .filter(|p| p.role == Role::Row)
            .map(|p| p.label)
            .collect();
        assert_eq!(books, ["a", "b"], "{}", app.described());
        assert!(!app.shows("Poems"), "{}", app.described());
        storage.set_config("");
    }
}

#[test]
fn back_goes_up_first_and_closes_the_chooser_at_the_top() {
    let _turn = turn();
    let card = Card::new("back");
    let mut app = chooser(X4, card.storage(), "core.library-folder");
    choose(&mut app, "Novels/");
    choose(&mut app, "Old/");
    assert!(app.shows("books/Novels/Old"), "{}", app.described());
    app.run(actions::BACK);
    app.run(actions::BACK);
    app.run(actions::BACK);
    assert_eq!(app.screen(), CHOOSER);
    assert!(app.shows("Device"), "{}", app.described());
    app.run(actions::BACK);
    assert_ne!(app.screen(), CHOOSER);
    // Nothing was chosen.
    assert!(!app.kernel.settings().is_set("core.library-folder"));

    // Touch: the arrow in the header does the same.
    let mut app = chooser(TOUCH, card.storage(), "core.library-folder");
    app.tap_back();
    assert!(app.shows("Device"), "{}", app.described());
    app.tap_back();
    assert_ne!(app.screen(), CHOOSER);
}

#[test]
fn a_folder_set_that_is_gone_starts_at_the_top() {
    let _turn = turn();
    let card = Card::new("gone");
    let storage = card.storage();
    storage.set_config("[core]\nlibrary-folder = books/Vanished\n");
    // Made again at start, the library's folder can only go while the
    // device runs (the card taken out and changed elsewhere).
    let mut app = App::new(TOUCH, storage.clone());
    assert!(storage.names("books").contains(&"Vanished/".to_string()));
    let _ = storage.remove_dir_all("books/Vanished");
    app.open_slot(
        slots::CHOOSE_FOLDER,
        Params::new().with("id", "core.library-folder"),
    );
    assert_eq!(app.screen(), CHOOSER);
    assert!(app.shows("Device"), "{}", app.described());
}

#[test]
fn a_new_folder_is_made_where_shown_and_opened() {
    let _turn = turn();
    let card = Card::new("new");
    for device in DEVICES {
        let storage = card.storage();
        let mut app = chooser(device, storage.clone(), "files.archive");
        choose(&mut app, "books/");
        app.will_type("  Finished books ");
        choose(&mut app, "New folder");
        assert_eq!(app.screen(), CHOOSER);
        assert!(app.shows("Finished books"), "{}", app.described());
        assert!(
            storage
                .names("books")
                .contains(&"Finished books/".to_string())
        );
        choose(&mut app, "Use this folder");
        assert_eq!(
            app.kernel.settings().text("files.archive"),
            "books/Finished books"
        );
        let _ = storage.remove_dir_all("books/Finished books");
        storage.set_config("");
    }
}

#[test]
fn new_folder_names_cannot_leave_the_folder_or_name_none() {
    let _turn = turn();
    let card = Card::new("escape");
    let storage = card.storage();
    let before = storage.names("books");
    for name in ["..", "...", "/", "\\", ":", "   ", "\u{7}", "../", "./"] {
        let mut app = chooser(X4, storage.clone(), "core.library-folder");
        app.will_type(name);
        choose(&mut app, "New folder");
        assert_eq!(app.screen(), CHOOSER, "{name:?}");
        assert!(app.shows("books"), "{name:?}: {}", app.described());
        assert_eq!(storage.names("books"), before, "{name:?}");
    }
    // Slashes and the like go from the name; what is left is made here.
    for (typed, made) in [
        ("../../escape", "escape"),
        ("a/b\\c:d", "abcd"),
        ("..hidden", "hidden"),
        ("tab\there", "tabhere"),
    ] {
        let mut app = chooser(X4, storage.clone(), "core.library-folder");
        app.will_type(typed);
        choose(&mut app, "New folder");
        let names = storage.names("books");
        assert!(names.contains(&format!("{made}/")), "{typed:?}: {names:?}");
        let _ = storage.remove_dir_all(&format!("books/{made}"));
    }
    assert_eq!(card.beside(), ["card"]);
    assert_eq!(storage.names("books"), before);
}

#[test]
fn many_folders_are_paged_and_each_is_reached() {
    let _turn = turn();
    let card = Card::new("many");
    let storage = card.storage();
    for n in 0..40 {
        storage
            .write(&format!("books/Many/Folder {n:02}/x.txt"), b"x")
            .unwrap();
    }
    for device in DEVICES {
        let mut app = chooser(device, storage.clone(), "core.library-folder");
        choose(&mut app, "Many/");
        let mut seen: Vec<String> = Vec::new();
        for _ in 0..60 {
            for row in rows(&mut app) {
                if row.starts_with("Folder") && !seen.contains(&row) {
                    seen.push(row);
                }
            }
            if device.input.touch {
                app.tap_next_page();
            } else {
                app.run(actions::NEXT_PAGE);
            }
        }
        assert_eq!(seen.len(), 40, "{seen:?}");
        // Opening the last one works from its page.
        if device.input.touch {
            app.tap_on("Folder 39/");
        } else {
            choose(&mut app, "Folder 39/");
        }
        assert!(app.shows("Many/Folder 39"), "{}", app.described());
    }
}

/// The library folder is the kernel's setting; its editor opens the
/// folder chooser in its place, which sets it, and then goes back to
/// where the editor was opened from.
#[test]
fn the_kernels_folder_editor_opens_the_chooser() {
    let _turn = turn();
    let card = Card::new("editor");
    let mut app = App::new(X4, card.storage());
    app.run(actions::LIBRARY);
    let editor = paperpaw_sdk::builtin::screens::FOLDER_EDITOR;
    app.open_screen(editor, Params::new().with("id", "core.library-folder"));
    assert_eq!(app.screen(), CHOOSER);
    choose(&mut app, "Novels/");
    choose(&mut app, "Use this folder");
    assert_eq!(app.screen(), "files.browser");
    assert_eq!(
        app.kernel.settings().text("core.library-folder"),
        "books/Novels"
    );
}

#[test]
fn turned_the_chooser_keeps_its_folder_and_its_row() {
    let _turn = turn();
    let card = Card::new("turned");
    let storage = card.storage();
    for n in 0..40 {
        storage
            .write(&format!("books/Many/Folder {n:02}/x.txt"), b"x")
            .unwrap();
    }
    let mut app = chooser(X4, storage, "core.library-folder");
    choose(&mut app, "Many/");
    app.repeat(actions::DOWN, 5);
    let row = app.focused();
    assert!(row.is_some(), "a row has the focus");
    app.resize(800, 480);
    assert_eq!(app.screen(), CHOOSER);
    assert!(app.shows("Many"), "{}", app.described());
    assert_eq!(app.focused(), row, "landscape");
    app.resize(480, 800);
    assert_eq!(app.focused(), row, "upright again");
}
