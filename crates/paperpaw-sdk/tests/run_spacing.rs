//! A book's letter spacing per run (`TextStyle::letter_spacing`) and its
//! lines over, under and through text in their styles
//! (`TextStyle::line_style`, `TextStyle::OVERLINE`), as the reading faces
//! measure, lay out and draw them.

#![cfg(feature = "unstable")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::Harness;
use paperpaw_graphics::Framebuffer;
use paperpaw_sdk::builtin::{fonts, groups, settings};
use paperpaw_sdk::document::{LETTER_SPACING_STEPS, LineStyle, TextStyle};
use paperpaw_sdk::layout::{Face, Typesetter};
use paperpaw_sdk::reading::{PageType, ReadingFaces, StatusRoom};
use paperpaw_sdk::{Color, Font, FontSpec, Id, PixelFormat, Priority, SettingSpec};

const LITERATA: &[u8] = include_bytes!("../../../plugins/fonts-default/fonts/Literata.ttf");
const FONT_ID: Id = Id::new_static("test.literata");

fn harness() -> Harness {
    let mut h = Harness::new();
    h.settings
        .define(SettingSpec::number(
            settings::FONT_SIZE.static_str(),
            "x",
            0,
            300,
            12,
            groups::OTHER,
        ))
        .unwrap();
    h.settings
        .define(SettingSpec::switch(
            settings::LIGATURES.static_str(),
            "x",
            true,
            groups::OTHER,
        ))
        .unwrap();
    let font = Font::from_data(LITERATA, &[("wght", 400.0)]).unwrap();
    h.fonts.add(FontSpec::new(FONT_ID, font)).unwrap();
    h.fonts.propose(fonts::READING, FONT_ID, Priority::DEFAULT);
    h
}

/// `face` with the run's style `style`.
fn styled(style: TextStyle) -> Face {
    Face::new(style, 0, 0)
}

#[test]
fn a_runs_letter_spacing_is_added_after_each_letter() {
    let h = harness();
    let env = h.env();
    let font = env.fonts().role(fonts::READING.as_str()).unwrap();
    let t = PageType::new(&env, &font, "en", false, StatusRoom::NONE);
    let faces = ReadingFaces::new(&env, &t, None).unwrap();
    let plain = styled(TextStyle::REGULAR);
    let spaced = styled(TextStyle::REGULAR.with_letter_spacing(8));
    let tight = styled(TextStyle::REGULAR.with_letter_spacing(-4));
    let word = "CHAPTER";
    let step = faces.size(plain) / f32::from(LETTER_SPACING_STEPS);
    let base = faces.measure(plain, word);
    let wide = faces.measure(spaced, word);
    assert!(
        (wide - base - 7.0 * 8.0 * step).abs() < 0.01,
        "{base} {wide}"
    );
    let narrow = faces.measure(tight, word);
    assert!(
        (base - narrow - 7.0 * 4.0 * step).abs() < 0.01,
        "{base} {narrow}"
    );
    // The layout's letters add up to the measured word: no ligature in
    // spaced letters, which are drawn one by one.
    for (face, text) in [(spaced, "office"), (spaced, word), (plain, word)] {
        let mut sum = 0.0;
        let mut before = None;
        for c in text.chars() {
            if let Some(left) = before {
                sum += faces.kern(face, left, c);
            }
            sum += faces.advance(face, c);
            before = Some(c);
        }
        let measured = faces.measure(face, text);
        assert!((sum - measured).abs() < 0.05, "{text}: {sum} {measured}");
    }
    // Drawn as wide as measured.
    let mut frame = Framebuffer::new(400, 60, PixelFormat::Gray8).unwrap();
    let mut canvas = frame.canvas();
    let drawn = faces.draw(&mut canvas, spaced, 4.0, 40, "office");
    assert!((drawn - faces.measure(spaced, "office")).abs() < 0.05);
}

/// The rows of `frame` with ink between `x` 20 and 180.
fn inked_rows(frame: &Framebuffer) -> Vec<u16> {
    let view = frame.view();
    (0..80u16)
        .filter(|&y| (20..180u16).any(|x| view.pixel(x, y) == Color::BLACK))
        .collect()
}

#[test]
fn lines_over_under_and_through_text_are_drawn_in_their_styles() {
    let h = harness();
    let env = h.env();
    let font = env.fonts().role(fonts::READING.as_str()).unwrap();
    let t = PageType::new(&env, &font, "en", false, StatusRoom::NONE);
    let faces = ReadingFaces::new(&env, &t, None).unwrap();
    let draw = |style: TextStyle| {
        let mut frame = Framebuffer::new(200, 80, PixelFormat::Gray8).unwrap();
        let mut canvas = frame.canvas();
        canvas.clear(Color::WHITE);
        faces.decorate(&mut canvas, styled(style), (10.0, 180.0), 50, Color::BLACK);
        frame
    };
    // Nothing without a line asked for.
    assert!(inked_rows(&draw(TextStyle::BOLD.with_line_style(LineStyle::Wavy))).is_empty());
    let under = inked_rows(&draw(TextStyle::UNDERLINE));
    let over = inked_rows(&draw(TextStyle::OVERLINE));
    assert!(!under.is_empty() && !over.is_empty());
    assert!(over.iter().all(|y| *y < 50) && under.iter().all(|y| *y > 50));
    // Two lines, a gap between them.
    let double = inked_rows(&draw(
        TextStyle::UNDERLINE.with_line_style(LineStyle::Double),
    ));
    assert_eq!(double.len(), 2 * under.len(), "{double:?}");
    assert!(double.windows(2).any(|w| w[1] > w[0] + 1));
    // Dotted: as high as one line, about half of it inked.
    let dotted = draw(TextStyle::UNDERLINE.with_line_style(LineStyle::Dotted));
    assert_eq!(inked_rows(&dotted), under);
    let y = under[0];
    let inked = (10..190u16)
        .filter(|&x| dotted.view().pixel(x, y) == Color::BLACK)
        .count();
    assert!((80..=100).contains(&inked), "{inked}");
    // Wavy: up and down, so on more rows than one line.
    let wavy = inked_rows(&draw(TextStyle::UNDERLINE.with_line_style(LineStyle::Wavy)));
    assert!(wavy.len() > under.len(), "{wavy:?}");
}
