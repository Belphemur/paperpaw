//! The statistics' screens with the real kernel, on an X4 (buttons
//! only) and on a touch screen: the overview, the calendar, the books
//! and a book's numbers with no reading, one day and many years of it,
//! without a clock and with stays at the edges of time; moving through
//! them by buttons and by touch.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{App, Card, DAY, TOUCH, X4};
use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::{MemoryStorage, Params, Point};

/// 14 October 2026 (a Wednesday), 10:41 in Berlin (UTC+2).
const NOW: i64 = 1_791_967_260_000;
/// That day, in days since 1970.
const TODAY: i64 = 20_740;

/// 19:00 in Berlin on the day `back` days before today.
fn evening(back: i64) -> i64 {
    (TODAY - back) * DAY + 17 * 3600
}

/// Anna Karenina (300 pages) read 23 minutes today, Effi Briest (pages
/// not counted) 41 minutes yesterday; a third book never read.
fn two_days() -> MemoryStorage {
    Card::new()
        .book(1, "Anna Karenina", 300, TODAY * DAY)
        .book(2, "Effi Briest", 0, (TODAY - 1) * DAY + 18 * 3600)
        .book(3, "Never read", 120, TODAY * DAY + 3600)
        .read(2, evening(1), 41)
        .read(1, TODAY * DAY + 7 * 3600, 23)
        .build()
}

fn app(device: paperpaw_sdk::DeviceInfo, storage: MemoryStorage) -> App {
    App::new(device, storage, Some(NOW), "", false)
}

fn lines(shown: &str) -> Vec<&str> {
    shown.lines().map(|l| l.trim_start_matches('>')).collect()
}

#[test]
fn the_overview_without_a_clock_or_reading_says_why() {
    for device in [X4, TOUCH] {
        let mut app = App::new(device, MemoryStorage::new(), None, "", false);
        app.open("stats.overview", Params::new());
        let shown = app.shown();
        assert!(shown.contains("Statistics need the time of day"), "{shown}");
        // The ways on are still there.
        for row in ["Calendar", "Books", "Sync with KOReader"] {
            assert!(lines(&shown).contains(&row), "{row}: {shown}");
        }
    }
}

#[test]
fn the_overview_with_no_reading_shows_zeros() {
    for device in [X4, TOUCH] {
        let mut app = app(device, MemoryStorage::new());
        app.open("stats.overview", Params::new());
        let shown = app.shown();
        let lines = lines(&shown);
        for line in ["Today", "0 min", "0 pages", "0 days", "The last 14 days"] {
            assert!(lines.contains(&line), "{line}: {shown}");
        }
        assert!(
            shown.contains("0 min in all · 0 books · 0 reading days"),
            "{shown}"
        );
        // Without a book being read, no row for it.
        assert!(!lines.contains(&"This book"), "{shown}");
    }
}

#[test]
fn the_overview_of_two_days() {
    let mut app = app(X4, two_days());
    app.open("stats.overview", Params::new().with(params::ID, "1"));
    let shown = app.shown();
    let lines = lines(&shown);
    // Today, the last seven days and the days in a row.
    for line in ["23 min", "23 pages", "1 h 04 min", "2 days", "This book"] {
        assert!(lines.contains(&line), "{line}: {shown}");
    }
    // The highest bar is labelled: yesterday's.
    assert!(lines.contains(&"41 min"), "{shown}");
    assert!(
        shown.contains("1 h 04 min in all · 3 books · 2 reading days"),
        "{shown}"
    );
}

#[test]
fn the_overview_of_many_years() {
    // Half an hour on the same day of each of 15 years back, ten minutes
    // today.
    let mut card = Card::new().book(1, "Long read", 900, TODAY * DAY);
    for year in (1..=15).rev() {
        card = card.read(1, evening(year * 365), 30);
    }
    let mut app = app(TOUCH, card.read(1, TODAY * DAY + 7 * 3600, 10).build());
    app.open("stats.overview", Params::new());
    let shown = app.shown();
    let lines = lines(&shown);
    // 15 * 30 + 10 minutes.
    assert!(
        shown.contains("7 h 40 min in all · 1 book · 16 reading days"),
        "{shown}"
    );
    // Today and this week only today's; a streak of one.
    assert!(lines.contains(&"10 min"), "{shown}");
    assert!(lines.contains(&"1 day"), "{shown}");
    // The calendar goes back a year by month without trouble.
    app.open("stats.calendar", Params::new());
    for _ in 0..12 {
        app.run(actions::PREV_PAGE);
    }
    let shown = app.shown();
    assert!(shown.contains("October 2025"), "{shown}");
    assert!(shown.contains("Not read"), "{shown}");
}

/// A stay in 1970 and one in 2106 (a clock that was wrong once), beside
/// today's reading.
fn stays_at_the_edges_of_time() -> MemoryStorage {
    Card::new()
        .book(1, "Odd", 300, TODAY * DAY)
        .stay(common::record(1, 1, 0, 600, 300))
        .stay(common::record(1, 2, i64::from(u32::MAX), 600, 300))
        .read(1, TODAY * DAY + 7 * 3600, 5)
        .build()
}

#[test]
fn stays_at_the_edges_of_time_do_not_count_today() {
    for device in [X4, TOUCH] {
        let mut app = app(device, stays_at_the_edges_of_time());
        app.open("stats.overview", Params::new());
        let shown = app.shown();
        let lines = lines(&shown);
        assert!(lines.contains(&"5 min"), "{shown}");
        assert!(lines.contains(&"5 pages"), "{shown}");
        // The streak counts no day after today.
        assert!(lines.contains(&"1 day"), "{shown}");
        app.open("stats.book", Params::new().with(params::ID, "1"));
        let shown = app.shown();
        assert!(
            shown.contains("First read\n") && shown.contains("1/1/1970"),
            "{shown}"
        );
    }
}

/// The calendar reads a day's stays from the part of the log around it
/// (`store::scan_range`), taking the log to be in the order of time; one
/// stay dated far ahead (2106, a clock wrong once) does not end that
/// reading, so the day's stays after it are found with their pages.
#[test]
fn the_calendar_finds_the_days_stays_after_one_at_the_edge_of_time() {
    for device in [X4, TOUCH] {
        let mut app = app(device, stays_at_the_edges_of_time());
        app.open("stats.calendar", Params::new());
        let shown = app.shown();
        assert!(shown.contains("5 min · 5 pages\nOdd"), "{shown}");
    }
}

#[test]
fn buttons_move_through_the_overview() {
    let mut app = app(X4, two_days());
    app.open("stats.overview", Params::new().with(params::ID, "1"));
    assert!(app.shown().contains(">This book\n"), "{}", app.shown());
    app.press("down");
    assert!(app.shown().contains(">Calendar\n"), "{}", app.shown());
    app.press("confirm");
    assert_eq!(app.screen(), "stats.calendar");
    app.press("back");
    assert_eq!(app.screen(), "stats.overview");
    // Up from the first row goes round to the last, and down from there
    // back to the first.
    app.open("stats.overview", Params::new());
    assert!(app.shown().contains(">Calendar\n"), "{}", app.shown());
    app.press("up");
    assert!(
        app.shown().contains(">Sync with KOReader\n"),
        "{}",
        app.shown()
    );
    app.press("confirm");
    assert_eq!(app.screen(), "stats.sync");
    app.press("back");
    app.press("down");
    assert!(app.shown().contains(">Calendar\n"), "{}", app.shown());
    app.press("down");
    assert!(app.shown().contains(">Reading rhythm\n"), "{}", app.shown());
    app.press("down");
    app.press("confirm");
    assert_eq!(app.screen(), "stats.books");
    // The books: the one last opened first, a book never read left out.
    let shown = app.shown();
    assert!(
        shown.contains(">Anna Karenina, 23 min, Jane Doe · last read 10/14/2026\n"),
        "{shown}"
    );
    assert!(shown.contains("Effi Briest, 41 min"), "{shown}");
    assert!(!shown.contains("Never read"), "{shown}");
    app.press("down");
    app.press("confirm");
    assert_eq!(app.screen(), "stats.book");
    assert!(app.shown().starts_with("Effi Briest\n"), "{}", app.shown());
}

#[test]
fn touch_moves_through_the_overview() {
    let mut app = app(TOUCH, two_days());
    app.open("stats.overview", Params::new().with(params::ID, "2"));
    // Nothing focused without buttons.
    assert!(!app.shown().contains('>'), "{}", app.shown());
    app.tap_on("This book");
    assert_eq!(app.screen(), "stats.book");
    assert!(app.shown().starts_with("Effi Briest\n"), "{}", app.shown());
    app.run(actions::BACK);
    app.tap_on("Calendar");
    assert_eq!(app.screen(), "stats.calendar");
    app.run(actions::BACK);
    app.tap_on("Books");
    assert_eq!(app.screen(), "stats.books");
    app.tap_on("Anna Karenina");
    assert_eq!(app.screen(), "stats.book");
    assert!(
        app.shown().starts_with("Anna Karenina\n"),
        "{}",
        app.shown()
    );
    // A tap beside the rows does nothing.
    app.run(actions::BACK);
    app.run(actions::BACK);
    app.tap(Point::new(5, 1440));
    assert_eq!(app.screen(), "stats.overview");
}

#[test]
fn buttons_move_through_the_calendar() {
    let mut app = app(X4, two_days());
    app.open("stats.calendar", Params::new());
    let shown = app.shown();
    assert!(shown.contains("October 2026"), "{shown}");
    assert!(shown.contains("2 days in a row"), "{shown}");
    assert!(
        shown.contains("Wednesday, October 14\n23 min · 23 pages\nAnna Karenina\n23 min"),
        "{shown}"
    );
    // A day back: yesterday, and its book.
    app.press("left");
    let shown = app.shown();
    assert!(
        shown.contains("Tuesday, October 13\n41 min · 41 pages\nEffi Briest\n41 min"),
        "{shown}"
    );
    // A week back, a day on.
    app.press("up");
    app.press("right");
    let shown = app.shown();
    assert!(shown.contains("Wednesday, October 7\nNot read"), "{shown}");
    // A month on and back: the first of the month.
    app.press("page-forward");
    assert!(app.shown().contains("November 2026"), "{}", app.shown());
    assert!(
        app.shown().contains("Sunday, November 1"),
        "{}",
        app.shown()
    );
    app.press("page-back");
    app.press("page-back");
    assert!(app.shown().contains("September 2026"), "{}", app.shown());
    assert!(
        app.shown().contains("Tuesday, September 1"),
        "{}",
        app.shown()
    );
    // Down by a week across the month's end.
    for _ in 0..5 {
        app.press("down");
    }
    assert!(
        app.shown().contains("Tuesday, October 6"),
        "{}",
        app.shown()
    );
}

#[test]
fn touch_moves_through_the_calendar() {
    let mut app = app(TOUCH, two_days());
    app.open("stats.calendar", Params::new());
    app.tap_on("13");
    assert!(
        app.shown().contains("Tuesday, October 13\n41 min"),
        "{}",
        app.shown()
    );
    // The arrows beside the month.
    app.tap_on("›");
    assert!(app.shown().contains("November 2026"), "{}", app.shown());
    app.tap_on("‹");
    app.tap_on("‹");
    assert!(app.shown().contains("September 2026"), "{}", app.shown());
    // A swipe turns the month like a page.
    let middle = app.where_is("15").unwrap();
    app.swipe(
        Point::new(middle.x + 300, middle.y),
        Point::new(middle.x - 300, middle.y),
    );
    assert!(app.shown().contains("October 2026"), "{}", app.shown());
    // A day of the month before, left out of the grid, is not chosen:
    // October 2026 begins on a Thursday, three days after the week's.
    let (first, second) = (app.where_is("1").unwrap(), app.where_is("2").unwrap());
    let before = app.shown();
    app.tap(Point::new(first.x - 3 * (second.x - first.x), first.y));
    assert_eq!(app.shown(), before);
}

/// Without a clock there is no today to show a month around: the calendar
/// says it needs the time rather than show January 1970, also after the
/// buttons try to move through it.
#[test]
fn the_calendar_without_a_clock_says_it_needs_the_time() {
    for device in [X4, TOUCH] {
        let mut app = App::new(device, MemoryStorage::new(), None, "", false);
        app.open("stats.calendar", Params::new());
        let shown = app.shown();
        assert!(shown.contains("Statistics need the time of day"), "{shown}");
        assert!(!shown.contains("1970"), "{shown}");
        app.run(actions::UP);
        app.run(actions::PREV_PAGE);
        let shown = app.shown();
        assert!(shown.contains("Statistics need the time of day"), "{shown}");
        assert!(!shown.contains("1969"), "{shown}");
    }
}

#[test]
fn a_book_shows_its_numbers() {
    for device in [X4, TOUCH] {
        let mut app = app(device, two_days());
        app.open("stats.book", Params::new().with(params::ID, "1"));
        let shown = app.shown();
        for (value, label) in [
            ("Jane Doe", "Author"),
            ("23 min", "Time read"),
            ("23 of 300", "Pages read"),
            // A minute a page: a minute a page left.
            ("60 s per page", "Pace"),
            // Cut to the tile's width on the X4.
            ("4 h 37 min", "Time left (estim"),
            ("1 day", "Reading days"),
            ("10/14/2026", "First read"),
        ] {
            // The label whole, or its start where cut to its tile.
            assert!(
                shown.contains(&format!("{value}\n{label}")),
                "{label}: {shown}"
            );
        }
        // Read on one day: no last day apart from the first.
        assert!(!shown.contains("Last read"), "{shown}");
        assert!(shown.contains("Reading time per day"), "{shown}");
    }
}

#[test]
fn a_book_without_its_pages_shows_what_was_read() {
    let mut app = app(X4, two_days());
    app.open("stats.book", Params::new().with(params::ID, "2"));
    let shown = app.shown();
    assert!(shown.contains("41 pages\nPages read\n"), "{shown}");
    assert!(shown.contains("60 s per page\nPace\n"), "{shown}");
    // How much is left is not known.
    assert!(!shown.contains("Time left"), "{shown}");
    // Only back does anything here.
    app.press("down");
    app.press("confirm");
    assert_eq!(app.screen(), "stats.book");
}

#[test]
fn a_book_read_on_many_days_shows_the_last_ones() {
    let mut card = Card::new().book(1, "Long read", 900, TODAY * DAY);
    for back in (0..30).rev() {
        card = card.read(1, evening(back), 2);
    }
    let mut app = app(X4, card.build());
    app.open("stats.book", Params::new().with(params::ID, "1"));
    let shown = app.shown();
    assert!(shown.contains("30 days\nReading days\n"), "{shown}");
    assert!(shown.contains("9/15/2026\nFirst read\n"), "{shown}");
    assert!(shown.contains("10/14/2026\nLast read\n"), "{shown}");
    // The same two pages a day, read in 300 pages: each stands for
    // three of the 900 there are now (as KOReader rescales them).
    assert!(shown.contains("6 of 900\nPages read\n"), "{shown}");
    assert!(shown.contains("600 s per page\nPace\n"), "{shown}");
    // The chart: the last ten days.
    assert!(shown.contains("10/14"), "{shown}");
    assert!(!shown.contains("\n10/4\n"), "{shown}");
}

#[test]
fn an_unknown_book_or_no_books_say_so() {
    for device in [X4, TOUCH] {
        let mut app = app(device, two_days());
        for id in ["99", "not a number", "-1", "70000"] {
            app.open("stats.book", Params::new().with(params::ID, id));
            assert!(app.shown().contains("Nothing recorded yet"), "{id}");
        }
        app.open("stats.book", Params::new());
        assert!(app.shown().contains("Nothing recorded yet"));
        let mut empty = self::app(device, MemoryStorage::new());
        empty.open("stats.books", Params::new());
        assert!(empty.shown().contains("Nothing recorded yet"));
        // Buttons and taps on the empty list do nothing.
        empty.press("down");
        empty.press("confirm");
        empty.tap(Point::new(100, 400));
        assert_eq!(empty.screen(), "stats.books");
    }
}

#[test]
fn many_books_page_through_the_list() {
    let mut card = Card::new();
    for id in 1..=40u16 {
        card = card
            .book(
                id,
                &format!("Book {id:02}"),
                100,
                TODAY * DAY - i64::from(id) * 60,
            )
            .read(id, evening(i64::from(id)), 1);
    }
    let mut app = app(X4, card.build());
    app.open("stats.books", Params::new());
    // The one opened last first.
    assert!(app.shown().contains(">Book 01, 1 min"), "{}", app.shown());
    app.press("page-forward");
    let shown = app.shown();
    assert!(!shown.contains("Book 01,"), "{shown}");
    for _ in 0..10 {
        app.press("page-forward");
    }
    app.press("confirm");
    assert_eq!(app.screen(), "stats.book");
    assert!(app.shown().starts_with("Book 40\n"), "{}", app.shown());
}

#[test]
fn the_rhythm_shows_the_days_and_the_parts_of_the_day() {
    for device in [X4, TOUCH] {
        let mut app = app(device, two_days());
        app.open("stats.rhythm", Params::new());
        // And something was drawn.
        let drawn = app.black();
        assert!(drawn > 0, "{} black={drawn}", device.id);
        let shown = app.shown();
        assert!(shown.contains("Reading rhythm"), "{shown}");
        // The parts of the day are named (where a bar's label fits; the
        // narrow ones go).
        assert!(
            lines(&shown)
                .iter()
                .any(|l| l.contains("Afternoon") || l.contains("Night")),
            "{shown}"
        );
        // How long a session lasted, from the log alone.
        assert!(shown.contains("a session"), "{shown}");
    }
}

#[test]
fn the_rhythm_without_a_clock_says_why() {
    for device in [X4, TOUCH] {
        let mut app = App::new(device, MemoryStorage::new(), None, "", false);
        app.open("stats.rhythm", Params::new());
        assert!(
            app.shown().contains("Statistics need the time of day"),
            "{}",
            app.shown()
        );
    }
}
