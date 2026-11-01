//! Any bytes as an SQLite database (as the statistics sync fetches one
//! over WebDAV): read the schema and every table, whole and with a limit
//! on row size.

#![no_main]

use libfuzzer_sys::fuzz_target;
use paperpaw_sdk::MemoryStorage;
use paperpaw_sqlite::Database;

/// Rows visited per table at most, so a large valid table stays quick.
const MAX_ROWS: usize = 10_000;

fuzz_target!(|data: &[u8]| {
    let storage = MemoryStorage::new().with("db.sqlite3", data.to_vec());
    let Ok(db) = Database::open(&storage, "db.sqlite3") else {
        return;
    };
    let Ok(schema) = db.schema() else {
        return;
    };
    for entry in schema.iter().filter(|e| e.kind == "table").take(8) {
        let _ = db.table(&entry.name);
        let mut rows = 0;
        let _ = db.scan(entry.root, &mut |_, _| {
            rows += 1;
            rows < MAX_ROWS
        });
        // Rows over a small limit passed over, as the Anki import reads.
        let mut rows = 0;
        let _ = db.scan_bounded(entry.root, 512, &mut |_, _| {
            rows += 1;
            rows < MAX_ROWS
        });
    }
});
