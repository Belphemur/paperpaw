//! What the firmware keeps in the RTC's fast memory through a reset (the
//! place it was at, the last panic's message): sealed with a magic word
//! and a checksum, so that what is there after a power-up, or what the
//! firmware before left (CrossPoint uses the same memory), is not taken
//! for the firmware's own notes. Anything that fails the check is
//! "unknown".

/// The bytes of a panic message's buffer.
pub const MESSAGE_LEN: usize = 400;
/// Where the message's text is in the buffer: after the magic word, before
/// the checksum.
pub const BODY: core::ops::Range<usize> = 4..MESSAGE_LEN - 4;

const MAGIC_MESSAGE: u32 = 0x5050_4D31;
const MAGIC_PLACE: u32 = 0x5050_4C31;

/// FNV-1a over `bytes`, started from `seed`.
fn sum(seed: u32, bytes: &[u8]) -> u32 {
    bytes.iter().fold(seed ^ 0x811C_9DC5, |hash, &b| {
        (hash ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

/// The panic buffer sealed: the magic word first, the checksum of the text
/// last (the text is in [`BODY`]).
pub fn seal_message(buf: &mut [u8; MESSAGE_LEN]) {
    buf[..4].copy_from_slice(&MAGIC_MESSAGE.to_le_bytes());
    let check = sum(MAGIC_MESSAGE, &buf[BODY]);
    buf[MESSAGE_LEN - 4..].copy_from_slice(&check.to_le_bytes());
}

/// The text of a sealed panic buffer; `None` for noise, a buffer never
/// sealed, text that is not printable or no text at all.
pub fn message(buf: &[u8; MESSAGE_LEN]) -> Option<&str> {
    if buf[..4] != MAGIC_MESSAGE.to_le_bytes() {
        return None;
    }
    let check = u32::from_le_bytes([
        buf[MESSAGE_LEN - 4],
        buf[MESSAGE_LEN - 3],
        buf[MESSAGE_LEN - 2],
        buf[MESSAGE_LEN - 1],
    ]);
    let body = &buf[BODY];
    if check != sum(MAGIC_MESSAGE, body) {
        return None;
    }
    let len = body.iter().position(|&b| b == 0).unwrap_or(body.len());
    let text = core::str::from_utf8(&body[..len]).ok()?;
    (!text.is_empty() && text.chars().all(|c| !c.is_control() || c == '\n')).then_some(text)
}

/// A place sealed as the words kept: the magic, the place, a checksum.
pub fn seal_place(place: u32) -> [u32; 3] {
    [MAGIC_PLACE, place, sum(MAGIC_PLACE, &place.to_le_bytes())]
}

/// The place of sealed words; `None` where they fail the check.
pub fn place(words: [u32; 3]) -> Option<u32> {
    (words[0] == MAGIC_PLACE && words[2] == sum(MAGIC_PLACE, &words[1].to_le_bytes()))
        .then_some(words[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(text: &str) -> [u8; MESSAGE_LEN] {
        let mut buf = [0u8; MESSAGE_LEN];
        buf[4..4 + text.len()].copy_from_slice(text.as_bytes());
        seal_message(&mut buf);
        buf
    }

    #[test]
    fn a_sealed_message_comes_back() {
        assert_eq!(message(&buffer("oops (a.rs:1:2)")), Some("oops (a.rs:1:2)"));
    }

    #[test]
    fn noise_and_unsealed_buffers_are_unknown() {
        assert_eq!(message(&[0; MESSAGE_LEN]), None);
        assert_eq!(message(&[0xFF; MESSAGE_LEN]), None);
        // Printable text another firmware left, never sealed.
        let mut other = [0u8; MESSAGE_LEN];
        other[..9].copy_from_slice(b"some text");
        assert_eq!(message(&other), None);
        // Sealed, then one byte of the text changed.
        let mut damaged = buffer("oops");
        damaged[5] ^= 1;
        assert_eq!(message(&damaged), None);
        // Sealed, then the checksum damaged.
        let mut damaged = buffer("oops");
        damaged[MESSAGE_LEN - 1] ^= 1;
        assert_eq!(message(&damaged), None);
        // A sealed empty message says nothing.
        assert_eq!(message(&buffer("")), None);
    }

    #[test]
    fn a_sealed_place_comes_back_and_a_damaged_one_is_unknown() {
        assert_eq!(place(seal_place(33)), Some(33));
        assert_eq!(place(seal_place(0)), Some(0));
        assert_eq!(place([0, 0, 0]), None);
        assert_eq!(place([u32::MAX; 3]), None);
        let mut words = seal_place(7);
        words[1] = 8;
        assert_eq!(place(words), None);
        let mut words = seal_place(7);
        words[0] ^= 1;
        assert_eq!(place(words), None);
    }
}
