//! How glyphs are grid-fitted for the panel they are drawn on.
//!
//! Hinting snaps a glyph's outline to the pixel grid before it is
//! rasterized, so small text keeps its weight: stems stay as wide as the
//! font means them, and round letters stay round where they would fade
//! into gray otherwise. The automatic hinter (skrifa's, the same rules
//! hinted antialiased text follows elsewhere) does the fitting; a font's
//! own instructions are not read, so no font's odd programs can change
//! a page.
//!
//! It never moves a glyph along the line: the drawing takes a glyph's
//! advance from the font's own metrics of its size whatever mode is on
//! (see [`Hinting::LIGHT`]), so a layout is the same how it is fitted.
//!
//! Bitmap packs hold what their build rendered, and their header's byte
//! says it (`Hinting::to_byte`). No reader of a pack needs the byte to
//! draw, it is there for what reads a pack's making (tests, tools).

/// How a scalable font's glyphs are grid-fitted before they are drawn.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum Hinting {
    /// As the outlines are. A font that comes with odd glyphs keeps
    /// them.
    OFF,
    /// Grid-fitted up and down, never sideways (skrifa's light smooth
    /// target, what antialiased text is drawn with elsewhere): stems
    /// meet whole pixels at small sizes, advances stay the font's own.
    #[default]
    LIGHT,
    /// Grid-fitted up and down and sideways (skrifa's normal smooth
    /// target with the linear metrics preserved): the strongest change
    /// to small text. Glyphs may reach a pixel wider or narrower than
    /// unhinted; advances stay the font's own, so lines break the same.
    NORMAL,
}

impl Hinting {
    /// How a pack's header stores a mode. New modes follow: a pack with
    /// a byte above the known ones reads as [`Hinting::OFF`], so an
    /// older build draws its glyphs as a pack it cannot picture the
    /// building of: e.g. no grid-fitting at all.
    pub fn to_byte(self) -> u8 {
        match self {
            Hinting::OFF => 0,
            Hinting::LIGHT => 1,
            Hinting::NORMAL => 2,
        }
    }

    /// The mode a pack's header byte says; unknown bytes read as
    /// [`Hinting::OFF`] (see [`Hinting::to_byte`]).
    pub fn from_byte(byte: u8) -> Hinting {
        match byte {
            0 => Hinting::OFF,
            1 => Hinting::LIGHT,
            2 => Hinting::NORMAL,
            _ => Hinting::OFF,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_its_byte() {
        for (mode, byte) in [
            (Hinting::OFF, 0u8),
            (Hinting::LIGHT, 1),
            (Hinting::NORMAL, 2),
        ] {
            assert_eq!(mode.to_byte(), byte);
            assert_eq!(Hinting::from_byte(byte), mode);
        }
        // Unknown bytes (of a newer version) read as OFF, not as a
        // fitting partly remembered.
        assert_eq!(Hinting::from_byte(3), Hinting::OFF);
        assert_eq!(Hinting::from_byte(255), Hinting::OFF);
    }

    #[test]
    fn the_default_is_light() {
        assert_eq!(Hinting::default(), Hinting::LIGHT);
    }
}
