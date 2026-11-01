//! The size of book text on a touch screen: its slider's track is the
//! sizes the reading font draws differently, evenly spaced, so that a
//! finger's width moves the knob as far everywhere on it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_platform::Point;

const SIZE: &str = "core.font-size";

fn slider_area(app: &mut App) -> paperpaw_sdk::Rect {
    app.parts()
        .into_iter()
        .find(|p| p.role == Role::Slider)
        .expect("the size's slider")
        .area
}

/// The track's width the knob moves over, in pixels.
fn travel() -> i32 {
    let mut app = settings_page();
    let track = slider_area(&mut app);
    i32::from(track.width) - i32::from(track.height)
}

fn settings_page() -> App {
    let mut app = App::new(TOUCH, Shared::memory("", &[]));
    app.run(actions::SETTINGS);
    app.open_screen(
        Id::new_static("settings.group"),
        Params::new().with(params::ID, "core.group.reading-text"),
    );
    app.set(SIZE, "6");
    app
}

/// The size a finger leaves where it is lifted: put down on the knob at
/// the smallest size, moved `along` pixels along the track.
fn dragged(along: i32) -> i64 {
    let mut app = settings_page();
    let track = slider_area(&mut app);
    let knob = i32::from(track.height);
    let (left, y) = (i32::from(track.x) + knob / 2, i32::from(track.y) + knob / 2);
    app.drag(Point::new(left, y), Point::new(left + along, y));
    app.kernel.settings().int(SIZE)
}

#[test]
fn the_sizes_are_spread_evenly_along_the_track() {
    let sizes: Vec<i64> = (0..=travel()).map(dragged).collect();
    // How many pixels of the track each size has.
    let mut runs: Vec<(i64, i32)> = Vec::new();
    for size in &sizes {
        match runs.last_mut() {
            Some((last, n)) if last == size => *n += 1,
            _ => runs.push((*size, 1)),
        }
    }
    assert!(sizes.windows(2).all(|w| w[0] <= w[1]), "{runs:?}");
    // Every size the font draws differently is there, once.
    assert!(runs.len() >= 5, "{runs:?}");
    // Equal widths but for the two ends (half a step each, and the first
    // size where the finger lands on the knob).
    let inner: Vec<i32> = runs[2..runs.len() - 2].iter().map(|r| r.1).collect();
    let (least, most) = (inner.iter().min().unwrap(), inner.iter().max().unwrap());
    assert!(most - least <= 3, "uneven steps: {runs:?}");
}

/// A card with a right to left language, `ar`: its catalog says the
/// direction and nothing else.
fn arabic_page() -> App {
    let storage = Shared::memory(
        "[core]\nlanguage = ar\n",
        &[
            (
                "plugins/language-ar/plugin.ini",
                b"[plugin]\nid = language-ar\nname = Arabic\nversion = 1.0.0\nformats = 3\n",
            ),
            (
                "plugins/language-ar/i18n/ar.ini",
                b"[meta]\nlanguage = ar\nname = Arabic\nplural = arabic\ndirection = rtl\nscript = Arab\n",
            ),
        ],
    );
    let mut app = App::new(TOUCH, storage);
    app.run(actions::SETTINGS);
    app.open_screen(
        Id::new_static("settings.group"),
        Params::new().with(params::ID, "core.group.reading-text"),
    );
    app.set(SIZE, "6");
    app
}

/// The size left by a finger put down on the knob at the smallest size and
/// moved `along` pixels to the right (negative: to the left); the knob is at
/// the track's right end in a right to left language.
fn dragged_in_arabic(along: i32) -> i64 {
    let mut app = arabic_page();
    let track = slider_area(&mut app);
    let knob = i32::from(track.height);
    let (right, y) = (
        i32::from(track.x) + i32::from(track.width) - knob / 2,
        i32::from(track.y) + knob / 2,
    );
    app.drag(Point::new(right, y), Point::new(right + along, y));
    app.kernel.settings().int(SIZE)
}

/// Where the UI runs right to left the slider does too: the smallest size
/// at the right, the largest at the left; dragging the knob to the left
/// makes the text larger, and to the right (past its end) keeps it.
#[test]
fn the_slider_runs_from_the_right_in_a_right_to_left_language() {
    let largest = dragged(travel());
    assert!(largest > 6);
    assert_eq!(dragged_in_arabic(-travel()), largest);
    assert_eq!(dragged_in_arabic(travel()), 6);
    // In between it is larger the further the knob is taken to the left.
    let sizes: Vec<i64> = (0..=travel())
        .map(|moved| dragged_in_arabic(-moved))
        .collect();
    assert!(sizes.windows(2).all(|w| w[0] <= w[1]), "{sizes:?}");
}
