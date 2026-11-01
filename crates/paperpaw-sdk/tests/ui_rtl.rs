//! The UI where its language runs right to left (`UiStyle::with_rtl`, as the
//! catalog's `[meta] direction = rtl` makes it): every block is the mirror
//! image of the same block left to right, in what it draws and in where a
//! touch is taken, its text starting at the right.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod support;

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::semantics::Role;
use paperpaw_sdk::ui::{self, Header, Paging, UiStyle};
use paperpaw_sdk::{Point, Rect};

use support::{Device, Harness, describe, drawn_outside};

const WIDTH: u16 = 480;

/// The mirror image of `rect` in a screen as wide as the tests'.
fn mirrored(rect: Rect) -> Rect {
    Rect::new(WIDTH - rect.x - rect.width, rect.y, rect.width, rect.height)
}

/// The first and the last column with ink in `rows`, against the paper
/// the frame was cleared to.
fn ink_columns(frame: &paperpaw_graphics::Framebuffer, rows: std::ops::Range<u16>) -> (u16, u16) {
    let view = frame.view();
    let inked = |x: u16| rows.clone().any(|y| view.pixel(x, y) != support::PAPER);
    let first = (0..frame.width()).find(|&x| inked(x)).expect("ink");
    let last = (0..frame.width()).rev().find(|&x| inked(x)).expect("ink");
    (first, last)
}

#[test]
fn the_switch_stands_at_the_rows_other_end_and_its_knob_goes_the_other_way() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, ltr.row());
    assert_eq!(
        ui::switch_rect(&rtl, row),
        mirrored(ui::switch_rect(&ltr, row))
    );
    // Flush with the margin, at the left.
    assert_eq!(ui::switch_rect(&rtl, row).x, rtl.margin());
    // On is the knob at the switch's far end from the row's start.
    let knob = |style: &UiStyle, on: bool| {
        let mut frame = support::frame(WIDTH, 800);
        ui::Ui::new(&mut frame.canvas(), &env, style).switch(row, on, false);
        let switch = ui::switch_rect(style, row);
        let rows = switch.y..switch.bottom() as u16;
        // Black pixels in the switch's left half against its right half.
        let view = frame.view();
        let mid = switch.x + switch.width / 2;
        let count = |xs: std::ops::Range<u16>| {
            xs.flat_map(|x| rows.clone().map(move |y| (x, y)))
                .filter(|&(x, y)| view.pixel(x, y) == paperpaw_sdk::Color::BLACK)
                .count() as i32
        };
        count(switch.x..mid) - count(mid..switch.right() as u16)
    };
    // Switched on the filled track is the same either way; off, the
    // knob is on the start side of the switch: left, and right when mirrored.
    let (ltr_off, rtl_off) = (knob(&ltr, false), knob(&rtl, false));
    assert!(
        ltr_off != rtl_off,
        "the knob of an off switch moved over: {ltr_off} vs {rtl_off}"
    );
}

#[test]
fn a_title_starts_at_the_right_margin_and_the_value_ends_at_the_left_one() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, ltr.row());
    let rows = row.y..row.bottom() as u16;
    for (title, value) in [("Brightness", ""), ("Brightness", "40 %")] {
        let draw = |style: &UiStyle| {
            let mut frame = support::frame(WIDTH, 800);
            ui::Row::value(title, value)
                .draw(&mut ui::Ui::new(&mut frame.canvas(), &env, style), row);
            ink_columns(&frame, rows.clone())
        };
        let ((first, last), (r_first, r_last)) = (draw(&ltr), draw(&rtl));
        let margin = ltr.margin();
        let bearing = 6;
        // Left to right the title begins at the left margin; mirrored it
        // ends at the right one.
        assert!(first >= margin && first <= margin + bearing, "{first}");
        assert!(
            r_last + margin + bearing >= WIDTH && r_last < WIDTH - margin + 1,
            "title ends at {r_last} ({title:?}, {value:?})"
        );
        if !value.is_empty() {
            // The value ends at the right margin; mirrored it begins at the left one.
            assert!(
                last + margin + bearing >= WIDTH && last < WIDTH - margin + 1,
                "{last}"
            );
            assert!(
                r_first >= margin && r_first <= margin + bearing,
                "{r_first}"
            );
        }
    }
}

#[test]
fn a_switch_row_keeps_to_its_rectangle_mirrored_and_tells_its_parts_there() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let rtl = UiStyle::new(&env).with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, rtl.row());
    let long = "A title much longer than the row could ever hold beside a switch";
    for title in ["Wi-Fi", long] {
        let (described, mut frame) = describe(WIDTH, 800, |c| {
            ui::Row::switch(title, true).draw(&mut ui::Ui::new(c, &env, &rtl), row);
        });
        assert_eq!(drawn_outside(&mut frame, row), None);
        let switch = described
            .iter()
            .find(|d| d.role == Role::Switch)
            .map(|d| d.area)
            .expect("the switch");
        assert_eq!(switch, ui::switch_rect(&rtl, row));
        let text_of = |style: &UiStyle| {
            let (described, _) = describe(WIDTH, 800, |c| {
                ui::Row::switch(title, true).draw(&mut ui::Ui::new(c, &env, style), row);
            });
            described
                .iter()
                .find(|d| d.role == Role::Text && !d.label.is_empty())
                .map(|d| d.area)
                .expect("the title")
        };
        // The title's room is the mirror of the one left to right: it starts at
        // the right margin and the switch is at the other end.
        let ltr = UiStyle::new(&env);
        let text = text_of(&rtl);
        assert_eq!(text, mirrored(text_of(&ltr)));
        assert!(text.x > switch.x, "{text:?} is not beside {switch:?}");
        // A tap on the switch's middle is on the switch.
        let row_hit = ui::Row::switch(title, true);
        let at = Point::new(
            i32::from(switch.x) + i32::from(switch.width) / 2,
            i32::from(switch.y) + i32::from(switch.height) / 2,
        );
        assert_eq!(row_hit.hit(&env, &rtl, row, at), Some(ui::RowPart::Switch));
    }
}

#[test]
fn the_headers_back_arrow_is_at_the_right_and_its_pager_at_the_left() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let paging = Some(Paging::new(1, 3));
    let header = Header::new(&env, &ltr, true);
    let band = header.area();
    let rows = band.y + band.height / 4..band.y + band.height * 3 / 4;
    let drawn = |style: &UiStyle| {
        let mut frame = support::frame(WIDTH, 800);
        Header::new(&env, style, true).draw(
            &mut ui::Ui::new(&mut frame.canvas(), &env, style),
            "",
            None,
            paging,
        );
        ink_columns(&frame, rows.clone())
    };
    // The arrows are at the margins either way round: the back arrow at
    // the one the UI starts from, the next one at the other.
    assert_eq!(drawn(&rtl), drawn(&ltr));
    // Where a tap goes: back at the right, the next page at the left.
    let header = Header::new(&env, &rtl, true);
    let y = i32::from(band.y) + i32::from(band.height) / 2;
    let at = |x: i32| header.tap(&env, &rtl, Point::new(x, y), paging);
    assert_eq!(at(i32::from(WIDTH) - 8), Some(actions::BACK));
    assert_eq!(at(8), Some(actions::NEXT_PAGE));
    let ltr_header = Header::new(&env, &ltr, true);
    let ltr_at = |x: i32| ltr_header.tap(&env, &ltr, Point::new(x, y), paging);
    assert_eq!(ltr_at(8), Some(actions::BACK));
    assert_eq!(ltr_at(i32::from(WIDTH) - 8), Some(actions::NEXT_PAGE));
}

#[test]
fn the_headers_title_and_note_swap_sides() {
    let h = Harness::new(Device::BUTTONS);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let area = |style: &UiStyle| Header::new(&env, style, false).area();
    assert_eq!(area(&ltr), area(&rtl));
    let places = |style: &UiStyle| {
        let (described, mut frame) = describe(WIDTH, 800, |c| {
            Header::new(&env, style, false).draw(
                &mut ui::Ui::new(c, &env, style),
                "Library",
                Some("A-Z"),
                None,
            );
        });
        assert_eq!(drawn_outside(&mut frame, area(style)), None);
        let find = |label: &str| {
            described
                .iter()
                .find(|d| d.label == label && d.role == Role::Text)
                .map(|d| d.area)
                .expect(label)
        };
        (find("Library"), find("A-Z"))
    };
    let ((title, note), (r_title, r_note)) = (places(&ltr), places(&rtl));
    let middle = |r: Rect| i32::from(r.x) * 2 + i32::from(r.width);
    assert!(
        middle(title) < middle(note),
        "the note is at the right: {title:?} {note:?}"
    );
    assert!(
        middle(r_note) < middle(r_title),
        "the note is at the left: {r_title:?} {r_note:?}"
    );
    // The title's room starts at the right margin, as it did at the left one.
    assert_eq!(r_title, mirrored(title));
}
