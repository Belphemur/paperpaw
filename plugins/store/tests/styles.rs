//! Literata's bold and italic from the card: the lean firmware (the X4
//! family's) has regular Literata alone, and the store fetches the data
//! plugin `literata-styles` by itself at the first start after the
//! first-run setup, once and quietly; a firmware that has the faces, or a
//! card that has the plugin, asks for nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{DEVICE, Shared};
use paperpaw_core::{Io, Kernel};
use paperpaw_sdk::builtin::fonts;
use paperpaw_sdk::{
    Font, FontSpec, Id, Instant, Manifest, Plugin, Priority, RegisterError, Registrar, Storage,
    Version,
};
use paperpaw_text::{Glyph, LineMetrics, PackBuilder};

/// What the lean firmware has of Literata: the regular face alone.
struct Lean;

impl Plugin for Lean {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            Id::new_static("fonts-default"),
            "Fonts",
            Version::new(1, 0, 0),
        )
        .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let mut pack = PackBuilder::new();
        let metrics = LineMetrics::new(8.0, 2.0, 12.0);
        pack.add_strike(10.0, metrics, vec![('a', Glyph::empty(5.0))]);
        let font = Font::from_data(pack.build(), &[]).unwrap();
        let id = Id::new_static("fonts-default.literata");
        r.add_font(FontSpec::new(id.clone(), font).family("Literata", false, false))?;
        r.bind_font(fonts::READING, id, Priority::DEFAULT)
    }
}

const SETUP_DONE: &str = ".paperpaw/plugins/setup/done";
const PAGE: &str = "https://paperpaw.org/api/v1/plugins/literata-styles.json?device=x4&lang=en";

/// A kernel started on `storage`, with the fonts of the firmware `lean`
/// or not, and what it asks the network for.
fn started(storage: Shared, lean: bool) -> Vec<String> {
    let mut builder = Kernel::builder(DEVICE)
        .storage(storage)
        .network(true)
        .catalog_device("x4")
        .plugin(paperpaw_plugin_menu::Menu)
        .plugin(paperpaw_plugin_store::Store);
    builder = if lean {
        builder.plugin(Lean)
    } else {
        builder.plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
    };
    let mut kernel = builder.build().unwrap();
    kernel.start(Instant(0));
    kernel
        .take_io()
        .into_iter()
        .filter_map(|io| match io {
            Io::Http(call) => Some(call.url),
            _ => None,
        })
        .collect()
}

fn after_setup() -> Shared {
    let storage = Shared::default();
    storage.write(SETUP_DONE, b"").unwrap();
    storage
}

#[test]
fn the_lean_firmware_asks_for_the_faces_once_the_setup_is_over() {
    let storage = after_setup();
    assert_eq!(started(storage.clone(), true), [PAGE]);
    let queue = paperpaw_sdk::read_file(&storage, ".paperpaw/plugins/store/queue.txt", 4096);
    assert_eq!(
        String::from_utf8(queue.unwrap()).unwrap(),
        "> ~literata-styles\n"
    );
}

#[test]
fn not_before_the_setup_is_over() {
    assert!(started(Shared::default(), true).is_empty());
}

#[test]
fn a_firmware_with_the_faces_asks_for_nothing() {
    assert!(started(after_setup(), false).is_empty());
}

#[test]
fn a_card_with_the_plugin_asks_for_nothing() {
    let storage = after_setup();
    storage
        .write(
            "plugins/literata-styles/plugin.ini",
            b"[plugin]\nid = literata-styles\nname = Styles\nversion = 1.0.0\n",
        )
        .unwrap();
    assert!(started(storage, true).is_empty());
}

/// Not asked again and again: three starts at most, then it is the
/// reader's to find.
#[test]
fn it_is_tried_three_times_at_most() {
    let storage = after_setup();
    for round in 1..=3 {
        assert_eq!(started(storage.clone(), true), [PAGE], "start {round}");
        // The page did not come: the queue gave up on it.
        storage.remove(".paperpaw/plugins/store/queue.txt").unwrap();
    }
    assert!(started(storage, true).is_empty());
}

/// A start while it still waits (no network yet) does not put it in
/// twice.
#[test]
fn it_waits_in_the_queue_without_doubling() {
    let storage = after_setup();
    storage
        .write(".paperpaw/plugins/store/queue.txt", b"~literata-styles\n")
        .unwrap();
    let asked = started(storage.clone(), true);
    // The queue goes on by itself (the reader asked for it by updating).
    assert_eq!(asked, [PAGE]);
    let queue = paperpaw_sdk::read_file(&storage, ".paperpaw/plugins/store/queue.txt", 4096);
    assert_eq!(
        String::from_utf8(queue.unwrap()).unwrap(),
        "> ~literata-styles\n"
    );
}
