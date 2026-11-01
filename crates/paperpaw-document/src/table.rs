//! A table's row ([`TableRow`]): a block of kind
//! [`BlockKind::TableRow`](crate::BlockKind::TableRow) whose text is its
//! cells' texts one after the other, each cell ending where it says.

use alloc::boxed::Box;

use crate::Align;

/// The most columns a table has: wider ones keep their first columns and
/// set the rest's text in the last.
pub const MAX_COLUMNS: usize = 16;

/// A row of a table: its cells, the table's columns, and where in the
/// table it is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct TableRow {
    /// Its cells in reading order, at most [`MAX_COLUMNS`].
    pub cells: Box<[TableCell]>,
    /// The table's columns, as weights of its width (each at least 1):
    /// the same in each of its rows, so that columns line up. Empty: all
    /// alike, as many as the row has cells.
    pub columns: Box<[u16]>,
    /// A header row (`th`, `thead`): set bold, and shown again at the top
    /// of each page the table goes on to.
    pub header: bool,
    /// The table's first row: a rule above it.
    pub first: bool,
    /// The table's last row: a rule below it.
    pub last: bool,
}

/// A cell of a [`TableRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TableCell {
    /// Where its text ends in the block's text (a byte offset, after the
    /// previous cell's end): the cells' texts are the block's, in order.
    pub end: u32,
    /// The columns it spans, at least 1.
    pub span: u8,
    pub align: Align,
}

impl TableCell {
    pub const fn new(end: u32) -> Self {
        Self {
            end,
            span: 1,
            align: Align::Start,
        }
    }

    pub const fn span(mut self, span: u8) -> Self {
        self.span = if span == 0 { 1 } else { span };
        self
    }

    pub const fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }
}

impl TableRow {
    pub fn new(cells: Box<[TableCell]>) -> Self {
        Self {
            cells,
            ..Self::default()
        }
    }

    /// The byte ranges of its cells in a block's `text` of `len` bytes:
    /// each from the previous end to its own, clamped to the text and
    /// never going back (a cell past the text is empty).
    pub fn ranges(&self, len: usize) -> impl Iterator<Item = core::ops::Range<usize>> + '_ {
        let mut start = 0usize;
        self.cells.iter().map(move |cell| {
            let end = (cell.end as usize).clamp(start, len.max(start));
            let range = start.min(len)..end.min(len);
            start = end;
            range
        })
    }

    /// The width weight of each of its cells (a cell's columns added),
    /// one per cell: by [`columns`](Self::columns) where they hold for
    /// the row, else alike.
    pub fn weights(&self) -> impl Iterator<Item = u32> + '_ {
        let mut column = 0usize;
        let fits = self
            .cells
            .iter()
            .map(|c| usize::from(c.span.max(1)))
            .sum::<usize>()
            <= self.columns.len();
        self.cells.iter().map(move |cell| {
            let span = usize::from(cell.span.max(1));
            let weight = if fits {
                self.columns[column..column + span]
                    .iter()
                    .map(|w| u32::from((*w).max(1)))
                    .sum()
            } else {
                span as u32
            };
            column += span;
            weight
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn cells_are_ranges_of_the_text_never_going_back_or_past_it() {
        let row = TableRow::new(
            vec![
                TableCell::new(3),
                TableCell::new(1),
                TableCell::new(9),
                TableCell::new(99),
            ]
            .into_boxed_slice(),
        );
        let ranges: Vec<_> = row.ranges(7).collect();
        assert_eq!(ranges, [0..3, 3..3, 3..7, 7..7]);
        assert_eq!(TableRow::default().ranges(5).count(), 0);
    }

    #[test]
    fn weights_follow_the_columns_where_they_hold() {
        let mut row =
            TableRow::new(vec![TableCell::new(1).span(2), TableCell::new(2)].into_boxed_slice());
        row.columns = vec![1, 3, 2].into_boxed_slice();
        assert_eq!(row.weights().collect::<Vec<_>>(), [4, 2]);
        // Columns that do not hold for the row (too few): alike.
        row.columns = vec![5].into_boxed_slice();
        assert_eq!(row.weights().collect::<Vec<_>>(), [2, 1]);
        // A weight of 0 counts as 1; a span of 0 as 1.
        let mut row = TableRow::new(vec![TableCell::new(1).span(0)].into_boxed_slice());
        row.columns = vec![0].into_boxed_slice();
        assert_eq!(row.weights().collect::<Vec<_>>(), [1]);
    }
}
