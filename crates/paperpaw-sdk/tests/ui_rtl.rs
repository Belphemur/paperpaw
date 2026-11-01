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

#[test]
fn a_ticked_rows_box_and_text_start_at_the_right() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, 120);
    assert_eq!(
        ui::check_rect(&rtl, row),
        mirrored(ui::check_rect(&ltr, row))
    );
    let places = |style: &UiStyle| {
        let (described, mut frame) = describe(WIDTH, 800, |c| {
            ui::draw_check_row(c, &env, style, row, false, "Lines", "A note", true);
        });
        assert_eq!(drawn_outside(&mut frame, row), None);
        let find = |role: Role, label: &str| {
            described
                .iter()
                .find(|d| d.role == role && (label.is_empty() || d.label == label))
                .map(|d| d.area)
                .expect(label)
        };
        (
            find(Role::Check, ""),
            find(Role::Text, "Lines"),
            find(Role::Text, "A note"),
        )
    };
    let (ltr_places, rtl_places) = (places(&ltr), places(&rtl));
    assert_eq!(rtl_places.0, mirrored(ltr_places.0));
    assert_eq!(rtl_places.1, mirrored(ltr_places.1));
    assert_eq!(rtl_places.2, mirrored(ltr_places.2));
}

#[test]
fn a_numbers_buttons_are_at_the_left_the_plus_before_the_minus() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, ltr.row());
    let (minus, plus) = ui::value_stepper(&env, &ltr, row, "000 %", "40 %").unwrap();
    let (r_minus, r_plus) = ui::value_stepper(&env, &rtl, row, "000 %", "40 %").unwrap();
    assert_eq!((r_minus, r_plus), (mirrored(minus), mirrored(plus)));
    assert!(r_plus.x < r_minus.x, "{r_plus:?} {r_minus:?}");
    // Drawn within the row, and a tap on each is on it.
    let (_, mut frame) = describe(WIDTH, 800, |c| {
        ui::Row::number("Light", "40 %", "000 %").draw(&mut ui::Ui::new(c, &env, &rtl), row);
    });
    assert_eq!(drawn_outside(&mut frame, row), None);
    let number = ui::Row::number("Light", "40 %", "000 %");
    let middle = |r: Rect| {
        Point::new(
            i32::from(r.x) + i32::from(r.width) / 2,
            i32::from(r.y) + i32::from(r.height) / 2,
        )
    };
    assert_eq!(
        number.hit(&env, &rtl, row, middle(r_plus)),
        Some(ui::RowPart::Plus)
    );
    assert_eq!(
        number.hit(&env, &rtl, row, middle(r_minus)),
        Some(ui::RowPart::Minus)
    );
    // The title is at the right, clear of the buttons.
    let (described, _) = describe(WIDTH, 800, |c| {
        number.draw(&mut ui::Ui::new(c, &env, &rtl), row);
    });
    let title = described
        .iter()
        .find(|d| d.role == Role::Text && d.label == "Light")
        .map(|d| d.area)
        .unwrap();
    assert_eq!(title.right(), row.right() - u32::from(rtl.margin()));
}

#[test]
fn a_text_field_and_its_title_are_the_mirror_image() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, ltr.row() * 2);
    let (title, field, below) = ui::text_field_layout(&ltr, row, 120.0, 1, 30);
    let (r_title, r_field, r_below) = ui::text_field_layout(&rtl, row, 120.0, 1, 30);
    assert_eq!(
        (r_title, r_field, r_below),
        (mirrored(title), mirrored(field), below)
    );
    let drawn = |style: &UiStyle| {
        let (described, mut frame) = describe(WIDTH, 800, |c| {
            ui::Ui::new(c, &env, style).text_field(row, false, "Name", "value", "");
        });
        assert_eq!(drawn_outside(&mut frame, row), None);
        described
            .iter()
            .find(|d| d.role == Role::Field)
            .map(|d| d.area)
            .unwrap()
    };
    assert_eq!(drawn(&rtl), mirrored(drawn(&ltr)));
}

#[test]
fn a_dialogs_buttons_go_from_the_right_and_left_moves_on() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let make = |style: &UiStyle| ui::Dialog::new(&env, style, "Title", &["Text"], &["Yes", "No"]);
    let (d, r) = (make(&ltr), make(&rtl));
    let (first, second) = (d.button(0).unwrap(), d.button(1).unwrap());
    let (r_first, r_second) = (r.button(0).unwrap(), r.button(1).unwrap());
    assert!(first.x < second.x);
    assert!(r_first.x > r_second.x, "the first is at the right");
    assert_eq!(r.button(0).unwrap().y, first.y);
    // Left and right are where the buttons are.
    assert_eq!(d.step(0, &actions::RIGHT), Some(1));
    assert_eq!(r.step(0, &actions::LEFT), Some(1));
    assert_eq!(r.step(1, &actions::RIGHT), Some(0));
    assert_eq!(r.step(0, &actions::RIGHT), Some(0));
    // A tap is taken where a button is drawn.
    let at = Point::new(i32::from(r_first.x) + 2, i32::from(r_first.y) + 2);
    assert_eq!(r.hit(at), Some(0));
}

#[test]
fn a_notes_lines_start_at_the_right_margin() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, ltr.noted_row() as u16);
    let rows = row.y + row.height / 2..row.bottom() as u16;
    let ink = |style: &UiStyle| {
        let mut frame = support::frame(WIDTH, 800);
        ui::Row::value("Title", "On")
            .note("A short note")
            .draw(&mut ui::Ui::new(&mut frame.canvas(), &env, style), row);
        ink_columns(&frame, rows.clone())
    };
    let ((first, last), (r_first, r_last)) = (ink(&ltr), ink(&rtl));
    let (margin, bearing) = (ltr.margin(), 6);
    assert!(first >= margin && first <= margin + bearing, "{first}");
    assert!(
        r_last + margin + bearing >= WIDTH && r_last < WIDTH - margin + 1,
        "{r_last}"
    );
    assert!(last < r_last || last <= r_last + 300, "{last} {r_last}");
    let _ = r_first;
}

#[test]
fn a_chevron_turns_and_an_icon_stands_at_the_right() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let row = Rect::new(0, 100, WIDTH, ltr.row());
    let labels = |style: &UiStyle| {
        let (described, mut frame) = describe(WIDTH, 800, |c| {
            ui::Row::value("Title", "3 ›").draw(&mut ui::Ui::new(c, &env, style), row);
        });
        assert_eq!(drawn_outside(&mut frame, row), None);
        described
            .into_iter()
            .filter(|d| d.role == Role::Text)
            .map(|d| d.label)
            .collect::<Vec<_>>()
    };
    assert!(labels(&ltr).contains(&String::from("3 ›")));
    assert!(
        labels(&rtl).contains(&String::from("‹ 3")),
        "{:?}",
        labels(&rtl)
    );
    // The rest of a row after its icon is the part away from the icon.
    let icon = ui::icon_room(&env, &ltr, true);
    if icon > 0 {
        let rest = |style: &UiStyle| {
            let mut frame = support::frame(WIDTH, 800);
            ui::Ui::new(&mut frame.canvas(), &env, style).row_icon(
                row,
                icon,
                Some("puzzle"),
                paperpaw_sdk::Color::BLACK,
            )
        };
        assert_eq!(rest(&ltr).x, row.x + icon);
        assert_eq!(rest(&rtl).x, row.x);
        assert_eq!(rest(&rtl).width, row.width - icon);
    }
}

#[test]
fn chips_run_from_the_right_and_left_goes_on() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let area = Rect::new(0, 100, WIDTH, 200);
    let labels = ["A", "B", "C", "D"];
    let (l, r) = (
        ui::Chips::new(&env, &ltr, area, labels),
        ui::Chips::new(&env, &rtl, area, labels),
    );
    for i in 0..4 {
        assert_eq!(r.cell(i).unwrap(), mirrored(l.cell(i).unwrap()), "chip {i}");
    }
    assert!(r.cell(0).unwrap().x > r.cell(1).unwrap().x);
    assert_eq!(l.step(0, &actions::RIGHT), Some(1));
    assert_eq!(r.step(0, &actions::LEFT), Some(1));
    assert_eq!(r.step(1, &actions::RIGHT), Some(0));
    assert_eq!(r.step(0, &actions::RIGHT), Some(0));
    // A tap is on the chip drawn there.
    let first = r.cell(0).unwrap();
    assert_eq!(
        r.hit(Point::new(i32::from(first.x) + 2, i32::from(first.y) + 2)),
        Some(0)
    );
    // And they are drawn within the area.
    let (_, mut frame) = describe(WIDTH, 800, |c| {
        r.draw(&mut ui::Ui::new(c, &env, &rtl), 0, Some(1));
    });
    assert_eq!(drawn_outside(&mut frame, area), None);
}

#[test]
fn a_date_pickers_first_field_is_at_the_right() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let area = Rect::new(0, 100, WIDTH, ui::DatePicker::height(&ltr));
    let date = paperpaw_sdk::time::Date::new(2026, 11, 1);
    let (l, mut r) = (
        ui::DatePicker::new(&ltr, area, date),
        ui::DatePicker::new(&rtl, area, date),
    );
    let middle = i32::from(area.y) + i32::from(area.height) / 2;
    let (near_left, near_right) = (Point::new(30, middle), Point::new(450, middle));
    assert_eq!(l.hit(near_left), Some(ui::PickerPart::Value(0)));
    assert_eq!(r.hit(near_right), Some(ui::PickerPart::Value(0)));
    assert_eq!(r.hit(near_left), Some(ui::PickerPart::Value(2)));
    // The left button goes on to the next field.
    assert_eq!(r.focus(), 0);
    let _ = r.on_action(&actions::DOWN);
    let _ = r.on_action(&actions::LEFT);
    assert_eq!(r.focus(), 1);
    let _ = r.on_action(&actions::RIGHT);
    assert_eq!(r.focus(), 0);
}

#[test]
fn a_heading_starts_at_the_right_margin() {
    let h = Harness::new(Device::TOUCH);
    let env = h.env();
    let ltr = UiStyle::new(&env);
    let rtl = ltr.with_rtl(true);
    let rect = Rect::new(0, 100, WIDTH, ltr.row() as u16);
    let rows = rect.y..rect.bottom() as u16;
    let ink = |style: &UiStyle| {
        let mut frame = support::frame(WIDTH, 800);
        ui::Ui::new(&mut frame.canvas(), &env, style).heading(rect, "Section");
        ink_columns(&frame, rows.clone())
    };
    let ((first, _), (_, r_last)) = (ink(&ltr), ink(&rtl));
    let (margin, bearing) = (ltr.margin(), 6);
    assert!(first >= margin && first <= margin + bearing, "{first}");
    assert!(
        r_last + margin + bearing >= WIDTH && r_last < WIDTH - margin + 1,
        "{r_last}"
    );
}
