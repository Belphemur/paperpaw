//! Where a block's lines start and how wide each is.

use paperpaw_document::Block;

use crate::{BookStyles, Style, block_em};

/// Where a block's lines start and how wide they are: the first line
/// (indented where the block starts, by half a line at most, perhaps
/// hanging out into the book's margin) and the others, within the book's
/// margins, each kept to a quarter of the width.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Measure {
    pub(crate) first: (f32, f32),
    pub(crate) rest: (f32, f32),
}

impl Measure {
    pub(crate) fn of(block: &Block, style: &Style, width: f32, indent: f32) -> Measure {
        let (start, end) = match style.book.has(BookStyles::MARGINS) {
            true => {
                let em = block_em(block, style);
                let side = |m: u16| (f32::from(m) / 100.0 * em).clamp(0.0, width / 4.0);
                (side(block.margins.0), side(block.margins.1))
            }
            false => (0.0, 0.0),
        };
        let rest = (start, (width - start - end).max(1.0));
        // Indented by half a line at most, so a narrow page keeps room.
        let first_x = (start + indent).clamp(0.0, start + rest.1 / 2.0);
        Measure {
            first: (first_x, (width - end - first_x).max(1.0)),
            rest,
        }
    }

    /// Where line `n` starts and how wide it is.
    pub(crate) fn line(&self, n: usize) -> (f32, f32) {
        if n == 0 { self.first } else { self.rest }
    }
}
