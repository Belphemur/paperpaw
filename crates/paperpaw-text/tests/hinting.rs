//! Hinting at pack-build time: the ink is fitted to the pixel grid and
//! nothing that lays a book out changes, so a book breaks into the same
//! pages whichever way its pack was made.

#![cfg(feature = "outline")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use paperpaw_text::charset;
use paperpaw_text::{BitmapFont, Glyph, Hinting, OutlineFont, build_pack, build_pack_hinted};

const LITERATA: &[u8] = include_bytes!("../../../plugins/fonts-default/fonts/Literata.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("../../../plugins/fonts-default/fonts/SourceSans3.ttf");

const SIZES: [f32; 3] = [20.0, 24.0, 32.0];

/// The faces the default packs are made of.
fn faces() -> Vec<(&'static str, OutlineFont)> {
    [
        ("literata", LITERATA, 400.0),
        ("literata-bold", LITERATA, 700.0),
        ("sans", SOURCE_SANS, 430.0),
    ]
    .map(|(name, data, weight)| (name, OutlineFont::new(data, &[("wght", weight)]).unwrap()))
    .into()
}

/// `ch` of `font` at `size`, fitted as `hinting` says.
fn hinted(font: &OutlineFont, ch: char, size: f32, hinting: Hinting) -> Glyph {
    let mut glyphs = font.rasterize_chars([ch], size, hinting);
    assert_eq!(glyphs.len(), 1);
    glyphs.pop().unwrap().1
}

/// Latin, Cyrillic and Greek letters: the scripts the default fonts have.
fn letters() -> Vec<char> {
    ["latin", "cyrillic", "greek"]
        .into_iter()
        .flat_map(|name| charset::chars(charset::named(name).unwrap()))
        .collect()
}

/// How much of the glyph is ink: the sum of its coverage.
fn ink(glyph: &Glyph) -> u32 {
    let glyph = glyph.as_ref();
    (0..glyph.height)
        .flat_map(|y| (0..glyph.width).map(move |x| (x, y)))
        .map(|(x, y)| u32::from(glyph.coverage(x, y)))
        .sum()
}

#[test]
fn it_is_off_unless_asked_for() {
    let font = OutlineFont::new(LITERATA, &[("wght", 400.0)]).unwrap();
    assert_eq!(
        font.rasterize('g', 24.0),
        hinted(&font, 'g', 24.0, Hinting::None)
    );
    let chars = || "Hag,ü".chars();
    assert_eq!(
        build_pack(&font, &SIZES, chars()),
        build_pack_hinted(&font, &SIZES, chars(), Hinting::None)
    );
    assert_ne!(
        build_pack(&font, &SIZES, chars()),
        build_pack_hinted(&font, &SIZES, chars(), Hinting::Light)
    );
}

#[test]
fn every_mode_keeps_the_advances_of_the_outline() {
    let letters = letters();
    for (name, font) in faces() {
        for size in SIZES {
            let chars = || letters.iter().copied().filter(|&ch| font.has_char(ch));
            let plain = font.rasterize_chars(chars(), size, Hinting::None);
            for hinting in [Hinting::Light, Hinting::Normal, Hinting::Mono] {
                let fitted = font.rasterize_chars(chars(), size, hinting);
                assert_eq!(plain.len(), fitted.len());
                for ((ch, a), (_, b)) in plain.iter().zip(&fitted) {
                    assert_eq!(a.advance, b.advance, "{name} {hinting:?} {ch} {size}");
                    assert_eq!(font.advance(*ch, size), b.advance);
                }
            }
        }
    }
}

#[test]
fn the_stored_advances_kerning_and_lines_are_the_same_in_the_packs() {
    let letters = letters();
    for (name, font) in faces() {
        let pack = |hinting| {
            BitmapFont::new(build_pack_hinted(
                &font,
                &SIZES,
                letters.iter().copied(),
                hinting,
            ))
            .unwrap()
        };
        let (plain, fitted) = (pack(Hinting::None), pack(Hinting::Light));
        for size in SIZES {
            assert_eq!(
                plain.line_metrics(size),
                fitted.line_metrics(size),
                "{name} {size}: line"
            );
            for &ch in &letters {
                assert_eq!(
                    plain.glyph(ch, size).advance,
                    fitted.glyph(ch, size).advance,
                    "{name} {ch} {size}"
                );
            }
            let kerned = "AVTWYLPFaeoywr.,";
            for (left, right) in kerned
                .chars()
                .flat_map(|l| kerned.chars().map(move |r| (l, r)))
            {
                assert_eq!(
                    plain.kern(left, right, size),
                    fitted.kern(left, right, size),
                    "{name} {left}{right} {size}: kerning"
                );
            }
        }
    }
}

#[test]
fn the_ink_stays_where_the_outline_puts_it() {
    let font = OutlineFont::new(LITERATA, &[("wght", 400.0)]).unwrap();
    for hinting in [Hinting::Light, Hinting::Normal, Hinting::Mono] {
        for size in SIZES {
            for ch in "Hamnoxgp,.ЖдφΣ".chars() {
                let (a, b) = (font.rasterize(ch, size), hinted(&font, ch, size, hinting));
                let apart = |x: i32, y: i32| (x - y).abs();
                let near = apart(i32::from(a.left), i32::from(b.left)) <= 1
                    && apart(i32::from(a.top), i32::from(b.top)) <= 2
                    && apart(i32::from(a.width), i32::from(b.width)) <= 2
                    && apart(i32::from(a.height), i32::from(b.height)) <= 2;
                assert!(near, "{hinting:?} {ch} {size}: box moved too far");
                // The light mode is the one the default packs are made in:
                // the same amount of ink, a fifth less to a quarter more.
                // Normal makes stems heavier and mono lighter, by design.
                if hinting == Hinting::Light {
                    let (before, after) = (ink(&a), ink(&b));
                    assert!(
                        after * 5 >= before * 4 && after * 4 <= before * 5,
                        "{ch} {size}: ink {before} -> {after}"
                    );
                }
            }
        }
    }
}
