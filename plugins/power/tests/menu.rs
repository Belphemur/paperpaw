//! The power menu with the real kernel: the power button held opens a
//! small menu of what the device can do (sleep, switch off where it can,
//! restart, lock or unlock touch on a touch device); a short press still
//! sleeps; the menu opens while touch is locked, and its *Unlock touch*
//! unlocks by the buttons.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use paperpaw_core::{Effects, Kernel};
use paperpaw_platform::{ButtonState, InputEvent, Rotation};
use paperpaw_plugin_lock::Lock;
use paperpaw_plugin_lock::touch::LOCKED;
use paperpaw_plugin_power::Power;
use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::semantics::Role;
use paperpaw_sdk::{
    DeviceInfo, DisplayInfo, Id, InputCaps, Instant, MemoryClass, MemoryStorage, PixelFormat,
};

const fn device(touch: bool) -> DeviceInfo {
    DeviceInfo::new(
        "test",
        "Test device",
        DisplayInfo::new(480, 800, 220, PixelFormat::Gray4, Rotation::Deg0, None),
        InputCaps::new(true, touch, false, &[]),
        MemoryClass::Large,
    )
}

struct Test {
    kernel: Kernel,
    now: u64,
}

impl Test {
    fn new(touch: bool, power_off: bool, with_lock: bool) -> Self {
        let mut builder = Kernel::builder(device(touch))
            .storage(MemoryStorage::new())
            .user_config("[core]\nlanguage = en\n")
            .sleep(true)
            .power_off(power_off)
            .plugin(paperpaw_plugin_fonts_default::DefaultFontPacks)
            .plugin(Power);
        if with_lock {
            builder = builder.plugin(Lock);
        }
        let mut kernel = builder.build().unwrap();
        kernel.start(Instant(0));
        Self { kernel, now: 1_000 }
    }

    fn at(&mut self, millis: u64) -> Instant {
        self.now += millis;
        Instant(self.now)
    }

    /// The button held for `held` milliseconds; what the release asked.
    fn button(&mut self, name: &'static str, held: u64) -> Effects {
        let at = self.at(200);
        self.kernel.handle(
            at,
            InputEvent::Button {
                name,
                state: ButtonState::Pressed,
            },
        );
        let at = self.at(held);
        let during = self.kernel.tick(at);
        let released = self.kernel.handle(
            at,
            InputEvent::Button {
                name,
                state: ButtonState::Released,
            },
        );
        // Either may carry the effect, by when the screen settled.
        if released.sleep() || released.restart() || released.power_off() {
            released
        } else {
            during
        }
    }

    fn on_top(&self) -> Option<&str> {
        self.kernel.active_screen().map(Id::as_str)
    }

    /// The texts of the options on the menu, from the top.
    fn options(&mut self) -> Vec<String> {
        self.kernel.render(&mut |_, _| {});
        self.kernel
            .describe()
            .into_iter()
            .filter(|p| matches!(p.role, Role::Row | Role::Button))
            .map(|p| p.label.to_string())
            .collect()
    }

    fn open(&mut self) {
        self.button("power", 1500);
        assert_eq!(self.on_top(), Some("core.choice-dialog"));
    }

    /// Moves to the option `text` and confirms it, by the keys.
    fn choose(&mut self, text: &str) -> Effects {
        let wanted = self.options().iter().position(|o| o == text).unwrap();
        for _ in 0..wanted {
            let at = self.at(100);
            self.kernel.run_action(at, actions::DOWN);
        }
        let at = self.at(100);
        self.kernel.run_action(at, actions::CONFIRM)
    }
}

#[test]
fn what_the_menu_offers_goes_by_the_device() {
    let mut t = Test::new(false, true, true);
    t.open();
    assert_eq!(
        t.options(),
        ["Sleep", "Switch off", "Restart"],
        "buttons only: no touch lock"
    );
    let mut t = Test::new(true, false, true);
    t.open();
    assert_eq!(
        t.options(),
        ["Sleep", "Restart", "Lock touch"],
        "switching off only where PaperPaw does it"
    );
    let mut t = Test::new(true, true, false);
    t.open();
    assert_eq!(
        t.options(),
        ["Sleep", "Switch off", "Restart"],
        "no touch lock plugin, no entry for it"
    );
}

#[test]
fn each_option_does_what_it_says() {
    let mut t = Test::new(true, true, true);
    t.open();
    assert!(t.choose("Sleep").sleep());
    let mut t = Test::new(true, true, true);
    t.open();
    assert!(t.choose("Switch off").power_off());
    let mut t = Test::new(true, true, true);
    t.open();
    assert!(t.choose("Restart").restart());
}

#[test]
fn back_leaves_the_menu_and_does_nothing() {
    let mut t = Test::new(true, true, true);
    t.open();
    let at = t.at(100);
    let effects = t.kernel.run_action(at, actions::BACK);
    assert_ne!(t.on_top(), Some("core.choice-dialog"));
    assert!(!effects.sleep() && !effects.power_off() && !effects.restart());
}

#[test]
fn a_short_press_still_sleeps() {
    let mut t = Test::new(true, true, true);
    let effects = t.button("power", 80);
    assert!(effects.sleep(), "the short press is the sleep it was");
    assert_ne!(t.on_top(), Some("core.choice-dialog"));
}

#[test]
fn the_menu_unlocks_touch_by_the_buttons_while_it_is_locked() {
    let mut t = Test::new(true, true, true);
    t.open();
    t.choose("Lock touch");
    assert!(t.kernel.settings().bool(LOCKED.as_str()));
    // Locked, it still opens, and offers the way out.
    t.open();
    assert_eq!(
        t.options(),
        ["Sleep", "Switch off", "Restart", "Unlock touch"]
    );
    t.choose("Unlock touch");
    assert!(!t.kernel.settings().bool(LOCKED.as_str()));
}
