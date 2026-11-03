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
use paperpaw_text::{BitmapFont, Glyph, Hinting, OutlineFont, build_pack};

const LITERATA: &[u8] = include_bytes!("../../../plugins/fonts-default/fonts/Literata.ttf");
const SOURCE_SANS: &[u8] = include_bytes!("../../../plugins/fonts-default/fonts/SourceSans3.ttf");

const SIZES: [f32; 3] = [20.0, 24.0, 32.0];

/// The faces the default packs are made of, in `hinting`.
fn faces(hinting: Hinting) -> Vec<(&'static str, OutlineFont)> {
    [
        ("literata", LITERATA, 400.0),
        ("literata-bold", LITERATA, 700.0),
        ("sans", SOURCE_SANS, 430.0),
    ]
    .map(|(name, data, weight)| {
        let font = OutlineFont::new(data, &[("wght", weight)]).unwrap();
        (name, font.with_hinting(hinting))
    })
    .into()
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
    assert_eq!(font.hinting(), Hinting::None);
    let same = font.clone().with_hinting(Hinting::None);
    assert_eq!(font.rasterize('g', 24.0), same.rasterize('g', 24.0));
}

#[test]
fn every_mode_lays_a_pack_out_as_the_outline_does() {
    let letters = letters();
    let plain = faces(Hinting::None);
    for hinting in [Hinting::Light, Hinting::Normal, Hinting::Mono] {
        for ((name, plain), (_, hinted)) in plain.iter().zip(faces(hinting)) {
            for size in SIZES {
                let line = (plain.line_metrics(size), hinted.line_metrics(size));
                assert_eq!(line.0, line.1, "{name} {hinting:?} {size}: line");
                for &ch in &letters {
                    if !plain.has_char(ch) {
                        continue;
                    }
                    let (a, b) = (plain.rasterize(ch, size), hinted.rasterize(ch, size));
                    assert_eq!(a.advance, b.advance, "{name} {hinting:?} {ch} {size}");
                    assert_eq!(plain.advance(ch, size), hinted.advance(ch, size));
                }
            }
            assert_eq!(
                plain.kerning_pairs(&letters),
                hinted.kerning_pairs(&letters),
                "{name} {hinting:?}: kerning"
            );
        }
    }
}

#[test]
fn the_stored_advances_are_the_same_in_the_packs() {
    let letters = letters();
    let (plain, hinted) = (faces(Hinting::None), faces(Hinting::Light));
    for ((name, plain), (_, hinted)) in plain.iter().zip(&hinted) {
        let pack = |font: &OutlineFont| {
            BitmapFont::new(build_pack(font, &SIZES, letters.iter().copied())).unwrap()
        };
        let (plain_pack, hinted_pack) = (pack(plain), pack(hinted));
        for size in SIZES {
            for &ch in &letters {
                assert_eq!(
                    plain_pack.glyph(ch, size).advance,
                    hinted_pack.glyph(ch, size).advance,
                    "{name} {ch} {size}"
                );
            }
        }
    }
}

#[test]
fn the_ink_stays_where_the_outline_puts_it() {
    let plain = OutlineFont::new(LITERATA, &[("wght", 400.0)]).unwrap();
    for hinting in [Hinting::Light, Hinting::Normal, Hinting::Mono] {
        let hinted = plain.clone().with_hinting(hinting);
        for size in SIZES {
            for ch in "Hamnoxgp,.ЖдφΣ".chars() {
                let (a, b) = (plain.rasterize(ch, size), hinted.rasterize(ch, size));
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
