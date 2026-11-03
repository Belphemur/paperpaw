//! Setting the front light with a slide along the screen's sides (see
//! `screen/light`): a vertical slide in a band shows the light as the
//! finger moves and keeps it when the finger is lifted, the warmth at
//! the left side and the brightness at the right one; nothing of the
//! page is drawn again. The bands take nothing the kernel keeps for
//! itself at the edges (page turns, back, the quick panel): a swipe in
//! from a side turns the page as before, a tap in a band does what a
//! tap there does, a move across in a band turns the page as a swipe
//! there would. Where the setting says 0, the bands are nothing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::*;
use paperpaw_core::{Effects, Kernel};
use paperpaw_platform::{InputEvent, PointerEvent, PointerPhase, PointerTool};
use paperpaw_sdk::builtin::{actions, groups, menus};
use paperpaw_sdk::document::Locator;
use paperpaw_sdk::{
    Id, Instant, Point, SettingExtra, SettingKind, SettingSpec, SettingValue, Storage,
};

const BOOK: &str = "books/numbered.txt";
const BRIGHTNESS_ID: &str = "fakelight.frontlight";
const WARMTH_ID: &str = "fakelight.warmth";

/// The light's settings, as the shells register theirs: slider numbers
/// (a unit, from 0) at the front of the reader's panel, the first the
/// brightness, the second the warmth where the device has it. The
/// warmth's step of 5 is its own sensitivity.
static BRIGHTNESS: SettingExtra = SettingExtra::new()
    .unit("core.unit.percent")
    .step(1)
    .slider();
static WARMTH: SettingExtra = SettingExtra::new()
    .unit("core.unit.percent")
    .step(5)
    .slider();

/// A light of a shell: slider numbers in the reader's panel and the
/// quick panel, brightness first, warmth second where there is one.
struct FakeLight {
    warmth: bool,
}

impl paperpaw_sdk::Plugin for FakeLight {
    fn manifest(&self) -> paperpaw_sdk::Manifest {
        paperpaw_sdk::Manifest::new(
            Id::new_static("fakelight"),
            "Light",
            paperpaw_sdk::Version::new(0, 1, 0),
        )
        .uses(paperpaw_sdk::Uses::NOTHING)
    }

    fn register(&self, r: &mut paperpaw_sdk::Registrar) -> Result<(), paperpaw_sdk::RegisterError> {
        r.add_setting(
            SettingSpec::new(
                Id::new_static(BRIGHTNESS_ID),
                "fakelight.brightness",
                SettingKind::Int { min: 0, max: 100 },
                SettingValue::Int(0),
                groups::DISPLAY,
            )
            .with_extra(&BRIGHTNESS),
        )?;
        let mut items = vec![BRIGHTNESS_ID];
        if self.warmth {
            r.add_setting(
                SettingSpec::new(
                    Id::new_static(WARMTH_ID),
                    "fakelight.warmth",
                    SettingKind::Int { min: 0, max: 100 },
                    SettingValue::Int(50),
                    groups::DISPLAY,
                )
                .with_extra(&WARMTH),
            )?;
            items.push(WARMTH_ID);
        }
        // First in the reader's panel, and in the quick panel, as the
        // shells order theirs.
        for (n, item) in items.into_iter().enumerate() {
            for (menu, order) in [(menus::READER_PANEL, -10), (menus::QUICK, 20)] {
                r.add_menu_entry(
                    paperpaw_sdk::MenuEntry::new(menu, Id::new_static(item))
                        .order(order + n as i16),
                )?;
            }
        }
        Ok(())
    }
}

/// A reader on a kernel with a light registered as the shells' are.
fn light(config: &str, warmth: bool) -> Reading {
    let storage = Shared::default();
    storage.create_dir("books").unwrap();
    let text = numbered_book(120, 12);
    storage.write(BOOK, text.as_bytes()).unwrap();
    let heard = Rc::new(RefCell::new(Heard::default()));
    let kernel = Kernel::builder(TOUCH)
        .storage(storage.clone())
        .user_config(format!("[core]\nlanguage = en\n{config}"))
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(paperpaw_plugin_format_txt::TxtFormat)
        .plugin(paperpaw_plugin_format_epub::EpubFormat)
        .plugin(paperpaw_plugin_reader::Reader)
        .plugin(FakeLight { warmth })
        .plugin(Recorder(heard.clone()))
        .build()
        .unwrap();
    let mut reading = Reading {
        kernel,
        storage,
        heard,
        now: 0,
        book: BOOK.to_string(),
        image: Vec::new(),
    };
    let effects = reading.kernel.start(Instant(0));
    reading.settle(effects);
    reading.open();
    reading
}

impl Reading {
    /// The value kept of a light setting (what a slider's shown value
    /// never touches).
    fn light_kept(&self, id: &str) -> i64 {
        match self.kernel.settings().global(id) {
            Some(SettingValue::Int(value)) => *value,
            _ => 0,
        }
    }

    /// The value shown of a light setting: kept, or a slider's shown
    /// while its finger holds.
    fn light_shown(&self, id: &str) -> i64 {
        self.kernel.settings().int(id)
    }

    /// One sample of a finger, the test's own (id 0).
    fn sample(&mut self, phase: PointerPhase, point: Point) -> Effects {
        self.now += 40;
        self.kernel.handle(
            Instant(self.now),
            InputEvent::Pointer(PointerEvent {
                id: 0,
                phase,
                point,
                tool: PointerTool::Finger,
                pressure: 1000,
            }),
        )
    }

    /// A slide in a band: down, moving along, lifted. The effects of
    /// every sample are gathered: a refresh among them is a frame the
    /// light asked the page for.
    fn slide(&mut self, start: Point, along: &[Point], lifted: bool) -> Vec<Effects> {
        let mut effects = vec![self.sample(PointerPhase::Down, start)];
        for point in along {
            effects.push(self.sample(PointerPhase::Move, *point));
        }
        if lifted {
            let end = along.last().copied().unwrap_or(start);
            effects.push(self.sample(PointerPhase::Up, end));
        }
        effects
    }

    /// No sample of `effects` asked the panel for a frame.
    fn no_frames(effects: &[Effects]) {
        assert!(
            effects.iter().all(|e| e.refresh().is_none()),
            "the page was drawn again for the light"
        );
    }

    /// Where the next page would be, and back (a turn and its turn
    /// back is the page again).
    fn next_page(&mut self) -> Locator {
        self.press(actions::NEXT_PAGE);
        self.shown().at()
    }
}

#[test]
fn a_slide_up_the_right_band_brightens_the_light_live() {
    let mut reading = light("", true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);

    // Down in the right band (a fifth of the screen's width from its
    // right side), a slide up of a quarter of the screen's height (the
    // light's whole range over half of it), lifted.
    let effects = reading.slide(
        Point::new(420, 700),
        &[Point::new(420, 600), Point::new(420, 500)],
        true,
    );

    // Shown while the finger held, kept when it was lifted.
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 50);
    Reading::no_frames(&effects);
    assert_eq!(reading.turned(), 0);
}

#[test]
fn the_light_shows_while_the_finger_holds_and_keeps_when_it_is_lifted() {
    let mut reading = light("", true);
    reading.sample(PointerPhase::Down, Point::new(420, 700));
    reading.sample(PointerPhase::Move, Point::new(420, 500));
    // Shown, not kept: the light is at 50 while the value kept is not.
    assert_eq!(reading.light_shown(BRIGHTNESS_ID), 50);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    reading.sample(PointerPhase::Up, Point::new(420, 500));
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 50);
    assert_eq!(reading.light_shown(BRIGHTNESS_ID), 50);
}

#[test]
fn a_slide_at_the_left_band_warms_the_light_on_its_step() {
    let mut reading = light("", true);
    let effects = reading.slide(Point::new(60, 400), &[Point::new(60, 300)], true);
    // From 50, a quarter of the screen up: +25, on the warmth's step of 5.
    assert_eq!(reading.light_kept(WARMTH_ID), 75);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    Reading::no_frames(&effects);
    assert_eq!(reading.turned(), 0);
}

#[test]
fn the_light_keeps_the_range_of_its_setting_and_a_tap_does_not_move_it() {
    let mut reading = light("", true);
    // Far up past the top of the range: clamped at its setting's top.
    reading.slide(Point::new(420, 700), &[Point::new(420, 100)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 100);
    // All the way down from there: off.
    reading.slide(Point::new(420, 150), &[Point::new(420, 550)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    // A stroke within the dead zone (a tap's wander, some 3 mm): the
    // light is not moved — and the touch is a tap of the zone it fell
    // in (right: the next page), as a tap in a band is.
    let after = reading.next_page();
    reading.press(actions::PREV_PAGE);
    let before = reading.turned();
    reading.slide(Point::new(420, 400), &[Point::new(422, 396)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    assert_eq!(reading.shown().at(), after);
    assert_eq!(reading.turned(), before + 1);
}

#[test]
fn a_finger_taken_away_shows_nothing_and_keeps_nothing() {
    let mut reading = light("", true);
    reading.sample(PointerPhase::Down, Point::new(420, 700));
    reading.sample(PointerPhase::Move, Point::new(420, 500));
    assert_eq!(reading.light_shown(BRIGHTNESS_ID), 50, "shown while held");
    assert_eq!(
        reading.light_kept(BRIGHTNESS_ID),
        0,
        "nothing kept while held"
    );
    reading.sample(PointerPhase::Cancel, Point::new(420, 500));
    // The value shown is gone; what was kept shows again.
    assert_eq!(reading.light_shown(BRIGHTNESS_ID), 0);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    assert_eq!(reading.turned(), 0);
}

#[test]
fn a_move_across_in_a_band_turns_the_page_as_a_swipe_from_the_side_would() {
    let mut reading = light("", true);
    let start = reading.shown().at();
    let on = reading.next_page();
    reading.press(actions::PREV_PAGE);
    assert_eq!(reading.shown().at(), start);
    let before = reading.turned();

    // Across in the right band, inward: the right side's turn.
    reading.slide(Point::new(420, 400), &[Point::new(300, 405)], true);
    assert_eq!(reading.shown().at(), on);

    // Across in the left band, inward: the left side's turn.
    reading.slide(Point::new(60, 400), &[Point::new(200, 395)], true);
    assert_eq!(reading.shown().at(), start);
    assert_eq!(reading.turned(), before + 2);
}

#[test]
fn a_tap_in_a_band_does_what_a_tap_there_does() {
    let mut reading = light("", true);
    let start = reading.shown().at();
    let on = reading.next_page();
    reading.press(actions::PREV_PAGE);
    assert_eq!(reading.shown().at(), start);
    let before = reading.turned();

    // A tap in the right band: its zone (bottom right), the next page,
    // as taps there turned pages before the bands were.
    reading.slide(Point::new(420, 700), &[], true);
    assert_eq!(reading.shown().at(), on);
    // And in the left band: its zone (left), the page before.
    reading.slide(Point::new(60, 400), &[], true);
    assert_eq!(reading.shown().at(), start);
    assert_eq!(reading.turned(), before + 2);
}

#[test]
fn a_held_touch_in_a_band_never_moves_the_light() {
    let mut reading = light("", true);
    // Held well past the long press, in the left band: a long tap of
    // the zone, or a word turned into — never a light.
    reading.sample(PointerPhase::Down, Point::new(60, 400));
    reading.now += 1_500;
    reading.sample(PointerPhase::Up, Point::new(60, 400));
    assert_eq!(reading.light_kept(WARMTH_ID), 50);
    assert_eq!(reading.turned(), 0);
}

#[test]
fn a_long_press_in_a_band_fires_at_its_time_while_the_finger_holds() {
    let mut reading = light("", true);
    // Held still in the left band past the long press: it fires at its
    // time as the kernel's would — a word turned into, the selection's
    // — and the finger can do no more: its move adjust no light, and
    // the taps of the zones turn no page while it is shown.
    reading.sample(PointerPhase::Down, Point::new(60, 400));
    let down = reading.now;
    reading.pass_until(down + 800);
    reading.sample(PointerPhase::Move, Point::new(60, 320));
    reading.sample(PointerPhase::Up, Point::new(60, 320));
    assert_eq!(reading.light_kept(WARMTH_ID), 50);
    let between = reading.turned();
    reading.slide(Point::new(420, 400), &[], true);
    assert_eq!(reading.turned(), between, "the selection keeps the page");
}

#[test]
fn a_double_tap_of_a_band_zone_makes_it_when_one_is_bound() {
    let mut reading = light("[keymap.global]\ndouble-tap.left = core.next-page\n", true);
    // Not on the first page: the single tap turns a page here.
    reading.press(actions::NEXT_PAGE);
    let before = reading.turned();
    // One tap in the left band: it is held for its second, and the
    // single tap of the zone comes only when the time is out.
    reading.sample(PointerPhase::Down, Point::new(60, 400));
    reading.sample(PointerPhase::Up, Point::new(60, 400));
    assert_eq!(reading.turned(), before);
    reading.pass_until(reading.now + 400);
    assert_eq!(reading.turned(), before + 1, "as tap.left is bound there");

    // Two taps within the time: the double is what they make, once.
    reading.sample(PointerPhase::Down, Point::new(60, 400));
    reading.sample(PointerPhase::Up, Point::new(60, 400));
    let between = reading.turned();
    reading.sample(PointerPhase::Down, Point::new(60, 400));
    reading.sample(PointerPhase::Up, Point::new(60, 400));
    reading.pass_until(reading.now + 400);
    assert_eq!(reading.turned(), between + 1, "as double-tap.left is bound");
}

#[test]
fn the_policies_the_kernel_keeps_the_key_map_also_keep_a_band() {
    let mut reading = light(
        "[keymap.global]\ngestures.policy = core.gestures.own\n",
        true,
    );
    let before = reading.turned();
    // The screen's own: the zone bindings are the kernel's, and none
    // of them answers for a tap of a band.
    reading.slide(Point::new(420, 400), &[], true);
    assert_eq!(reading.turned(), before);

    let mut reading = light(
        "[keymap.global]\ngestures.policy = core.gestures.off\n",
        true,
    );
    let before = reading.turned();
    // Gestures off: no edge swipe answers, so a stroke across the band
    // does nothing, where else it would go back.
    reading.slide(Point::new(60, 400), &[Point::new(220, 400)], true);
    assert_eq!(reading.turned(), before);
}

#[test]
fn a_swipe_out_of_a_band_turns_pages_as_a_swipe_along_it_would() {
    let mut reading = light("", true);
    let before = reading.turned();
    // Out of the left band, far enough: a plain swipe (the kernel's
    // edge is gone the other way), which the key map turns pages with.
    reading.slide(Point::new(90, 400), &[Point::new(5, 400)], true);
    assert_eq!(reading.turned(), before + 1, "as swipe.left is bound there");
}

#[test]
fn the_strips_the_kernel_keeps_at_the_edges_stay_its_own() {
    let mut reading = light("", true);
    let start = reading.shown().at();
    reading.next_page();
    let before = reading.turned();
    // Down on the outermost strip at the left (what the kernel keeps
    // for its own side swipes), in: the reader's edges, the page before.
    reading.slide(Point::new(10, 400), &[Point::new(150, 405)], true);
    assert_eq!(reading.shown().at(), start);
    assert_eq!(reading.turned(), before + 1);
}

#[test]
fn at_zero_the_setting_switches_the_bands_off() {
    let mut reading = light("[reader]\nfrontlight-swipe = 0\n", true);
    // A slide up in the right band is no band's now: a plain swipe up,
    // which the global key map turns pages with.
    let on = reading.next_page();
    let after = reading.next_page();
    reading.press(actions::PREV_PAGE);
    let before = reading.turned();
    reading.slide(Point::new(420, 700), &[Point::new(420, 500)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    assert_eq!(reading.shown().at(), after);
    assert_eq!(reading.turned(), before + 1);
    let _ = on;
}

#[test]
fn a_light_without_warmth_leaves_the_left_side_as_it_was() {
    let mut reading = light("", false);
    // A slide up in the left band: no warmth to set, so the finger was
    // never taken; the kernel's recognizer takes it and turns the page
    // with its own swipe.up bound there.
    let before = reading.turned();
    reading.slide(Point::new(60, 700), &[Point::new(60, 500)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    assert_eq!(reading.turned(), before + 1, "as swipe.up is bound there");
    // Nothing of the panel's other sliders came with it (the reader's
    // margins are a zero-minimum slider of the same panel): shown as
    // kept, both what there was.
    assert_eq!(
        reading.light_shown("core.margin"),
        reading.light_kept("core.margin")
    );

    // The right band still works.
    reading.slide(Point::new(420, 700), &[Point::new(420, 500)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 50);
}

#[test]
fn a_second_finger_is_never_taken_for_the_light() {
    let mut reading = light("", true);
    // A finger already held in the right band; another coming down in
    // the left one is not taken (the pinch's), its lift either.
    reading.slide(Point::new(420, 700), &[Point::new(420, 650)], false);
    assert_eq!(reading.light_shown(BRIGHTNESS_ID), 12);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
    assert_eq!(reading.light_kept(WARMTH_ID), 50);
    for phase in [PointerPhase::Down, PointerPhase::Up] {
        reading.now += 40;
        let _ = reading.kernel.handle(
            Instant(reading.now),
            InputEvent::Pointer(PointerEvent {
                id: 1,
                phase,
                point: Point::new(60, 400),
                tool: PointerTool::Finger,
                pressure: 1000,
            }),
        );
    }
    // The first finger's drag is undisturbed, and its value kept only
    // when it is lifted.
    assert_eq!(reading.light_kept(WARMTH_ID), 50);
    reading.sample(PointerPhase::Up, Point::new(420, 500));
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 50);
    assert_eq!(reading.turned(), 0);
}

#[test]
fn a_device_without_touch_never_takes_a_band() {
    // The X4 has no touch screen: the light is set in its panel only.
    let storage = Shared::default();
    storage.create_dir("books").unwrap();
    let text = numbered_book(20, 12);
    storage.write(BOOK, text.as_bytes()).unwrap();
    let heard = Rc::new(RefCell::new(Heard::default()));
    let kernel = Kernel::builder(BUTTONS)
        .storage(storage.clone())
        .user_config(String::from("[core]\nlanguage = en\n"))
        .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
        .plugin(paperpaw_plugin_format_txt::TxtFormat)
        .plugin(paperpaw_plugin_format_epub::EpubFormat)
        .plugin(paperpaw_plugin_reader::Reader)
        .plugin(FakeLight { warmth: true })
        .plugin(Recorder(heard.clone()))
        .build()
        .unwrap();
    let mut reading = Reading {
        kernel,
        storage,
        heard,
        now: 0,
        book: BOOK.to_string(),
        image: Vec::new(),
    };
    let effects = reading.kernel.start(Instant(0));
    reading.settle(effects);
    reading.open();
    reading.slide(Point::new(420, 700), &[Point::new(420, 500)], true);
    assert_eq!(reading.light_kept(BRIGHTNESS_ID), 0);
}
