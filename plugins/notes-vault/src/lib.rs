//! Notes vault: a folder of Markdown notes read on the device, as a vault
//! of notes keeps them (it works with Obsidian vaults: plain `.md` files in
//! folders, with `[[wikilinks]]`, front matter and `#tags`).
//!
//! - *Notes vault* in the tools: the vault's folders, its latest changes,
//!   its tags, and a search in the notes' titles. A note opens in the
//!   reader; its links to other notes (by name, as a vault writes them)
//!   open them, and *Back* returns.
//! - Home widgets: the recent notes, a pinned note, a note of the day, a
//!   random note, the tags, and a search (see `widgets`).
//!
//! The vault is read as it is on the card, never changed. Its folders are
//! listed as they are; the latest changes, the tags and the search come
//! from an index of the notes made in the background, a piece at a time
//! while the vault's page is open, and kept in a file (see `index`): a
//! vault of thousands of notes costs time the first time, and a window of
//! rows of memory, never its size.

#![no_std]

extern crate alloc;

mod home;
mod index;
mod lines;
mod links;
mod list;
pub(crate) mod markdown;
mod note;
mod page;
mod source;
mod tags;
mod widgets;

pub use markdown::MarkdownDocument;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use paperpaw_sdk::builtin::{events, menus};
use paperpaw_sdk::document::Document;
use paperpaw_sdk::setting::{Setting, Text};
use paperpaw_sdk::{
    ActionSpec, Command, Commands, Env, FixedAction, FormatSpec, Id, ItemState, Manifest,
    MenuEntry, Params, Plugin, RegisterError, Registrar, ScreenSpec, SettingExtra, SettingSpec,
};

pub const PLUGIN: Id = Id::new_static("notes-vault");
/// The settings: the vault's folder.
pub const GROUP: Id = Id::new_static("notes-vault.group");
/// The folder of the vault, chosen with the folder chooser.
pub const FOLDER: Setting<Text> = Setting::new("notes-vault.folder");
/// Opens the vault's page.
/// The Markdown notes are written in, which replaces the built-in format
/// for `.md` while this plugin is on (a later format wins an extension).
pub const MARKDOWN: Id = Id::new_static("notes-vault.markdown");
pub const OPEN: Id = Id::new_static("notes-vault.open");
/// The vault's page: where to go, and the index.
pub(crate) const HOME_SCREEN: Id = Id::new_static("notes-vault.screen");
/// A list of notes: folder, latest changes, tag, search results.
pub(crate) const LIST_SCREEN: Id = Id::new_static("notes-vault.list");
/// The list of tags.
pub(crate) const TAGS_SCREEN: Id = Id::new_static("notes-vault.tags");
/// *Links in this note*, in the reader's menu while a note is open.
pub const LINKS: Id = Id::new_static("notes-vault.links");
/// The links of the open note.
pub(crate) const LINKS_SCREEN: Id = Id::new_static("notes-vault.links-screen");

static ON_FOLDER: SettingExtra =
    SettingExtra::new().editor(paperpaw_sdk::builtin::screens::FOLDER_EDITOR);

static SETTINGS: [SettingSpec; 1] =
    [
        SettingSpec::text(FOLDER.static_str(), "notes-vault.setting.folder", "", GROUP)
            .with_extra(&ON_FOLDER)
            .description("notes-vault.setting.folder.help"),
    ];

static ACTIONS: [FixedAction; 1] = [FixedAction::new(OPEN, "notes-vault.title")
    .handler(|_, _, out| out.push(Command::OpenScreen(HOME_SCREEN, Params::new())))];
static ENTRIES: [MenuEntry; 2] = [
    MenuEntry::new(menus::TOOLS, OPEN).order(38),
    MenuEntry::new(menus::READER, LINKS).order(70),
];
static ICONS: [(Id, &str); 3] = [(OPEN, "note"), (GROUP, "note"), (LINKS, "note")];

static HOW: [(&str, &str); 2] = [
    ("en", include_str!("../how/en.txt")),
    ("de", include_str!("../how/de.txt")),
];

/// The vault's folder, as set, without slashes at its ends; empty where
/// none is chosen.
pub(crate) fn vault(env: &Env<'_>) -> String {
    env.settings()
        .read(&FOLDER)
        .trim()
        .trim_matches('/')
        .to_string()
}

#[derive(Debug, Default)]
pub struct NotesVault;

impl Plugin for NotesVault {
    fn manifest(&self) -> Manifest {
        Manifest::new(PLUGIN, "Notes vault", paperpaw_sdk::package_version!())
            .description(
                "Reads a folder of Markdown notes by folders, tags and latest changes, with \
                 widgets for the home screen. Works with Obsidian vaults.",
            )
            .how(&HOW)
            .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        // What is converted of each note, moved and removed with it.
        r.add_book_cache(Env::plugin_dir(PLUGIN.as_str()), "md-")?;
        r.add_format(FormatSpec::new(
            MARKDOWN,
            "notes-vault.markdown",
            alloc::vec![Cow::Borrowed("md"), Cow::Borrowed("markdown")],
            Box::new(|storage, path, options| {
                markdown::MarkdownDocument::open(storage, path, options.publisher_styles)
                    .map(|d| Box::new(d) as Box<dyn Document>)
            }),
        ))?;
        r.add_setting_group({
            static SPEC: paperpaw_sdk::SettingGroupSpec =
                paperpaw_sdk::SettingGroupSpec::new(GROUP, "notes-vault.title");
            &SPEC
        })?;
        r.add_settings(&SETTINGS)?;
        r.add_actions(&ACTIONS)?;
        r.add_menu_entries(&ENTRIES)?;
        r.add_icons(&ICONS)?;
        r.add_screen(ScreenSpec::new(
            HOME_SCREEN,
            "notes-vault.title",
            Box::new(|env, _| Box::new(home::HomeScreen::new(env))),
        ))?;
        r.add_screen(ScreenSpec::new(
            LIST_SCREEN,
            "notes-vault.title",
            Box::new(|env, params| Box::new(list::ListScreen::new(env, params))),
        ))?;
        r.add_screen(ScreenSpec::new(
            LINKS_SCREEN,
            "notes-vault.links",
            Box::new(|env, params| Box::new(links::LinksScreen::new(env, params))),
        ))?;
        r.add_screen(ScreenSpec::new(
            TAGS_SCREEN,
            "notes-vault.tags",
            Box::new(|env, _| Box::new(home::TagsScreen::new(env))),
        ))?;
        register_links(r)?;
        widgets::register(r)?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/en.ini")))?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/de.ini")))?;
        Ok(())
    }
}

/// The notes open in the reader, the last on top: what *Links in this
/// note* lists the links of.
type Open = Rc<RefCell<Vec<String>>>;

/// *Links in this note* in the reader's menu, shown while the note on top
/// is one of a vault's: the links of a note to other notes, which
/// tapping them in the text does not follow (the reader follows links
/// in its book only).
fn register_links(r: &mut Registrar) -> Result<(), RegisterError> {
    let open: Open = Rc::new(RefCell::new(Vec::new()));
    let opened = open.clone();
    r.on(events::BOOK_OPENED, move |_, reading, _| {
        let mut open = opened.borrow_mut();
        open.retain(|p| *p != reading.path);
        open.push(reading.path.clone());
    })?;
    let closed = open.clone();
    r.on(events::BOOK_CLOSED, move |_, reading, _| {
        closed.borrow_mut().retain(|p| *p != reading.path);
    })?;
    let note = open.clone();
    r.add_action(ActionSpec::new(LINKS, "notes-vault.links").handler(
        move |_: &Env<'_>, _, out: &mut Commands| {
            if let Some(path) = note.borrow().last().filter(|p| note::is_note(p)) {
                out.push(Command::OpenScreen(
                    LINKS_SCREEN,
                    Params::new().with(links::NOTE, path.clone()),
                ));
            }
        },
    ))?;
    r.set_item_state(LINKS, move |_| {
        let mut state = ItemState::new();
        state.shown = open.borrow().last().is_some_and(|p| note::is_note(p));
        state
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use paperpaw_i18n::{Catalog, check};

    #[test]
    fn translations_are_complete() {
        let en = Catalog::parse(include_str!("../i18n/en.ini")).unwrap();
        let de = Catalog::parse(include_str!("../i18n/de.ini")).unwrap();
        let issues = check::compare(&en, &de);
        assert!(issues.is_empty(), "{issues:?}");
    }
}
