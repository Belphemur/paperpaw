//! A bounded scan passes over a row too large for the memory at hand: on
//! the X4 a 200 KiB row (a picture kept in a note) cannot be allocated,
//! and without a limit it fails the whole table. An allocator that
//! refuses what goes past a budget stands in for the X4's heap.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::{Memory, text, write_table};
use paperpaw_sqlite::{Database, Error, Row, Value, encode_record};

/// The system allocator, noting how much each thread holds at most and
/// refusing what would go past the thread's limit.
struct Limited;

thread_local! {
    static HELD: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
    static LIMIT: Cell<usize> = const { Cell::new(usize::MAX) };
}

/// Notes `size` more bytes held, unless that goes past the limit.
fn grow(size: usize) -> bool {
    HELD.try_with(|held| {
        let now = held.get() + size;
        if now > LIMIT.try_with(Cell::get).unwrap_or(usize::MAX) {
            return false;
        }
        held.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
        true
    })
    .unwrap_or(true)
}

fn shrink(size: usize) {
    let _ = HELD.try_with(|held| held.set(held.get().saturating_sub(size)));
}

unsafe impl GlobalAlloc for Limited {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !grow(layout.size()) {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !grow(layout.size()) {
            return std::ptr::null_mut();
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if !grow(new_size) {
            return std::ptr::null_mut();
        }
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if new.is_null() {
            shrink(new_size);
        } else {
            shrink(layout.size());
        }
        new
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        shrink(layout.size());
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Limited = Limited;

/// Runs `f` with at most `budget` bytes more than are held now, and says
/// the most it held at once.
fn within<T>(budget: usize, f: impl FnOnce() -> T) -> (T, usize) {
    let before = HELD.with(Cell::get);
    PEAK.with(|peak| peak.set(before));
    LIMIT.with(|limit| limit.set(before + budget));
    let result = f();
    LIMIT.with(|limit| limit.set(usize::MAX));
    (result, PEAK.with(Cell::get) - before)
}

/// A budget like what a plugin has on the X4 beside everything else.
const BUDGET: usize = 48 * 1024;

/// Rows 1 and 3 small, row 2 a 200 KiB text between them.
fn table() -> (Memory, Vec<Vec<Value>>) {
    let rows = vec![
        vec![text("one"), Value::Int(1)],
        vec![text(&"x".repeat(200 * 1024)), Value::Int(2)],
        vec![text("three"), Value::Int(3)],
    ];
    let storage = Memory::default();
    let numbered: Vec<(i64, Vec<Value>)> = (1..).zip(rows.iter().cloned()).collect();
    write_table(&storage, &numbered);
    (storage, rows)
}

#[test]
fn a_huge_row_fails_an_unbounded_scan_within_the_budget() {
    let (storage, _) = table();
    let db = Database::open(&storage, "db").unwrap();
    let (_, root) = db.table("t").unwrap();
    let (result, _) = within(BUDGET, || db.scan(root, &mut |_, _| true));
    assert_eq!(result, Err(Error::OutOfMemory));
}

#[test]
fn a_bounded_scan_passes_over_a_huge_row_within_the_budget() {
    let (storage, rows) = table();
    let huge = encode_record(&rows[1]).len() as u64;
    let db = Database::open(&storage, "db").unwrap();
    let (_, root) = db.table("t").unwrap();
    let reads = storage.reads.get();
    let mut seen = Vec::with_capacity(3);
    let (result, peak) = within(BUDGET, || {
        db.scan_bounded(root, 16 * 1024, &mut |id, row| {
            seen.push(match row {
                Row::Values(values) => (id, Ok(values.to_vec())),
                Row::Oversized(size) => (id, Err(size)),
            });
            true
        })
    });
    assert_eq!(result, Ok(()));
    assert_eq!(
        seen,
        [
            (1, Ok(rows[0].clone())),
            (2, Err(huge)),
            (3, Ok(rows[2].clone())),
        ]
    );
    // A page and the small rows, not the huge one; and none of its fifty
    // overflow pages read.
    assert!(peak < 8 * 1024, "{peak} bytes");
    let reads = storage.reads.get() - reads;
    assert!(reads < 5, "{reads} reads");
    // With room for it, the row is read as it was written.
    let mut read = Vec::new();
    db.scan_bounded(root, huge as usize, &mut |_, row| {
        read.push(row == Row::Values(&rows[read.len()]));
        true
    })
    .unwrap();
    assert_eq!(read, [true; 3]);
}
