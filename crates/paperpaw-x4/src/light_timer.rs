//! The X4 Pro's front light goes off after a while without input (a
//! setting, *Light off after*, never by default) and comes back, as it
//! was, with the next press or touch. That press only brings the light
//! back: it is not passed on, so it turns no page and picks no row (a
//! reader who reads in the dark would turn a page by every wake-up
//! otherwise). The press is held back until it is let go. The power
//! button is no input for this: it puts the reader to sleep, and a light
//! that stays off for it is no loss.
//!
//! This is the decision, on a computer-tested state; the firmware
//! (`shells/xteink`) turns the light on and off and asks it what to pass
//! on. Nothing here polls: the main loop comes by anyway, and a light that
//! is off lets the processor sleep lightly between readings of the
//! buttons.

use paperpaw_platform::{ButtonState, InputEvent, PointerPhase, buttons};

/// The reader's choices of *Light off after*.
pub const NEVER: &str = "x4.light-off.never";
pub const S30: &str = "x4.light-off.s30";
pub const M1: &str = "x4.light-off.m1";
pub const M2: &str = "x4.light-off.m2";
pub const M5: &str = "x4.light-off.m5";
/// In the order the settings screen lists them.
pub const CHOICES: [&str; 5] = [NEVER, S30, M1, M2, M5];
/// The choice until the reader makes another: the light stays on.
pub const DEFAULT: &str = NEVER;

/// How long without input the light stays on for `choice` (ms); `None`
/// for *Never* and for a choice not known.
pub fn delay_ms(choice: &str) -> Option<u64> {
    match choice {
        S30 => Some(30_000),
        M1 => Some(60_000),
        M2 => Some(120_000),
        M5 => Some(300_000),
        _ => None,
    }
}

/// The light's state: when the last input was, whether the light is off
/// for want of any, and what was pressed to bring it back (held back
/// until it is let go).
#[derive(Debug, Default)]
pub struct LightTimer {
    last: u64,
    dark: bool,
    /// Buttons whose press brought the light back (bits of [`slot`]).
    held: u16,
    /// A touch brought the light back and is still on the screen.
    touching: bool,
}

impl LightTimer {
    /// A timer for a light that has just come on at `now` (ms).
    pub const fn new(now: u64) -> Self {
        Self {
            last: now,
            dark: false,
            held: 0,
            touching: false,
        }
    }

    /// Whether the light is off at `now` (ms), the choice being
    /// `after_ms` (see [`delay_ms`]). Off once that long passed without
    /// input; on again with the choice *Never*.
    pub fn dark(&mut self, now: u64, after_ms: Option<u64>) -> bool {
        match after_ms {
            None => self.dark = false,
            Some(after) if now.saturating_sub(self.last) >= after => self.dark = true,
            Some(_) => {}
        }
        self.dark
    }

    /// Whether `event`, which came at `now` (ms), goes on to the kernel.
    /// Not the press (nor what follows it until it is let go) that
    /// brought the light back.
    pub fn pass(&mut self, now: u64, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Button { name, state } => self.button(now, name, state),
            InputEvent::Pointer(pointer) => self.touch(now, pointer.phase),
        }
    }

    fn button(&mut self, now: u64, name: &str, state: ButtonState) -> bool {
        // The power button is no sign that the reader reads.
        if name == buttons::POWER {
            return true;
        }
        let bit = slot(name);
        let pressed = state == ButtonState::Pressed;
        if pressed && self.dark {
            self.dark = false;
            self.held |= bit;
        }
        self.last = now;
        if pressed {
            return self.held & bit == 0;
        }
        let held = self.held & bit != 0;
        self.held &= !bit;
        !held
    }

    fn touch(&mut self, now: u64, phase: PointerPhase) -> bool {
        if phase == PointerPhase::Down && self.dark {
            self.dark = false;
            self.touching = true;
        }
        self.last = now;
        let held = self.touching;
        if matches!(phase, PointerPhase::Up | PointerPhase::Cancel) {
            self.touching = false;
        }
        !held
    }
}

/// The buttons told apart: those of the X4s, and the tap on the Pro's
/// home mark (a button to the kernel).
const NAMES: [&str; 8] = [
    buttons::BACK,
    buttons::CONFIRM,
    buttons::LEFT,
    buttons::RIGHT,
    buttons::PAGE_BACK,
    buttons::PAGE_FORWARD,
    buttons::HOME,
    buttons::POWER,
];

/// The bit of a button in `held`; none for a name not known, whose press
/// is then passed on at once (that only costs a turned page, never a
/// stuck button).
fn slot(name: &str) -> u16 {
    NAMES
        .iter()
        .position(|button| *button == name)
        .map_or(0, |n| 1 << n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_platform::{Point, PointerEvent, PointerTool};

    fn press(name: &'static str) -> InputEvent {
        InputEvent::Button {
            name,
            state: ButtonState::Pressed,
        }
    }

    fn release(name: &'static str) -> InputEvent {
        InputEvent::Button {
            name,
            state: ButtonState::Released,
        }
    }

    fn touch(phase: PointerPhase) -> InputEvent {
        InputEvent::Pointer(PointerEvent::new(
            0,
            phase,
            Point::new(10, 10),
            PointerTool::Finger,
            1000,
        ))
    }

    #[test]
    fn the_choices_have_their_delays() {
        assert_eq!(delay_ms(NEVER), None);
        assert_eq!(delay_ms(S30), Some(30_000));
        assert_eq!(delay_ms(M1), Some(60_000));
        assert_eq!(delay_ms(M2), Some(120_000));
        assert_eq!(delay_ms(M5), Some(300_000));
        assert_eq!(delay_ms("something else"), None);
        assert_eq!(delay_ms(DEFAULT), None);
        assert_eq!(CHOICES[0], DEFAULT);
    }

    #[test]
    fn the_light_goes_off_after_the_time_without_input() {
        let mut timer = LightTimer::new(1_000);
        assert!(!timer.dark(30_999, delay_ms(S30)));
        assert!(timer.dark(31_000, delay_ms(S30)));
        // And stays off, with no input.
        assert!(timer.dark(500_000, delay_ms(S30)));
    }

    #[test]
    fn never_keeps_the_light_on_and_passes_everything() {
        let mut timer = LightTimer::new(0);
        assert!(!timer.dark(10_000_000, None));
        assert!(timer.pass(10_000_000, &press(buttons::CONFIRM)));
        assert!(timer.pass(10_000_050, &release(buttons::CONFIRM)));
    }

    #[test]
    fn input_while_the_light_is_on_puts_the_time_off() {
        let mut timer = LightTimer::new(0);
        assert!(timer.pass(20_000, &press(buttons::PAGE_FORWARD)));
        assert!(timer.pass(20_100, &release(buttons::PAGE_FORWARD)));
        assert!(!timer.dark(49_000, delay_ms(S30)));
        assert!(timer.dark(50_100, delay_ms(S30)));
    }

    #[test]
    fn the_press_that_brings_the_light_back_turns_no_page() {
        let mut timer = LightTimer::new(0);
        assert!(timer.dark(60_000, delay_ms(S30)));
        // The press only lights; so does its release.
        assert!(!timer.pass(61_000, &press(buttons::PAGE_FORWARD)));
        assert!(!timer.dark(61_000, delay_ms(S30)));
        assert!(!timer.pass(61_120, &release(buttons::PAGE_FORWARD)));
        // The next press is an ordinary one.
        assert!(timer.pass(62_000, &press(buttons::PAGE_FORWARD)));
        assert!(timer.pass(62_100, &release(buttons::PAGE_FORWARD)));
        // The time counts from the last input.
        assert!(!timer.dark(91_000, delay_ms(S30)));
        assert!(timer.dark(92_100, delay_ms(S30)));
    }

    #[test]
    fn a_button_held_through_the_wake_up_stays_held_back() {
        let mut timer = LightTimer::new(0);
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(!timer.pass(40_000, &press(buttons::CONFIRM)));
        // Another button pressed meanwhile is an ordinary one.
        assert!(timer.pass(40_200, &press(buttons::BACK)));
        assert!(timer.pass(40_300, &release(buttons::BACK)));
        // Only now is the first let go, and still not passed.
        assert!(!timer.pass(41_000, &release(buttons::CONFIRM)));
        assert!(timer.pass(42_000, &press(buttons::CONFIRM)));
    }

    #[test]
    fn a_touch_that_brings_the_light_back_is_held_back_to_its_end() {
        let mut timer = LightTimer::new(0);
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(!timer.pass(40_000, &touch(PointerPhase::Down)));
        assert!(!timer.dark(40_000, delay_ms(S30)));
        assert!(!timer.pass(40_050, &touch(PointerPhase::Move)));
        assert!(!timer.pass(40_100, &touch(PointerPhase::Up)));
        for phase in [PointerPhase::Down, PointerPhase::Move, PointerPhase::Up] {
            assert!(timer.pass(41_000, &touch(phase)));
        }
    }

    #[test]
    fn a_touch_cancelled_ends_the_holding_back() {
        let mut timer = LightTimer::new(0);
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(!timer.pass(40_000, &touch(PointerPhase::Down)));
        assert!(!timer.pass(40_020, &touch(PointerPhase::Cancel)));
        assert!(timer.pass(41_000, &touch(PointerPhase::Down)));
    }

    #[test]
    fn the_power_button_neither_lights_nor_is_held_back() {
        let mut timer = LightTimer::new(0);
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(timer.pass(40_000, &press(buttons::POWER)));
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(timer.pass(40_500, &release(buttons::POWER)));
        assert!(timer.dark(41_000, delay_ms(S30)));
        // With the light on it is no sign of reading either.
        let mut timer = LightTimer::new(0);
        assert!(timer.pass(20_000, &press(buttons::POWER)));
        assert!(timer.dark(30_000, delay_ms(S30)));
    }

    #[test]
    fn a_release_with_no_press_before_is_passed() {
        // A press let go that the timer never saw held back (it came
        // before the light went off): the kernel needs its release.
        let mut timer = LightTimer::new(0);
        assert!(timer.pass(1_000, &press(buttons::LEFT)));
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(timer.pass(40_000, &release(buttons::LEFT)));
        assert!(timer.dark(40_000, delay_ms(S30)));
    }

    #[test]
    fn choosing_never_while_off_brings_the_light_back() {
        let mut timer = LightTimer::new(0);
        assert!(timer.dark(40_000, delay_ms(S30)));
        assert!(!timer.dark(40_000, delay_ms(NEVER)));
        // Chosen again, the time counts from the last input, long ago.
        assert!(timer.dark(40_001, delay_ms(S30)));
    }
}
