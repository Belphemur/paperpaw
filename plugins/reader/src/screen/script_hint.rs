//! A book in a script the reading font has no letters of (the X4 family's
//! firmware carries Latin alone) asks for the packs of letters from the
//! store, instead of leaving the reader with question marks: the store
//! shows what they are and what they take, and downloads them if the reader
//! says so ([`Command::request_plugins`]).
//!
//! Which packs a book needs is seen in the language it declares and in its
//! text (the first blocks: a book may mix scripts, or declare none). Each
//! pack is asked for once for a book; what was asked is kept in the book's
//! data folder, so that closing and opening the book asks no more, and
//! so that a pack that could not be fetched (no connection) is asked for
//! again at the next opening ([`forget`]).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::document::{Document, Locator};
use paperpaw_sdk::{Command, Cx, Env, FontHandle, Id};

/// A script (ISO 15924), a letter every font of it has, and the pack that
/// brings its letters.
const SCRIPTS: [(&str, char, &str); 5] = [
    ("Grek", 'α', "script-greek"),
    ("Cyrl", 'ж', "script-cyrillic"),
    ("Arab", 'ع', "script-arabic"),
    ("Hebr", 'א', "script-hebrew"),
    ("Kore", '한', KOREAN),
];

/// The Korean pack (the syllables of everyday text) and the one that adds
/// the rare ones, which a book needs only if it uses them.
const KOREAN: &str = "script-korean";
const KOREAN_EXTRA: &str = "script-korean-extra";

/// The text looked at: its first sections, and in them as many blocks and
/// letters as a few pages hold, so that opening a book costs little.
const SECTIONS: u32 = 3;
const BLOCKS: usize = 80;
const LETTERS: usize = 12_000;
/// The packs asked for at most, for a book at once, and kept for it.
const MOST: usize = 4;

/// The script `c` is written in, as [`SCRIPTS`] names it, where it is one
/// of those.
fn script_of(c: char) -> Option<&'static str> {
    Some(match u32::from(c) {
        0x0370..=0x03FF | 0x1F00..=0x1FFF => "Grek",
        0x0400..=0x052F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => "Cyrl",
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => "Hebr",
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => {
            "Arab"
        }
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7A3 => "Kore",
        _ => return None,
    })
}

/// Where the packs asked for are kept for a book, one id a line.
fn kept(dir: &str) -> String {
    alloc::format!("{dir}/packs-asked.txt")
}

/// The packs asked for before, for the book with data folder `dir`.
fn asked(env: &Env<'_>, dir: &str) -> Vec<String> {
    paperpaw_sdk::read_file(env.storage(), &kept(dir), 1024)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .map(|text| text.lines().map(ToString::to_string).collect())
        .unwrap_or_default()
}

/// Keeps `ids` as asked for (the file is written anew, at most [`MOST`] of
/// them: it stays a few lines long); an empty list removes it.
fn keep(env: &Env<'_>, dir: &str, ids: &[String]) {
    #[expect(
        clippy::let_underscore_must_use,
        reason = "what was asked is asked again at the next opening if it was not kept: nothing is lost"
    )]
    {
        if ids.is_empty() {
            let _ = env.storage().remove(&kept(dir));
            return;
        }
        let ids = &ids[ids.len().saturating_sub(MOST)..];
        let _ = env.storage().create_dir(dir);
        let _ = env.storage().write(&kept(dir), ids.join("\n").as_bytes());
    }
}

/// What the first blocks of `document` hold of the scripts of [`SCRIPTS`]
/// that `font` lacks letters of: the scripts, and whether a Hangul
/// syllable was among the letters it lacks.
fn lacking(
    env: &Env<'_>,
    document: &dyn Document,
    font: &FontHandle<'_>,
) -> (Vec<&'static str>, bool) {
    let mut scripts: Vec<&'static str> = Vec::new();
    let (mut syllable, mut blocks, mut letters) = (false, 0, 0);
    'sections: for section in 0..document.sections().min(SECTIONS) {
        let mut at = Locator::new(section, 0);
        while let Ok(Some(block)) = document.block_at(env.storage(), at) {
            blocks += 1;
            for c in block.text.chars() {
                letters += 1;
                if let Some(script) = script_of(c).filter(|_| !font.has_char(c)) {
                    syllable |= script == "Kore" && ('\u{ac00}'..='\u{d7a3}').contains(&c);
                    if !scripts.contains(&script) {
                        scripts.push(script);
                    }
                }
            }
            if blocks >= BLOCKS || letters >= LETTERS {
                break 'sections;
            }
            // Past the end of what was read: the next block is further on.
            if block.end <= at.offset {
                break;
            }
            at = Locator::new(section, block.end);
        }
    }
    (scripts, syllable)
}

/// The packs the book `document` (language `tag`, data folder `dir`)
/// needs for letters the reading `font` lacks, and that were not asked for
/// before: its declared language says one, its text says more.
pub(super) fn needed(
    env: &Env<'_>,
    document: &dyn Document,
    font: &FontHandle<'_>,
    tag: Option<&str>,
    dir: &str,
) -> Vec<String> {
    let (mut scripts, syllable) = lacking(env, document, font);
    let declared = tag
        .and_then(paperpaw_sdk::lang::likely_script_of)
        .and_then(|script| SCRIPTS.iter().find(|(s, ..)| *s == script));
    if let Some((script, letter, _)) = declared
        && !font.has_char(*letter)
        && !scripts.contains(script)
    {
        scripts.push(script);
    }
    let before = asked(env, dir);
    let mut ids: Vec<String> = Vec::new();
    for script in scripts {
        let Some((_, _, pack)) = SCRIPTS.iter().find(|(s, ..)| *s == script) else {
            continue;
        };
        // Once the Korean letters are there, a book that still has syllables
        // the font lacks needs the rare ones as well; it is not asked for
        // them before, not to push their size on a book that does not.
        let pack = match *pack == KOREAN && env.plugins().get(KOREAN).is_some() {
            true if syllable => KOREAN_EXTRA,
            true => continue,
            false => pack,
        };
        let id = (*pack).to_string();
        if !before.contains(&id) && !ids.contains(&id) && ids.len() < MOST {
            ids.push(id);
        }
    }
    ids
}

/// Asks for `ids` (from [`needed`]) and keeps that they were asked for.
pub(super) fn ask(cx: &mut Cx<'_, '_>, dir: &str, ids: &[String]) {
    if ids.is_empty() {
        return;
    }
    let env = *cx.env();
    let mut all = asked(&env, dir);
    let new: Vec<String> = ids.iter().filter(|id| !all.contains(id)).cloned().collect();
    all.extend(new);
    keep(&env, dir, &all);
    let wanted = ids.iter().filter_map(|id| Id::parse(id).ok());
    cx.command(Command::request_plugins(crate::PLUGIN, wanted));
}

/// Takes `ids` off what was asked for the book with data folder `dir`: the
/// store could not get them (no connection, a failed download), so they
/// are asked for again at the next opening.
pub(super) fn forget(env: &Env<'_>, dir: &str, ids: impl Iterator<Item = Id>) {
    let ids: Vec<Id> = ids.collect();
    let rest: Vec<String> = asked(env, dir)
        .into_iter()
        .filter(|id| !ids.iter().any(|gone| gone.as_str() == id))
        .collect();
    keep(env, dir, &rest);
}
