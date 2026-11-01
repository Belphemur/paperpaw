//! Broken and hostile database files: wrong headers, odd page sizes,
//! truncated files, cell offsets outside their page, page pointers that
//! loop, huge varints. Each is an error, never a panic and never a loop
//! without end (a storage that stops answering after a budget of reads
//! turns a hang into a failed test).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{Memory, PAGE, read_rows, text, write_table};
use paperpaw_platform::StorageError;
use paperpaw_sqlite::{Database, Error, Row, Value, decode_record};
use proptest::prelude::*;

/// A database whose table `t` is a single leaf, page 2.
fn leaf_db() -> Vec<u8> {
    let storage = Memory::default();
    let root = write_table(
        &storage,
        &[
            (1, vec![Value::Int(1)]),
            (2, vec![text("two")]),
            (3, vec![Value::Blob(vec![3; 30])]),
        ],
    );
    assert_eq!(root, 2);
    storage.get("db").unwrap()
}

/// An interior table page with a cell per child and a right-most child.
fn interior(children: &[u32], right: u32) -> Vec<u8> {
    let mut page = vec![0u8; PAGE];
    page[0] = 0x05;
    page[3..5].copy_from_slice(&(children.len() as u16).to_be_bytes());
    page[8..12].copy_from_slice(&right.to_be_bytes());
    let mut content = PAGE;
    for (i, child) in children.iter().enumerate() {
        content -= 5;
        page[content..content + 4].copy_from_slice(&child.to_be_bytes());
        page[content + 4] = i as u8 + 1;
        page[12 + 2 * i..14 + 2 * i].copy_from_slice(&(content as u16).to_be_bytes());
    }
    page[5..7].copy_from_slice(&(content as u16).to_be_bytes());
    page
}

/// `leaf_db` with page 2 replaced by `page` and the old leaf as page 3.
fn with_root(page: Vec<u8>) -> Vec<u8> {
    let file = leaf_db();
    let mut out = file[..PAGE].to_vec();
    out.extend_from_slice(&page);
    out.extend_from_slice(&file[PAGE..2 * PAGE]);
    counted(out)
}

/// `file` with the header's page count set to the pages it holds: the
/// reader takes no page past the count.
fn counted(mut file: Vec<u8>) -> Vec<u8> {
    let pages = (file.len() / PAGE) as u32;
    file[28..32].copy_from_slice(&pages.to_be_bytes());
    file
}

/// Opens `file` and reads its schema and every row of `t`.
fn read_all(storage: &Memory) -> Result<Vec<(i64, Vec<u8>)>, Error> {
    let db = Database::open(storage, "db")?;
    let (_, root) = db.table("t")?;
    read_rows(&db, root)
}

fn open(file: Vec<u8>) -> Result<(), Error> {
    let storage = Memory::with("db", file);
    Database::open(&storage, "db").map(|_| ())
}

#[test]
fn files_that_are_not_sqlite_are_refused() {
    let good = leaf_db();
    assert_eq!(open(Vec::new()), Err(Error::NotSqlite));
    assert_eq!(open(good[..99].to_vec()), Err(Error::NotSqlite));
    let mut magic = good.clone();
    magic[..16].copy_from_slice(b"SQLite format 2\0");
    assert_eq!(open(magic), Err(Error::NotSqlite));
    assert_eq!(
        open(b"PK\x03\x04 a zip file, not a database".repeat(10)),
        Err(Error::NotSqlite)
    );
    let storage = Memory::default();
    assert_eq!(
        Database::open(&storage, "missing.db").err(),
        Some(Error::Storage(StorageError::NotFound))
    );
}

#[test]
fn page_sizes_that_are_not_powers_of_two_are_corrupt() {
    for size in [0u16, 3, 256, 511, 1000, 4097, 32_769, 65_535] {
        let mut file = leaf_db();
        file[16..18].copy_from_slice(&size.to_be_bytes());
        assert_eq!(open(file), Err(Error::Corrupt("page size")), "{size}");
    }
    // 1 stands for 65536: valid, but this file is too short for it, so
    // it holds no page at all.
    let mut file = leaf_db();
    file[16..18].copy_from_slice(&1u16.to_be_bytes());
    let storage = Memory::with("db", file);
    let db = Database::open(&storage, "db").unwrap();
    assert_eq!(db.schema(), Err(Error::Corrupt("page number")));
}

#[test]
fn utf16_text_is_refused() {
    for encoding in [2u32, 3, 0xffff_ffff] {
        let mut file = leaf_db();
        file[56..60].copy_from_slice(&encoding.to_be_bytes());
        assert_eq!(open(file), Err(Error::NotSqlite), "{encoding}");
    }
}

/// Cut anywhere, a file reads as an error: no rows from half a page.
#[test]
fn truncated_files_are_errors() {
    let file = leaf_db();
    assert_eq!(
        read_all(&Memory::with("db", file.clone())).unwrap().len(),
        3
    );
    for len in (0..file.len())
        .step_by(41)
        .chain([PAGE - 1, PAGE + 1, file.len() - 1])
    {
        let storage = Memory::with("db", file[..len].to_vec());
        assert!(read_all(&storage).is_err(), "cut at {len}");
    }
}

#[test]
fn unknown_page_types_are_corrupt() {
    for kind in [0u8, 0x02, 0x0a, 0x0c, 0xff] {
        let mut file = leaf_db();
        file[PAGE] = kind;
        let storage = Memory::with("db", file);
        assert_eq!(
            read_all(&storage),
            Err(Error::Corrupt("not a table page")),
            "{kind}"
        );
    }
}

#[test]
fn cell_offsets_outside_the_page_are_corrupt() {
    // The first cell pointer of the leaf, past the page's end.
    let mut file = leaf_db();
    file[PAGE + 8..PAGE + 10].copy_from_slice(&0xffffu16.to_be_bytes());
    let storage = Memory::with("db", file);
    assert_eq!(read_all(&storage), Err(Error::Corrupt("cell offset")));
    // More cells than pointers fit in the page.
    let mut file = leaf_db();
    file[PAGE + 3..PAGE + 5].copy_from_slice(&0xffffu16.to_be_bytes());
    let storage = Memory::with("db", file);
    assert!(read_all(&storage).is_err());
    // A cell at the very last byte: its sizes run off the page.
    let mut file = leaf_db();
    file[PAGE + 8..PAGE + 10].copy_from_slice(&(PAGE as u16 - 1).to_be_bytes());
    file[2 * PAGE - 1] = 0x7f;
    let storage = Memory::with("db", file);
    assert_eq!(read_all(&storage), Err(Error::Corrupt("cell")));
}

/// An interior cell whose offset leaves less than the four bytes of its
/// child's page number in the page.
#[test]
fn an_interior_cell_at_the_end_of_its_page_is_an_error_not_a_panic() {
    let mut page = interior(&[3], 3);
    page[12..14].copy_from_slice(&(PAGE as u16 - 2).to_be_bytes());
    let storage = Memory::with("db", with_root(page));
    assert!(matches!(read_all(&storage), Err(Error::Corrupt(_))));
}

#[test]
fn child_pages_that_do_not_exist_are_corrupt() {
    // A valid tree first: the interior page and its leaf.
    let storage = Memory::with("db", with_root(interior(&[], 3)));
    assert_eq!(read_all(&storage).unwrap().len(), 3);
    for missing in [0, 4, 999, u32::MAX] {
        let storage = Memory::with("db", with_root(interior(&[3], missing)));
        assert_eq!(
            read_all(&storage),
            Err(Error::Corrupt("page number")),
            "{missing}"
        );
    }
    let storage = Memory::with("db", with_root(interior(&[u32::MAX], 3)));
    assert_eq!(read_all(&storage), Err(Error::Corrupt("page number")));
    // A page past the header's page count is not taken, though the file
    // holds it.
    let mut file = with_root(interior(&[], 3));
    file[28..32].copy_from_slice(&2u32.to_be_bytes());
    let storage = Memory::with("db", file);
    assert_eq!(read_all(&storage), Err(Error::Corrupt("page number")));
}

/// A page that is its own right-most child: a scan reads each page once,
/// so the second visit ends it.
#[test]
fn a_page_that_points_to_itself_is_corrupt() {
    let storage = Memory::with("db", with_root(interior(&[], 2)));
    storage.budget.set(Some(1000));
    assert_eq!(read_all(&storage), Err(Error::Corrupt("page used twice")));
    assert!(storage.reads.get() < 50, "{} reads", storage.reads.get());
}

/// A page whose several children are all itself: the first page that
/// comes again ends the whole scan, so the pages read do not multiply
/// level by level.
#[test]
fn a_page_that_is_its_own_child_many_times_is_corrupt_soon() {
    let storage = Memory::with("db", with_root(interior(&[3, 2, 2], 2)));
    storage.budget.set(Some(10_000));
    assert_eq!(read_all(&storage), Err(Error::Corrupt("page used twice")));
    assert!(storage.reads.get() < 100, "{} reads", storage.reads.get());
}

/// A leaf cell whose payload size is a nine-byte varint.
#[test]
fn a_huge_payload_size_is_an_error() {
    let mut file = leaf_db();
    let cell = usize::from(u16::from_be_bytes([file[PAGE + 8], file[PAGE + 9]]));
    // The payload size: nine bytes of all ones; the row id after it and
    // the overflow pointer it would need are whatever follows.
    let mut cut = file[..PAGE + cell].to_vec();
    cut.extend_from_slice(&[0xff; 9]);
    cut.push(1);
    cut.resize(2 * PAGE, 0);
    file = cut;
    let storage = Memory::with("db", file);
    storage.budget.set(Some(10_000));
    let result = read_all(&storage);
    assert!(matches!(result, Err(Error::Corrupt(_))), "{result:?}");
}

/// The part of a table leaf payload of `size` bytes kept in its cell
/// (SQLite's file format, pages of 4096 bytes).
fn local_size(size: u64) -> usize {
    let (max, min) = (4096 - 35, (4096 - 12) * 32 / 255 - 23);
    if size <= max {
        return size as usize;
    }
    let k = min + ((size - min) % (4096 - 4));
    (if k <= max { k } else { min }) as usize
}

/// A leaf whose only row overflows into page 3, page 3 pointing on to
/// `next`.
fn overflowing(size: u64, next: u32) -> Vec<u8> {
    let file = leaf_db();
    let mut leaf = vec![0u8; PAGE];
    leaf[0] = 0x0d;
    leaf[3..5].copy_from_slice(&1u16.to_be_bytes());
    let mut cell = Vec::new();
    // The size as a varint (small enough for eight bytes), row id 1.
    let mut v = size;
    let mut bytes = vec![(v & 0x7f) as u8];
    v >>= 7;
    while v > 0 {
        bytes.push((v & 0x7f) as u8 | 0x80);
        v >>= 7;
    }
    bytes.reverse();
    cell.extend_from_slice(&bytes);
    cell.push(1);
    // The part kept in the cell, then the first overflow page.
    cell.resize(cell.len() + local_size(size), 0);
    cell.extend_from_slice(&3u32.to_be_bytes());
    let start = PAGE - cell.len();
    leaf[start..].copy_from_slice(&cell);
    leaf[5..7].copy_from_slice(&(start as u16).to_be_bytes());
    leaf[8..10].copy_from_slice(&(start as u16).to_be_bytes());
    let mut overflow = vec![0u8; PAGE];
    overflow[..4].copy_from_slice(&next.to_be_bytes());
    let mut out = file[..PAGE].to_vec();
    out.extend_from_slice(&leaf);
    out.extend_from_slice(&overflow);
    counted(out)
}

/// A payload size that needs two overflow pages but is no longer than
/// the three pages of [`overflowing`]'s file.
const TWO_OVERFLOW_PAGES: u64 = 9_000;

#[test]
fn broken_overflow_chains_are_corrupt() {
    // The chain ends before the payload does.
    let storage = Memory::with("db", overflowing(TWO_OVERFLOW_PAGES, 0));
    assert_eq!(read_all(&storage), Err(Error::Corrupt("overflow chain")));
    // It goes on to a page that is not there.
    let storage = Memory::with("db", overflowing(TWO_OVERFLOW_PAGES, 77));
    assert_eq!(read_all(&storage), Err(Error::Corrupt("page number")));
    // A payload longer than the whole file is refused before any of it is
    // read.
    let storage = Memory::with("db", overflowing(20_000, 0));
    assert_eq!(read_all(&storage), Err(Error::Corrupt("payload size")));
}

/// An overflow page that is its own next page, under a payload said to
/// be a terabyte or one that fits in the file: the chain must end once it
/// comes back to a page, not after a million pages (and gigabytes of
/// memory).
#[test]
fn a_looping_overflow_chain_is_corrupt_soon() {
    for size in [1 << 40, TWO_OVERFLOW_PAGES] {
        let storage = Memory::with("db", overflowing(size, 3));
        storage.budget.set(Some(10_000));
        let result = read_all(&storage);
        assert!(
            matches!(result, Err(Error::Corrupt(_))),
            "{size}: {result:?}"
        );
        assert!(storage.reads.get() < 50, "{} reads", storage.reads.get());
    }
}

/// A row a bounded scan saw: its record, or its payload size.
type Bounded = Result<Vec<u8>, u64>;

/// The rows of `t` a scan bounded to `max_payload` bytes sees: each one's
/// id and its record, or its payload size if it was passed over.
fn read_bounded(storage: &Memory, max_payload: usize) -> Result<Vec<(i64, Bounded)>, Error> {
    let db = Database::open(storage, "db")?;
    let (_, root) = db.table("t")?;
    let mut rows = Vec::new();
    db.scan_bounded(root, max_payload, &mut |id, row| {
        rows.push((
            id,
            match row {
                Row::Values(values) => Ok(paperpaw_sqlite::encode_record(values)),
                Row::Oversized(size) => Err(size),
            },
        ));
        true
    })?;
    Ok(rows)
}

/// A bounded scan passes over a payload past its limit without reading
/// its overflow chain, so a broken chain there goes unseen; but a payload
/// longer than the file, or a first overflow page that is not there, is
/// still corrupt, and a payload within the limit is read and checked.
#[test]
fn a_bounded_scan_still_refuses_corrupt_payload_sizes() {
    let storage = Memory::with("db", overflowing(20_000, 0));
    assert_eq!(
        read_bounded(&storage, 1000),
        Err(Error::Corrupt("payload size"))
    );
    let storage = Memory::with("db", overflowing(1 << 40, 0));
    assert_eq!(
        read_bounded(&storage, 1000),
        Err(Error::Corrupt("payload size"))
    );
    // A chain that loops: passed over unread, or refused once read.
    let storage = Memory::with("db", overflowing(TWO_OVERFLOW_PAGES, 3));
    storage.budget.set(Some(10_000));
    assert_eq!(
        read_bounded(&storage, 1000),
        Ok(vec![(1, Err(TWO_OVERFLOW_PAGES))])
    );
    assert!(storage.reads.get() < 10, "{} reads", storage.reads.get());
    assert_eq!(
        read_bounded(&storage, TWO_OVERFLOW_PAGES as usize),
        Err(Error::Corrupt("page used twice"))
    );
    // The first overflow page is past the end of the file.
    let mut file = overflowing(TWO_OVERFLOW_PAGES, 0);
    file[2 * PAGE - 4..2 * PAGE].copy_from_slice(&77u32.to_be_bytes());
    let storage = Memory::with("db", file);
    assert_eq!(
        read_bounded(&storage, 1000),
        Err(Error::Corrupt("page number"))
    );
    // The last cell of the page claims more than the page holds after it.
    let mut file = leaf_db();
    let last = (0..3)
        .map(|i| {
            usize::from(u16::from_be_bytes([
                file[PAGE + 8 + 2 * i],
                file[PAGE + 9 + 2 * i],
            ]))
        })
        .max()
        .unwrap();
    assert!(PAGE - last < 100);
    file[PAGE + last] = 127;
    let storage = Memory::with("db", file);
    assert_eq!(read_bounded(&storage, 10), Err(Error::Corrupt("payload")));
}

/// A record whose header claims to be as large as a u64 can be.
#[test]
fn a_record_header_of_huge_size_is_malformed_not_a_panic() {
    let mut record = vec![0xff; 9];
    // A one-byte integer, then its byte.
    record.extend_from_slice(&[1, 42]);
    assert_eq!(decode_record(&record), None);
}

#[test]
fn malformed_records_are_none() {
    // Header longer than the record.
    assert_eq!(decode_record(&[10, 1]), None);
    // A value running past the end.
    assert_eq!(decode_record(&[2, 6, 0, 0]), None);
    // A text of a million bytes in a few.
    assert_eq!(decode_record(&[4, 0x87, 0xa1, 0x0d, b'a']), None);
    // A cut varint.
    assert_eq!(decode_record(&[0x81]), None);
    assert_eq!(decode_record(&[]), None);
    // Text that is not UTF-8 is read, not refused.
    assert_eq!(
        decode_record(&[2, 17, 0xff, 0xfe]),
        Some(vec![Value::Text("\u{fffd}\u{fffd}".to_string())])
    );
    // Reserved serial types 10 and 11 read as NULL.
    assert_eq!(
        decode_record(&[3, 10, 11]),
        Some(vec![Value::Null, Value::Null])
    );
}

/// A database without interior pages: a leaf with a short row and one
/// that goes on in two overflow pages.
fn small_db() -> Vec<u8> {
    let storage = Memory::default();
    write_table(
        &storage,
        &[
            (1, vec![text("a"), Value::Int(-7), Value::Real(0.5)]),
            (2, vec![text(&"o".repeat(9000))]),
        ],
    );
    storage.get("db").unwrap()
}

fn mutations() -> impl Strategy<Value = Vec<(usize, u8)>> {
    prop::collection::vec((0usize..4 * PAGE, any::<u8>()), 1..8)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(std::env::var("PROPTEST_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(256)))]

    /// Where bytes of the file are changed, it is read or refused,
    /// without a panic and in a bounded number of reads. (The page type
    /// bytes stay, so no interior page comes up.)
    #[test]
    fn changed_bytes_never_panic(changes in mutations()) {
        let mut file = small_db();
        for (at, byte) in changes {
            if at < file.len() && at != 100 && at % PAGE != 0 {
                file[at] = byte;
            }
        }
        let storage = Memory::with("db", file);
        storage.budget.set(Some(10_000));
        if let Ok(db) = Database::open(&storage, "db")
            && let Ok(schema) = db.schema()
        {
            for entry in schema {
                let _ = db.scan(entry.root, &mut |_, _| true);
                let _ = db.scan_bounded(entry.root, 100, &mut |_, _| true);
            }
        }
        prop_assert!(storage.reads.get() <= 10_002);
    }
}
