//! Reading JSON as it streams, e.g. from a file, with little memory: a
//! [`Parser`] reports each value with its path ([`Event`]), and the
//! caller keeps only what it needs. Strings and other plain values are
//! cut at [`MAX_STRING`] bytes, keys at 256, and objects and arrays nested
//! deeper than [`MAX_DEPTH`] are skipped.
//!
//! ```
//! use paperpaw_json::stream::{Event, Parser};
//!
//! let mut titles = Vec::new();
//! let mut parser = Parser::default();
//! parser.feed(br#"{"items": [{"title": "A"}, {"title": "B"}]}"#, &mut |event| {
//!     if let Event::Value(path, text) = event
//!         && path == ["items", "[]", "title"]
//!     {
//!         titles.push(text.to_string());
//!     }
//! });
//! assert_eq!(titles, ["A", "B"]);
//! ```

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_platform::Storage;

/// Pieces a file is read in.
const CHUNK: usize = 2048;
/// Strings and other plain values are cut at this many bytes.
pub const MAX_STRING: usize = 4096;
/// Objects and arrays nested deeper than this are skipped whole, without
/// events.
pub const MAX_DEPTH: usize = 64;
/// Keys are cut at this many bytes: the path keeps one for each level.
const MAX_KEY: usize = 256;
/// What a broken `\u` escape or a lone surrogate stands for.
const REPLACEMENT: &[u8] = "\u{fffd}".as_bytes();

/// Where a value is: the keys from the top, `[]` for array items.
pub type Path = Vec<String>;

/// What the parser reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a> {
    /// An object or array starts at this path.
    Open(&'a [String]),
    /// It ends.
    Close(&'a [String]),
    /// A string (or other plain value, as text) at this path.
    Value(&'a [String], &'a str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Container {
    Object,
    Array,
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Between,
    /// In a string: whether it is a key, and an escape seen.
    Text {
        key: bool,
        escape: bool,
        /// A `\u` escape so far: how many hex digits, and their value.
        unicode: Option<(u8, u32)>,
    },
    /// In a number or `true`/`false`/`null`.
    Plain,
}

/// A streaming JSON parser that reports values with their paths.
#[derive(Debug, Default)]
pub struct Parser {
    /// The path of the innermost open object or array.
    path: Path,
    containers: Vec<Container>,
    /// The key of the next value in an object.
    key: Option<String>,
    state: State,
    /// The text of the string or plain value being read.
    bytes: Vec<u8>,
    /// A high surrogate waiting for its low half.
    surrogate: Option<u32>,
    /// How deep the parser is in objects and arrays past [`MAX_DEPTH`].
    skipped: usize,
}

impl Parser {
    paperpaw_table::shared! {
    /// Reads `data`, the next piece of the text, reporting what it finds.
    pub fn feed(&mut self, data: &[u8], emit: &mut dyn FnMut(Event<'_>)) {
        for &b in data {
            self.byte(b, emit);
        }
    } @ data::JSON_STREAM_FEED
    }

    paperpaw_table::shared! {
    /// Ends the text: reports a number or `true`/`false`/`null` it ends
    /// with, which only what follows it could end otherwise (e.g. `42` as
    /// the whole text). A string left open is not reported.
    pub fn finish(&mut self, emit: &mut dyn FnMut(Event<'_>)) {
        if matches!(self.state, State::Plain) {
            self.end_plain(emit);
        }
    } @ data::JSON_STREAM_FINISH
    }

    /// Adds `bytes` to the text being read.
    fn push_bytes(&mut self, bytes: &[u8]) {
        self.flush_surrogate();
        if self.bytes.len() < MAX_STRING {
            self.bytes.extend_from_slice(bytes);
        }
    }

    fn push_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        self.push_bytes(c.encode_utf8(&mut buf).as_bytes());
    }

    /// Adds the UTF-16 code unit of a `\u` escape.
    fn push_unit(&mut self, unit: u32) {
        match (self.surrogate, unit) {
            (Some(high), 0xdc00..=0xdfff) => {
                self.surrogate = None;
                let code = 0x10000 + ((high - 0xd800) << 10) + (unit - 0xdc00);
                self.push_char(char::from_u32(code).unwrap_or('\u{fffd}'));
            }
            (_, 0xd800..=0xdbff) => {
                self.flush_surrogate();
                self.surrogate = Some(unit);
            }
            _ => self.push_char(char::from_u32(unit).unwrap_or('\u{fffd}')),
        }
    }

    /// A high surrogate not followed by its low half stands for U+FFFD.
    fn flush_surrogate(&mut self) {
        if self.surrogate.take().is_some() && self.bytes.len() < MAX_STRING {
            self.bytes.extend_from_slice(REPLACEMENT);
        }
    }

    /// Reports `text` as the next value of the open object or array.
    fn value(&mut self, text: &str, emit: &mut dyn FnMut(Event<'_>)) {
        match self.containers.last() {
            Some(Container::Array) => {
                self.path.push(String::from("[]"));
                emit(Event::Value(&self.path, text));
                self.path.pop();
            }
            Some(Container::Object) => {
                let key = self.key.take();
                let has_key = key.is_some();
                self.path.push(key.unwrap_or_default());
                emit(Event::Value(&self.path, text));
                // The key stays until the next `,`.
                self.key = self.path.pop().filter(|_| has_key);
            }
            None => emit(Event::Value(&self.path, text)),
        }
    }

    fn open(&mut self, container: Container, emit: &mut dyn FnMut(Event<'_>)) {
        if self.skipped > 0 || self.containers.len() >= MAX_DEPTH {
            self.skipped = self.skipped.saturating_add(1);
            return;
        }
        match self.containers.last() {
            Some(Container::Array) => self.path.push(String::from("[]")),
            Some(Container::Object) => self.path.push(self.key.take().unwrap_or_default()),
            None => {}
        }
        emit(Event::Open(&self.path));
        self.containers.push(container);
        self.key = None;
    }

    fn close(&mut self, emit: &mut dyn FnMut(Event<'_>)) {
        if self.skipped > 0 {
            self.skipped -= 1;
            return;
        }
        self.containers.pop();
        emit(Event::Close(&self.path));
        let last = self.path.pop();
        // Back in an object: the key it was under.
        self.key = match self.containers.last() {
            Some(Container::Object) => last,
            _ => None,
        };
    }

    fn end_string(&mut self, key: bool, emit: &mut dyn FnMut(Event<'_>)) {
        self.flush_surrogate();
        if self.skipped == 0 {
            if key {
                self.bytes.truncate(MAX_KEY);
            }
            let text = String::from_utf8_lossy(&self.bytes).into_owned();
            if key {
                self.key = Some(text);
            } else {
                self.value(&text, emit);
            }
        }
        self.bytes.clear();
        self.state = State::Between;
    }

    fn end_plain(&mut self, emit: &mut dyn FnMut(Event<'_>)) {
        if self.skipped == 0 {
            let text = String::from_utf8_lossy(&self.bytes).into_owned();
            self.value(&text, emit);
        }
        self.bytes.clear();
        self.state = State::Between;
    }

    fn byte(&mut self, b: u8, emit: &mut dyn FnMut(Event<'_>)) {
        match &mut self.state {
            State::Text {
                key,
                escape,
                unicode,
            } => {
                if let Some((digits, value)) = unicode {
                    let Some(digit) = char::from(b).to_digit(16) else {
                        // A cut escape: the byte is read as it is.
                        *unicode = None;
                        self.push_bytes(REPLACEMENT);
                        self.byte(b, emit);
                        return;
                    };
                    *value = *value * 16 + digit;
                    *digits += 1;
                    if *digits == 4 {
                        let unit = *value;
                        *unicode = None;
                        self.push_unit(unit);
                    }
                    return;
                }
                if *escape {
                    *escape = false;
                    let b = match b {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        b'b' | b'f' => b' ',
                        b'u' => {
                            *unicode = Some((0, 0));
                            return;
                        }
                        other => other,
                    };
                    self.push_bytes(&[b]);
                    return;
                }
                match b {
                    b'\\' => *escape = true,
                    b'"' => {
                        let key = *key;
                        self.end_string(key, emit);
                    }
                    _ => self.push_bytes(&[b]),
                }
            }
            State::Plain => {
                if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+') {
                    self.push_bytes(&[b]);
                } else {
                    self.end_plain(emit);
                    self.byte(b, emit);
                }
            }
            State::Between => match b {
                b'{' => self.open(Container::Object, emit),
                b'[' => self.open(Container::Array, emit),
                b'}' | b']' => self.close(emit),
                b'"' => {
                    let key = self.skipped == 0
                        && self.containers.last() == Some(&Container::Object)
                        && self.key.is_none();
                    self.state = State::Text {
                        key,
                        escape: false,
                        unicode: None,
                    };
                }
                b',' => {
                    if self.skipped == 0 && self.containers.last() == Some(&Container::Object) {
                        self.key = None;
                    }
                }
                b':' => {}
                b if b.is_ascii_whitespace() => {}
                b => {
                    self.state = State::Plain;
                    self.push_bytes(&[b]);
                }
            },
        }
    }
}

paperpaw_table::shared! {
/// Reads the JSON file at `path`, a piece at a time, reporting what it
/// finds to `emit`.
pub fn read_file(storage: &dyn Storage, path: &str, emit: &mut dyn FnMut(Event<'_>)) {
    let mut parser = Parser::default();
    let mut buf = [0u8; CHUNK];
    let mut offset = 0;
    while let Ok(n) = storage.read_at(path, offset, &mut buf) {
        if n == 0 {
            break;
        }
        parser.feed(&buf[..n], emit);
        offset += n as u64;
    }
    parser.finish(emit);
} @ data::JSON_READ_FILE
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;

    /// The values of `json` with their paths joined by `/`.
    fn values(json: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        Parser::default().feed(json, &mut |event| {
            if let Event::Value(path, text) = event {
                out.push((path.join("/"), text.to_string()));
            }
        });
        out
    }

    fn pair(path: &str, text: &str) -> (String, String) {
        (path.to_string(), text.to_string())
    }

    #[test]
    fn reports_paths() {
        let mut events = Vec::new();
        Parser::default().feed(br#"{"a": [1, {"b": "x"}], "c": true}"#, &mut |event| {
            events.push(match event {
                Event::Open(path) => format!("open {}", path.join("/")),
                Event::Close(path) => format!("close {}", path.join("/")),
                Event::Value(path, text) => format!("{} = {text}", path.join("/")),
            });
        });
        assert_eq!(
            events,
            [
                "open ",
                "open a",
                "a/[] = 1",
                "open a/[]",
                "a/[]/b = x",
                "close a/[]",
                "close a",
                "c = true",
                "close ",
            ]
        );
    }

    #[test]
    fn cut_unicode_escapes_end() {
        // A non-hex byte, multibyte or not, ends the escape.
        let json = r#"{"a": "\u0éz", "b": "\u12", "c": "x"}"#;
        assert_eq!(
            values(json.as_bytes()),
            [
                pair("a", "\u{fffd}\u{e9}z"),
                pair("b", "\u{fffd}"),
                pair("c", "x")
            ]
        );
    }

    #[test]
    fn surrogates_pair_only_with_low_halves() {
        let json =
            br#"["\ud83d\ude00", "\ud800A", "\ud800", "\udc00", "\ud800\ud83d\ude00", "\ud800\n"]"#;
        let texts: Vec<String> = values(json).into_iter().map(|(_, t)| t).collect();
        assert_eq!(
            texts,
            [
                "\u{1f600}",
                "\u{fffd}A",
                "\u{fffd}",
                "\u{fffd}",
                "\u{fffd}\u{1f600}",
                "\u{fffd}\n"
            ]
        );
    }

    #[test]
    fn plain_values_are_cut() {
        let mut json = String::from("[");
        json.push_str(&"1".repeat(3 * MAX_STRING));
        json.push_str(", 2]");
        let out = values(json.as_bytes());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1.len(), MAX_STRING);
        assert_eq!(out[1], pair("[]", "2"));
    }

    #[test]
    fn long_keys_are_cut() {
        let json = format!("{{\"{}\": 1}}", "k".repeat(MAX_STRING));
        let out = values(json.as_bytes());
        assert_eq!(out[0].0.len(), MAX_KEY);
    }

    #[test]
    fn deep_nesting_is_skipped() {
        let depth = 10_000;
        let json = format!(
            "{{\"deep\": {}\"x]\", {{\"k\": 1}}{}, \"after\": \"y\"}}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        let mut deepest = 0;
        let mut out = Vec::new();
        Parser::default().feed(json.as_bytes(), &mut |event| {
            let (Event::Open(path) | Event::Close(path) | Event::Value(path, _)) = event;
            deepest = deepest.max(path.len());
            if let Event::Value(path, text) = event {
                out.push((path.join("/"), text.to_string()));
            }
        });
        assert!(deepest <= MAX_DEPTH);
        assert_eq!(out, [pair("after", "y")]);
    }
}
