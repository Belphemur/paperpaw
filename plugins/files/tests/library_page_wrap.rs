//! The library with buttons and "Arrow keys stay on the page" (#112):
//! off (as before), up from the first book reaches the row above the
//! books and then the last book of the whole library; on, the last book
//! of the page shown, and down from the page's last book the row and
//! then its first. Only the page buttons turn pages.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;

/// What the row above the books says while all books show.
const ALL_BOOKS: &str = "All books";

/// The library of `count` books on the X4, the setting `on` or off.
fn library(count: usize, on: bool) -> App {
    let files: Vec<(String, Vec<u8>)> = (1..=count)
        .map(|n| (format!("books/Book {n:02}.txt"), b"Some text.".to_vec()))
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
    assert_eq!(app.screen(), "files.browser");
    app
}

/// The title the focus is on.
fn focused(app: &mut App) -> String {
    let label = app.focused().unwrap_or_default();
    label.split(", ").next().unwrap_or_default().to_string()
}

/// The books the page shows, in order.
fn shown(app: &mut App) -> Vec<String> {
    app.parts()
        .into_iter()
        .filter(|p| p.role == Role::Row)
        .map(|p| p.label.split(", ").next().unwrap_or_default().to_string())
        .filter(|t| t.starts_with("Book "))
        .collect()
}

/// The focus on the first book of the page shown.
fn on_first(app: &mut App) {
    let first = shown(app)[0].clone();
    for _ in 0..60 {
        if focused(app) == first {
            return;
        }
        app.run(actions::UP);
    }
    panic!("not on {first}: {}", app.described());
}

#[test]
fn off_up_from_the_first_book_goes_round_the_whole_library() {
    let _turn = turn();
    let mut app = library(30, false);
    app.run(actions::DOWN);
    on_first(&mut app);
    assert_eq!(focused(&mut app), "Book 01");
    app.run(actions::UP);
    assert_eq!(focused(&mut app), ALL_BOOKS);
    app.run(actions::UP);
    assert_eq!(focused(&mut app), "Book 30", "the last of the last page");
}

#[test]
fn on_the_arrows_go_round_the_page_shown() {
    let _turn = turn();
    let mut app = library(30, true);
    app.run(actions::DOWN);
    on_first(&mut app);
    let page = shown(&mut app);
    assert!(page.len() < 30, "{page:?}");
    let (first, last) = (page[0].clone(), page[page.len() - 1].clone());
    // Up from the first: the row above the books, then the page's last.
    app.run(actions::UP);
    assert_eq!(focused(&mut app), ALL_BOOKS);
    app.run(actions::UP);
    assert_eq!(focused(&mut app), last);
    assert_eq!(shown(&mut app), page, "the same page");
    // Down from the last: the row, then the page's first.
    app.run(actions::DOWN);
    assert_eq!(focused(&mut app), ALL_BOOKS);
    app.run(actions::DOWN);
    assert_eq!(focused(&mut app), first);
    // Next and previous as down and up.
    app.run(actions::PREVIOUS);
    app.run(actions::PREVIOUS);
    assert_eq!(focused(&mut app), last);
    // The page buttons turn pages; on the next page the same.
    app.run(actions::NEXT_PAGE);
    let next = shown(&mut app);
    assert_ne!(next, page);
    on_first(&mut app);
    app.run(actions::UP);
    assert_eq!(focused(&mut app), ALL_BOOKS);
    app.run(actions::UP);
    assert_eq!(focused(&mut app), next[next.len() - 1]);
    assert_eq!(shown(&mut app), next);
}

#[test]
fn on_a_short_last_page_goes_round_among_its_books() {
    let _turn = turn();
    let mut app = library(30, true);
    app.repeat(actions::NEXT_PAGE, 20);
    let page = shown(&mut app);
    assert!(page.contains(&String::from("Book 30")), "{page:?}");
    on_first(&mut app);
    app.run(actions::UP);
    app.run(actions::UP);
    assert_eq!(focused(&mut app), "Book 30");
    app.run(actions::DOWN);
    app.run(actions::DOWN);
    assert_eq!(focused(&mut app), page[0]);
}

/// The library of `count` books as covers, the setting `on` or off.
fn covers(count: usize, on: bool) -> App {
    let mut app = library(count, on);
    app.set("files.view", "files.view.covers");
    app.run(actions::DOWN);
    app
}

/// The covers a row has: those level with the page's first.
fn columns(app: &mut App) -> usize {
    let tops: Vec<_> = app
        .parts()
        .into_iter()
        .filter(|p| p.role == Role::Row && p.label.starts_with("Book "))
        .map(|p| p.area.y)
        .collect();
    tops.iter().filter(|y| **y == tops[0]).count()
}

/// The focus on cover `n` of the page shown, going by next.
fn on_cover(app: &mut App, n: usize) {
    let want = shown(app)[n].clone();
    for _ in 0..80 {
        if focused(app) == want {
            return;
        }
        app.run(actions::NEXT);
    }
    panic!("not on {want}: {}", app.described());
}

/// Two rows up or down from cover `n` of the page shown: where the
/// focus is after the first and after the second.
fn rows_from(app: &mut App, n: usize, action: Id) -> (String, String) {
    on_cover(app, n);
    app.run(action.clone());
    let between = focused(app);
    app.run(action);
    (between, focused(app))
}

/// The books a page of covers has and the covers a row.
fn measures(app: &mut App) -> (usize, usize) {
    (shown(app).len(), columns(app))
}

#[test]
fn off_a_row_of_covers_leaves_the_page_as_before() {
    let _turn = turn();
    let mut app = covers(60, false);
    let first = shown(&mut app);
    let c = columns(&mut app);
    assert!(c > 1 && first.len() > c, "{c} columns, {first:?}");
    app.run(actions::NEXT_PAGE);
    on_cover(&mut app, 1);
    app.run(actions::UP);
    assert_eq!(
        focused(&mut app),
        first[first.len() - c + 1],
        "the row above"
    );
    assert_eq!(shown(&mut app), first);
}

#[test]
fn on_a_full_page_a_row_of_covers_goes_round_in_its_column() {
    let _turn = turn();
    let mut app = covers(60, true);
    let page = shown(&mut app);
    let c = columns(&mut app);
    assert!(
        c > 1 && page.len().is_multiple_of(c) && page.len() > c,
        "{c}, {page:?}"
    );
    let bottom = page.len() - c;
    let row = ALL_BOOKS.to_string();
    // Up from the top row: the row above the books, then the same
    // column on the bottom row; down from there the other way.
    let up = rows_from(&mut app, 1, actions::UP);
    assert_eq!(up, (row.clone(), page[bottom + 1].clone()));
    let down = rows_from(&mut app, bottom + 1, actions::DOWN);
    assert_eq!(down, (row, page[1].clone()));
    // Within the page as before.
    let within = rows_from(&mut app, bottom, actions::UP);
    assert_eq!(within.0, page[bottom - c]);
    assert_eq!(shown(&mut app), page, "the same page");
}

#[test]
fn on_a_short_last_page_a_row_goes_to_the_lowest_cover_of_its_column() {
    let _turn = turn();
    let (per_page, c) = measures(&mut covers(60, false));
    // The last page: a full row and one cover below its first.
    let mut app = covers(per_page + c + 1, true);
    app.run(actions::NEXT_PAGE);
    let page = shown(&mut app);
    assert_eq!(page.len(), c + 1, "{page:?}");
    let row = ALL_BOOKS.to_string();
    // The first column has two rows, the last only the top one.
    let up = rows_from(&mut app, 0, actions::UP);
    assert_eq!(up, (row.clone(), page[c].clone()));
    let up = rows_from(&mut app, c - 1, actions::UP);
    assert_eq!(up, (row.clone(), page[c - 1].clone()));
    let down = rows_from(&mut app, c, actions::DOWN);
    assert_eq!(down, (row, page[0].clone()));
    assert_eq!(shown(&mut app), page, "the same page");
}

#[test]
fn on_a_single_row_a_row_comes_back_to_its_cover() {
    let _turn = turn();
    let (per_page, c) = measures(&mut covers(60, false));
    let mut app = covers(per_page + 2, true);
    app.run(actions::NEXT_PAGE);
    let page = shown(&mut app);
    assert_eq!(page.len(), 2, "{page:?}");
    let row = ALL_BOOKS.to_string();
    assert!(c > 1);
    for action in [actions::UP, actions::DOWN] {
        let back = rows_from(&mut app, 1, action);
        assert_eq!(back, (row.clone(), page[1].clone()));
    }
}
