//! Moving rows of a list: by dragging their grip ([`Reorder`]) and with
//! the buttons (a row moved up or down a place at a time). Both give the
//! same order, and no item is ever lost or doubled; a finger that does
//! not start on a grip, or is another finger, leaves the rows alone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod support;

use paperpaw_sdk::ui::reorder::Reorder;
use paperpaw_sdk::ui::reorder::ReorderStep;
use paperpaw_sdk::ui::reorder::moved;
use paperpaw_sdk::ui::{ListView, UiStyle};
use paperpaw_sdk::{PointerPhase, Rect};
use proptest::prelude::*;

use support::{drawn_outside, frame, middle, pointer};

/// Sizes for tests: no fonts needed.
fn style() -> UiStyle {
    UiStyle::PLAIN
}

/// Rows 50 high, 8 a page.
fn list() -> ListView {
    ListView::new(Rect::new(0, 100, 400, 400), 50)
}

/// Where a finger takes item `index` (on the page with the focus) by its
/// grip.
fn grip_of(list: &ListView, index: usize) -> (i32, i32) {
    let slot = index - list.page() * list.rows();
    let grip = middle(Reorder::grip(list.row_rect(slot), &style()));
    (grip.x, grip.y)
}

/// Item `from` moved to `to` as the buttons do it: a place at a time.
fn by_buttons<T>(items: &mut [T], from: usize, to: usize) {
    if from < to {
        for at in from..to {
            items.swap(at, at + 1);
        }
    } else {
        for at in (to..from).rev() {
            items.swap(at, at + 1);
        }
    }
}

/// Drags item `from` to the row of `to` and lets go.
fn drag(
    reorder: &mut Reorder,
    list: &ListView,
    movable: usize,
    from: usize,
    to: usize,
) -> ReorderStep {
    let (x, y) = grip_of(list, from);
    let down = reorder.on_pointer(&pointer(PointerPhase::Down, x, y), list, movable);
    assert_eq!(down, Some(ReorderStep::Changed));
    let slot = to - list.page() * list.rows();
    let target = middle(list.row_rect(slot));
    // Along the way, a little off the row's middle: the same place.
    reorder.on_pointer(&pointer(PointerPhase::Move, x, target.y - 3), list, movable);
    reorder
        .on_pointer(&pointer(PointerPhase::Up, x, target.y), list, movable)
        .unwrap()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(std::env::var("PROPTEST_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(256)))]

    /// Dragged or moved with the buttons, an item lands at the same
    /// place, the others keeping their order: the same list either way.
    #[test]
    fn dragging_and_buttons_give_the_same_order(
        len in 1usize..30,
        page_from in 0usize..8,
        page_to in 0usize..8,
        page in 0usize..4,
    ) {
        let mut list = list();
        let rows = list.rows();
        let first = (page * rows).min((len - 1) / rows * rows);
        list.set_focus(first);
        let on_page = (len - first).min(rows);
        let (from, to) = (first + page_from % on_page, first + page_to % on_page);
        let mut reorder = Reorder::default();
        let step = drag(&mut reorder, &list, len, from, to);
        prop_assert_eq!(step, ReorderStep::Dropped { from, to });
        prop_assert_eq!(reorder.dragging(), None);
        let mut dragged: Vec<usize> = (0..len).collect();
        moved(&mut dragged, from, to);
        let mut pressed: Vec<usize> = (0..len).collect();
        by_buttons(&mut pressed, from, to);
        prop_assert_eq!(&dragged, &pressed);
        prop_assert_eq!(dragged[to], from);
    }

    /// Any number of moves, in or out of range: every item is still
    /// there once.
    #[test]
    fn moves_never_lose_or_double_an_item(
        len in 0usize..20,
        moves in proptest::collection::vec((0usize..25, 0usize..25), 0..30),
    ) {
        let mut items: Vec<usize> = (0..len).collect();
        for (from, to) in moves {
            let before = items.clone();
            moved(&mut items, from, to);
            if from >= len || to >= len {
                prop_assert_eq!(&items, &before, "out of range: as it was");
            }
            let mut sorted = items.clone();
            sorted.sort_unstable();
            prop_assert_eq!(sorted, (0..len).collect::<Vec<_>>());
        }
    }

    /// Wherever the finger is let go (above the list, below it, past the
    /// movable rows, on another page's rows), the row lands among the
    /// movable rows of the page it was taken on.
    #[test]
    fn a_row_lands_among_the_movable_rows_of_its_page(
        len in 1usize..30,
        movable in 1usize..30,
        y in -200i32..900,
        page in 0usize..4,
    ) {
        let movable = movable.min(len);
        let mut list = list();
        let rows = list.rows();
        list.set_focus((page * rows).min((movable - 1) / rows * rows));
        let first = list.page() * rows;
        let mut reorder = Reorder::default();
        let (x, gy) = grip_of(&list, first);
        reorder.on_pointer(&pointer(PointerPhase::Down, x, gy), &list, movable).unwrap();
        let step = reorder.on_pointer(&pointer(PointerPhase::Up, x, y), &list, movable);
        let Some(ReorderStep::Dropped { from, to }) = step else {
            return Err(TestCaseError::fail(format!("{step:?}")));
        };
        prop_assert_eq!(from, first);
        prop_assert!(to >= first && to < (first + rows).min(movable), "{to}");
    }
}

#[test]
fn only_a_finger_on_a_movable_rows_grip_takes_it() {
    let list = list();
    let mut reorder = Reorder::default();
    // On the row but not its grip: left to the screen's tap.
    let row = list.row_rect(0);
    assert_eq!(
        reorder.on_pointer(&pointer(PointerPhase::Down, 20, middle(row).y), &list, 3),
        None
    );
    // The grip of a row that cannot move (past the movable ones).
    let (x, y) = grip_of(&list, 5);
    assert_eq!(
        reorder.on_pointer(&pointer(PointerPhase::Down, x, y), &list, 3),
        None
    );
    // Moves and lifts without a row taken: not the reorder's.
    assert_eq!(
        reorder.on_pointer(&pointer(PointerPhase::Move, x, y), &list, 3),
        None
    );
    assert_eq!(
        reorder.on_pointer(&pointer(PointerPhase::Up, x, y), &list, 3),
        None
    );
    assert_eq!(reorder.dragging(), None);
}

#[test]
fn another_finger_does_not_move_the_row_and_a_cancel_drops_nothing() {
    let list = list();
    let mut reorder = Reorder::default();
    let (x, y) = grip_of(&list, 1);
    reorder
        .on_pointer(&pointer(PointerPhase::Down, x, y), &list, 6)
        .unwrap();
    let mut other = pointer(PointerPhase::Move, x, y + 200);
    other.id = 1;
    assert_eq!(reorder.on_pointer(&other, &list, 6), None);
    other.phase = PointerPhase::Up;
    assert_eq!(reorder.on_pointer(&other, &list, 6), None);
    assert_eq!(reorder.dragging(), Some((1, 1)), "still the first finger's");
    // Taken away by the system: nothing dropped, the row as it was.
    let step = reorder.on_pointer(&pointer(PointerPhase::Cancel, x, y + 200), &list, 6);
    assert_eq!(step, Some(ReorderStep::Changed));
    assert_eq!(reorder.dragging(), None);
    assert_eq!(
        reorder.on_pointer(&pointer(PointerPhase::Up, x, y + 200), &list, 6),
        None
    );
}

#[test]
fn the_grip_is_a_square_at_the_rows_end_and_the_content_the_rest() {
    let row = Rect::new(10, 20, 300, 50);
    let style = style();
    let grip = Reorder::grip(row, &style);
    assert_eq!(grip, Rect::new(260, 20, 50, 50));
    let content = Reorder::content(row, &style);
    assert_eq!(content, Rect::new(10, 20, 250, 50));
    assert_eq!(content.right(), u32::from(grip.x), "side by side");
    // A row narrower than high: the grip takes all of it.
    let narrow = Rect::new(0, 0, 30, 50);
    assert_eq!(Reorder::grip(narrow, &style), narrow);
    assert_eq!(Reorder::content(narrow, &style).width, 0);
}

/// Right to left, the row ends at the left: the grip is there, and a
/// finger takes the row there once the list was drawn that way.
#[test]
fn right_to_left_the_grip_is_at_the_left() {
    let rtl = style().with_rtl(true);
    let row = Rect::new(10, 20, 300, 50);
    let grip = Reorder::grip(row, &rtl);
    assert_eq!(grip, Rect::new(10, 20, 50, 50));
    let content = Reorder::content(row, &rtl);
    assert_eq!(content, Rect::new(60, 20, 250, 50));
    assert_eq!(grip.right(), u32::from(content.x), "side by side");

    let list = list();
    let mut reorder = Reorder::default();
    let mut frame = frame(400, 600);
    let harness = support::Harness::new(support::Device::BUTTONS);
    let env = harness.env();
    reorder.draw(
        &mut paperpaw_sdk::ui::Ui::new(&mut frame.canvas(), &env, &rtl),
        &list,
    );
    let left = middle(Reorder::grip(list.row_rect(1), &rtl));
    let right = middle(Reorder::grip(list.row_rect(1), &style()));
    let down = |x, y| pointer(PointerPhase::Down, x, y);
    assert_eq!(reorder.on_pointer(&down(right.x, right.y), &list, 8), None);
    assert_eq!(
        reorder.on_pointer(&down(left.x, left.y), &list, 8),
        Some(ReorderStep::Changed)
    );
    assert_eq!(reorder.dragging(), Some((1, 1)));
}

/// The bar lies on the line between two rows, half on either side: at
/// the first and last rows, half of it past the list.
#[test]
fn the_bar_where_a_row_lands_is_drawn_in_the_list() {
    let half = (u32::from(style().line()) * 2) as u16;
    for (from, to) in [(0, 7), (7, 0), (3, 4), (2, 2)] {
        let list = list();
        let mut reorder = Reorder::default();
        let (x, y) = grip_of(&list, from);
        reorder
            .on_pointer(&pointer(PointerPhase::Down, x, y), &list, 8)
            .unwrap();
        let target = middle(list.row_rect(to));
        reorder.on_pointer(&pointer(PointerPhase::Move, x, target.y), &list, 8);
        assert_eq!(reorder.dragging(), Some((from, to)));
        let mut frame = frame(400, 600);
        let harness = support::Harness::new(support::Device::BUTTONS);
        let env = harness.env();
        reorder.draw(
            &mut paperpaw_sdk::ui::Ui::new(&mut frame.canvas(), &env, &style()),
            &list,
        );
        let a = list.area();
        let bound = Rect::new(a.x, a.y - half, a.width, a.height + 2 * half);
        assert_eq!(
            drawn_outside(&mut frame, bound),
            None,
            "from {from} to {to}"
        );
    }
}
