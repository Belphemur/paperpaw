//! A font plugin installed while a book is open (the store fetches
//! Literata's bold and italic in the background): the book is laid out
//! again with the faces there are now, under the page index of its new
//! key, not left with pages measured for the regular face.

use super::*;
use paperpaw_sdk::{
    ActionSpec, Command, Commands, Env, Font, FontSpec, MemoryStorage as Memory, Priority, Uses,
};
use paperpaw_text::{Glyph, LineMetrics, PackBuilder};

const INSTALL: Id = Id::new_static("installer.install");

/// A pack of the letters a, e and t, `advance` wide.
fn pack(advance: f32) -> Vec<u8> {
    let mut builder = PackBuilder::new();
    let metrics = LineMetrics::new(16.0, 4.0, 24.0);
    let glyphs = "aet".chars().map(|c| (c, Glyph::empty(advance))).collect();
    builder.add_strike(20.0, metrics, glyphs);
    builder.build()
}

/// What the lean firmware has of Literata: the regular face alone.
struct Lean;

impl Plugin for Lean {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            Id::new_static("fonts-default"),
            "Fonts",
            Version::new(1, 0, 0),
        )
        .uses(Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let font = Font::from_data(pack(10.0), &[]).unwrap();
        let id = Id::new_static("fonts-default.literata");
        r.add_font(FontSpec::new(id.clone(), font).family("Literata", false, false))?;
        r.bind_font(paperpaw_sdk::builtin::fonts::READING, id, Priority::DEFAULT)
    }
}

/// The store, as far as it matters: an action that installs the plugin.
struct Installer;

impl Plugin for Installer {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            Id::new_static("installer"),
            "Installer",
            Version::new(0, 1, 0),
        )
        .uses(Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        r.add_action(ActionSpec::new(INSTALL, "installer.install").handler(
            |_: &Env<'_>, _, out: &mut Commands| {
                out.push(Command::InstallPlugin(
                    Id::new_static("literata-styles"),
                    String::from("plugins/.styles.part"),
                ));
            },
        ))
    }
}

/// The data plugin: a bold face for the reading role.
fn styles() -> Memory {
    Memory::new()
        .with(
            "plugins/.styles.part/plugin.ini",
            "[plugin]\nid = literata-styles\nname = Styles\nversion = 1.0.0\nformats = 3\n\n\
             [family]\nname = Literata\nbold = bold.ppfp\n\n\
             [fonts]\ncore.font-reading-bold = literata-styles.bold\n",
        )
        .with("plugins/.styles.part/fonts/bold.ppfp", pack(12.0))
}

/// Opens the book on a lean firmware, with the pack waiting on the card.
fn lean_reading() -> Reading {
    let storage = Shared::default();
    let text = include_str!("../../../../samples/library/books/alice-in-wonderland.txt");
    storage.create_dir("books").unwrap();
    storage.write(BOOK, text.as_bytes()).unwrap();
    let stored = styles();
    for path in [
        "plugins/.styles.part/plugin.ini",
        "plugins/.styles.part/fonts/bold.ppfp",
    ] {
        let data = paperpaw_sdk::read_file(&stored, path, 1 << 16).unwrap();
        storage.create_dir(path.rsplit_once('/').unwrap().0).ok();
        storage.write(path, &data).unwrap();
    }
    let heard = Heard::default();
    let kernel = Kernel::builder(DEVICE)
        .storage(storage.clone())
        .plugin(Lean)
        .plugin(Installer)
        .plugin(paperpaw_plugin_format_txt::TxtFormat)
        .plugin(Reader)
        .plugin(Listener(heard.clone()))
        .build()
        .unwrap();
    let mut reading = Reading {
        kernel,
        storage,
        heard,
        now: 0,
        book: BOOK,
    };
    let effects = reading.kernel.start(Instant(0));
    reading.settle(effects);
    reading
}

#[test]
fn a_book_open_when_the_faces_arrive_is_laid_out_again() {
    let mut reading = lean_reading();
    reading.open();
    assert!(
        reading
            .kernel
            .fonts()
            .bound("core.font-reading-bold")
            .is_none()
    );
    let before = indexes(&reading.storage, BOOK);
    assert_eq!(before.len(), 1, "{before:?}");
    // The pack arrives while the book is on the screen.
    reading.now += 100;
    let effects = reading.kernel.run_action(Instant(reading.now), INSTALL);
    reading.settle(effects);
    assert!(
        reading
            .kernel
            .fonts()
            .bound("core.font-reading-bold")
            .is_some()
    );
    assert_eq!(
        reading.kernel.active_screen().map(Id::as_str),
        Some("reader.screen")
    );
    // Its pages are counted anew, under the key of the new faces.
    let after = indexes(&reading.storage, BOOK);
    assert_eq!(after.len(), 2, "{after:?}");
    assert!(after.iter().any(|name| !before.contains(name)));
}
