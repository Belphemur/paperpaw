//! Plugins keeping the device from going to sleep by itself
//! ([`paperpaw_sdk::power::WakeHold`], ADR 0030): holds by id, each with
//! its end, a screen's counting only while it is on top and ending with
//! it. They hold only the sleep after `core.sleep-after` without input,
//! never while the reader lets no plugin hold it, nor once the battery is
//! nearly empty. The library's walk holds the sleep too (the kernel's own, not
//! a plugin's: `Kernel::walk_holds_sleep`), whatever the reader chose, for an
//! hour without input at most.

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_platform::Instant;
use paperpaw_sdk::Id;
use paperpaw_sdk::builtin::settings as core_settings;
use paperpaw_sdk::power::WakeHold;

use crate::Kernel;

/// At or below this charge, not charging, no hold counts: the device
/// sleeps as it would.
pub(crate) const LOW_BATTERY_PERCENT: u8 = 10;

/// The library's walk keeps the device awake no longer than this without
/// the reader pressing anything: a first walk of 3,000 books takes about
/// 13 minutes on the X4, a slower card or a larger library several times
/// that. An hour is generous; a walk that goes on after it sleeps and goes
/// on after the wake.
pub(crate) const WALK_AWAKE_MS: u64 = 60 * 60_000;

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
    /// The library's walk kept the sleep off, since the reader's last
    /// input, and the time the sleep was put off to (the activity to
    /// compare with: another means the reader pressed something).
    walk: Option<(Instant, Instant)>,
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

    /// Whether the battery is nearly empty and not charging.
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

    /// The sleep is due at `now`: whether something puts it off (reading
    /// aloud, a download, a server, the library's walk).
    pub(crate) fn busy_at_sleep(&mut self, now: Instant) -> bool {
        let walking = self.walk_holds_sleep(now);
        walking || self.io.speech.speaking() || self.io.http.busy() || self.io.servers.busy()
    }

    /// The sleep is due at `now`: whether the library's walk puts it off.
    /// The walk keeps going while it takes steps (not while a book is open
    /// or the work is held back), so that the sleep does not stop a first
    /// walk that outlasts "Sleep after"; it puts the sleep off by one
    /// "Sleep after" at a time, for [`WALK_AWAKE_MS`] since the reader's
    /// last input at most, and never at a nearly empty battery. "Switch
    /// off after" counts from the sleep, so it waits as well.
    pub(crate) fn walk_holds_sleep(&mut self, now: Instant) -> bool {
        if !self.works.stepping(&crate::core_plugin::CATALOG_WORK) {
            self.holds.walk = None;
            return false;
        }
        // Put off before and nothing pressed since: counted from the
        // reader's last input then.
        let since = match self.holds.walk {
            Some((since, marked)) if marked == self.last_activity => since,
            _ => self.last_activity,
        };
        if self.battery_low() || now.since(since) >= WALK_AWAKE_MS {
            self.holds.walk = None;
            return false;
        }
        self.holds.walk = Some((since, now));
        true
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
