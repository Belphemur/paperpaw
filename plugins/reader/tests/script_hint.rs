//! A book in a script the reading font has no letters of says so once, and
//! names the pack from the store that brings them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{BUTTONS, Shared};
use paperpaw_core::Kernel;
use paperpaw_plugin_reader::Reader;
use paperpaw_sdk::builtin::{params, slots};
use paperpaw_sdk::{
    Instant, Manifest, Params, Plugin, Priority, RegisterError, Registrar, Storage, Version,
};
use paperpaw_zip::testing::ZipBuilder;

const BOOK_IN: &str = "books/lang.epub";

/// The notices shown.
type Notices = Rc<RefCell<Vec<String>>>;

/// A plugin that shows notices as nothing and keeps their texts.
struct NoticeTaker(Notices);

/// A notice, shown as nothing.
struct Notice;

impl paperpaw_sdk::Screen for Notice {
    fn draw(&mut self, _: &mut paperpaw_sdk::Canvas<'_>, _: &paperpaw_sdk::Env<'_>) {}

    fn on_action(
        &mut self,
        _: &paperpaw_sdk::Id,
        _: &mut paperpaw_sdk::Cx<'_, '_>,
    ) -> paperpaw_sdk::Handled {
        paperpaw_sdk::Handled::No
    }
}

impl Plugin for NoticeTaker {
    fn manifest(&self) -> Manifest {
        Manifest::new(
            paperpaw_sdk::Id::new_static("notices"),
            "Notices",
            Version::new(0, 1, 0),
        )
        .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let notices = self.0.clone();
        r.add_screen(paperpaw_sdk::ScreenSpec::new(
            paperpaw_sdk::Id::new_static("notices.notice"),
            "notice",
            Box::new(move |_, params| {
                notices
                    .borrow_mut()
                    .push(params.get(params::TEXT).unwrap_or_default().to_string());
                Box::new(Notice)
            }),
        ))?;
        r.bind_slot(
            slots::NOTICE,
            paperpaw_sdk::Id::new_static("notices.notice"),
            Priority::DEFAULT,
        )
    }
}

/// A one-chapter EPUB that says it is in `language`.
fn epub_in(language: &str) -> Vec<u8> {
    let container = r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles>
</container>"#;
    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Lang</dc:title><dc:language>{language}</dc:language></metadata>
  <manifest><item id="c0" href="c0.xhtml" media-type="application/xhtml+xml"/></manifest>
  <spine><itemref idref="c0"/></spine>
</package>"#
    );
    let page = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>c0</title></head><body><p>Some words.</p></body></html>"#;
    ZipBuilder::new()
        .stored("mimetype", "application/epub+zip")
        .deflated("META-INF/container.xml", container)
        .deflated("OEBPS/content.opf", opf)
        .deflated("OEBPS/c0.xhtml", page)
        .build()
}

/// The notices said on opening the book in `language` over `storage`.
fn opened_in(language: &str, storage: &Shared) -> Vec<String> {
    storage.create_dir("books").ok();
    storage.write(BOOK_IN, &epub_in(language)).unwrap();
    let notices = Notices::default();
    let mut kernel = Kernel::builder(BUTTONS)
        .storage(storage.clone())
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(paperpaw_plugin_format_epub::EpubFormat)
        .plugin(Reader)
        .plugin(NoticeTaker(notices.clone()))
        .build()
        .unwrap();
    kernel.start(Instant(0));
    let mut now = 10;
    let book = Params::new().with(params::PATH, BOOK_IN);
    let mut effects = kernel.open_slot(Instant(now), slots::READER, book);
    for _ in 0..20_000 {
        let Some(at) = effects.wake_at() else {
            break;
        };
        if at.0 > now + 1000 {
            break;
        }
        now = now.max(at.0);
        effects = kernel.tick(Instant(now));
    }
    notices.take()
}

#[test]
fn a_book_in_greek_names_the_pack_once() {
    let storage = Shared::default();
    let said = opened_in("el", &storage);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("Greek letters"), "{said:?}");
    // Not again, the next time it is opened.
    assert!(opened_in("el", &storage).is_empty());
}

#[test]
fn each_script_names_its_own_pack() {
    for (language, pack) in [
        ("ru", "Cyrillic letters"),
        ("ar", "Arabic letters"),
        ("fa", "Arabic letters"),
        ("he", "Hebrew letters"),
    ] {
        let said = opened_in(language, &Shared::default());
        assert_eq!(said.len(), 1, "{language}: {said:?}");
        assert!(said[0].contains(pack), "{language}: {said:?}");
    }
}

#[test]
fn books_in_latin_script_or_with_no_language_say_nothing() {
    for language in ["en", "de", "fr", "pt-BR", ""] {
        let said = opened_in(language, &Shared::default());
        assert!(said.is_empty(), "{language}: {said:?}");
    }
}
