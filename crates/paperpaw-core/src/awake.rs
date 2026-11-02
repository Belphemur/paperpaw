//! Plugins keeping the device from going to sleep by itself
//! ([`paperpaw_sdk::power::WakeHold`], ADR 0030): holds by id, each with
//! its end, a screen's counting only while it is on top and ending with
//! it. They hold only the sleep after `core.sleep-after` without input,
//! never while the reader lets no plugin hold it, nor once the battery is
//! nearly empty.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_platform::Instant;
use paperpaw_sdk::Id;
use paperpaw_sdk::builtin::settings as core_settings;
use paperpaw_sdk::power::WakeHold;

use crate::{Effects, Kernel};

/// At or below this charge, not charging, no hold counts: the device
/// sleeps as it would.
pub(crate) const LOW_BATTERY_PERCENT: u8 = 10;

/// A hold in force.
#[derive(Debug)]
struct Held {
    id: Id,
    plugin: String,
    reason: Cow<'static, str>,
    /// The screen it is for, by serial.
    screen: Option<u32>,
    until: Instant,
}

/// The holds in force, and when their time was last counted.
#[derive(Debug, Default)]
pub(crate) struct Holds {
    held: Vec<Held>,
    counted_at: Option<Instant>,
    /// The shell says the device is on outer power and stays awake there
    /// ([`Kernel::set_awake_on_usb`]).
    usb: bool,
}

impl Kernel {
    /// Takes `hold` (in place of one of the same id).
    pub(crate) fn keep_awake(&mut self, hold: WakeHold) {
        // What the holds before kept counted up to now.
        self.count_awake();
        let id = hold.id().clone();
        self.holds.held.retain(|h| h.id != id);
        let until = self
            .now
            .add_millis(u64::try_from(hold.duration().as_millis()).unwrap_or(u64::MAX));
        self.holds.held.push(Held {
            plugin: String::from(id.namespace()),
            reason: Cow::Owned(String::from(hold.reason())),
            screen: hold.screen().map(|key| key.serial()),
            id,
            until,
        });
    }

    /// Lets go of the hold of `id`.
    pub(crate) fn let_sleep(&mut self, id: &Id) {
        self.count_awake();
        self.holds.held.retain(|h| h.id != *id);
    }

    /// The battery is nearly empty and does not charge.
    fn battery_low(&self) -> bool {
        self.shared
            .status
            .battery
            .is_some_and(|b| !b.charging && b.percent <= LOW_BATTERY_PERCENT)
    }

    /// Whether holds count now: the reader lets plugins hold the device,
    /// and the battery is not nearly empty.
    fn holds_count(&self) -> bool {
        !self.battery_low() && self.shared.settings.read(&core_settings::AWAKE_HOLDS)
    }

    /// Whether the device is kept awake by outer power (a developer's
    /// choice, [`Kernel::set_awake_on_usb`]): not once the battery is
    /// nearly empty and does not charge.
    pub(crate) fn awake_on_usb(&self) -> bool {
        self.holds.usb && !self.battery_low()
    }

    /// The shell says the device is on outer power and the reader chose to
    /// keep it awake there (`on`): the sleep after `core.sleep-after`
    /// without input does not come. An explicit sleep is the shell's to
    /// handle; only the timer is held. When it ends (the cable is out),
    /// the timer counts from `now`, not from the last use, so that the
    /// device does not fall asleep the moment the cable is pulled.
    pub fn set_awake_on_usb(&mut self, now: Instant, on: bool) -> Effects {
        let _running = self.shared.meter.enter(crate::meter::CORE);
        self.now = now;
        if self.holds.usb && !on {
            self.last_activity = now;
        }
        self.holds.usb = on;
        self.finish()
    }

    /// The holds that count now: in force, and a screen's only while it is
    /// on top.
    fn holding(&self) -> impl Iterator<Item = &Held> {
        let top = self.stack.last().map(|s| s.serial);
        let now = self.now;
        let count = self.holds_count();
        self.holds
            .held
            .iter()
            .filter(move |h| count && h.until > now && h.screen.is_none_or(|s| Some(s) == top))
    }

    /// Until when holds keep the device awake, if any does now.
    pub(crate) fn held_until(&self) -> Option<Instant> {
        self.holding().map(|h| h.until).max()
    }

    /// Counts the time holds kept the device awake for their plugins, and
    /// lets go of holds that ended or whose screen closed.
    pub(crate) fn count_awake(&mut self) {
        let now = self.now;
        if let Some(since) = self.holds.counted_at {
            let millis = now.since(since);
            let holding: Vec<(String, Cow<'static, str>, u64)> = self
                .holding()
                .map(|h| {
                    (
                        h.plugin.clone(),
                        h.reason.clone(),
                        h.until.since(since).min(millis),
                    )
                })
                .collect();
            for (plugin, reason, millis) in holding {
                self.shared.meter.awake(&plugin, millis, reason);
            }
        }
        self.holds.counted_at = Some(now);
        let open: Vec<u32> = self.stack.iter().map(|s| s.serial).collect();
        self.holds
            .held
            .retain(|h| h.until > now && h.screen.is_none_or(|s| open.contains(&s)));
    }
}
