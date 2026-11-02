//! The calls of the ui family that need the device's state (`Env`: its
//! fonts, translations and size), for `tools/table-compat.sh`: run in the
//! plugin, through the table or with its own copies, and in the test with
//! this tree's bodies (`../../native_plugin.rs`). Every entry of the ui
//! family is reached, left to right and mirrored, and the text the
//! interface draws right to left: `fit` and a handle's `measure` of Arabic
//! and Hebrew with the right-to-left ink.
//!
//! `table-compat.sh` builds this file of the tree it checks against the
//! baseline's crates: it calls only what the baseline already has (the
//! right-to-left flag as its bit, `TextInk::with_flags(1)`). Included by
//! both with `#[path]`.

#![allow(
    clippy::unwrap_used,
    reason = "a test's probe: the canvas is the test's own"
)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_graphics::Framebuffer;
use paperpaw_platform::{Color, PixelFormat, Point, Rect};
use paperpaw_sdk::Env;
use paperpaw_sdk::ui::{
    Header, Label, Paging, Row, RowKind, TextRole, Ui, UiStyle, button_height_for, draw_arrow,
    draw_icon, draw_icon_at, draw_label, draw_label_in, empty_height, fit, fit_pair,
    heading_height, icon_room, icon_width, message_height, note_line_height, tr_input,
};

/// Its frame's height.
const HEIGHT: u16 = 160;

/// Arabic (joined letters, a word break), Hebrew with a bracket and a
/// number, and Latin: what `fit` shortens and a handle measures.
const TEXTS: [&str; 3] = [
    "\u{645}\u{631}\u{62d}\u{628}\u{627} \u{628}\u{627}\u{644}\u{639}\u{627}\u{644}\u{645}",
    "\u{5e9}\u{5dc}\u{5d5}\u{5dd} (12) \u{5e2}\u{5d5}\u{5dc}\u{5dd}",
    "AAA AA AAAA",
];

/// What the calls came to: sizes, hits, taps and texts as written, and
/// the ink each drawing left.
#[derive(Debug, PartialEq)]
pub struct UiCalls {
    pub numbers: Vec<String>,
    pub gray: Vec<u64>,
}

/// The plugin's side, a service: its own code (and so its copies of the
/// SDK, or its table's stubs) run with the kernel's `Env`.
#[repr(C)]
pub struct UiProbe {
    pub run: fn(&Env<'_>) -> UiCalls,
}

paperpaw_sdk::service_type!(UiProbe, "hello.ui-probe/1");

/// The plugin's [`UiProbe`] (the test runs [`ui_calls`] itself).
#[allow(dead_code, reason = "the plugin's; the test calls ui_calls")]
pub const UI_PROBE: UiProbe = UiProbe { run: ui_calls };

/// The sum of a frame's gray levels.
fn gray_of(frame: &Framebuffer) -> u64 {
    let view = frame.view();
    let mut gray = 0u64;
    for y in 0..frame.height() {
        for x in 0..frame.width() {
            gray += u64::from(view.pixel(x, y).r);
        }
    }
    gray
}

/// `draw` on a cleared frame; the ink it left.
fn drawn(frame: &mut Framebuffer, draw: impl FnOnce(&mut paperpaw_graphics::Canvas<'_>)) -> u64 {
    {
        let mut canvas = frame.canvas();
        canvas.clear(Color::WHITE);
        draw(&mut canvas);
    }
    gray_of(frame)
}

pub fn ui_calls(env: &Env<'_>) -> UiCalls {
    let width = env.device().display.width.min(480);
    let mut frame = Framebuffer::new(width, HEIGHT, PixelFormat::Gray8).unwrap();
    let mut calls = UiCalls {
        numbers: Vec::new(),
        gray: Vec::new(),
    };
    for rtl in [false, true] {
        let style = UiStyle::new(env).with_rtl(rtl);
        calls.numbers.push(format!("{}", style.noted_row()));
        rows(env, &style, &mut frame, &mut calls);
        header(env, &style, &mut frame, &mut calls);
        parts(env, &style, &mut frame, &mut calls);
        texts(env, &style, &mut frame, &mut calls);
    }
    calls
}

/// Rows of every kind (the setting rows' title lines, value and note),
/// their heights, drawings and hits.
fn rows(env: &Env<'_>, style: &UiStyle, frame: &mut Framebuffer, calls: &mut UiCalls) {
    let width = frame.width();
    let rows = [
        Row::plain("AAAA"),
        Row::value("AAA", "AA"),
        Row::value("AAA AAAA AA AAA", "AAAA").note("AA AAA A"),
        Row::switch("AA A", true).focused(true),
        Row::number("A", "AA", "AAAA").step_focus(Some(true)),
        Row::of("AA", RowKind::Value("A")).note("A"),
    ];
    for row in &rows {
        let height = row.height(env, style, width);
        let rect = Rect::new(0, 0, width, height.min(HEIGHT));
        calls.gray.push(drawn(frame, |canvas| {
            row.draw(&mut Ui::new(canvas, env, style), rect)
        }));
        calls.numbers.push(format!("{height}"));
        for x in [8, width / 2, width - 40, width - 8] {
            let at = Point::new(i32::from(x), i32::from(rect.height / 2));
            calls
                .numbers
                .push(format!("{:?}", row.hit(env, style, rect, at)));
        }
    }
}

/// The header (its arrows), drawn and tapped.
fn header(env: &Env<'_>, style: &UiStyle, frame: &mut Framebuffer, calls: &mut UiCalls) {
    let width = frame.width();
    let header = Header::new(env, style, true);
    let page = Some(Paging::new(1, 3));
    calls.gray.push(drawn(frame, |canvas| {
        let mut ui = Ui::new(canvas, env, style);
        header.draw(&mut ui, "AAA", Some("A"), page);
    }));
    for x in [4, width / 2, width - 60, width - 4] {
        let at = Point::new(i32::from(x), 10);
        calls
            .numbers
            .push(format!("{:?}", header.tap(env, style, at, page)));
    }
    calls.gray.push(drawn(frame, |canvas| {
        draw_arrow(canvas, env, style, (20, 20), "back", true, Color::BLACK);
        draw_arrow(canvas, env, style, (60, 20), "", false, Color::BLACK);
    }));
}

/// What `Ui` draws: a heading, a row's icon, a separator, the focus, a
/// message, a note line, an empty list, buttons, a label.
fn parts(env: &Env<'_>, style: &UiStyle, frame: &mut Framebuffer, calls: &mut UiCalls) {
    let width = frame.width();
    let mut room = Rect::new(0, 0, 0, 0);
    calls.gray.push(drawn(frame, |canvas| {
        let mut ui = Ui::new(canvas, env, style);
        ui.heading(Rect::new(0, 0, width, 40), "AA AAA");
        room = ui.row_icon(
            Rect::new(0, 40, width, 40),
            40,
            Some("folder"),
            Color::BLACK,
        );
        ui.separator(Rect::new(0, 80, width, 40), 16);
        ui.focus(Rect::new(8, 120, width / 2, 36));
    }));
    calls.numbers.push(format!("{room:?}"));
    calls.gray.push(drawn(frame, |canvas| {
        let mut ui = Ui::new(canvas, env, style);
        ui.message(Rect::new(0, 0, width, 50), "AAA AA AAAA AAA AA");
        ui.note_line(Rect::new(0, 50, width, 30), "AA AAA");
        ui.empty(Rect::new(0, 80, width, 80), "folder", "AAA", "AA AAA");
    }));
    calls.gray.push(drawn(frame, |canvas| {
        let mut ui = Ui::new(canvas, env, style);
        ui.button(Rect::new(0, 0, width / 2, 40), "AA", true, true);
        ui.filled_button(Rect::new(0, 40, width / 2, 40), "AAA", false);
        let ink = ui.icon_button(Rect::new(0, 80, 40, 40), "folder", false, true);
        ui.icon("folder", 20.0, 50.0, Rect::new(40, 80, 60, 40), ink);
        let label = Label::new("AAA AA AAAA AAA").wrap();
        let height = label.height(env, style, width / 2);
        let end = label.draw_from(&mut ui, 4.0, Rect::new(width / 2, 0, width / 2, height));
        ui.canvas().fill_rect(0, 150, end as u32, 4, Color::BLACK);
    }));
}

/// The text helpers, with the interface's font as it is and inked right to
/// left.
fn texts(env: &Env<'_>, style: &UiStyle, frame: &mut Framebuffer, calls: &mut UiCalls) {
    let width = frame.width();
    let numbers = &mut calls.numbers;
    numbers.push(format!("{}", heading_height(env, style)));
    numbers.push(format!("{}", icon_room(env, style, true)));
    numbers.push(format!("{}", icon_width(env, "folder", 20.0)));
    numbers.push(String::from(tr_input(env, "probe.key")));
    numbers.push(format!(
        "{}",
        message_height(env, style, width, "AAA AA AAAA AAA AA")
    ));
    numbers.push(format!("{}", note_line_height(env, style, width, "AA AAA")));
    numbers.push(format!(
        "{}",
        empty_height(env, style, width, "AAA", "AA AAA")
    ));
    numbers.push(format!(
        "{}",
        button_height_for(env, style, width / 3, "AA AAA AAAA")
    ));
    let Some(font) = style.font(env) else {
        numbers.push(String::from("no font"));
        return;
    };
    // The right-to-left bit of the ink (`TextInk::RIGHT_TO_LEFT`).
    let rtl = font.inked(paperpaw_text::TextInk::DEFAULT.with_flags(1));
    for handle in [&font, &rtl] {
        for text in TEXTS {
            numbers.push(format!("{}", handle.measure(16.0, text)));
            numbers.push(fit(handle, 16.0, text, 40.0));
            numbers.push(format!(
                "{:?}",
                fit_pair(handle, 16.0, text, "AA", 90.0, 4.0)
            ));
        }
        let gray = drawn(frame, |canvas| {
            for (line, text) in TEXTS.into_iter().enumerate() {
                let area = Rect::new(0, 40 * line as u16, width, 40);
                let end = draw_label(canvas, handle, 16.0, 4.0, area, text, Color::BLACK);
                let role = TextRole::BODY;
                let area = Rect::new(0, 40 * line as u16 + 20, width, 20);
                let x = draw_label_in(
                    canvas,
                    handle,
                    style,
                    role,
                    14.0,
                    end,
                    area,
                    text,
                    Color::BLACK,
                );
                canvas.fill_rect(0, 150, x as u32, 2, Color::BLACK);
            }
            draw_icon(
                canvas,
                env,
                "folder",
                20.0,
                2.0,
                Rect::new(0, 120, 40, 30),
                Color::BLACK,
            );
            draw_icon_at(canvas, env, "folder", 20.0, (60, 130), Color::BLACK);
        });
        calls.gray.push(gray);
    }
}
