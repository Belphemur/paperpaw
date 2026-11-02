//! Text of several lines on the keyboard (`ask::text(..).multiline()`), with
//! the real kernel: line breaks come back as typed, and a plain question
//! never gets any.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use std::cell::RefCell;
use std::rc::Rc;

use paperpaw_core::Kernel;
use paperpaw_platform::Rotation;
use paperpaw_plugin_keyboard::Keyboard;
use paperpaw_sdk::ask;
use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::event::{Event, EventData};
use paperpaw_sdk::replies::Typed;
use paperpaw_sdk::{
    ActionSpec, Commands, DeviceInfo, DisplayInfo, Env, Id, InputCaps, Instant, Manifest,
    MemoryClass, MemoryStorage, Params, PixelFormat, Plugin, RegisterError, Registrar, Version,
};

const DEVICE: DeviceInfo = DeviceInfo::new(
    "test",
    "Test device",
    DisplayInfo::new(480, 800, 220, PixelFormat::Gray4, Rotation::Deg0, None),
    InputCaps::new(true, true, false, &[]),
    MemoryClass::Large,
);

const ASK: Id = Id::new_static("asker.ask");
const TYPED: Event<Typed> = Event::new("asker.typed");

/// Asks for text and keeps what comes back.
struct Asker(Rc<RefCell<Vec<String>>>, bool);

impl Plugin for Asker {
    fn manifest(&self) -> Manifest {
        Manifest::new(Id::new_static("asker"), "Asker", Version::new(0, 1, 0))
            .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        let multiline = self.1;
        r.add_action(ActionSpec::new(ASK, "asker.ask").handler(
            move |_: &Env<'_>, _, out: &mut Commands| {
                let question = ask::text("Message", &TYPED);
                out.push(
                    if multiline {
                        question.multiline()
                    } else {
                        question
                    }
                    .command(),
                );
            },
        ))?;
        let got = self.0.clone();
        r.on(TYPED, move |_, typed, _| {
            got.borrow_mut().push(typed.text.clone());
        })
    }
}

fn typed(text: &str, multiline: bool) -> String {
    let got = Rc::new(RefCell::new(Vec::new()));
    let mut kernel = Kernel::builder(DEVICE)
        .storage(MemoryStorage::new())
        .plugin(Keyboard)
        .plugin(Asker(got.clone(), multiline))
        .build()
        .unwrap();
    kernel.start(Instant(0));
    kernel.run_action(Instant(100), ASK);
    // Typed on a phone (or pasted): it takes the field's place.
    let text = Typed::new(text, Params::new());
    kernel.send_event(
        Instant(200),
        Id::new_static("keyboard.remote-text"),
        text.to_params(),
    );
    // Done (a long press on confirm).
    kernel.run_action(Instant(300), actions::MENU);
    let got = got.borrow();
    assert_eq!(got.len(), 1, "one answer");
    got[0].clone()
}

#[test]
fn lines_come_back_as_typed() {
    assert_eq!(
        typed("Hi Ada,\n\nSee you.\r\nJane", true),
        "Hi Ada,\n\nSee you.\nJane"
    );
}

#[test]
fn a_plain_question_gets_one_line() {
    assert_eq!(typed("Hi Ada,\n\nJane", false), "Hi Ada,  Jane");
}

#[test]
fn more_than_a_line_of_text_may_be_typed() {
    assert_eq!(typed(&"line\n".repeat(1000), true).len(), 4999);
    // A plain question stops at its own limit.
    assert_eq!(typed(&"x".repeat(5000), false).len(), 2000);
    assert_eq!(typed(&"x".repeat(7000), true).len(), 6000);
}

/// The keyboard opened in `mode` with `text` in the field, on a device
/// with fonts: the field's part, and the lines it shows (its label is
/// them, joined by a comma, so the text has no commas).
fn field_parts(mode: &str, text: &str) -> (paperpaw_sdk::semantics::Part, Vec<String>) {
    use paperpaw_sdk::builtin::{params, slots};
    use paperpaw_sdk::semantics::Role;
    let mut kernel = Kernel::builder(DEVICE)
        .storage(MemoryStorage::new())
        .user_config("[core]\nlanguage = en\n")
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(Keyboard)
        .build()
        .unwrap();
    kernel.start(Instant(0));
    let open = Params::new()
        .with(params::TITLE, "Message")
        .with(params::REPLY, "asker.typed")
        .with(params::MODE, mode)
        .with(params::TEXT, text);
    kernel.open_slot(Instant(100), slots::KEYBOARD, open);
    kernel.render(&mut |_, _| {});
    let field = kernel
        .describe()
        .into_iter()
        .find(|p| p.role == Role::Field)
        .expect("the field is drawn");
    let lines = field.label.split(", ").map(str::to_string).collect();
    (field, lines)
}

#[test]
fn the_field_shows_the_lines_as_typed() {
    let (_, lines) = field_parts("multiline", "Hi Ada\n\nSee you.\nJane");
    assert_eq!(lines, ["Hi Ada", "See you.", "Jane"]);
}

#[test]
fn a_long_line_wraps_in_the_field() {
    let text = "The quick brown fox jumps over the lazy dog. ".repeat(3);
    let (field, lines) = field_parts("multiline", text.trim());
    assert!(lines.len() >= 3, "wrapped on lines: {lines:?}");
    assert_eq!(lines.join(" "), text.trim());
    let (single, _) = field_parts("text", "x");
    assert!(
        field.area.height > 3 * single.area.height,
        "the field of several lines is taller: {} against {}",
        field.area.height,
        single.area.height
    );
}

#[test]
fn the_field_scrolls_to_the_end_of_the_text() {
    let text: String = (0..200).map(|n| format!("line {n}\n")).collect();
    let (_, lines) = field_parts("multiline", &text);
    assert!(lines.len() >= 3 && lines.len() < 20, "{lines:?}");
    assert_eq!(lines.last().map(String::as_str), Some("line 199"));
    assert_ne!(lines.first().map(String::as_str), Some("line 0"));
}

#[test]
fn a_plain_field_stays_one_line() {
    let (field, lines) = field_parts("text", "Hi Ada Jane");
    assert_eq!(lines, ["Hi Ada Jane"]);
    assert!(field.area.height < 120, "{}", field.area.height);
}

/// A kernel with the keyboard open in `mode` on text `text`.
fn open_keyboard(mode: &str, text: &str) -> Kernel {
    open_keyboard_on(DEVICE, mode, text)
}

fn open_keyboard_on(device: DeviceInfo, mode: &str, text: &str) -> Kernel {
    use paperpaw_sdk::builtin::{params, slots};
    let mut kernel = Kernel::builder(device)
        .storage(MemoryStorage::new())
        .user_config("[core]\nlanguage = en\n")
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(Keyboard)
        .build()
        .unwrap();
    kernel.start(Instant(0));
    let open = Params::new()
        .with(params::TITLE, "Message")
        .with(params::REPLY, "asker.typed")
        .with(params::MODE, mode)
        .with(params::TEXT, text);
    kernel.open_slot(Instant(100), slots::KEYBOARD, open);
    kernel.render(&mut |_, _| {});
    kernel
}

#[test]
fn the_panel_leaves_the_status_line_free_on_every_screen() {
    use paperpaw_platform::{DisplayInfo, PixelFormat, Rotation};
    let long = "line\n".repeat(100);
    let field_top = |device: DeviceInfo, mode: &str| {
        open_keyboard_on(device, mode, &long)
            .describe()
            .into_iter()
            .find(|p| p.role == paperpaw_sdk::semantics::Role::Field)
            .unwrap()
            .area
            .y
    };
    // Turned on its side the plain field's panel fills the screen from the
    // top; the panel of several lines starts under the status line.
    let wide = DEVICE.with_display(DisplayInfo::new(
        800,
        480,
        220,
        PixelFormat::Gray4,
        Rotation::Deg0,
        None,
    ));
    assert!(field_top(wide.clone(), "multiline") > field_top(wide, "text"));
    // Upright, the lines of the field stop short of the top.
    assert!(field_top(DEVICE, "multiline") >= 60);
}

#[test]
fn the_line_break_key_adds_a_line_break() {
    use paperpaw_platform::{InputEvent, PointerEvent, PointerPhase, PointerTool};
    use paperpaw_sdk::semantics::Role;
    let got = Rc::new(RefCell::new(Vec::new()));
    let mut kernel = Kernel::builder(DEVICE)
        .storage(MemoryStorage::new())
        .user_config("[core]\nlanguage = en\n")
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(Keyboard)
        .plugin(Asker(got.clone(), true))
        .build()
        .unwrap();
    kernel.start(Instant(0));
    kernel.run_action(Instant(100), ASK);
    let text = Typed::new("Hi", Params::new());
    kernel.send_event(
        Instant(150),
        Id::new_static("keyboard.remote-text"),
        text.to_params(),
    );
    let mut now = 200;
    let mut tap = |kernel: &mut Kernel, label: &str| {
        kernel.render(&mut |_, _| {});
        let key = kernel
            .describe()
            .into_iter()
            .find(|p| p.role == Role::Button && p.label == label)
            .unwrap_or_else(|| panic!("no key {label:?}"));
        let at = paperpaw_sdk::Point::new(
            i32::from(key.area.x) + i32::from(key.area.width) / 2,
            i32::from(key.area.y) + i32::from(key.area.height) / 2,
        );
        for phase in [PointerPhase::Down, PointerPhase::Up] {
            now += 50;
            let event =
                InputEvent::Pointer(PointerEvent::new(0, phase, at, PointerTool::Finger, 1000));
            kernel.handle(Instant(now), event);
        }
    };
    tap(&mut kernel, "New line");
    tap(&mut kernel, "z");
    tap(&mut kernel, "OK");
    assert_eq!(got.borrow().as_slice(), ["Hi\nz"]);
}
