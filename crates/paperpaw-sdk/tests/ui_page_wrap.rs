//! "Arrow keys stay on the page" (`core.arrows-stay-on-page`, #112): off
//! (as before), a step past the first or the last entry of a list goes
//! round to the other end of the whole list; on, to the other end of the
//! page it is on (on a last page shorter than the others, among what it
//! has), and only the page keys turn pages. The same for [`ListView`]
//! (a list or a grid), [`Rows`], [`Sections`] and screens that move a
//! focus of their own ([`step_focus_in`]).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod support;

use paperpaw_sdk::builtin::{actions, groups, settings};
use paperpaw_sdk::ui::sections::{Section, Sections};
use paperpaw_sdk::ui::{ListChange, ListView, Rows, step_focus, step_focus_in};
use paperpaw_sdk::{Id, Rect, SettingSpec, SettingValue};

use support::{Device, Harness};

/// A harness with the setting `on` or off.
fn harness(on: bool) -> Harness {
    let mut h = Harness::new(Device::BUTTONS);
    let spec = SettingSpec::switch(
        settings::ARROWS_STAY_ON_PAGE.static_str(),
        "core.setting.arrows-stay-on-page",
        false,
        groups::CONTROLS,
    );
    h.settings.define(spec).unwrap();
    let id = settings::ARROWS_STAY_ON_PAGE.static_str();
    h.settings.set(id, SettingValue::Bool(on)).unwrap();
    h
}

/// Ten things, four a page: pages 0..4, 4..8 and the short 8..10.
const LEN: usize = 10;
const PAGE: usize = 4;

#[test]
fn off_a_step_goes_round_the_whole_list_as_before() {
    let h = harness(false);
    let env = h.env();
    for focus in 0..LEN {
        for delta in [-1, 1, -4, 4, -40, 40] {
            assert_eq!(
                step_focus_in(&env, focus, delta, LEN, PAGE),
                step_focus(focus, delta, LEN),
                "{focus} {delta}"
            );
        }
    }
    assert_eq!(step_focus_in(&env, 0, -1, LEN, PAGE), 9);
    assert_eq!(step_focus_in(&env, 9, 1, LEN, PAGE), 0);
}

#[test]
fn on_a_step_goes_round_the_page_it_is_on() {
    let h = harness(true);
    let env = h.env();
    let step = |focus, delta| step_focus_in(&env, focus, delta, LEN, PAGE);
    // The first page, and one in the middle.
    assert_eq!(step(0, -1), 3);
    assert_eq!(step(3, 1), 0);
    assert_eq!(step(4, -1), 7);
    assert_eq!(step(7, 1), 4);
    // Within a page, as before.
    assert_eq!(step(5, 1), 6);
    assert_eq!(step(6, -1), 5);
    // The short last page: among the two it has.
    assert_eq!(step(8, -1), 9);
    assert_eq!(step(9, 1), 8);
    // Page steps turn pages as before.
    assert_eq!(step(3, 4), 7);
    assert_eq!(step(9, -4), 5);
    // Every step of one stays on its page.
    for focus in 0..LEN {
        for delta in [-1, 1] {
            assert_eq!(step(focus, delta) / PAGE, focus / PAGE, "{focus} {delta}");
        }
    }
}

#[test]
fn on_a_single_page_goes_round_as_before() {
    let h = harness(true);
    let env = h.env();
    for (focus, delta) in [(0, -1), (2, 1), (1, 1), (1, -1)] {
        assert_eq!(
            step_focus_in(&env, focus, delta, 3, 8),
            step_focus(focus, delta, 3)
        );
    }
    assert_eq!(step_focus_in(&env, 0, -1, 0, 4), 0, "nothing to move to");
    assert_eq!(step_focus_in(&env, 0, 1, 1, 4), 0);
}

/// A list of four rows a page, or a grid of two by two.
fn list(columns: u32) -> ListView {
    let rows = if columns == 1 { 100 } else { 200 };
    ListView::new(Rect::new(0, 0, 200, 400), rows).columns(columns)
}

fn press(list: &mut ListView, env: &paperpaw_sdk::Env<'_>, action: Id) -> ListChange {
    list.navigate_in(env, &action, LEN).unwrap()
}

#[test]
fn a_list_or_grid_goes_round_its_page_only_where_asked() {
    for columns in [1, 2] {
        let mut off = list(columns);
        assert_eq!(off.rows(), PAGE);
        let h = harness(false);
        let env = h.env();
        assert_eq!(press(&mut off, &env, actions::PREVIOUS), ListChange::Page);
        assert_eq!(off.focus(), 9, "the last of the last page");

        let h = harness(true);
        let env = h.env();
        let mut on = list(columns);
        assert!(press(&mut on, &env, actions::PREVIOUS).is_focus());
        assert_eq!(on.focus(), 3, "the last of the same page");
        assert!(press(&mut on, &env, actions::NEXT).is_focus());
        assert_eq!(on.focus(), 0);
        // Up and left move as previous does, down and right as next.
        assert!(press(&mut on, &env, actions::UP).is_focus());
        assert_eq!(on.focus(), 3);
        assert!(press(&mut on, &env, actions::RIGHT).is_focus());
        assert_eq!(on.focus(), 0);
        // The page keys turn pages, onto the short last one.
        assert_eq!(press(&mut on, &env, actions::NEXT_PAGE), ListChange::Page);
        assert_eq!(on.focus(), 4);
        assert_eq!(press(&mut on, &env, actions::NEXT_PAGE), ListChange::Page);
        assert_eq!(on.focus(), 8);
        assert!(press(&mut on, &env, actions::PREVIOUS).is_focus());
        assert_eq!(on.focus(), 9);
        assert!(press(&mut on, &env, actions::DOWN).is_focus());
        assert_eq!(on.focus(), 8);
        assert_eq!(press(&mut on, &env, actions::PREV_PAGE), ListChange::Page);
        assert_eq!(on.focus(), 4);
    }
}

/// The first press on a hidden focus shows it, the setting on or off.
#[test]
fn a_hidden_focus_shows_first() {
    let h = harness(true);
    let env = h.env();
    let mut on = list(1);
    on.set_shown(false);
    assert!(press(&mut on, &env, actions::PREVIOUS).is_focus());
    assert_eq!(on.focus(), 0);
    assert!(press(&mut on, &env, actions::PREVIOUS).is_focus());
    assert_eq!(on.focus(), 3);
}

/// Rows of a settings page: a heading, entries, pages of five rows.
fn rows() -> Rows {
    let headings = vec![true, false, false, false, false, true, false, false];
    let mut heights = vec![20; 8];
    heights.extend([20; 4]);
    Rows::new(Rect::new(0, 0, 200, 100), heights, headings)
}

#[test]
fn rows_go_round_their_page_past_its_headings_only_where_asked() {
    let h = harness(false);
    let env = h.env();
    let mut off = rows();
    // The first entry, below the heading.
    let first = 1;
    off.set_focus(first);
    assert!(!off.is_heading(first));
    off.step_in(&env, false);
    assert_eq!(off.focus(), off.len() - 1, "round the whole list");

    let h = harness(true);
    let env = h.env();
    let mut on = rows();
    on.set_focus(first);
    let page = on.paging();
    assert!(page.pages > 1);
    on.step_in(&env, false);
    assert_eq!(on.paging(), page, "the same page");
    let last = on.focus();
    assert!(last > first && !on.is_heading(last));
    on.step_in(&env, true);
    assert_eq!(on.focus(), first, "and round again");
    // Every step stays on its page and never on a heading.
    for _ in 0..12 {
        on.step_in(&env, true);
        assert_eq!(on.paging(), page);
        assert!(!on.is_heading(on.focus()));
    }
    // The page buttons turn it; on the last page it goes round there.
    on.turn(true);
    let last_page = on.paging();
    assert_ne!(last_page, page);
    for _ in 0..12 {
        on.step_in(&env, false);
        assert_eq!(on.paging(), last_page);
        assert!(!on.is_heading(on.focus()));
    }
}

#[test]
fn sections_go_round_their_page_only_where_asked() {
    let make = || {
        let sections = vec![
            Section {
                heading: true,
                count: 4,
            },
            Section {
                heading: true,
                count: 5,
            },
        ];
        Sections::new(Rect::new(0, 0, 100, 100), 20, 20, sections)
    };
    let h = harness(false);
    let env = h.env();
    let mut off = make();
    off.step_in(&env, false);
    assert_eq!(off.focus(), 8, "round the whole list");

    let h = harness(true);
    let env = h.env();
    let mut on = make();
    let page = on.page_entries();
    assert!(page.end < 9, "{page:?}");
    on.step_in(&env, false);
    assert_eq!(on.focus(), page.end - 1);
    on.step_in(&env, true);
    assert_eq!(on.focus(), page.start);
    on.turn(true);
    let next = on.page_entries();
    on.step_in(&env, false);
    assert_eq!(on.focus(), next.end - 1);
    on.step_in(&env, true);
    assert_eq!(on.focus(), next.start);
}
