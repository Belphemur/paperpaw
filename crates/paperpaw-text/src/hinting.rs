//! How outlines are fitted to the pixel grid when a font is rasterized:
//! for the packs the pack generators make (`paperpaw-fontgen`, the default
//! fonts' build), where it costs nothing on the device.

/// Grid-fitting of an outline at one size. A stem that falls between two
/// pixels comes out as two grays that a black-and-white frame makes thin,
/// double or broken; fitted, x-height, baseline, caps and stems lie on
/// whole pixels and letters come out even and crisp.
///
/// Only the ink moves: advances, kerning and line metrics are the font's
/// unfitted ones in every mode, so a book is laid out and broken into
/// pages the same whichever one made its pack.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Hinting {
    /// The outline as drawn: stems at any sub-pixel position.
    #[default]
    None,
    /// The automatic hinter's light mode (as FreeType's `LIGHT`): the
    /// height of things snapped, their width left as drawn. About the
    /// weight of the unfitted letters, flat x-height and baseline.
    Light,
    /// The automatic hinter's normal mode: stems snapped to whole pixels
    /// both ways: crisp, and heavier at small sizes.
    Normal,
    /// The automatic hinter's strong mode for one bit a pixel (as
    /// FreeType's `MONO`): one-pixel stems, the crispest and the lightest.
    Mono,
}

impl Hinting {
    /// Every mode, in the order of the names below.
    pub const ALL: [Hinting; 4] = [
        Hinting::None,
        Hinting::Light,
        Hinting::Normal,
        Hinting::Mono,
    ];

    /// The mode named `name` (`none`, `light`, `normal`, `mono`; case
    /// does not matter), `None` for anything else.
    pub fn parse(name: &str) -> Option<Hinting> {
        Self::ALL
            .into_iter()
            .find(|h| h.name().eq_ignore_ascii_case(name.trim()))
    }

    /// The name [`Hinting::parse`] reads.
    pub const fn name(self) -> &'static str {
        match self {
            Hinting::None => "none",
            Hinting::Light => "light",
            Hinting::Normal => "normal",
            Hinting::Mono => "mono",
        }
    }

    /// What the hinter is asked for; `None` for the outline as drawn.
    #[cfg(feature = "outline")]
    pub(crate) fn options(self) -> Option<skrifa::outline::HintingOptions> {
        use skrifa::outline::{Engine, HintingOptions, SmoothMode, Target};
        let smooth = |mode| Target::Smooth {
            mode,
            symmetric_rendering: true,
            preserve_linear_metrics: true,
        };
        let target = match self {
            Hinting::None => return None,
            Hinting::Light => smooth(SmoothMode::Light),
            Hinting::Normal => smooth(SmoothMode::Normal),
            Hinting::Mono => Target::Mono,
        };
        // Always the automatic hinter: the same fitting for every font,
        // whether it has instructions of its own or not.
        Some(HintingOptions {
            engine: Engine::Auto(None),
            target,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_back_and_none_is_the_default() {
        assert_eq!(Hinting::default(), Hinting::None);
        for h in Hinting::ALL {
            assert_eq!(Hinting::parse(h.name()), Some(h));
        }
        assert_eq!(Hinting::parse(" Light "), Some(Hinting::Light));
        assert_eq!(Hinting::parse("strong"), None);
        assert_eq!(Hinting::parse(""), None);
    }
}
