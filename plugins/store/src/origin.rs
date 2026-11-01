//! Which installed plugins the store updates: only those it installed
//! itself (see [`install::origin`]), never one put in place otherwise.

use core::cmp::Ordering;

use paperpaw_sdk::firmware::compare_versions;
use paperpaw_sdk::install::{self, Origin, Origins};
use paperpaw_sdk::{Env, Version};

use crate::catalog::Entry;

impl Entry {
    /// Whether a newer version than the one installed is offered: only
    /// for a plugin this store installed, never over one put in place
    /// otherwise (see [`install::origin`]).
    pub(crate) fn newer(&self, env: &Env<'_>) -> bool {
        self.newer_in(env, &Origins::read(env.storage()))
    }

    /// The same, the origins read once for many (a list).
    pub(crate) fn newer_in(&self, env: &Env<'_>, origins: &Origins) -> bool {
        // Its newer version is made for a newer PaperPaw: what is
        // installed stays.
        if self.needs_newer {
            return false;
        }
        let Some(installed) = self.installed(env) else {
            return false;
        };
        if origins.of(&self.id, installed) != Origin::Store {
            return false;
        }
        // As the store named it: betas of one release share its numbers.
        let numbers = alloc::format!("{installed}");
        let here = origins.release(&self.id, installed).unwrap_or(&numbers);
        offers(&self.version, here, takes_betas(env))
    }

    /// Whether the plugin is installed, but not by this store (the Wi-Fi
    /// transfer, a memory card, one's own build): its version here may be
    /// used in its place, when the reader says so, never as an update.
    pub(crate) fn local(&self, env: &Env<'_>) -> bool {
        !self.needs_newer
            && self
                .installed(env)
                .is_some_and(|installed| self.origin(env, installed) == Origin::Local)
    }

    fn origin(&self, env: &Env<'_>, installed: Version) -> Origin {
        install::origin(env.storage(), &self.id, installed)
    }
}

/// The update plugin's setting: betas offered too.
const BETAS: &str = "update.betas";

/// Whether this device takes betas: it runs one, or the reader asked for
/// them (*Beta updates*), as firmware updates go.
fn takes_betas(env: &Env<'_>) -> bool {
    env.settings().bool(BETAS) || env.firmware().is_some_and(|f| is_pre_release(&f.version))
}

/// Whether `version` is a pre-release (`0.5.0-beta.1`).
fn is_pre_release(version: &str) -> bool {
    version.split('+').next().is_some_and(|v| v.contains('-'))
}

/// Whether `offered` is an update over `here`, whole versions compared
/// (`0.5.0-beta.2` after `0.5.0-beta.1`, `0.5.0` after both); a beta only
/// where the device takes `betas`.
fn offers(offered: &str, here: &str, betas: bool) -> bool {
    Version::parse(offered).is_some()
        && compare_versions(offered, here) == Ordering::Greater
        && (betas || !is_pre_release(offered))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn betas_are_updates_in_their_order_where_they_are_taken() {
        assert!(offers("0.5.0-beta.2", "0.5.0-beta.1", true));
        assert!(!offers("0.5.0-beta.2", "0.5.0-beta.1", false));
        assert!(offers("0.5.0", "0.5.0-beta.2", false));
        assert!(!offers("0.5.0-beta.1", "0.5.0-beta.2", true));
        assert!(!offers("0.5.0-beta.3", "0.5.0", true));
        assert!(offers("0.5.0-beta.10", "0.5.0-beta.9", true));
        assert!(offers("1.1.0", "1.0.0", false));
        assert!(!offers("1.0.0", "1.0.0", true));
        assert!(!offers("not.a.version", "1.0.0", true));
    }
}
