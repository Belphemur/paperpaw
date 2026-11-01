//! PalmDOC compression (LZ77 with a 2 KB window) and the data a Mobipocket
//! text record carries after its text.
//!
//! Each record is compressed alone, so any record can be read on its own.
//! Decompression never writes more than `limit` bytes and never reads
//! before the start of the record, whatever the input says.

use alloc::vec::Vec;

/// The input was not PalmDOC data; what was decompressed before the fault
/// is in the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Malformed;

/// Decompresses `input` onto `out` (which is empty), at most `limit` bytes.
/// Where the output reaches `limit` the rest is left out.
pub(crate) fn decompress(input: &[u8], out: &mut Vec<u8>, limit: usize) -> Result<(), Malformed> {
    let mut i = 0;
    while i < input.len() && out.len() < limit {
        let c = input[i];
        i += 1;
        match c {
            0 | 0x09..=0x7f => out.push(c),
            0x01..=0x08 => {
                let n = usize::from(c);
                let end = (i + n).min(input.len());
                let room = limit - out.len();
                let take = (end - i).min(room);
                out.extend_from_slice(&input[i..i + take]);
                if end - i < n {
                    return Err(Malformed);
                }
                i = end;
            }
            0x80..=0xbf => {
                let Some(&next) = input.get(i) else {
                    return Err(Malformed);
                };
                i += 1;
                let pair = (u16::from(c) << 8) | u16::from(next);
                let distance = usize::from((pair >> 3) & 0x7ff);
                let length = usize::from(pair & 7) + 3;
                if distance == 0 || distance > out.len() {
                    return Err(Malformed);
                }
                for _ in 0..length.min(limit - out.len()) {
                    let byte = out[out.len() - distance];
                    out.push(byte);
                }
            }
            0xc0..=0xff => {
                out.push(b' ');
                if out.len() < limit {
                    out.push(c ^ 0x80);
                }
            }
        }
    }
    Ok(())
}

/// The size of the variable-length number that ends `data` (7 bits a
/// byte, the last byte of the number is the one with its high bit set, read
/// from the end), at most four bytes.
fn trailing_size(data: &[u8]) -> usize {
    let mut size = 0usize;
    let mut shift = 0;
    for &byte in data.iter().rev().take(4) {
        size |= usize::from(byte & 0x7f) << shift;
        shift += 7;
        if byte & 0x80 != 0 {
            break;
        }
    }
    size
}

/// How long the text of a record is, of its `record` bytes, given the
/// header's extra-data `flags`: what follows the text (the entries of the
/// flags' bits 1 and up, then the bytes of a character cut off by the
/// record's end for bit 0) is not text.
pub(crate) fn text_len(record: &[u8], flags: u16) -> usize {
    let mut len = record.len();
    for bit in 1..16 {
        if flags & (1 << bit) != 0 {
            let entry = trailing_size(&record[..len]);
            len = len.saturating_sub(entry.max(1));
        }
    }
    if flags & 1 != 0
        && let Some(&last) = len.checked_sub(1).and_then(|i| record.get(i))
    {
        len = len.saturating_sub(usize::from(last & 3) + 1);
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn run(input: &[u8], limit: usize) -> (Vec<u8>, Result<(), Malformed>) {
        let mut out = Vec::new();
        let result = decompress(input, &mut out, limit);
        (out, result)
    }

    #[test]
    fn literals_copies_and_spaces() {
        // "hello ", a copy of 3 bytes from 4 back, a space and 'w' in one
        // byte, a literal run of 3.
        let input = b"hello \x80\x20\xf7\x03abc";
        let (out, result) = run(input, 100);
        assert!(result.is_ok());
        assert_eq!(out, b"hello llo wabc");
    }

    #[test]
    fn faults_stop_without_panicking() {
        // A copy from before the start, from nothing, a cut pair, a cut run.
        for input in [
            &[0x80u8, 0x08][..],
            &[0x80],
            &[0x05, b'a'],
            &[b'a', 0x80, 0x00],
        ] {
            let (_, result) = run(input, 100);
            assert!(result.is_err(), "{input:?}");
        }
    }

    #[test]
    fn output_is_bounded() {
        // "ab" then copies of up to 10 bytes, again and again.
        let mut input = vec![b'a', b'b'];
        for _ in 0..1000 {
            input.extend_from_slice(&[0x80, (1 << 3) | 7]);
        }
        let (out, result) = run(&input, 50);
        assert!(result.is_ok());
        assert_eq!(out.len(), 50);
        let (out, _) = run(&[b'x'; 500], 20);
        assert_eq!(out.len(), 20);
    }

    #[test]
    fn trailing_entries_are_cut_off() {
        // Text, then an entry of 3 bytes whose size byte is last (0x83).
        let mut record = b"some text".to_vec();
        record.extend_from_slice(&[1, 2, 0x83]);
        assert_eq!(text_len(&record, 0b10), 9);
        assert_eq!(text_len(&record, 0), 12);
        // A cut character (2 bytes: the last byte's low bits say 1 more).
        let mut record = b"abc".to_vec();
        record.extend_from_slice(&[0xc3, 0x01]);
        assert_eq!(text_len(&record, 1), 3);
        // Nonsense never goes below zero.
        assert_eq!(text_len(&[0xff, 0xff], 0xffff), 0);
        assert_eq!(text_len(&[], 0xffff), 0);
    }
}
