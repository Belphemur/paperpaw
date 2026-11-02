//! The readable column: on a screen wider than a line of text reads
//! well, what is text and rows stands in a column in the middle.

use super::UiStyle;
use crate::{Env, Rect};

/// A readable column is at least this many millimetres wide.
const READABLE_MM: u32 = 110;

/// A readable column is at least as wide as this many text heights (about
/// 65 to 75 characters), so large text gets a wider column.
const READABLE_EMS: u32 = 36;

/// The widest a line of rows or text is on a screen, in pixels.
fn most(env: &Env<'_>, style: &UiStyle) -> u16 {
    let ems = (style.text() * READABLE_EMS as f32) as u32;
    env.metrics()
        .mm(READABLE_MM)
        .max(ems)
        .min(u32::from(u16::MAX)) as u16
}

/// `area` cut to a column in its middle, at most 110
/// millimetres or 36 text heights wide (whichever is more),
/// the same height; `area` itself where it is not wider (so a narrow
/// screen, such as the X4's in either orientation, is never cut). The
/// column is in the middle in a right-to-left interface as well.
///
/// The header and other bars keep the whole width (the back button stays
/// at the screen's edge); rows and text, which are hard to read when a
/// line crosses the screen, take the column.
pub fn readable(env: &Env<'_>, style: &UiStyle, area: Rect) -> Rect {
    let width = most(env, style);
    if area.width <= width {
        return area;
    }
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y,
        width,
        area.height,
    )
}
