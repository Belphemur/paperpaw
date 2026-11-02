//! Mirroring a layout made from the left where the interface runs right
//! to left (`UiStyle::flip`, `UiStyle::flip_x`).

use paperpaw_sdk::Rect;
use paperpaw_sdk::ui::UiStyle;

const ROW: Rect = Rect::new(20, 100, 400, 60);

fn rtl() -> UiStyle {
    UiStyle::PLAIN.with_rtl(true)
}

#[test]
fn left_to_right_nothing_moves() {
    let ltr = UiStyle::PLAIN.with_rtl(false);
    let cover = Rect::new(28, 104, 40, 52);
    assert_eq!(ltr.flip(ROW, cover), cover);
    assert_eq!(ltr.flip_x(ROW, 76.5, 120.25), 76.5);
}

#[test]
fn right_to_left_a_part_is_as_far_from_the_right_edge() {
    let style = rtl();
    // 8 from the row's left edge, 40 wide: 8 from its right edge.
    let cover = style.flip(ROW, Rect::new(28, 104, 40, 52));
    assert_eq!(cover, Rect::new(372, 104, 40, 52));
    assert_eq!(ROW.x + ROW.width - (cover.x + cover.width), 8);
    // Text 120.25 wide from 76.5: it ends 56.5 from the right edge.
    let x = style.flip_x(ROW, 76.5, 120.25);
    assert_eq!(x, 243.25);
    assert_eq!(f32::from(ROW.x + ROW.width) - (x + 120.25), 56.5);
}

#[test]
fn mirrored_twice_a_part_is_where_it_was() {
    let style = rtl();
    // Every rectangle inside the row.
    for x in [20, 21, 100, 379, 419] {
        for width in [1, 2, 40, 420 - x].into_iter().filter(|w| x + w <= 420) {
            let rect = Rect::new(x, 104, width, 10);
            assert_eq!(style.flip(ROW, style.flip(ROW, rect)), rect, "{rect:?}");
        }
    }
    let x = style.flip_x(ROW, 76.5, 120.25);
    assert_eq!(style.flip_x(ROW, x, 120.25), 76.5);
}

#[test]
fn a_point_mirrors_as_a_rectangle_one_pixel_wide() {
    let style = rtl();
    // The row's first pixel and its last swap.
    assert_eq!(style.flip(ROW, Rect::new(20, 120, 1, 1)).x, 419);
    assert_eq!(style.flip(ROW, Rect::new(419, 120, 1, 1)).x, 20);
}

#[test]
fn a_part_wider_than_the_row_is_never_left_of_the_screen() {
    let style = rtl();
    let wide = style.flip(Rect::new(0, 0, 100, 10), Rect::new(0, 0, 300, 10));
    assert_eq!(wide.x, 0);
    assert_eq!(wide.width, 300);
    // In fractions it may start left of the row: text can be cut there.
    assert_eq!(style.flip_x(Rect::new(0, 0, 100, 10), 0.0, 300.0), -200.0);
}
