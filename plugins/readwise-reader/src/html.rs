//! The text of an article out of Readwise's answer, without holding it:
//! the answer is a JSON file with the article's HTML as one long string
//! (`html_content`), longer than the JSON reader of the SDK keeps (see
//! `paperpaw_sdk::json::stream`). It is read from storage in small
//! pieces, found by its key, unescaped as it goes and appended to the
//! book being made, so an article of any length needs the same few
//! kilobytes.

use alloc::vec::Vec;

use paperpaw_sdk::{Storage, StorageError};

/// The key whose string is the article.
const KEY: &[u8] = b"\"html_content\"";
/// Bytes read from the answer at a time, and written to the book.
const CHUNK: usize = 4096;
/// The most of an article taken, in bytes: more is left off (a book of
/// this size is long, and pictures are not in it).
pub const MAX_TEXT: usize = 1024 * 1024;

/// How copying the article ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Copied {
    /// The article's string was read to its end (or to [`MAX_TEXT`]).
    Whole,
    /// The answer has no text for it (`null`, or no such key).
    Missing,
    /// The answer ends inside the string: cut off.
    Cut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Looking for the key: how many of its bytes match so far.
    Seek(usize),
    /// After the key, before its colon.
    Colon,
    /// After the colon, before the value.
    Value,
    /// Inside the string.
    Text,
    /// After a backslash.
    Escape,
    /// In a `\u` escape: digits so far and their value.
    Unicode(u8, u32),
    /// The string ended.
    End,
}

/// Unescapes the string as it comes.
struct Decoder {
    state: State,
    /// The first half of a surrogate pair, waiting for its second.
    high: Option<u32>,
    out: Vec<u8>,
    taken: usize,
}

impl Decoder {
    fn put(&mut self, c: char) {
        let mut buf = [0u8; 4];
        let bytes = c.encode_utf8(&mut buf).as_bytes();
        self.push(bytes);
    }

    fn push(&mut self, bytes: &[u8]) {
        if self.taken + bytes.len() > MAX_TEXT {
            self.state = State::End;
            return;
        }
        self.taken += bytes.len();
        self.out.extend_from_slice(bytes);
    }

    /// A lone half of a pair stands for the replacement character.
    fn flush_high(&mut self) {
        if self.high.take().is_some() {
            self.put('\u{fffd}');
        }
    }

    fn unit(&mut self, unit: u32) {
        match (self.high.take(), unit) {
            (Some(high), 0xdc00..=0xdfff) => {
                let code = 0x10000 + ((high - 0xd800) << 10) + (unit - 0xdc00);
                self.put(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            (Some(_), _) => {
                self.put('\u{fffd}');
                self.unit(unit);
            }
            (None, 0xd800..=0xdbff) => self.high = Some(unit),
            (None, 0xdc00..=0xdfff) => self.put('\u{fffd}'),
            (None, 0) => {}
            (None, _) => self.put(char::from_u32(unit).unwrap_or('\u{fffd}')),
        }
    }

    fn byte(&mut self, b: u8) {
        match self.state {
            State::Seek(at) => {
                self.state = if b == KEY[at] {
                    if at + 1 == KEY.len() {
                        State::Colon
                    } else {
                        State::Seek(at + 1)
                    }
                } else {
                    // The key's two quotes are its only ones: a quote
                    // that is not its next byte begins it anew.
                    State::Seek(usize::from(b == b'"'))
                };
            }
            State::Colon => match b {
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b':' => self.state = State::Value,
                // A string that only looked like the key.
                _ => self.state = State::Seek(usize::from(b == b'"')),
            },
            State::Value => match b {
                b' ' | b'\t' | b'\r' | b'\n' => {}
                b'"' => self.state = State::Text,
                _ => self.state = State::Seek(0),
            },
            State::Text => match b {
                b'"' => {
                    self.flush_high();
                    self.state = State::End;
                }
                b'\\' => self.state = State::Escape,
                _ => {
                    self.flush_high();
                    self.push(&[b]);
                }
            },
            State::Escape => {
                let plain = match b {
                    b'n' => Some('\n'),
                    b't' => Some('\t'),
                    b'r' => Some('\r'),
                    b'b' => Some('\u{8}'),
                    b'f' => Some('\u{c}'),
                    b'u' => {
                        self.state = State::Unicode(0, 0);
                        return;
                    }
                    // `"`, `\`, `/` and anything else stand for themselves.
                    _ => None,
                };
                self.flush_high();
                match plain {
                    Some(c) => self.put(c),
                    None => self.push(&[b]),
                }
                if self.state == State::Escape {
                    self.state = State::Text;
                }
            }
            State::Unicode(digits, value) => {
                let Some(digit) = char::from(b).to_digit(16) else {
                    // Not a digit: the escape is broken; go on as text.
                    self.put('\u{fffd}');
                    self.state = State::Text;
                    return self.byte(b);
                };
                let value = value * 16 + digit;
                if digits == 3 {
                    self.state = State::Text;
                    self.unit(value);
                } else {
                    self.state = State::Unicode(digits + 1, value);
                }
            }
            State::End => {}
        }
    }
}

/// Appends the string `html_content` of the JSON file `from` to the file
/// `to`, unescaped; what is appended is at most [`MAX_TEXT`] bytes.
pub fn copy_article(storage: &dyn Storage, from: &str, to: &str) -> Result<Copied, StorageError> {
    let mut decoder = Decoder {
        state: State::Seek(0),
        high: None,
        out: Vec::with_capacity(2 * CHUNK),
        taken: 0,
    };
    let mut buf = [0u8; CHUNK];
    let mut at = 0u64;
    let mut text = false;
    loop {
        let n = storage.read_at(from, at, &mut buf)?;
        if n == 0 {
            break;
        }
        at += n as u64;
        for &b in &buf[..n] {
            decoder.byte(b);
            text |= matches!(decoder.state, State::Text);
        }
        if decoder.out.len() >= CHUNK {
            storage.append(to, &decoder.out)?;
            decoder.out.clear();
        }
        if decoder.state == State::End {
            break;
        }
    }
    if !decoder.out.is_empty() {
        storage.append(to, &decoder.out)?;
    }
    Ok(match decoder.state {
        State::End => Copied::Whole,
        _ if text => Copied::Cut,
        _ => Copied::Missing,
    })
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;

    use paperpaw_sdk::{MemoryStorage, Storage};

    use super::*;

    fn copy(json: &str) -> (Copied, String) {
        let storage = MemoryStorage::new().with("in.json", json);
        storage.write("out", b"").unwrap();
        let copied = copy_article(&storage, "in.json", "out").unwrap();
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        let mut at = 0;
        loop {
            let n = storage.read_at("out", at, &mut buf).unwrap();
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
            at += n as u64;
        }
        (copied, String::from_utf8(out).unwrap())
    }

    #[test]
    fn unescapes_the_article() {
        let json = r#"{"count":1,"results":[{"id":"x","title":"html_content","html_content": "<p>A \"q\" \\ \/ é\n😀 A</p>","author":"B"}]}"#;
        let (copied, text) = copy(json);
        assert_eq!(copied, Copied::Whole);
        assert_eq!(text, "<p>A \"q\" \\ / é\n😀 A</p>");
    }

    #[test]
    fn what_is_not_there_is_missing() {
        assert_eq!(
            copy(r#"{"results":[{"html_content":null}]}"#).0,
            Copied::Missing
        );
        assert_eq!(copy(r#"{"results":[]}"#).0, Copied::Missing);
        assert_eq!(copy("").0, Copied::Missing);
        // A value that only looks like the key.
        assert_eq!(
            copy(r#"{"title":"\"html_content\": \"x\""}"#).0,
            Copied::Missing
        );
    }

    #[test]
    fn a_cut_answer_is_cut() {
        let (copied, text) = copy(r#"{"html_content":"<p>Half"#);
        assert_eq!(copied, Copied::Cut);
        assert_eq!(text, "<p>Half");
    }

    #[test]
    fn broken_escapes_do_no_harm() {
        let (copied, text) = copy(r#"{"html_content":"a\ud800b\udc00c\u12x\q"}"#);
        assert_eq!(copied, Copied::Whole);
        assert_eq!(text, "a\u{fffd}b\u{fffd}c\u{fffd}xq");
    }

    #[test]
    fn long_articles_are_cut_at_the_most() {
        let body = "x".repeat(MAX_TEXT + 5000);
        let (copied, text) = copy(&alloc::format!(r#"{{"html_content":"{body}"}}"#));
        assert_eq!(copied, Copied::Whole);
        assert_eq!(text.len(), MAX_TEXT);
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_are_copied_without_harm(
            head in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300),
            tail in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..3000),
        ) {
            let mut json = head;
            json.extend_from_slice(b"\"html_content\": \"");
            json.extend_from_slice(&tail);
            let storage = MemoryStorage::new().with("in.json", json);
            storage.write("out", b"").unwrap();
            let _ = copy_article(&storage, "in.json", "out").unwrap();
            proptest::prop_assert!(storage.size("out").unwrap() as usize <= MAX_TEXT);
        }
    }
}
