//! A window or a browser's canvas that sets the size of the screen
//! (`--free-size`, the web emulator's free size): which size the screen is
//! made at, and when. The kernel remakes every open screen for a new size
//! (`Kernel::resize`), which takes some time, so a window dragged to
//! another size is looked at once it has settled, not at every step.
//!
//! Devices keep their panel's size by default: an e-ink panel's pixels are
//! the device, and the window only scales them (as the reader's own device
//! would be as large as it is). A free size is for trying a screen that
//! adapts, on a phone, a tablet or a window: the profile's density and
//! buttons stay, the size is the window's.

/// The shortest side a screen is made with: the smallest screens PaperPaw
/// runs on are about 240 pixels wide.
pub const MIN_SIDE: u32 = 240;
/// The longest side: far above any e-reader and a 4K window halved.
pub const MAX_SIDE: u32 = 2400;
/// How long a window keeps its size before the screen follows it.
pub const SETTLE_MS: u64 = 200;

/// The size of the screen for a window of `window` pixels: its own size,
/// divided by the smallest whole number that brings it under [`MAX_SIDE`]
/// (the window shows each pixel of the screen as that many), at least
/// [`MIN_SIDE`] on each side. `None` for a window without room (minimised
/// or not yet shown).
#[must_use]
pub fn panel_size(window: (u32, u32)) -> Option<(u16, u16)> {
    let (width, height) = window;
    if width == 0 || height == 0 {
        return None;
    }
    let zoom = width.max(height).div_ceil(MAX_SIDE);
    let side = |pixels: u32| (pixels / zoom).clamp(MIN_SIDE, MAX_SIDE) as u16;
    Some((side(width), side(height)))
}

/// Waits for a size to hold: it is asked for at every step of a drag
/// ([`Settle::asked`]) and given back once it has held for [`SETTLE_MS`]
/// ([`Settle::due`]); the same size as the screen has is never given.
#[derive(Debug, Default)]
pub struct Settle {
    /// The size asked last, and when it was.
    pending: Option<((u16, u16), u64)>,
    /// The size the screen has.
    applied: Option<(u16, u16)>,
}

impl Settle {
    /// The screen has `size`.
    pub fn is_now(&mut self, size: (u16, u16)) {
        self.applied = Some(size);
        if self.pending.is_some_and(|(asked, _)| asked == size) {
            self.pending = None;
        }
    }

    /// The window has `window` pixels at `now` (milliseconds).
    pub fn asked(&mut self, now: u64, window: (u32, u32)) {
        self.pending = match panel_size(window) {
            Some(size) if self.applied != Some(size) => match self.pending {
                // The same size asked again does not start the wait over.
                Some((asked, _)) if asked == size => self.pending,
                _ => Some((size, now)),
            },
            _ => None,
        };
    }

    /// The size to make the screen, once it has held long enough.
    pub fn due(&mut self, now: u64) -> Option<(u16, u16)> {
        let (size, since) = self.pending?;
        if now.saturating_sub(since) < SETTLE_MS {
            return None;
        }
        self.pending = None;
        self.applied = Some(size);
        Some(size)
    }

    /// How many milliseconds from `now` until [`Settle::due`] says so, if a
    /// size is waiting.
    #[must_use]
    pub fn wait(&self, now: u64) -> Option<u64> {
        let (_, since) = self.pending?;
        Some((since + SETTLE_MS).saturating_sub(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_sets_the_size_of_the_screen() {
        assert_eq!(panel_size((900, 700)), Some((900, 700)));
        assert_eq!(panel_size((480, 800)), Some((480, 800)));
        assert_eq!(panel_size((2400, 1000)), Some((2400, 1000)));
    }

    #[test]
    fn a_window_without_room_sets_nothing() {
        assert_eq!(panel_size((0, 700)), None);
        assert_eq!(panel_size((900, 0)), None);
    }

    #[test]
    fn a_small_window_keeps_the_screens_smallest_size() {
        assert_eq!(panel_size((100, 100)), Some((240, 240)));
        assert_eq!(panel_size((600, 50)), Some((600, 240)));
    }

    #[test]
    fn a_huge_window_shows_each_pixel_of_the_screen_more_than_once() {
        // 4K: two window pixels for one of the screen.
        assert_eq!(panel_size((3840, 2160)), Some((1920, 1080)));
        // Just over the limit: halved, not cut.
        assert_eq!(panel_size((2401, 1000)), Some((1200, 500)));
        let (w, h) = panel_size((u32::MAX, u32::MAX)).unwrap();
        assert!(u32::from(w) <= MAX_SIDE && u32::from(h) <= MAX_SIDE);
    }

    #[test]
    fn a_drag_is_looked_at_once_it_has_settled() {
        let mut settle = Settle::default();
        settle.is_now((480, 800));
        settle.asked(1000, (500, 800));
        settle.asked(1050, (520, 800));
        settle.asked(1100, (540, 800));
        // Each step of the drag starts the wait over.
        assert_eq!(settle.due(1250), None);
        assert_eq!(settle.wait(1250), Some(50));
        assert_eq!(settle.due(1300), Some((540, 800)));
        // Taken: nothing left to wait for.
        assert_eq!(settle.due(5000), None);
        assert_eq!(settle.wait(5000), None);
    }

    #[test]
    fn the_same_size_asked_again_does_not_start_the_wait_over() {
        let mut settle = Settle::default();
        settle.asked(0, (500, 500));
        settle.asked(150, (500, 500));
        assert_eq!(settle.due(200), Some((500, 500)));
    }

    #[test]
    fn the_size_the_screen_has_is_not_made_again() {
        let mut settle = Settle::default();
        settle.is_now((480, 800));
        settle.asked(0, (480, 800));
        assert_eq!(settle.wait(0), None);
        assert_eq!(settle.due(1000), None);
        // A drag that ends where it began cancels what it asked.
        settle.asked(2000, (500, 800));
        settle.asked(2050, (480, 800));
        assert_eq!(settle.due(3000), None);
        // The screen was made so by something else meanwhile.
        settle.asked(4000, (600, 800));
        settle.is_now((600, 800));
        assert_eq!(settle.due(5000), None);
    }

    #[test]
    fn a_minimised_window_leaves_the_screen_as_it_is() {
        let mut settle = Settle::default();
        settle.is_now((480, 800));
        settle.asked(0, (600, 800));
        settle.asked(10, (0, 0));
        assert_eq!(settle.due(1000), None);
    }
}
