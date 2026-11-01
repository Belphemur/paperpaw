//! Letters drawn that no font has ([`Fonts::missing_letters`]).

use super::{FontHandle, FontSet, Fonts};

/// What the handles of a registry or set share as they draw.
#[derive(Debug, Default)]
pub(super) struct Drawing {
    /// Only described, not drawn (see [`Fonts::set_describing`]): no
    /// glyph is made.
    pub(super) describing: core::cell::Cell<bool>,
    /// Letters drawn that no font has ([`Fonts::missing_letters`]).
    pub(super) missing: core::cell::Cell<u32>,
}

impl FontHandle<'_> {
    /// Counts the letters of `text` that neither this font nor its
    /// fallbacks draw, nor a stand-in of them ([`Fonts::missing_letters`]).
    pub(super) fn count_missing(&self, text: &str) {
        let missing = text.chars().filter(|&ch| self.lacks(ch)).count();
        if missing > 0 {
            let count = &self.drawing.missing;
            count.set(count.get().wrapping_add(missing as u32));
        }
    }

    /// Whether `ch` is a letter drawn missing: not a space, a mark, a
    /// control or a character only formatting text (a soft hyphen, a
    /// joiner, a direction mark), and drawn by no font, nor as a
    /// stand-in ([`charset::stand_in`](paperpaw_text::charset::stand_in)).
    fn lacks(&self, ch: char) -> bool {
        let unseen = ch.is_whitespace()
            || ch.is_control()
            || paperpaw_text::is_mark(ch)
            || matches!(ch, '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2069}' | '\u{fe00}'..='\u{fe0f}' | '\u{feff}');
        if unseen || self.has_char(ch) {
            return false;
        }
        // Stand-ins are followed as drawing follows them (U+2011, U+2010,
        // `-`).
        let mut at = ch;
        for _ in 0..3 {
            match paperpaw_text::charset::stand_in(at) {
                Some(stand_in) if self.has_char(stand_in) => return false,
                Some(stand_in) => at = stand_in,
                None => break,
            }
        }
        true
    }
}

impl Fonts {
    /// How many letters were drawn that none of these fonts has (shown as
    /// a question mark or not at all), wrapping: a screen that takes it
    /// before and after drawing a page knows whether letters were
    /// missing there (e.g. to offer a font pack with them). Spaces,
    /// marks, controls and letters drawn as a stand-in are not counted.
    pub fn missing_letters(&self) -> u32 {
        self.drawing.missing.get()
    }
}

impl FontSet {
    /// How many letters were drawn that the set's fonts lack, as
    /// [`Fonts::missing_letters`] counts them.
    pub fn missing_letters(&self) -> u32 {
        self.drawing.missing.get()
    }
}
