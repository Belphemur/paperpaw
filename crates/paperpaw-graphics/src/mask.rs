//! Masks (glyphs, icons) blended a row at a time
//! ([`Canvas::draw_mask`](crate::Canvas::draw_mask)) where the frame is
//! black and white or in packed grays, as most panels are: every glyph
//! of a page comes here, pixel by pixel.
//!
//! What does not change from pixel to pixel is worked out once per mask:
//! the stored value of the colour, the threshold of black and white, a
//! level's step. The pixels come out exactly as
//! [`Framebuffer::blend_pixel`] blends each one.

use paperpaw_platform::{Color, PixelFormat, Rect};

use crate::framebuffer::Framebuffer;

impl Framebuffer {
    /// Blends `color` over `rect` (inside the band) as
    /// [`Framebuffer::blend_pixel`] blends each pixel, with the coverage
    /// `coverage(dx, dy)` (0 to 255), counted from `from` at `rect`'s
    /// corner. `false` where the format is drawn otherwise: nothing is
    /// drawn then. Does not mark anything dirty.
    pub(crate) fn blend_mask(
        &mut self,
        rect: Rect,
        from: (u32, u32),
        color: Color,
        coverage: impl Fn(u32, u32) -> u8,
    ) -> bool {
        if self.is_blind() {
            return true;
        }
        match self.format() {
            PixelFormat::Mono1 => self.mask_mono(rect, from, color, coverage),
            PixelFormat::Gray2 | PixelFormat::Gray4 => self.mask_gray(rect, from, color, coverage),
            _ => return false,
        }
        true
    }

    /// The pixels from the coverage set on take the colour, the bits of a
    /// byte gathered and written together.
    fn mask_mono(
        &mut self,
        rect: Rect,
        from: (u32, u32),
        color: Color,
        coverage: impl Fn(u32, u32) -> u8,
    ) {
        let ink: u8 = if self.encode(color) & 1 != 0 { 0xFF } else { 0 };
        let least = self.mono_coverage().max(1);
        let (data, stride, top) = self.rows_mut();
        let (left, right) = (usize::from(rect.x), rect.right() as usize);
        for (dy, y) in (from.1..).zip(rect.y..rect.bottom() as u16) {
            let row = usize::from(y - top) * stride;
            let mut byte = left / 8;
            // The pixels of `byte` that take the colour.
            let mut bits = 0u8;
            for (dx, x) in (from.0..).zip(left..right) {
                if x / 8 != byte {
                    data[row + byte] = (data[row + byte] & !bits) | (ink & bits);
                    (byte, bits) = (x / 8, 0);
                }
                if coverage(dx, dy) >= least {
                    bits |= 0x80 >> (x % 8);
                }
            }
            data[row + byte] = (data[row + byte] & !bits) | (ink & bits);
        }
    }

    /// Each pixel mixed in gray with what lies under it, as
    /// `blend_pixel` mixes, without its division: a level's step divides
    /// 255 (85 for four grays, 17 for sixteen).
    fn mask_gray(
        &mut self,
        rect: Rect,
        from: (u32, u32),
        color: Color,
        coverage: impl Fn(u32, u32) -> u8,
    ) {
        let bpp = usize::from(self.bpp());
        let max = (1u32 << bpp) - 1;
        let step = 255 / max;
        let full = self.encode(color) as u8;
        let over = u32::from(color.luma());
        let inverted = self.is_inverted();
        let (data, stride, top) = self.rows_mut();
        let (left, right) = (usize::from(rect.x), rect.right() as usize);
        for (dy, y) in (from.1..).zip(rect.y..rect.bottom() as u16) {
            let row = usize::from(y - top) * stride;
            for (dx, x) in (from.0..).zip(left..right) {
                let a = u32::from(coverage(dx, dy));
                if a == 0 {
                    continue;
                }
                let bit = x * bpp;
                let (at, shift) = (row + bit / 8, 8 - bpp - bit % 8);
                let value = if a == 255 {
                    full
                } else {
                    // What shows there now and the mix, as the page means
                    // them (not inverted).
                    let level = u32::from(data[at] >> shift) & max;
                    let under = if inverted {
                        255 - level * step
                    } else {
                        level * step
                    };
                    let mixed = (over * a + under * (255 - a) + 127) / 255;
                    let shown = if inverted { 255 - mixed } else { mixed };
                    ((shown * max + 127) / 255) as u8
                };
                let mask = (max as u8) << shift;
                data[at] = (data[at] & !mask) | ((value << shift) & mask);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::vec::Vec;

    use paperpaw_platform::Rect;

    use super::*;

    /// A random number from `state` (xorshift), for masks of every shape.
    fn next(state: &mut u32) -> u32 {
        *state ^= *state << 13;
        *state ^= *state >> 17;
        *state ^= *state << 5;
        *state
    }

    /// A glyph's coverage: empty and full pixels most of all, as in
    /// letters, and edges of any coverage.
    fn glyph(state: &mut u32, width: usize, height: usize) -> Vec<u8> {
        (0..width * height)
            .map(|_| match next(state) % 8 {
                0..=2 => 0,
                3..=4 => 255,
                _ => next(state) as u8,
            })
            .collect()
    }

    /// Masks drawn a row at a time come out byte for byte as drawn pixel
    /// by pixel: in black and white and grays, in colours light and dark,
    /// inverted or not, at every place in a byte, cut by a clip and moved
    /// by an offset, over what is drawn there.
    #[test]
    fn masks_are_drawn_as_each_pixel_is_blended() {
        let colors = [
            Color::BLACK,
            Color::WHITE,
            Color::gray(90),
            Color::gray(200),
            Color::rgb(200, 30, 90),
        ];
        let mut state = 0x2545_f491;
        for format in [PixelFormat::Mono1, PixelFormat::Gray2, PixelFormat::Gray4] {
            for inverted in [false, true] {
                for _ in 0..400 {
                    let mut fast = Framebuffer::banded(61, 40, 24, format).unwrap();
                    fast.set_top(next(&mut state) as u16 % 17);
                    fast.set_inverted(inverted);
                    fast.set_mono_coverage(next(&mut state) as u8);
                    // Something under the masks to blend with.
                    for _ in 0..4 {
                        let gray = Color::gray(next(&mut state) as u8);
                        let (x, y) = (next(&mut state) % 61, next(&mut state) % 40);
                        let area = Rect::new(x as u16, y as u16, 20, 9);
                        fast.canvas().fill(area, gray);
                    }
                    let mut each = Framebuffer::banded(61, 40, 24, format).unwrap();
                    each.set_top(fast.band().y);
                    each.set_inverted(inverted);
                    each.set_mono_coverage(fast.mono_coverage());
                    each.write_rows(fast.band().y, fast.view());
                    fast.take_dirty();

                    let (width, height) = (next(&mut state) % 23 + 1, next(&mut state) % 19 + 1);
                    let mask = glyph(&mut state, width as usize, height as usize);
                    let level = |dx: u32, dy: u32| mask[(dy * width + dx) as usize];
                    let x = (next(&mut state) % 80) as i32 - 15;
                    let y = (next(&mut state) % 60) as i32 - 15;
                    let (dx, dy) = (
                        (next(&mut state) % 9) as i32 - 4,
                        (next(&mut state) % 9) as i32,
                    );
                    let clip = Rect::new(
                        (next(&mut state) % 30) as u16,
                        (next(&mut state) % 30) as u16,
                        (next(&mut state) % 50) as u16,
                        (next(&mut state) % 40) as u16,
                    );
                    let [r, g, b, pick] = next(&mut state).to_le_bytes();
                    let color = colors
                        .get(usize::from(pick % 8))
                        .copied()
                        .unwrap_or(Color::rgb(r, g, b));
                    let mut canvas = fast.canvas().offset(dx, dy);
                    canvas
                        .clipped(clip)
                        .draw_mask(x, y, width, height, color, level);
                    let mut canvas = each.canvas().offset(dx, dy);
                    canvas.clipped(clip).mask_wide(
                        x.into(),
                        y.into(),
                        width.into(),
                        height.into(),
                        color,
                        |dx, dy| level(dx as u32, dy as u32),
                    );
                    let case = (format, inverted, x, y, width, height, clip, color);
                    assert_eq!(fast.view().data, each.view().data, "{case:?}");
                    assert_eq!(fast.take_dirty(), each.take_dirty(), "{case:?}");
                }
            }
        }
    }
}
