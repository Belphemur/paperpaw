//! Literata's bold and italic from the card: the firmware leaves them out
//! (`PAPERPAW_FONT_LEAN`), the data plugin `literata-styles` brings them.
//! The sections of its `plugin.ini` are the files the release builds it
//! from (`tools/plugin-files`), so what is tested is what is shipped.
//!
//! - Without the plugin the roles of bold and italic are unbound, and
//!   readers set that text in the regular face: nothing is missing or
//!   drawn as `?`.
//! - With it they are bound to its faces, in the family Literata.
//! - The larger sizes (`literata-large`) extend those faces, and are
//!   harmless without them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::DEVICE;
use paperpaw_core::Kernel;
use paperpaw_core::sdk::builtin::fonts;
use paperpaw_core::sdk::{
    Font, FontSpec, Id, Manifest, MemoryStorage, Plugin, Priority, RegisterError, Registrar,
    Version,
};
use paperpaw_platform::Instant;
use paperpaw_text::{Glyph, LineMetrics, PackBuilder};

const STYLES: &str = include_str!("../../../tools/plugin-files/literata-styles.ini");
const LARGE: &str = include_str!("../../../tools/plugin-files/literata-large.ini");

/// A pack of `sizes`, its `a` `advance` wide: the advance tells the
/// faces apart.
fn pack(sizes: &[f32], advance: f32) -> Vec<u8> {
    let mut builder = PackBuilder::new();
    for &size in sizes {
        let metrics = LineMetrics::new(size * 0.8, size * 0.2, size * 1.2);
        builder.add_strike(size, metrics, vec![('a', Glyph::empty(advance))]);
    }
    builder.build()
}

/// What the lean firmware has of Literata: the regular face up to 32 px.
struct Lean;

impl Plugin for Lean {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            Id::new_static("fonts-default"),
            "Fonts",
            Version::new(1, 0, 0),
        )
        .uses(paperpaw_core::sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let font = Font::from_data(pack(&[20.0, 32.0], 10.0), &[]).unwrap();
        let id = Id::new_static("fonts-default.literata");
        r.add_font(FontSpec::new(id.clone(), font).family("Literata", false, false))?;
        r.bind_font(fonts::READING, id, Priority::DEFAULT)
    }
}

/// As `tools/plugin-files/data.py` and `tools/literata-packs.py` write it.
fn plugin_ini(id: &str, name: &str, sections: &str) -> String {
    format!("[plugin]\nid = {id}\nname = {name}\nversion = 1.0.0\nformats = 3\n\n{sections}")
}

/// The card with the plugins in `which`.
fn card(styles: bool, large: bool) -> MemoryStorage {
    let mut card = MemoryStorage::new();
    if styles {
        card = card
            .with(
                "plugins/literata-styles/plugin.ini",
                plugin_ini("literata-styles", "Literata bold and italic", STYLES),
            )
            .with(
                "plugins/literata-styles/fonts/literata-bold.ppfp",
                pack(&[20.0, 32.0], 12.0),
            )
            .with(
                "plugins/literata-styles/fonts/literata-italic.ppfp",
                pack(&[20.0, 32.0], 8.0),
            );
    }
    if large {
        card = card
            .with(
                "plugins/literata-large/plugin.ini",
                plugin_ini("literata-large", "Literata, larger sizes", LARGE),
            )
            .with(
                "plugins/literata-large/fonts/literata-large.ppfp",
                pack(&[48.0], 10.0),
            )
            .with(
                "plugins/literata-large/fonts/literata-bold-large.ppfp",
                pack(&[48.0], 12.0),
            )
            .with(
                "plugins/literata-large/fonts/literata-italic-large.ppfp",
                pack(&[48.0], 8.0),
            );
    }
    card
}

fn started(card: MemoryStorage) -> Kernel {
    started_with(card, "")
}

fn started_with(card: MemoryStorage, config: &str) -> Kernel {
    let mut kernel = Kernel::builder(DEVICE)
        .plugin(Lean)
        .storage(card)
        .user_config(config)
        .build()
        .unwrap();
    kernel.start(Instant(0));
    assert!(kernel.take_warnings().is_empty());
    kernel
}

#[test]
fn without_the_plugin_bold_and_italic_are_unbound_and_regular_text_stands_in() {
    let kernel = started(card(false, false));
    let fonts = kernel.fonts();
    assert!(fonts.bound(fonts::READING_BOLD.as_str()).is_none());
    assert!(fonts.bound(fonts::READING_ITALIC.as_str()).is_none());
    assert!(fonts.bound(fonts::READING_BOLD_ITALIC.as_str()).is_none());
    // The regular face is there and draws (readers use it for them).
    let regular = fonts.role(fonts::READING.as_str()).unwrap();
    assert_eq!(regular.id().as_str(), "fonts-default.literata");
    assert_eq!(regular.measure(20.0, "a"), 10.0);
}

#[test]
fn with_the_plugin_bold_and_italic_come_from_the_card() {
    let kernel = started(card(true, false));
    let fonts = kernel.fonts();
    let bold = fonts.bound(fonts::READING_BOLD.as_str()).unwrap();
    let italic = fonts.bound(fonts::READING_ITALIC.as_str()).unwrap();
    assert_eq!(bold.id().as_str(), "literata-styles.bold");
    assert_eq!(italic.id().as_str(), "literata-styles.italic");
    assert_eq!(bold.measure(20.0, "a"), 12.0);
    assert_eq!(italic.measure(20.0, "a"), 8.0);
    // Regular stays the firmware's; bold italic is the other plugin's.
    let regular = fonts.role(fonts::READING.as_str()).unwrap();
    assert_eq!(regular.id().as_str(), "fonts-default.literata");
    assert!(fonts.bound(fonts::READING_BOLD_ITALIC.as_str()).is_none());
}

#[test]
fn with_the_family_chosen_too() {
    let kernel = started_with(card(true, false), "[core]\nreading-font = Literata\n");
    let bold = kernel.fonts().bound(fonts::READING_BOLD.as_str()).unwrap();
    assert_eq!(bold.id().as_str(), "literata-styles.bold");
}

#[test]
fn the_larger_sizes_extend_regular_bold_and_italic() {
    let kernel = started(card(true, true));
    let fonts = kernel.fonts();
    for (role, advance) in [
        (fonts::READING, 10.0),
        (fonts::READING_BOLD, 12.0),
        (fonts::READING_ITALIC, 8.0),
    ] {
        let face = fonts.bound(role.as_str()).unwrap();
        assert_eq!(face.largest_size(), Some(48.0), "{role}");
        assert_eq!(face.measure(48.0, "a"), advance, "{role} at 48 px");
    }
}

#[test]
fn the_larger_sizes_without_the_styles_extend_regular_and_do_no_harm() {
    let kernel = started(card(false, true));
    let fonts = kernel.fonts();
    let regular = fonts.role(fonts::READING.as_str()).unwrap();
    assert_eq!(regular.largest_size(), Some(48.0));
    assert!(fonts.bound(fonts::READING_BOLD.as_str()).is_none());
}

/// A family of the reader's own takes the reading roles: the pack's faces
/// are not opened for nothing.
#[test]
fn with_another_family_chosen_the_packs_faces_stay_closed() {
    let card = card(true, false)
        .with(
            "plugins/other/plugin.ini",
            plugin_ini(
                "other",
                "Other",
                "[family]\nname = Other\nregular = other.ppfp\n",
            ),
        )
        .with("plugins/other/fonts/other.ppfp", pack(&[20.0, 32.0], 9.0));
    let kernel = started_with(card, "[core]\nreading-font = Other\n");
    let fonts = kernel.fonts();
    assert!(fonts.get("literata-styles.bold").is_none());
    assert!(fonts.get("literata-styles.italic").is_none());
    let regular = fonts.role(fonts::READING.as_str()).unwrap();
    assert_eq!(regular.id().as_str(), "other.regular");
    assert!(fonts.bound(fonts::READING_BOLD.as_str()).is_none());
}

/// A face on the card the reader binds in their own `[fonts]` is loaded.
#[test]
fn a_face_the_reader_binds_is_loaded() {
    let kernel = started_with(
        card(true, false),
        "[fonts]\ncore.font-reading-bold = literata-styles.italic\n",
    );
    let bold = kernel.fonts().bound(fonts::READING_BOLD.as_str()).unwrap();
    assert_eq!(bold.id().as_str(), "literata-styles.italic");
    // The proposal it replaces is not opened for nothing.
    assert!(kernel.fonts().get("literata-styles.bold").is_none());
}

/// A proposal naming a font nobody added does not hide the next one.
#[test]
fn a_proposal_without_a_font_gives_way_to_the_next() {
    let card = card(true, false).with(
        "plugins/zzz-ghost/plugin.ini",
        plugin_ini(
            "zzz-ghost",
            "Ghost",
            "[fonts]\ncore.font-reading-bold = zzz-ghost.bold\n",
        ),
    );
    let mut kernel = Kernel::builder(DEVICE)
        .plugin(Lean)
        .storage(card)
        .user_config("")
        .build()
        .unwrap();
    kernel.start(Instant(0));
    let bold = kernel.fonts().bound(fonts::READING_BOLD.as_str()).unwrap();
    assert_eq!(bold.id().as_str(), "literata-styles.bold");
}
