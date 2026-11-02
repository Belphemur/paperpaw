//! A book open while the screen is turned or resized stays open: its pages
//! are laid out for the new size from the page shown, nothing is said to
//! have been closed, and turning goes on.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_sdk::builtin::actions;

const BOOK: &str = "books/numbered.txt";

fn open_book() -> Reading {
    let text = numbered_book(120, 12);
    let mut reading = Reading::new(BUTTONS, Shared::default(), BOOK, text.as_bytes(), "");
    reading.open();
    reading
}

#[test]
fn turned_the_book_stays_open_at_its_place() {
    let mut reading = open_book();
    for _ in 0..3 {
        reading.press(actions::NEXT_PAGE);
    }
    let place = reading.shown().at();
    reading.resize(800, 480);
    assert_eq!(reading.screen(), "reader.screen");
    let events = reading.events();
    assert!(!events.contains(&"closed"), "not closed: {events:?}");
    // Laid out for the new size: the page shows text, and turning goes on.
    reading.render();
    assert!(
        reading.image.iter().any(|row| row.iter().any(|&l| l < 128)),
        "the page is blank"
    );
    reading.press(actions::NEXT_PAGE);
    let ahead = reading.shown().at();
    assert!(ahead > place, "turned on from {place:?} to {ahead:?}");
    reading.resize(480, 800);
    reading.press(actions::PREV_PAGE);
    assert_eq!(reading.screen(), "reader.screen");
}
