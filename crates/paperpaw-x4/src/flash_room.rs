//! How much room a full flash would give code plugins on the X4, as the
//! store says it where an SD install from CrossPoint left the stock
//! partition table (slots of 6,553,600 bytes against PaperPaw's own of
//! 8,323,072).
//!
//! The raw difference of the slots is not the room: plugins run through
//! the chip's MMU entries that the firmware leaves free (64 KiB each, 128
//! in all, the firmware's own image taking most), so the region behind the
//! image can be no longer than those entries span. This is that bound, on
//! a computer-tested function; the firmware (`native/flash.rs`) gives it
//! what it found.

/// The bytes behind a firmware image that code plugins could use in a slot
/// of PaperPaw's own layout (`own_slot` bytes long): the image starting at
/// `slot_offset` in flash, the region behind it at `start`, and at most
/// `free_pages` MMU entries of `page` bytes to map it with (the first page
/// of the region counted from its own start, so `start % page` is lost).
pub fn full_flash_len(
    start: u32,
    slot_offset: u32,
    own_slot: u32,
    free_pages: u32,
    page: u32,
) -> u32 {
    let behind_slot = own_slot.saturating_sub(start.saturating_sub(slot_offset));
    let mapped = free_pages
        .saturating_mul(page)
        .saturating_sub(start % page.max(1));
    behind_slot.min(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: u32 = 0x1_0000;
    const OWN: u32 = 0x7F_0000;

    #[test]
    fn the_slot_bounds_it_where_the_mmu_has_entries_enough() {
        // An image of 0x300000 bytes: 0x310000 is the end of its sector
        // run; 100 entries span more than the slot has behind it.
        let start = 0x10000 + 0x30_0000;
        assert_eq!(
            full_flash_len(start, 0x10000, OWN, 100, PAGE),
            OWN - 0x30_0000
        );
    }

    #[test]
    fn the_mmu_bounds_it_where_it_has_fewer_entries_than_the_slot() {
        // An image of 5.4 MB leaves about 43 of 128 entries: 2.7 MB, not
        // the 2.9 MB the slot has behind the image.
        let image = 5_424_656u32.next_multiple_of(4096);
        let start = 0x10000 + image;
        let len = full_flash_len(start, 0x10000, OWN, 43, PAGE);
        assert_eq!(len, 43 * PAGE - start % PAGE);
        assert!(len < OWN - image);
    }

    #[test]
    fn the_region_does_not_start_at_a_page_so_that_part_of_its_first_is_lost() {
        let start = 0x10000 + 0x1000;
        assert_eq!(
            full_flash_len(start, 0x10000, OWN, 2, PAGE),
            2 * PAGE - 0x1000
        );
    }

    #[test]
    fn it_never_wraps_on_odd_numbers() {
        assert_eq!(full_flash_len(0, 0x10000, OWN, 0, PAGE), 0);
        assert_eq!(full_flash_len(u32::MAX, 0, OWN, u32::MAX, PAGE), 0);
        assert_eq!(full_flash_len(1, 0, 0, 5, 0), 0);
    }
}
