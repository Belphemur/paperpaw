//! The firmware's shared function tables, firmware side (ADR 0026, #44;
//! the plugin side is `paperpaw_table::shared!`).
//!
//! [`table`] is what a shell that loads code plugins for a RISC-V chip
//! writes into the family words of every plugin it loads
//! (`PAPERPAW_TABLE_<n>`): the address of each function the SDK and the
//! crates below it share, in the order of `shared.list`, which is the
//! plugin interface. A plugin built for such a chip calls these instead
//! of carrying its own copies.

use paperpaw_table::{Family, Table};

/// A plain array of function addresses, never written.
#[derive(Debug)]
pub struct Addresses<const N: usize>(pub [*const (); N]);

// SAFETY: addresses of functions that live as long as the program runs,
// never written.
unsafe impl<const N: usize> Sync for Addresses<N> {}

// Only where a plugin can call the tables: a RISC-V chip's firmware, and
// a computer's build (the tests of table mode). Elsewhere (the X4 Pro's
// Xtensa chip) the addresses would only keep the firmware's compiler from
// inlining the shared functions into their one caller.
#[cfg(any(target_arch = "riscv32", not(target_os = "none")))]
#[path = "shared_table.rs"]
mod tables;

/// A chip whose plugins carry their own copies: no tables.
#[cfg(not(any(target_arch = "riscv32", not(target_os = "none"))))]
mod tables {
    pub(super) const fn of(_: usize) -> paperpaw_table::Table {
        paperpaw_table::Table::NONE
    }
}

/// The table of `family`, [`Table::NONE`] for one the firmware does not
/// share. The revisions of the families are `paperpaw_table::REVISIONS`.
#[must_use]
pub const fn table(family: Family) -> Table {
    tables::of(family.number())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every family with revisions has a table of one non-null address for
    /// each index of `paperpaw_table::index`; one without has none.
    #[test]
    fn the_tables_are_whole() {
        let families = [
            (Family::TEXT, paperpaw_table::index::text::COUNT),
            (Family::FONTS, paperpaw_table::index::fonts::COUNT),
            (Family::UI, paperpaw_table::index::ui::COUNT),
            (Family::GRAPHICS, paperpaw_table::index::graphics::COUNT),
            (Family::CONTRIBUTION, paperpaw_table::index::contribution::COUNT),
        ];
        for (family, count) in families {
            let t = table(family);
            let revision = paperpaw_table::REVISIONS[family.number()];
            assert_eq!(t.0.is_null(), revision == 0, "family {}", family.number());
            assert_eq!(count == 0, revision == 0, "family {}", family.number());
            for i in 0..count {
                // SAFETY: within the table.
                assert!(!unsafe { *t.0.add(i) }.is_null(), "entry {i}");
            }
        }
    }
}
