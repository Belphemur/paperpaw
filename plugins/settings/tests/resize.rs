//! The settings screens turned or resized: they keep where they are (the
//! row with the focus) and lay out for the new size.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::*;

#[test]
fn the_main_page_keeps_its_focus_when_turned() {
    let mut app = App::new(X4, Shared::memory("", &[]));
    app.home();
    app.run(actions::SETTINGS);
    app.repeat(actions::DOWN, 3);
    let focused = app.focused().expect("a row has the focus");
    app.resize(800, 480);
    assert_eq!(app.screen(), "settings.list");
    assert_eq!(app.focused(), Some(focused.clone()), "landscape");
    for part in app.parts() {
        let area = part.area;
        assert!(
            u32::from(area.x) + u32::from(area.width) <= 800
                && u32::from(area.y) + u32::from(area.height) <= 480,
            "{} is off the screen: {area:?}",
            part.label
        );
    }
    app.resize(480, 800);
    assert_eq!(app.focused(), Some(focused), "upright again");
}

#[test]
fn a_settings_page_keeps_its_focus_when_turned() {
    let mut app = App::new(X4, Shared::memory("", &[]));
    app.home();
    app.run(actions::SETTINGS);
    // The first group (Reading), then down its rows.
    app.run(actions::DOWN);
    app.run(actions::CONFIRM);
    let page = app.screen();
    assert_ne!(page, "settings.list", "a group's page");
    app.repeat(actions::DOWN, 2);
    let focused = app.focused().expect("a row has the focus");
    app.resize(800, 480);
    assert_eq!(app.screen(), page);
    assert_eq!(app.focused(), Some(focused), "landscape");
}
