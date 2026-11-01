//! Numbers the widgets write themselves come out in the language's
//! digits: Latin in English, German and Arabic (which names none),
//! Persian ones where the catalog says `arabext`; what the reader typed
//! or a book says stays as it is.

#![allow(
    clippy::unwrap_used,
    reason = "tests panic on purpose when something is wrong"
)]

use alloc::format;
use alloc::string::String;

use paperpaw_i18n::Catalog;
use paperpaw_sdk::SettingValue;

use super::{countdown, date_text, world, year};
use crate::harness::Harness;

/// A harness speaking `language`: Latin digits where `digits` is empty.
fn speaking(language: &str, digits: &str) -> Harness {
    let mut h = Harness::new(paperpaw_sdk::MemoryStorage::new());
    let meta = if digits.is_empty() {
        format!("[meta]\nlanguage = {language}\n")
    } else {
        format!("[meta]\nlanguage = {language}\ndigits = {digits}\n")
    };
    h.i18n.add(Catalog::parse(meta).unwrap());
    h.i18n.set_language(language);
    h
}

fn written(h: &mut Harness, format: &'static str) -> String {
    h.set(
        "widgets.countdown.date-format",
        SettingValue::Text(format.into()),
    );
    date_text(&h.env(), "widgets.countdown", 2026, 10, 4, true)
}

#[test]
fn latin_digits_where_the_language_names_none() {
    for (language, digits) in [("en", ""), ("de", ""), ("ar", "")] {
        let mut h = speaking(language, digits);
        assert_eq!(
            written(&mut h, "widgets.date-format.iso"),
            "2026-10-04",
            "{language}"
        );
        assert_eq!(
            world::offset_text(&h.env(), 330)
                .chars()
                .filter(char::is_ascii_digit)
                .count(),
            3,
            "{language}"
        );
        let progress = year::Progress {
            day: 100,
            days: 365,
            permille: 275,
            week: 15,
            left: 265,
        };
        assert_eq!(
            year::percent_text(&h.env(), &progress),
            "27 %",
            "{language}"
        );
        assert_eq!(countdown::shown(&h.i18n, 12, "Exam").big, "12");
    }
}

#[test]
fn persian_digits_where_the_catalog_says_arabext() {
    let mut h = speaking("fa", "arabext");
    assert_eq!(written(&mut h, "widgets.date-format.iso"), "۲۰۲۶-۱۰-۰۴");
    assert!(
        world::offset_text(&h.env(), 330).contains("۵:۳۰"),
        "{}",
        world::offset_text(&h.env(), 330)
    );
    let progress = year::Progress {
        day: 100,
        days: 365,
        permille: 275,
        week: 15,
        left: 265,
    };
    assert_eq!(year::percent_text(&h.env(), &progress), "۲۷ %");
    assert_eq!(countdown::shown(&h.i18n, 12, "Exam").big, "۱۲");
    assert_eq!(countdown::shown(&h.i18n, -3, "Exam").big, "۳");
}

#[test]
fn what_people_and_books_wrote_stays_as_written() {
    let h = speaking("fa", "arabext");
    // The title is the reader's own text: its digits are not touched,
    // in the sentence either.
    let shown = countdown::shown(&h.i18n, 12, "Exam 2026");
    assert_eq!(shown.big, "۱۲");
    assert_eq!(shown.note.matches("2026").count(), 1);
    assert!(!shown.note.contains("۲۰۲۶"), "{}", shown.note);
}
