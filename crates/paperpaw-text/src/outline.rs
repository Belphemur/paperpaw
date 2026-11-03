//! Runtime rendering of OpenType/TrueType fonts via `skrifa`.
//!
//! Glyphs are drawn to the panel's pixel grid as [`Hinting`] says, a
//! font's own choice unless turned. The grid-fitting is work done once
//! for a size, so an instance of it is kept for a font and reused for
//! every glyph of that size: the advances and shapes seen on a page
//! come from one fit, not from a re-run for each glyph.

use alloc::borrow::Cow;
use alloc::vec::Vec;
use core::cell::RefCell;
use core::fmt;

use skrifa::instance::{Location, Size};
use skrifa::outline::{
    DrawSettings, Engine, HintingInstance, HintingOptions, OutlinePen, SmoothMode, Target,
};
use skrifa::{FontRef, GlyphId, MetadataProvider};

use crate::font::{FontError, LineMetrics};
use crate::glyph::{self, Glyph, pack_coverage};
use crate::hinting::Hinting;
use crate::kerning::Kerning;
use crate::raster::{self, Path, Pt};

/// A scalable font, optionally at a fixed position of its variation axes
/// (e.g. weight 700 of a variable font).
///
/// The font data is borrowed for built-in fonts or owned for fonts loaded
/// at runtime. A clone comes with no grid-fitting kept: it builds its own
/// as it draws.
pub struct OutlineFont {
    data: Cow<'static, [u8]>,
    location: Location,
    hinting: Hinting,
    /// Grid-fitting instances, one per size and mode, and how many have
    /// been built (tests watch the count: a glyph drawn again of a size
    /// already drawn must build no new one).
    hints: RefCell<HintCache>,
}

impl Clone for OutlineFont {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
            location: self.location.clone(),
            hinting: self.hinting,
            hints: RefCell::default(),
        }
    }
}

/// An instance kept for a size's quarter pixel and a mode.
struct Hint {
    /// Size in quarter pixels.
    quarters: u16,
    mode: Hinting,
    instance: HintingInstance,
}

/// The instances kept, and how many have been built in all.
#[derive(Default)]
struct HintCache {
    built: u32,
    valid: Vec<Hint>,
}

/// Sizes a font's instances are kept for at once. A page draws at a size
/// or two, and a book turns at mostly one; past once of that the block
/// rids the fits it made, and builds again for a size asked later.
const KEPT_HINTS: usize = 6;

impl fmt::Debug for OutlineFont {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OutlineFont")
            .field("bytes", &self.data.len())
            .finish_non_exhaustive()
    }
}

impl OutlineFont {
    /// Loads a font. `variations` sets axes such as `[("wght", 700.0)]`;
    /// unknown axes are ignored.
    pub fn new(
        data: impl Into<Cow<'static, [u8]>>,
        variations: &[(&str, f32)],
    ) -> Result<OutlineFont, FontError> {
        let data = data.into();
        let font = FontRef::new(&data).map_err(|_| FontError::Invalid)?;
        // Any mapping will do: icon fonts have no letters.
        if font.charmap().mappings().next().is_none() {
            return Err(FontError::NoCharmap);
        }
        let location = font.axes().location(variations.iter().copied());
        Ok(OutlineFont {
            data,
            location,
            hinting: Hinting::LIGHT,
            hints: RefCell::new(HintCache::default()),
        })
    }

    /// How this font's glyphs are grid-fitted.
    pub fn hinting(&self) -> Hinting {
        self.hinting
    }

    /// Chooses how this font's glyphs are grid-fitted from now on. The
    /// pages already laid out stay laid out (advances never move): what
    /// was drawn from this font's glyphs so far is drawn again in the
    /// choice, on a screen drawn anew. Kept grid-fittings of another
    /// mode give room for the new one.
    pub fn set_hinting(&mut self, hinting: Hinting) {
        if self.hinting == hinting {
            return;
        }
        self.hints.borrow_mut().valid.clear();
        self.hinting = hinting;
    }

    /// How many grid-fittings have been built, built or kept again.
    #[cfg(test)]
    pub(crate) fn hinting_builds(&self) -> u32 {
        self.hints.borrow().built
    }

    /// The grid-fitting of `mode` at `size`, kept or built. Where it
    /// cannot be built (a font left over, memory short), the glyphs draw
    /// without fitting rather than failing the page.
    fn hint_instance(
        &self,
        font: &FontRef<'_>,
        size: f32,
        mode: Hinting,
    ) -> Option<core::cell::Ref<'_, HintingInstance>> {
        use core::cell::Ref;
        let quarters = (size * 4.0) as u16;
        let kept = self
            .hints
            .borrow()
            .valid
            .iter()
            .position(|kept| kept.quarters == quarters && kept.mode == mode);
        let index = match kept {
            Some(index) => index,
            None => {
                // What FreeType shows on antialiased panels, with the
                // automatic hinter forced: a font's own instructions are
                // not read (they differ per font and some do nothing —
                // the shipped ones' tables are stubs), every glyph
                // instead is fitted the same way. ClearType's Double
                // shown as it is (FreeType has it always on), and the
                // font's measured advances kept (see `Hinting`).
                let target = match mode {
                    Hinting::OFF => return None,
                    Hinting::LIGHT => Target::Smooth {
                        mode: SmoothMode::Light,
                        symmetric_rendering: true,
                        preserve_linear_metrics: true,
                    },
                    Hinting::NORMAL => Target::Smooth {
                        mode: SmoothMode::Normal,
                        symmetric_rendering: true,
                        preserve_linear_metrics: true,
                    },
                };
                let instance = HintingInstance::new(
                    &font.outline_glyphs(),
                    Size::new(size),
                    &self.location,
                    HintingOptions {
                        engine: Engine::Auto(None),
                        target,
                    },
                )
                // A font whose fit cannot be built draws as its glyphs
                // are, never failing the page.
                .ok()?;
                // Past what is kept the block starts anew: a turn of
                // pages draws at one or two sizes, and what a font drew
                // last is drawn again as it is needed.
                let mut cache = self.hints.borrow_mut();
                let index = if cache.valid.len() >= KEPT_HINTS {
                    cache.valid.clear();
                    0
                } else {
                    cache.valid.len()
                };
                cache.built += 1;
                cache.valid.push(Hint {
                    quarters,
                    mode,
                    instance,
                });
                drop(cache);
                index
            }
        };
        Some(Ref::map(self.hints.borrow(), |cache| {
            &cache.valid[index].instance
        }))
    }

    fn font_ref(&self) -> Option<FontRef<'_>> {
        // Validated in `new`; `None` is unreachable in practice.
        FontRef::new(&self.data).ok()
    }

    pub fn line_metrics(&self, size: f32) -> LineMetrics {
        let size = quarter_size(size);
        let Some(font) = self.font_ref() else {
            return LineMetrics::fallback(size);
        };
        let metrics = font.metrics(Size::new(size), &self.location);
        let descent = -metrics.descent;
        LineMetrics {
            ascent: metrics.ascent,
            descent,
            line_height: metrics.ascent + descent + metrics.leading,
        }
    }

    pub fn has_char(&self, ch: char) -> bool {
        self.font_ref()
            .is_some_and(|font| font.charmap().map(ch).is_some())
    }

    fn glyph_id(font: &FontRef<'_>, ch: char) -> GlyphId {
        let charmap = font.charmap();
        charmap
            .map(ch)
            .or_else(|| charmap.map('\u{fffd}'))
            .unwrap_or(GlyphId::NOTDEF)
    }

    pub fn advance(&self, ch: char, size: f32) -> f32 {
        let size = quarter_size(size);
        let Some(font) = self.font_ref() else {
            return 0.0;
        };
        font.glyph_metrics(Size::new(size), &self.location)
            .advance_width(Self::glyph_id(&font, ch))
            .unwrap_or(0.0)
            .max(0.0)
    }

    /// How much the pair `left`, `right` moves the second character, in
    /// pixels at `size` (negative: closer): the font's kerning
    /// ([`OutlineFont::kerning_pairs`]). 0 where either character is
    /// missing.
    pub fn kern(&self, left: char, right: char, size: f32) -> f32 {
        let size = quarter_size(size);
        let Some(font) = self.font_ref() else {
            return 0.0;
        };
        let charmap = font.charmap();
        let (Some(l), Some(r)) = (charmap.map(left), charmap.map(right)) else {
            return 0.0;
        };
        let units = Kerning::new(font.clone(), self.location.coords()).pair((left, right), l, r);
        if units == 0.0 {
            return 0.0;
        }
        units * size / units_per_em(&font)
    }

    /// The kerning pairs among `chars`, each with how much it moves the
    /// second character, in ems (negative: closer): the pairs of the
    /// font's GPOS `kern` feature, or of its legacy `kern` table where it
    /// has no GPOS kerning; at the instance of a variable font. Pairs that
    /// change nothing and characters the font lacks are left out.
    pub fn kerning_pairs(&self, chars: &[char]) -> Vec<(char, char, f32)> {
        let Some(font) = self.font_ref() else {
            return Vec::new();
        };
        let charmap = font.charmap();
        let glyphs: Vec<(char, GlyphId)> = chars
            .iter()
            .filter_map(|&ch| Some((ch, charmap.map(ch)?)))
            .collect();
        let kerning = Kerning::new(font.clone(), self.location.coords());
        if kerning.is_empty() {
            return Vec::new();
        }
        let em = units_per_em(&font);
        let mut pairs = Vec::new();
        for &(left, l) in &glyphs {
            for &(right, r) in &glyphs {
                let units = kerning.pair((left, right), l, r);
                if units != 0.0 {
                    pairs.push((left, right, units / em));
                }
            }
        }
        pairs
    }

    /// Rasterizes `ch`; missing characters render as U+FFFD or `.notdef`.
    pub fn rasterize(&self, ch: char, size: f32) -> Glyph {
        let size = quarter_size(size);
        let Some(font) = self.font_ref() else {
            return Glyph::empty(0.0);
        };
        let id = Self::glyph_id(&font, ch);
        let advance = font
            .glyph_metrics(Size::new(size), &self.location)
            .advance_width(id)
            .unwrap_or(0.0)
            .max(0.0);
        let mut pen = Pen::default();
        if let Some(outline) = font.outline_glyphs().get(id) {
            // A font whose fit cannot be built draws its glyphs as they
            // are, never failing the page; so does a fit that cannot
            // draw one glyph on its own.
            let drawn = self
                .hint_instance(&font, size, self.hinting)
                .and_then(|instance| {
                    outline
                        .draw(DrawSettings::hinted(&instance, false), &mut pen)
                        .ok()
                });
            if drawn.is_none() {
                pen = Pen::default();
                let settings = DrawSettings::unhinted(Size::new(size), &self.location);
                // A broken glyph renders as empty instead of failing the page.
                let _ = outline.draw(settings, &mut pen);
            }
        }
        pen.0.close();

        let Some((x0, y0, x1, y1)) = pen.0.bounds() else {
            return Glyph::empty(advance);
        };
        // Points far out (or an em of a few units) would ask for a bitmap
        // of gigabytes: such a glyph renders as empty.
        let reach = glyph::reach(size);
        if [x0, y0, x1, y1]
            .iter()
            .any(|c| !(-reach..=reach).contains(c))
        {
            return Glyph::empty(advance);
        }
        let (width, height) = ((x1 - x0) as usize, (y1 - y0) as usize);
        if !glyph::fits(width, height, size) {
            return Glyph::empty(advance);
        }
        // Shifted in place rather than copied: outlines can be long.
        let mut lines = pen.0.lines;
        for p in lines.iter_mut().flat_map(|(a, b)| [a, b]) {
            p.x -= x0 as f32;
            p.y -= y0 as f32;
        }
        let Some(data) = raster::rasterize(&lines, width, height)
            .and_then(|coverage| pack_coverage(&coverage, width, height))
        else {
            return Glyph::empty(advance);
        };
        // Within `reach`: offsets fit an `i16`, sides a `u16`.
        Glyph {
            left: x0 as i16,
            top: y0 as i16,
            width: width as u16,
            height: height as u16,
            advance,
            data,
        }
    }
}

/// Sizes meet on the quarter-pixel grid the fits are kept for (see
/// `Hint`): a page's 20.2 is measured and drawn of 20.0, whichever of
/// the two was asked first, so what a glyph's fit was built for is
/// what measures it too.
fn quarter_size(size: f32) -> f32 {
    f32::from((size * 4.0) as u16) / 4.0
}

/// The font's units per em; 1000 where it does not say.
fn units_per_em(font: &FontRef<'_>) -> f32 {
    use skrifa::raw::TableProvider;
    match font.head().map(|head| head.units_per_em()) {
        Ok(units) if units > 0 => f32::from(units),
        _ => 1000.0,
    }
}

/// Receives outlines from skrifa (y up) and stores them y-down.
#[derive(Default)]
struct Pen(Path);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to(Pt { x, y: -y });
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to(Pt { x, y: -y });
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(Pt { x: cx0, y: -cy0 }, Pt { x, y: -y });
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.cubic_to(
            Pt { x: cx0, y: -cy0 },
            Pt { x: cx1, y: -cy1 },
            Pt { x, y: -y },
        );
    }

    fn close(&mut self) {
        self.0.close();
    }
}

#[cfg(all(test, feature = "outline"))]
mod tests {
    use super::*;

    const FONT: &[u8] = include_bytes!("../../../plugins/fonts-default/fonts/SourceSans3.ttf");

    /// Some text of a page, enough characters for the modes to differ on.
    const TEXT: &str = "How reads a page of PaperPaw, fitted or not?";

    #[test]
    fn a_new_font_draws_light() {
        let font = OutlineFont::new(FONT, &[("wght", 400.0)]).unwrap();
        assert_eq!(font.hinting(), Hinting::LIGHT);
        font.rasterize('A', 32.0);
        assert_eq!(font.hinting_builds(), 1);
    }

    #[test]
    fn a_page_draws_from_the_fit_it_kept() {
        let font = OutlineFont::new(FONT, &[("wght", 400.0)]).unwrap();
        for ch in TEXT.chars() {
            font.rasterize(ch, 32.0);
        }
        assert_eq!(font.hinting_builds(), 1, "the page's size is kept");
        for ch in TEXT.chars() {
            font.rasterize(ch, 36.0);
        }
        assert_eq!(font.hinting_builds(), 2, "another size, one fit more");
    }

    #[test]
    fn a_mode_change_draws_fits_of_its_own() {
        let mut font = OutlineFont::new(FONT, &[("wght", 400.0)]).unwrap();
        font.rasterize('A', 32.0);
        font.set_hinting(Hinting::NORMAL);
        font.rasterize('A', 32.0);
        assert_eq!(font.hinting_builds(), 2, "a fit of one serves no other");
        font.set_hinting(Hinting::OFF);
        font.rasterize('A', 32.0);
        assert_eq!(font.hinting_builds(), 2, "off builds no fit");
    }

    #[test]
    fn fits_past_the_kept_ones_are_dropped() {
        let font = OutlineFont::new(FONT, &[("wght", 400.0)]).unwrap();
        for size in (20..28).map(|px| f32::from(u8::try_from(px).unwrap_or(20))) {
            font.rasterize('A', size);
        }
        assert!(
            font.hinting_builds() as usize > KEPT_HINTS,
            "builds go on, least the block hold every size ever asked"
        );
    }

    #[test]
    fn advances_stay_the_fonts_own() {
        let font = OutlineFont::new(FONT, &[("wght", 400.0)]).unwrap();
        for size in [20.0, 21.5, 32.0, 48.0] {
            let advances = |mode: Hinting| -> Vec<f32> {
                let mut fitted = font.clone();
                fitted.set_hinting(mode);
                fitted.rasterize('A', size);
                TEXT.chars().map(|ch| fitted.advance(ch, size)).collect()
            };
            let expected = advances(Hinting::OFF);
            for mode in [Hinting::OFF, Hinting::LIGHT, Hinting::NORMAL] {
                assert_eq!(advances(mode), expected, "mode {mode:?} at {size}");
            }
        }
    }

    #[test]
    fn light_grid_fitting_changes_small_text() {
        let font = OutlineFont::new(FONT, &[("wght", 400.0)]).unwrap();
        let off = {
            let mut font = font.clone();
            font.set_hinting(Hinting::OFF);
            font.rasterize('e', 24.0)
        };
        let light = font.rasterize('e', 24.0);
        assert_ne!(
            off.data, light.data,
            "grid-fitting moves points at small sizes on this font"
        );
    }
}
