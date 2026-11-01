//! Icons by name, e.g. `books` or `settings`, drawn from the icon font
//! bound to [`fonts::ICONS`](crate::builtin::fonts::ICONS).
//!
//! The names are those of [Tabler Icons](https://tabler.io/icons), all of
//! them known here; the icon font holds the drawings of those PaperPaw
//! uses (see `tools/icons/subset.py`), icon packs add others (font packs
//! that extend it, see `tools/icons/pack.py`). An icon theme is a font
//! with other drawings at the same code points, bound to the role in
//! place of the default.
//!
//! Plugins give their menu entries icons by name
//! ([`Registrar::add_icon`](crate::Registrar::add_icon)); an icon without
//! a drawing draws nothing.
//!
//! A native plugin file (ADR 0024) built with `--cfg paperpaw_plugin`
//! (as `samples/hello-native` builds) asks the firmware for an icon's
//! character instead of carrying the names of all of them.

include!("icon_table.rs");

/// What an icon says rather than which it is: drawn with the icon a
/// plugin names for it ([`Registrar::add_icon`](crate::Registrar::add_icon),
/// e.g. `r.add_icon(icons::BACK, "arrow-big-left".into())`), else with
/// its default ([`default_of`]), else as PaperPaw draws it itself (the
/// arrows of headers and the keyboard's keys).
pub const BACK: &str = "core.icon.back";
pub const PREV: &str = "core.icon.prev";
pub const NEXT: &str = "core.icon.next";

/// The semantic arrow icon pointing the other way, for a UI that runs
/// right to left: back and previous point right, next points left.
pub(crate) fn turned(icon: &str) -> &str {
    match icon {
        BACK | PREV => NEXT,
        NEXT => PREV,
        other => other,
    }
}

/// Beside a button's hint where holding it does something too.
pub const HOLD: &str = "core.icon.hold";
/// In a button's hint while its press is being worked on.
pub const BUSY: &str = "core.icon.busy";
/// PaperPaw's paw: the busy mark in a corner of the screen.
pub const PAW: &str = "paw";
/// The keyboard's keys.
pub const SHIFT: &str = "core.icon.shift";
pub const DELETE: &str = "core.icon.delete";
pub const SPACE: &str = "core.icon.space";

/// The icon a semantic icon ([`BACK`] and the others) is drawn with
/// unless a plugin names another; `None` where PaperPaw draws it itself.
pub fn default_of(name: &str) -> Option<&'static str> {
    match name {
        HOLD => Some("hand-click"),
        BUSY => Some("paw"),
        _ => None,
    }
}

/// The name of icon `index` (in the order of [`STARTS`]).
fn name(index: usize) -> &'static str {
    let start = usize::from(STARTS[index]);
    let length = usize::from(LENGTHS[index] & ((1 << LENGTH_BITS) - 1));
    NAMES.get(start..start + length).unwrap_or("")
}

/// The character of icon `index` (in the order of [`STARTS`]).
#[cfg_attr(
    all(paperpaw_plugin, target_os = "none"),
    allow(dead_code, reason = "a plugin file asks the firmware")
)]
fn code(index: usize) -> Option<char> {
    let high = u32::from(LENGTHS[index] >> LENGTH_BITS) << 16;
    char::from_u32(CODE_BASE + (high | u32::from(CODES[index])))
}

/// The character of icon `name` in the icon font; a name `U+F8000` is
/// that code point, for icons of one's own outside the Tabler names.
#[cfg(not(all(paperpaw_plugin, target_os = "none")))]
pub fn char(name: &str) -> Option<char> {
    code_point(name).or_else(|| by_name(name))
}

/// The character a name `U+` and hexadecimal digits names.
fn code_point(name: &str) -> Option<char> {
    let hex = name.strip_prefix("U+")?;
    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
}

/// The character of icon `name` in the icon font, as the firmware that
/// loaded this plugin file knows it.
#[cfg(all(paperpaw_plugin, target_os = "none"))]
pub fn char(name: &str) -> Option<char> {
    if let Some(ch) = code_point(name) {
        return Some(ch);
    }
    unsafe extern "C" {
        /// The firmware's table: `paperpaw-plugin.x` makes the word, the
        /// loader writes its address before the plugin's code runs.
        static PAPERPAW_API: *const paperpaw_native::Api;
    }
    // SAFETY: a table that lives as long as the firmware runs, written
    // before any code of the plugin ran.
    let api = unsafe { &*PAPERPAW_API };
    let has_icon = core::mem::offset_of!(paperpaw_native::Api, icon)
        + core::mem::size_of::<usize>()
        <= api.size as usize;
    if !has_icon {
        return None;
    }
    // SAFETY: the firmware reads `name` while it answers.
    let code = unsafe { (api.icon)(name.as_ptr(), name.len()) };
    core::char::from_u32(code).filter(|&c| c != '\0')
}

/// The character of icon `name`, from the names of all icons.
#[cfg(not(all(paperpaw_plugin, target_os = "none")))]
fn by_name(name: &str) -> Option<char> {
    let (mut low, mut high) = (0, STARTS.len());
    while low < high {
        let middle = (low + high) / 2;
        match self::name(middle).cmp(name) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => return code(middle),
        }
    }
    None
}

/// The names of all icons, sorted.
pub fn names() -> impl Iterator<Item = &'static str> {
    (0..STARTS.len()).map(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_found_by_name() {
        let all: alloc::vec::Vec<&str> = names().collect();
        assert!(all.windows(2).all(|w| w[0] < w[1]), "sorted");
        assert_eq!(char("home"), Some('\u{eac1}'));
        assert!(char("books").is_some());
        // The first and the last.
        assert_eq!(char(all[0]), code(0));
        assert_eq!(char(all[all.len() - 1]), code(all.len() - 1));
        assert_eq!(char("no-such-icon"), None);
        assert_eq!(char(""), None);
        // Icons of one's own by their code point.
        assert_eq!(char("U+F8000"), Some('\u{f8000}'));
        assert_eq!(char("U+ZZ"), None);
        assert_eq!(char("U+D800"), None);
        assert_eq!(by_name("home"), char("home"));
        assert!(all.len() > 5000);
    }
}
