//! Punctuation hung into the margins ([`ScriptOptions::HANGING_PUNCTUATION`]):
//! an opening quote that starts a line, and a stop, comma, hyphen or
//! closing quote that ends one, are set outside the line's measure, so
//! its letters line up with the edge (optical margin alignment). Lines
//! break as without it; only where they are set changes, so a page holds
//! the same text and starts where it did.
//!
//! [`ScriptOptions::HANGING_PUNCTUATION`]: crate::ScriptOptions::HANGING_PUNCTUATION

use paperpaw_document::Block;
use paperpaw_text::Line;

use crate::levels::BlockLevels;
use crate::{Align, Face, LayoutContext, face_at};

/// Marks hung at a line's start: opening quotes (and guillemets, which
/// open quotes either way round).
const START: &[char] = &[
    '"', '\'', '\u{201c}', '\u{2018}', '\u{201e}', '\u{201a}', '\u{ab}', '\u{bb}', '\u{2039}',
    '\u{203a}',
];

/// Marks hung at a line's end: stops, commas, hyphens and closing quotes
/// (`“` closes a German quote).
const END: &[char] = &[
    '.', ',', '-', '\u{2010}', '\u{2011}', '\u{2013}', '"', '\'', '\u{201d}', '\u{2019}',
    '\u{201c}', '\u{ab}', '\u{bb}', '\u{2039}', '\u{203a}', '\u{3001}', '\u{3002}', '\u{ff0c}',
    '\u{ff0e}',
];

/// How far `line` of `block` hangs out of its measure at its start and at
/// its end, in pixels: the widths of the marks hung there, as `align`
/// sets them (a centred line hangs nothing, a line set to its start
/// nothing at its end, one set to its end nothing at its start). Only
/// lines all left to right hang; and a first line `indented` keeps its
/// quote in the indent.
pub(crate) fn hanging(
    cx: &LayoutContext<'_>,
    block: &Block,
    base: Face,
    line: &Line,
    levels: &BlockLevels,
    align: Align,
    indented: bool,
) -> (f32, f32) {
    let text = &block.text[line.range.clone()];
    let ltr = !levels.paragraph.is_rtl()
        && (levels.is_plain() || line.range.clone().all(|i| levels.at(i).number() == 0));
    if !ltr || text.is_empty() {
        return (0.0, 0.0);
    }
    let width = |i: usize, c: char| {
        cx.typesetter
            .advance(face_at(block, base, i, cx.style.book), c)
    };
    let start = match text.chars().next() {
        Some(c) if !indented && START.contains(&c) => width(line.range.start, c),
        _ => 0.0,
    };
    let end = match text.char_indices().last() {
        // The hyphen drawn after a word broken there, in the face of the
        // letter before it.
        Some((i, _)) if line.hyphen => width(line.range.start + i, '-'),
        Some((i, c)) if END.contains(&c) => width(line.range.start + i, c),
        _ => 0.0,
    };
    match align {
        Align::Justify if !line.last && line.spaces > 0 => (start, end),
        Align::End => (0.0, end),
        Align::Center => (0.0, 0.0),
        _ => (start, 0.0),
    }
}
