//! The X4's buttons: six on two resistor ladders, each read as one
//! voltage on an ADC pin, and the power button on its own pin. The X4 Pro
//! and the X4 Classic have each button on a pin of its own
//! ([`State::pins`], [`State::keys`]).
//!
//! The front buttons (back, confirm, left, right) share one ladder, the
//! two on the side (PaperPaw's page buttons) the other. Each pressed
//! button pulls its pin to its own level; nothing pressed leaves it near
//! the top. The levels (12-bit readings, the pin attenuated to read up to
//! about 3.1 V) were measured on several devices; a reading counts for
//! the button whose level is nearest.

use paperpaw_platform::buttons;

/// The front ladder's buttons, from the highest level down.
const FRONT: [&str; 4] = [
    buttons::BACK,
    buttons::CONFIRM,
    buttons::LEFT,
    buttons::RIGHT,
];
/// Where one button's range ends and the next one's starts; above the
/// first, nothing is pressed.
const FRONT_LIMITS: [u16; 5] = [3900, 3100, 2090, 750, 0];
/// The side ladder's: the upper button turns back, the lower forward.
const SIDE: [&str; 2] = [buttons::PAGE_BACK, buttons::PAGE_FORWARD];
const SIDE_LIMITS: [u16; 3] = [3900, 1120, 0];

/// The front buttons, left to right as they lie below the screen.
pub const ROW: [&str; 4] = FRONT;

/// Every button, in the order of [`State`]'s bits.
pub const ALL: [&str; 7] = [
    buttons::BACK,
    buttons::CONFIRM,
    buttons::LEFT,
    buttons::RIGHT,
    buttons::PAGE_BACK,
    buttons::PAGE_FORWARD,
    buttons::POWER,
];

/// A reading must hold this long to count: three readings 5 ms apart.
pub const DEBOUNCE_MS: u64 = 15;

/// The longest the processor sleeps between readings of the buttons when
/// the power level asks to save (Saving and Offline, ADR 0025): a press
/// lasts 80 ms and more, so a reading every 60 ms still sees each one.
pub const SAVING_READ_MS: u64 = 60;

/// How long the processor sleeps between readings of the buttons: the
/// reader's own choice (`response_ms`), and not less than
/// [`SAVING_READ_MS`] where the power level saves.
pub const fn read_interval_ms(response_ms: u64, saving: bool) -> u64 {
    if saving && response_ms < SAVING_READ_MS {
        SAVING_READ_MS
    } else {
        response_ms
    }
}

/// The buttons of each ladder, as bits of [`State`]: one ladder reads one
/// button at a time.
const LADDERS: [u8; 2] = [0b0000_1111, 0b0011_0000];

/// Which buttons are down: a bit each, as in [`ALL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct State(pub u8);

impl State {
    /// The buttons down by the two ladders' readings and the power pin.
    pub fn read(front: u16, side: u16, power_low: bool) -> Self {
        let mut bits = 0u8;
        if let Some(i) = ladder(front, &FRONT_LIMITS) {
            bits |= 1 << i;
        }
        if let Some(i) = ladder(side, &SIDE_LIMITS) {
            bits |= 1 << (FRONT.len() + i);
        }
        if power_low {
            bits |= 1 << (FRONT.len() + SIDE.len());
        }
        Self(bits)
    }

    /// The buttons down on a device with the page buttons and the power
    /// button on pins of their own (the X4 Pro).
    pub fn pins(page_back: bool, page_forward: bool, power: bool) -> Self {
        let side = FRONT.len();
        Self(
            u8::from(page_back) << side
                | u8::from(page_forward) << (side + 1)
                | u8::from(power) << (side + SIDE.len()),
        )
    }

    /// The buttons down on a device with every button on a pin of its own
    /// (the X4 Classic): whether each is down, in the order of [`ALL`].
    pub fn keys(down: [bool; ALL.len()]) -> Self {
        Self(
            down.iter()
                .enumerate()
                .fold(0, |bits, (i, &d)| bits | u8::from(d) << i),
        )
    }

    pub fn is_down(self, button: &str) -> bool {
        ALL.iter()
            .position(|b| *b == button)
            .is_some_and(|i| self.0 & (1 << i) != 0)
    }
}

/// The button of a ladder whose range holds `reading`.
fn ladder(reading: u16, limits: &[u16]) -> Option<usize> {
    limits
        .windows(2)
        .position(|range| reading <= range[0] && reading > range[1])
        .or_else(|| (reading == 0 && limits.len() > 1).then_some(limits.len() - 2))
}

/// Turns readings into presses and releases, once a reading has held for
/// [`DEBOUNCE_MS`].
///
/// A ladder goes from one button to another only through nothing pressed:
/// let go, its voltage passes the levels of the buttons between, and the
/// lower page button let go read as the upper one pressed (a page turned
/// back by itself).
#[derive(Debug, Default)]
pub struct Debouncer {
    /// What counts as down.
    state: State,
    /// The last reading, and since when it holds.
    reading: State,
    since: u64,
}

impl Debouncer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Takes a reading at time `now` (ms); `changed` gets each button that
    /// went down (`true`) or up (`false`), and when: when the reading that
    /// counts now was first seen, not when it had held long enough, so
    /// that how long a press lasted is as it was (a press is long from
    /// some 500 ms on).
    pub fn update(
        &mut self,
        reading: State,
        now: u64,
        changed: &mut dyn FnMut(&'static str, bool, u64),
    ) {
        if reading != self.reading {
            self.reading = reading;
            self.since = now;
            return;
        }
        if now.saturating_sub(self.since) < DEBOUNCE_MS {
            return;
        }
        let before = self.state.0;
        let mut after = reading.0;
        for ladder in LADDERS {
            let (was, is) = (before & ladder, after & ladder);
            if was != 0 && is != 0 && was != is {
                // Another button of the same ladder: the one down stays.
                after = (after & !ladder) | was;
            }
        }
        if after == before {
            return;
        }
        self.state = State(after);
        for (i, name) in ALL.iter().enumerate() {
            let bit = 1 << i;
            if before & bit != after & bit {
                changed(name, after & bit != 0, self.since);
            }
        }
    }

    /// Whether `button` counts as down.
    pub fn is_down(&self, button: &str) -> bool {
        self.state.is_down(button)
    }

    /// Whether a press or a release is under way: the last reading is not
    /// what counts as down yet (it has to hold first). The reader waits
    /// for it: read again soon, not after a long sleep.
    pub fn settling(&self) -> bool {
        self.reading != self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn a_press_is_settling_until_it_counts() {
        let mut debouncer = Debouncer::new();
        let mut changes = Vec::new();
        let down = State::read(0, 4095, false);
        let up = State::read(4095, 4095, false);
        assert!(!debouncer.settling());
        debouncer.update(down, 0, &mut |b, d, _| changes.push((b, d)));
        assert!(debouncer.settling(), "seen once, not counted yet");
        debouncer.update(down, DEBOUNCE_MS, &mut |b, d, _| changes.push((b, d)));
        assert!(!debouncer.settling());
        assert_eq!(changes, [(buttons::RIGHT, true)]);
        debouncer.update(up, 100, &mut |b, d, _| changes.push((b, d)));
        assert!(debouncer.settling(), "a release too");
    }

    #[test]
    fn presses_and_releases_are_timed_when_first_seen() {
        let mut debouncer = Debouncer::new();
        let mut changes = Vec::new();
        let right = State::read(0, 4095, false);
        // Read every 30 ms, as between light sleeps: down at 60, up at 150.
        for (now, reading) in [
            (0, State(0)),
            (30, State(0)),
            (60, right),
            (90, right),
            (120, right),
        ]
        .into_iter()
        .chain([(150, State(0)), (180, State(0)), (210, State(0))])
        {
            debouncer.update(reading, now, &mut |b, d, at| changes.push((b, d, at)));
        }
        // Held 90 ms, not 120: no long press made of a click.
        assert_eq!(
            changes,
            [(buttons::RIGHT, true, 60), (buttons::RIGHT, false, 150)]
        );
    }

    #[test]
    fn readings_name_buttons() {
        // Levels measured on devices.
        let front = |r| State::read(r, 4095, false);
        assert!(front(3512).is_down(buttons::BACK));
        assert!(front(2694).is_down(buttons::CONFIRM));
        assert!(front(1493).is_down(buttons::LEFT));
        assert!(front(5).is_down(buttons::RIGHT));
        assert!(front(0).is_down(buttons::RIGHT));
        assert_eq!(front(4095), State(0));
        let side = |r| State::read(4095, r, false);
        assert!(side(2242).is_down(buttons::PAGE_BACK));
        assert!(side(3).is_down(buttons::PAGE_FORWARD));
        assert!(State::read(4095, 4095, true).is_down(buttons::POWER));
        // One of each ladder at once.
        let both = State::read(2694, 3, false);
        assert!(both.is_down(buttons::CONFIRM) && both.is_down(buttons::PAGE_FORWARD));
    }

    #[test]
    fn keys_on_their_own_pins_name_buttons() {
        for (i, name) in ALL.into_iter().enumerate() {
            let mut down = [false; ALL.len()];
            down[i] = true;
            let state = State::keys(down);
            assert!(ALL.iter().all(|b| state.is_down(b) == (*b == name)));
        }
        assert_eq!(State::keys([false; ALL.len()]), State(0));
        assert_eq!(
            State::keys([false, false, false, false, true, false, true]),
            State::pins(true, false, true)
        );
    }

    #[test]
    fn letting_go_passes_no_other_button() {
        let mut debouncer = Debouncer::new();
        let mut events = Vec::new();
        let forward = State::read(4095, 3, false);
        let back = State::read(4095, 2242, false);
        let nothing = State(0);
        // Every 5 ms, as the X4 reads: forward held, let go through the
        // level of back (two readings), then nothing.
        let readings = [
            forward, forward, forward, forward, back, back, back, back, nothing, nothing, nothing,
            nothing,
        ];
        for (i, reading) in readings.into_iter().enumerate() {
            debouncer.update(reading, i as u64 * 5, &mut |name, down, _| {
                events.push((name, down))
            });
        }
        assert_eq!(
            events,
            [
                (buttons::PAGE_FORWARD, true),
                (buttons::PAGE_FORWARD, false)
            ]
        );
        // The other ladder at the same time is another button.
        let both = State::read(2694, 3, false);
        let mut events = Vec::new();
        for i in 0..5 {
            debouncer.update(both, 100 + i * 5, &mut |name, down, _| {
                events.push((name, down))
            });
        }
        assert_eq!(
            events,
            [(buttons::CONFIRM, true), (buttons::PAGE_FORWARD, true)]
        );
    }

    #[test]
    fn presses_count_once_they_hold() {
        let mut debouncer = Debouncer::new();
        let mut events = Vec::new();
        let confirm = State::read(2694, 4095, false);
        let mut at = |reading, now, events: &mut Vec<(&str, bool)>| {
            debouncer.update(reading, now, &mut |name, down, _| events.push((name, down)));
        };
        at(confirm, 0, &mut events);
        at(State(0), 2, &mut events);
        at(confirm, 3, &mut events);
        assert!(events.is_empty(), "a bounce");
        at(confirm, 10, &mut events);
        assert!(events.is_empty(), "held 7 ms");
        at(confirm, 18, &mut events);
        assert_eq!(events, [(buttons::CONFIRM, true)]);
        at(confirm, 20, &mut events);
        at(State(0), 30, &mut events);
        at(State(0), 45, &mut events);
        assert_eq!(
            events,
            [(buttons::CONFIRM, true), (buttons::CONFIRM, false)]
        );
    }

    #[test]
    fn saving_reads_the_buttons_no_oftener_than_every_sixty_milliseconds() {
        // 10 (Faster), 30 (Normal), 50 (Saving) are the reader's choices.
        for response in [10, 30, 50] {
            assert_eq!(read_interval_ms(response, false), response);
            assert_eq!(read_interval_ms(response, true), SAVING_READ_MS);
        }
        // A slower choice is kept.
        assert_eq!(read_interval_ms(100, true), 100);
        // A press of 80 ms is seen at that rate.
        assert!(SAVING_READ_MS < 80);
    }
}
