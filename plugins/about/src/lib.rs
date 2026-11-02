//! *About PaperPaw*, in the device's settings: a page as the other
//! settings pages, its rows the version, the address of the source, and
//! where PaperPaw is the firmware the plugins built into it, which an
//! update brings again; where it loads native code plugins, what they
//! must be built with to fit it (its fingerprint, as the plugin library
//! lists builds).
//!
//! The long texts open in the reader (as Markdown), so they page and read
//! like a book, every word of them: the licences, PaperPaw's and those of
//! what it bundles (fonts, icons, hyphenation patterns), which every copy
//! of it must carry, a chapter each, PaperPaw's logo above them (the
//! picture [`logo::LOGO`] names, if one does); and the list of the
//! plugins built in.
//!
//! Other plugins add lines to the licences through the point [`LINES`]
//! (`about.lines`), each a paragraph below the address: a theme's
//! author, a dictionary's source, a font's licence that is not bundled.

#![no_std]

extern crate alloc;

mod screen;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::menus;
use paperpaw_sdk::logo;
use paperpaw_sdk::{
    Arg, Command, Env, FixedAction, FixedScreen, Id, Manifest, MenuEntry, Params, Plugin,
    RegisterError, Registrar,
};

pub const PLUGIN: Id = Id::new_static("about");
/// Shows the page.
pub const OPEN: Id = Id::new_static("about.open");
/// The page.
pub const SCREEN: Id = Id::new_static("about.screen");
/// The point other plugins add lines to the page through, a paragraph
/// each, after the plugin's name; as plain text (what Markdown makes of
/// signs is not made), at most [`MOST_LINES`] bytes a plugin.
pub const LINES: &str = "about.lines";
/// Bytes of lines a plugin adds, at most.
pub const MOST_LINES: usize = 512;
/// The licences of the build's product (the firmware's or the apps'),
/// from `tools/licences/build.py`, deflated in parts by `build.rs`: each
/// its deflated length and its length (both `u32`, little-endian), then
/// its bytes.
const LICENCES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/licences.bin"));
/// PaperPaw's logo, as the page shows it.
const LOGO: &[u8] = include_bytes!("../pictures/logo.png");
/// PaperPaw's version: its kernel's (this plugin has a version of its
/// own).
pub(crate) fn version(env: &Env<'_>) -> String {
    let kernel = env.plugins().kernel();
    format!(
        "{}",
        kernel.map_or(paperpaw_sdk::SDK_VERSION, |k| k.manifest.version)
    )
}
/// Where PaperPaw's source is.
pub(crate) const SOURCE: &str = "https://github.com/hthoene/paperpaw";

/// Where the licences are written for the reader: in the plugin's
/// folder, so they are no book.
pub(crate) fn licences_path() -> String {
    format!("{}/licences.md", Env::plugin_dir(PLUGIN.as_str()))
}

/// Where the list of the plugins built in is written for the reader.
pub(crate) fn plugins_path() -> String {
    format!("{}/plugins.md", Env::plugin_dir(PLUGIN.as_str()))
}

/// Writes `text` to `path` for the reader and opens it there; nothing
/// where it cannot be written.
pub(crate) fn read(env: &Env<'_>, path: String, text: &str) -> Option<Command> {
    let _ = env.storage().create_dir(&Env::plugin_dir(PLUGIN.as_str()));
    env.storage()
        .write(&path, text.as_bytes())
        .ok()
        .map(|()| paperpaw_sdk::readers::open(path))
}

/// The licences' parts, inflated one at a time; none where one cannot be.
fn licence_parts() -> impl Iterator<Item = Vec<u8>> {
    let mut rest = LICENCES;
    core::iter::from_fn(move || {
        let word = |at: usize| -> Option<usize> {
            let bytes: [u8; 4] = rest.get(at..at + 4)?.try_into().ok()?;
            usize::try_from(u32::from_le_bytes(bytes)).ok()
        };
        let (packed, len) = (word(0)?, word(4)?);
        let data = rest.get(8..8 + packed)?;
        rest = &rest[8 + packed..];
        miniz_oxide::inflate::decompress_to_vec_with_limit(data, len).ok()
    })
}

/// Writes the licences to [`licences_path`] for the reader and opens
/// them there: the page's start ([`licences`]), then the licences' parts
/// appended one by one, so they are never all in memory; nothing where
/// they cannot be written.
pub(crate) fn read_licences(env: &Env<'_>) -> Option<Command> {
    let path = licences_path();
    let _ = env.storage().create_dir(&Env::plugin_dir(PLUGIN.as_str()));
    env.storage().write(&path, licences(env).as_bytes()).ok()?;
    for part in licence_parts() {
        env.storage().append(&path, &part).ok()?;
    }
    Some(paperpaw_sdk::readers::open(path))
}

/// The start of the licences' page, the logo first: the picture set, or
/// PaperPaw's own written beside it; the licences follow it, a chapter
/// each ([`read_licences`]).
pub(crate) fn licences(env: &Env<'_>) -> String {
    let intro = env.i18n().tr("about.intro");
    let set = env.settings().read(&logo::LOGO).trim().trim_matches('/');
    let picture = if !set.is_empty() && env.storage().exists(set) {
        format!("/{set}")
    } else {
        let path = format!("{}/logo.png", Env::plugin_dir(PLUGIN.as_str()));
        if env.storage().size(&path).ok() != Some(LOGO.len() as u64) {
            let _ = env.storage().write(&path, LOGO);
        }
        String::from("logo.png")
    };
    let version = shown_version(env.i18n(), &version(env));
    format!(
        "![PaperPaw]({picture})\n\n# PaperPaw {version}\n\n{intro}\n\n{SOURCE}\n\n{}",
        lines(env),
    )
}

/// `version` as About shows it: a beta as one, calmly (`0.5.0 Beta 1`).
fn shown_version(i18n: &paperpaw_sdk::Translator, version: &str) -> String {
    match paperpaw_sdk::firmware::beta(version) {
        Some((release, number)) => i18n.format(
            "about.version.beta",
            &[("version", Arg::Str(release)), ("number", Arg::Str(number))],
        ),
        None => String::from(version),
    }
}

/// The lines other plugins add ([`LINES`]), a paragraph each, named by
/// their plugin; as plain text, so none makes a heading, a picture or a
/// list of its own; at most [`MOST_LINES`] bytes a plugin.
fn lines(env: &Env<'_>) -> String {
    let mut text = String::new();
    let mut used: Vec<(&Id, usize)> = Vec::new();
    for (plugin, contribution) in env.services().contributions(LINES) {
        for line in contribution
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
        {
            let at = match used.iter().position(|(p, _)| *p == plugin) {
                Some(at) => at,
                None => {
                    used.push((plugin, 0));
                    used.len() - 1
                }
            };
            let left = MOST_LINES.saturating_sub(used[at].1);
            if left == 0 {
                break;
            }
            let mut end = line.len().min(left);
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            used[at].1 += end;
            let name = env
                .plugins()
                .get(plugin.as_str())
                .map_or(plugin.as_str(), |info| info.name(env.i18n()));
            text.push('*');
            plain(name, &mut text);
            text.push_str(":* ");
            plain(&line[..end], &mut text);
            text.push_str("\n\n");
        }
    }
    text
}

/// `text` as plain text in Markdown: its signs escaped.
fn plain(text: &str, out: &mut String) {
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
}

/// The plugins built into the firmware, where PaperPaw is it, as a text
/// to read.
pub(crate) fn built_in(env: &Env<'_>) -> String {
    let Some(firmware) = env.firmware().filter(|f| !f.plugins.is_empty()) else {
        return String::new();
    };
    let mut text = format!(
        "# {}\n\n{}\n\n",
        env.i18n().tr("about.plugins.title"),
        env.i18n().tr("about.plugins.help")
    );
    for plugin in firmware.plugins.iter() {
        text.push_str(&format!("- {} {}\n", plugin.name, plugin.version));
    }
    text.push('\n');
    text
}

/// What native code plugins must be built with, where PaperPaw loads
/// them: its fingerprint, and the note that says so.
pub(crate) fn native(env: &Env<'_>) -> Option<(String, String)> {
    let native = env.native()?;
    let note = env.i18n().format(
        "about.native.help",
        &[("toolchain", Arg::Str(&native.toolchain))],
    );
    Some((native.fingerprint.to_string(), note))
}

/// The about page.
#[derive(Debug, Default)]
pub struct About;

/// How it works, by language (see `Manifest::how`).
static HOW: [(&str, &str); 2] = [
    ("en", include_str!("../how/en.txt")),
    ("de", include_str!("../how/de.txt")),
];

static ACTIONS: [FixedAction; 1] = [FixedAction::new(OPEN, "about.title")
    .handler(|_, _, out| out.push(Command::OpenScreen(SCREEN, Params::new())))];
/// In the device's part of the settings, last.
static ENTRIES: [MenuEntry; 1] = [MenuEntry::new(menus::SETTINGS, OPEN).order(99)];
static ICONS: [(Id, &str); 1] = [(OPEN, "info-circle")];

static SCREENS: [FixedScreen; 1] = [FixedScreen::new(SCREEN, "about.title", |env, _| {
    Box::new(screen::AboutScreen::new(env))
})
.instances(paperpaw_sdk::Instances::SINGLE)];

impl Plugin for About {
    fn manifest(&self) -> Manifest {
        Manifest::new(PLUGIN, "About", paperpaw_sdk::package_version!())
            .description("PaperPaw's version and licences.")
            .how(&HOW)
            .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        r.add_screens(&SCREENS)?;
        r.add_actions(&ACTIONS)?;
        r.add_menu_entries(&ENTRIES)?;
        r.add_icons(&ICONS)?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/en.ini")))?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/de.ini")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use paperpaw_i18n::{Catalog, check};

    use super::{licence_parts, shown_version};

    /// The licences as the page writes them.
    fn licences() -> std::string::String {
        let parts: std::vec::Vec<u8> = licence_parts().flatten().collect();
        std::string::String::from_utf8(parts).unwrap()
    }

    #[test]
    fn a_beta_shows_as_one() {
        let mut i18n = paperpaw_sdk::Translator::new();
        i18n.add(Catalog::parse(include_str!("../i18n/en.ini")).unwrap());
        assert_eq!(shown_version(&i18n, "0.5.0-beta.2"), "0.5.0 Beta 2");
        assert_eq!(shown_version(&i18n, "0.5.0"), "0.5.0");
        assert_eq!(shown_version(&i18n, "0.5.0-dev"), "0.5.0-dev");
    }

    #[test]
    fn translations_are_complete() {
        let en = Catalog::parse(include_str!("../i18n/en.ini")).unwrap();
        let de = Catalog::parse(include_str!("../i18n/de.ini")).unwrap();
        let issues = check::compare(&en, &de);
        assert!(issues.is_empty(), "{issues:?}");
    }

    /// Words only, so lines joined into paragraphs compare equal.
    fn words(text: &str) -> std::string::String {
        text.split_whitespace()
            .collect::<std::vec::Vec<_>>()
            .join(" ")
    }

    #[test]
    fn every_licence_is_there() {
        // As they are in the repository: run tools/licences/build.py
        // after one changed.
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../");
        let all = words(&licences());
        for file in [
            "tools/licences/NOTICE.in",
            "LICENSE",
            "plugins/fonts-default/fonts/Literata-OFL.txt",
            "plugins/fonts-default/fonts/SourceSans3-OFL.txt",
            "plugins/fonts-default/fonts/TablerIcons-MIT.txt",
        ] {
            let Ok(text) = std::fs::read_to_string(std::format!("{root}{file}")) else {
                continue;
            };
            assert!(
                all.contains(&words(&text)),
                "{file} changed: run tools/licences/build.py"
            );
        }
        assert!(all.contains("Gerard D.C. Kuiken"));
        assert!(all.contains("Stephan Hennig"));
        // The crates' notices of this build's product (the apps' here),
        // whole from their parts.
        assert!(all.contains("Third-party crates: MIT License"));
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/data/licences-app.md");
        assert_eq!(licences(), std::fs::read_to_string(file).unwrap());
    }
}
