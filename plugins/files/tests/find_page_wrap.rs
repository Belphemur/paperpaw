//! The Find screen with buttons and "Arrow keys stay on the page" (#112):
//! off (as before), a row down from the last row of letters lands
//! wherever the count leads; on, the search and the rows of letters are
//! the page and a row step goes round it in the column the focus left.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;
use paperpaw_sdk::Id;

const FIND: Id = Id::new_static("files.list.find");
const SEARCH: &str = "Search title or author";

/// A book for every letter A to Z (five letters to a row on the X4: six
/// rows, the last with Z alone), the setting `on` or off, the Find screen
/// open on its search.
fn find(on: bool) -> App {
    let files: Vec<(String, Vec<u8>)> = ('A'..='Z')
        .map(|l| (format!("books/{l}ook.txt"), b"Some text.".to_vec()))
        .collect();
    let files: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(p, d)| (p.as_str(), d.as_slice()))
        .collect();
    let mut app = App::new(X4, Shared::memory("", &files));
    if on {
        app.set("core.arrows-stay-on-page", "true");
    }
    app.run(actions::LIBRARY);
    app.wait(61_000);
    app.run(FIND);
    assert_eq!(app.screen(), "files.list.find");
    assert_eq!(app.focused().as_deref(), Some(SEARCH));
    app
}

/// What the focus is on after each of `steps`.
fn walk(app: &mut App, steps: &[&Id]) -> Vec<String> {
    steps
        .iter()
        .map(|step| {
            app.run((*step).clone());
            app.focused().unwrap_or_default()
        })
        .collect()
}

#[test]
fn off_a_row_down_from_the_last_row_goes_where_counting_on_leads() {
    let mut app = find(false);
    app.run(actions::RIGHT);
    let down = &actions::DOWN;
    // A, then a row at a time; from Z the count wraps round the 27 places.
    assert_eq!(
        walk(&mut app, &[down, down, down, down, down, down]),
        ["F", "K", "P", "U", "Z", "D"]
    );
    // Up from the first row is the search, and from the search the count
    // goes back five places: no column kept.
    let up = &actions::UP;
    assert_eq!(walk(&mut app, &[up, up, up]), [SEARCH, "V", "Q"]);
}

#[test]
fn on_a_row_down_from_the_last_row_goes_to_the_search_and_back_in_the_column() {
    let mut app = find(true);
    let (down, up) = (&actions::DOWN, &actions::UP);
    app.run(actions::RIGHT);
    app.run(actions::RIGHT);
    app.run(actions::RIGHT);
    assert_eq!(app.focused().as_deref(), Some("C"));
    // Column C has five rows; below the fifth there is no C: the search.
    assert_eq!(
        walk(&mut app, &[down, down, down, down, down]),
        ["H", "M", "R", "W", SEARCH]
    );
    // Down from the search comes back to the top in the column it left.
    assert_eq!(walk(&mut app, &[down]), ["C"]);
    // Up from the top row is the search, up from the search the lowest row
    // with the column.
    assert_eq!(walk(&mut app, &[up, up]), [SEARCH, "W"]);
    // The page buttons step a row too.
    assert_eq!(walk(&mut app, &[&actions::NEXT_PAGE]), [SEARCH]);
    assert_eq!(walk(&mut app, &[&actions::PREV_PAGE]), ["W"]);
}

#[test]
fn on_the_last_row_has_one_letter_and_the_first_column_reaches_it() {
    let mut app = find(true);
    let (down, up) = (&actions::DOWN, &actions::UP);
    app.run(actions::RIGHT);
    assert_eq!(
        walk(&mut app, &[down, down, down, down, down, down]),
        ["F", "K", "P", "U", "Z", SEARCH]
    );
    assert_eq!(walk(&mut app, &[up]), ["Z"]);
}

#[test]
fn left_and_right_go_round_the_whole_screen_either_way() {
    for on in [false, true] {
        let mut app = find(on);
        let (left, right) = (&actions::LEFT, &actions::RIGHT);
        assert_eq!(walk(&mut app, &[left, right, right]), ["Z", SEARCH, "A"]);
    }
}
