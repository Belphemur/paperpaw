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
