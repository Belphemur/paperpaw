//! A text file read a line at a time, so that a file of any size costs a
//! small buffer, not its size.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::{Storage, StorageError};

/// The longest a line is kept, in bytes; the rest of it is dropped.
pub(crate) const MAX_LINE: usize = 1024;

/// Calls `each` with the lines of the file at `path` (without their line
/// breaks), until it answers `false`; the lines are cut at [`MAX_LINE`].
pub(crate) fn each(
    storage: &dyn Storage,
    path: &str,
    each: &mut dyn FnMut(&str) -> bool,
) -> Result<(), StorageError> {
    let mut chunk = [0u8; 1024];
    let mut line: Vec<u8> = Vec::new();
    let mut at = 0u64;
    loop {
        let n = storage.read_at(path, at, &mut chunk)?;
        if n == 0 {
            break;
        }
        at += n as u64;
        for &byte in &chunk[..n] {
            if byte == b'\n' {
                let text = String::from_utf8_lossy(&line);
                if !each(text.trim_end_matches('\r')) {
                    return Ok(());
                }
                line.clear();
            } else if line.len() < MAX_LINE {
                line.push(byte);
            }
        }
    }
    if !line.is_empty() {
        each(&String::from_utf8_lossy(&line));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::MemoryStorage;

    fn lines(data: &[u8]) -> Vec<String> {
        let storage = MemoryStorage::new().with("f", data.to_vec());
        let mut found = Vec::new();
        each(&storage, "f", &mut |l| {
            found.push(String::from(l));
            true
        })
        .unwrap();
        found
    }

    #[test]
    fn lines_are_read_whole_across_chunks() {
        assert_eq!(lines(b"a\nb\r\n\nlast"), ["a", "b", "", "last"]);
        assert!(lines(b"").is_empty());
        let long = "x".repeat(5000);
        let text = alloc::format!("{long}\nshort\n");
        let found = lines(text.as_bytes());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].len(), MAX_LINE);
        assert_eq!(found[1], "short");
    }

    #[test]
    fn reading_stops_when_asked() {
        let storage = MemoryStorage::new().with("f", b"a\nb\nc\n".to_vec());
        let mut seen = 0;
        each(&storage, "f", &mut |_| {
            seen += 1;
            seen < 2
        })
        .unwrap();
        assert_eq!(seen, 2);
        assert!(each(&storage, "none", &mut |_| true).is_err());
    }
}
