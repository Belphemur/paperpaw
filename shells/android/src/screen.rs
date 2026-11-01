//! Where in the window PaperPaw draws, and how large its interface is to
//! start with. Plain arithmetic, tested on any computer.
//!
//! **Insets.** The system covers parts of the window: the status bar, the
//! navigation or gesture bar, a display cutout (a notch, a punched hole),
//! rounded corners. The shell takes them off the window: the rectangle that
//! is left is the screen the kernel is told of (`Kernel::resize`), so
//! screens and plugins are laid out within it and never see an inset. The
//! window's own pixels around it are black, and what is pressed there
//! (the gesture bar) is not a press on the screen.
//!
//! **Size.** `core.ui-size` is the interface's text size in points (7 to
//! 24), which the density turns into the same size to the eye on every
//! screen. What differs is how far the screen is from the eye and how much
//! room it has: a phone's narrow width wants the smaller text, a large
//! tablet, held further away, the larger. [`ui_size`] chooses a first
//! value by the physical size of the screen's short side; the reader
//! changes it as ever.

use core::ops::Range;

/// What the system covers on each side of the window, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Insets {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// The part of the window PaperPaw's screen is: its size and where its
/// top left corner lies in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Content {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Content {
    /// The window of `window` pixels less `insets`. Insets that would take
    /// half the window or more along one side (a report that cannot be
    /// right) are left out along it: a screen is always there.
    #[must_use]
    pub fn of(window: (u32, u32), insets: Insets) -> Self {
        let (width, height) = window;
        let (left, right) = Self::along(width, insets.left, insets.right);
        let (top, bottom) = Self::along(height, insets.top, insets.bottom);
        Self {
            x: left,
            y: top,
            width: width - left - right,
            height: height - top - bottom,
        }
    }

    fn along(length: u32, start: u32, end: u32) -> (u32, u32) {
        if start.saturating_add(end) < length / 2 {
            (start, end)
        } else {
            (0, 0)
        }
    }

    /// Whether the window has no inset on this side of it: the content is
    /// the window.
    #[must_use]
    pub fn is_window(&self, window: (u32, u32)) -> bool {
        (self.x, self.y, self.width, self.height) == (0, 0, window.0, window.1)
    }

    /// The point `(x, y)` of the window as a point of the screen, or
    /// `None` where it lies outside it (on the gesture bar).
    #[must_use]
    pub fn point(&self, x: f32, y: f32) -> Option<(i32, i32)> {
        let (x, y) = (x - self.x as f32, y - self.y as f32);
        let inside = x >= 0.0 && y >= 0.0 && x < self.width as f32 && y < self.height as f32;
        inside.then_some((x as i32, y as i32))
    }

    /// The point `(x, y)` of the window as a point of the screen, held to
    /// the screen: for a finger that began on it and moved past its edge,
    /// which ends where it left.
    #[must_use]
    pub fn point_held(&self, x: f32, y: f32) -> (i32, i32) {
        let hold = |value: f32, start: u32, length: u32| {
            (value - start as f32).clamp(0.0, length.saturating_sub(1) as f32) as i32
        };
        (hold(x, self.x, self.width), hold(y, self.y, self.height))
    }

    /// Where `frame_rows` of the screen are in a window buffer of
    /// `stride` pixels a row: the buffer's rows, and the offset in bytes
    /// (RGBA) that its first pixel of those rows has from the buffer's
    /// start for the frame's first one.
    #[must_use]
    pub fn in_buffer(&self, frame_rows: &Range<u16>, stride: usize) -> (Range<usize>, usize) {
        let (x, y) = (self.x as usize, self.y as usize);
        let rows = y + usize::from(frame_rows.start)..y + usize::from(frame_rows.end);
        (rows, (y * stride + x) * 4)
    }
}

/// The interface's text size, in points, a new reader starts with, for a
/// screen of `width`×`height` pixels at `dpi`: by its short side. A phone
/// (under 100 mm: 68 to 80 mm are usual) is small and held close, and its
/// width wants 8 points; the e-readers and small tablets that PaperPaw
/// ran on before (under 150 mm: a Boox Go 7 has 107) keep 9; a large
/// tablet is held further away: 10 under 190 mm, 11 above.
#[must_use]
pub fn ui_size(width: u32, height: u32, dpi: u32) -> i64 {
    let tenth_mm = u64::from(width.min(height)) * 254 / u64::from(dpi.max(1));
    match tenth_mm {
        0..1000 => 8,
        1000..1500 => 9,
        1500..1900 => 10,
        _ => 11,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: (u32, u32) = (1080, 2400);

    fn insets(left: u32, top: u32, right: u32, bottom: u32) -> Insets {
        Insets {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn no_insets_leave_the_window() {
        let content = Content::of(PHONE, Insets::default());
        assert!(content.is_window(PHONE));
        assert_eq!((content.width, content.height), PHONE);
    }

    #[test]
    fn a_status_bar_a_cutout_and_a_gesture_bar_are_taken_off() {
        // Status bar 120 px at the top, gesture bar 63 at the bottom.
        let content = Content::of(PHONE, insets(0, 120, 0, 63));
        assert_eq!(
            content,
            Content {
                x: 0,
                y: 120,
                width: 1080,
                height: 2400 - 120 - 63
            }
        );
        // Landscape with the cutout on the left.
        let content = Content::of((2400, 1080), insets(110, 0, 0, 63));
        assert_eq!(content.x, 110);
        assert_eq!(content.width, 2400 - 110);
        assert_eq!(content.height, 1080 - 63);
        assert!(!content.is_window((2400, 1080)));
    }

    #[test]
    fn insets_that_cannot_be_right_are_left_out_along_their_side() {
        // Half the height and more: the vertical ones go, the horizontal stay.
        let content = Content::of(PHONE, insets(20, 1200, 20, 100));
        assert_eq!((content.x, content.y), (20, 0));
        assert_eq!((content.width, content.height), (1040, 2400));
        // Overflowing sums do not wrap.
        let content = Content::of(PHONE, insets(u32::MAX, 0, u32::MAX, 0));
        assert_eq!((content.x, content.width), (0, 1080));
        // Nothing of a zero window: still no panic.
        let content = Content::of((0, 0), insets(1, 1, 1, 1));
        assert_eq!((content.width, content.height), (0, 0));
    }

    #[test]
    fn a_touch_on_the_screen_is_a_point_of_it() {
        let content = Content::of(PHONE, insets(0, 120, 0, 63));
        assert_eq!(content.point(10.0, 120.0), Some((10, 0)));
        assert_eq!(content.point(540.4, 1000.9), Some((540, 880)));
        // On the status bar, the gesture bar, past the right edge.
        assert_eq!(content.point(10.0, 119.9), None);
        assert_eq!(content.point(10.0, 2400.0 - 63.0), None);
        assert_eq!(content.point(1080.0, 500.0), None);
        assert_eq!(content.point(-1.0, 500.0), None);
    }

    #[test]
    fn a_finger_that_began_on_the_screen_ends_at_its_edge() {
        let content = Content::of(PHONE, insets(0, 120, 0, 63));
        assert_eq!(content.point_held(500.0, 2390.0), (500, 2216));
        assert_eq!(content.point_held(-30.0, 40.0), (0, 0));
        assert_eq!(content.point_held(5000.0, 130.0), (1079, 10));
        // A screen without width or height: a point, not a panic.
        let none = Content::of((0, 0), Insets::default());
        assert_eq!(none.point_held(5.0, 5.0), (0, 0));
    }

    #[test]
    fn the_frame_lies_in_the_window_buffer_below_and_beside_the_insets() {
        let content = Content::of((100, 200), insets(10, 20, 0, 0));
        let (rows, offset) = content.in_buffer(&(5..8), 104);
        assert_eq!(rows, 25..28);
        // Row 20, column 10 of a buffer of 104 pixels a row, four bytes each.
        assert_eq!(offset, (20 * 104 + 10) * 4);
    }

    #[test]
    fn the_size_follows_the_screens_short_side() {
        // A phone 1080×2400 at 420 dpi: 65 mm.
        assert_eq!(ui_size(1080, 2400, 420), 8);
        // Landscape is the same screen.
        assert_eq!(ui_size(2400, 1080, 420), 8);
        // A Boox Go 7 (107 mm) and a Note Air (157 mm at 227 dpi).
        assert_eq!(ui_size(1264, 1680, 300), 9);
        assert_eq!(ui_size(1404, 1872, 227), 10);
        // A 10" tablet (1600×2560 at 280: 145 mm) and a 13" one (190 mm +).
        assert_eq!(ui_size(1600, 2560, 280), 9);
        assert_eq!(ui_size(1600, 2560, 240), 10);
        assert_eq!(ui_size(2000, 3000, 250), 11);
    }

    #[test]
    fn a_density_that_is_not_known_gives_a_size_not_a_panic() {
        assert_eq!(ui_size(1080, 2400, 0), 11);
        assert_eq!(ui_size(0, 0, 160), 8);
    }

    #[test]
    fn the_sizes_are_within_the_setting_s_range() {
        for (width, dpi) in [(100, 600), (600, 160), (1200, 120), (u32::MAX, 1)] {
            let size = ui_size(width, width, dpi);
            assert!((7..=24).contains(&size), "{width} {dpi}: {size}");
        }
    }
}
