//! How a handle sets a line without being told what lies around it
//! ([`FontHandle::line`]), and draws a run of one font, kerned and with
//! ligatures as measured.

use paperpaw_graphics::Canvas;
use paperpaw_text::{Font, GlyphCache};

use super::{FontHandle, LIGATURE_FIT, ligature};

impl FontHandle<'_> {
    /// What lies around a line that [`FontHandle::measure`] and
    /// [`FontHandle::draw`] set with letters of right-to-left scripts:
    /// nothing; in the order its characters are shown, Arabic letters
    /// joined, brackets mirrored (see [`FontHandle::draw_in`]), on a line
    /// that runs right to left where the handle's ink says so (the
    /// interface's font, in Arabic: [`TextInk::RIGHT_TO_LEFT`]), else as
    /// its first strong letter goes; kerned as the handle kerns.
    ///
    /// [`TextInk::RIGHT_TO_LEFT`]: paperpaw_text::TextInk::RIGHT_TO_LEFT
    pub(super) fn line(&self) -> paperpaw_text::Around {
        let base = match self.ink.is_right_to_left() {
            true => paperpaw_lang::Direction::Rtl,
            false => paperpaw_lang::Direction::Auto,
        };
        let around = paperpaw_text::Around::new(base, paperpaw_lang::Lang::NONE);
        match self.kerning {
            true => around,
            false => around.with_flags(paperpaw_text::Around::NO_KERN),
        }
    }

    /// Draws `run`, all in `style`'s font, from `x`; its width. At once
    /// where the font kerns it as this handle does; else character by
    /// character, each where measuring puts it, ligatures in place of
    /// their characters.
    pub(super) fn draw_run(
        &self,
        canvas: &mut Canvas<'_>,
        cache: &mut GlyphCache,
        style: &paperpaw_text::TextStyle<'_>,
        x: f32,
        baseline: i32,
        run: &str,
    ) -> f32 {
        if self.kerning && !(self.ligatures && run.contains('f')) {
            return paperpaw_text::draw_text(canvas, cache, style, x, baseline, run);
        }
        let (font, size) = (style.font, style.size);
        let mut style = *style;
        let mut pen = x;
        let mut before: Option<char> = None;
        let mut at = 0;
        let mut buffer = [0u8; 4];
        while let Some(ch) = run[at..].chars().next() {
            if let Some(left) = before {
                pen += self.pair_in(font, size, left, ch);
            }
            let joined = match self.ligatures && ch == 'f' {
                true => self.ligature_at(font, size, &run[at..]),
                false => None,
            };
            let (drawn, len, width) = joined.unwrap_or((ch, ch.len_utf8(), font.advance(ch, size)));
            paperpaw_text::draw_text(
                canvas,
                cache,
                &style,
                pen,
                baseline,
                drawn.encode_utf8(&mut buffer),
            );
            if !paperpaw_text::is_mark(ch) {
                style = style.based(true);
            }
            pen += width;
            before = run[..at + len].chars().next_back();
            at += len;
        }
        pen - x
    }

    /// The ligature drawn for the start of `text` (an `f`): its
    /// character, the bytes of `text` it stands for and the width those
    /// take as measured, with the pairs between them. `None` where the
    /// `f` is drawn alone: the font lacks the ligature, or one of three
    /// characters or `ff` would not take their room, or the next `f`
    /// makes a ligature with the character after it.
    pub(super) fn ligature_at(
        &self,
        font: &Font,
        size: f32,
        text: &str,
    ) -> Option<(char, usize, f32)> {
        let mut chars = text.chars();
        let (Some('f'), Some(second)) = (chars.next(), chars.next()) else {
            return None;
        };
        let advance = |ch: char| font.advance(ch, size);
        if let Some(joined) = ligature('f', second) {
            let width = advance('f') + self.pair_in(font, size, 'f', second) + advance(second);
            return font.has_char(joined).then_some((joined, 2, width));
        }
        if second != 'f' {
            return None;
        }
        let two = advance('f') + self.pair_in(font, size, 'f', 'f') + advance('f');
        let fits = |joined: char, width: f32| {
            font.has_char(joined) && (advance(joined) - width).abs() <= LIGATURE_FIT
        };
        if let Some(third @ ('i' | 'l')) = chars.next() {
            let three = two + self.pair_in(font, size, 'f', third) + advance(third);
            let joined = if third == 'i' { '\u{fb03}' } else { '\u{fb04}' };
            if fits(joined, three) {
                return Some((joined, 3, three));
            }
            if ligature('f', third).is_some_and(|l| font.has_char(l)) {
                return None;
            }
        }
        fits('\u{fb00}', two).then_some(('\u{fb00}', 2, two))
    }
}
