//! IMAP (RFC 3501) as text: what a server's line says, and the commands
//! to send. No connection here: the bytes come from and go to whoever
//! holds the socket (see `session`).

use alloc::string::String;
use alloc::vec::Vec;

/// The longest response line taken; a server's longer line is an error.
pub const MAX_LINE: usize = 16 * 1024;
/// Tokens nested in lists, at most.
const MAX_DEPTH: usize = 6;

/// Bytes from a socket, cut into lines and literals.
#[derive(Debug, Default, Clone)]
pub struct LineBuffer {
    data: Vec<u8>,
}

impl LineBuffer {
    pub fn push(&mut self, bytes: &[u8]) {
        self.data.extend_from_slice(bytes);
    }

    /// The next line, without its line break; `Ok(None)` while none is
    /// complete, `Err` when the line grows past [`MAX_LINE`].
    pub fn line(&mut self) -> Result<Option<Vec<u8>>, TooLong> {
        match self.data.iter().position(|&b| b == b'\n') {
            Some(end) => {
                let mut line: Vec<u8> = self.data.drain(..=end).collect();
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if line.len() > MAX_LINE {
                    return Err(TooLong);
                }
                Ok(Some(line))
            }
            None if self.data.len() > MAX_LINE => Err(TooLong),
            None => Ok(None),
        }
    }

    /// Up to `n` bytes from the front (a literal).
    pub fn take(&mut self, n: usize) -> Vec<u8> {
        let n = n.min(self.data.len());
        self.data.drain(..n).collect()
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// A response line longer than [`MAX_LINE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong;

/// One piece of a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Atom(String),
    Quoted(String),
    List(Vec<Token>),
    /// `{n}` or `{n+}` at the end of a line: n bytes follow.
    Literal(usize),
    Nil,
}

impl Token {
    /// The text of an atom or a quoted string.
    pub fn text(&self) -> Option<&str> {
        match self {
            Token::Atom(s) | Token::Quoted(s) => Some(s),
            _ => None,
        }
    }

    pub fn list(&self) -> Option<&[Token]> {
        match self {
            Token::List(list) => Some(list),
            _ => None,
        }
    }

    pub fn number(&self) -> Option<u64> {
        self.text()?.parse().ok()
    }
}

/// The tokens of `line` (what is no token is skipped, an open list is
/// closed).
pub fn tokens(line: &[u8]) -> Vec<Token> {
    let text = String::from_utf8_lossy(line);
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0;
    tokens_in(&chars, &mut at, 0, false)
}

fn tokens_in(chars: &[char], at: &mut usize, depth: usize, in_list: bool) -> Vec<Token> {
    let mut out = Vec::new();
    while *at < chars.len() {
        let c = chars[*at];
        match c {
            ' ' | '\t' => *at += 1,
            ')' => {
                *at += 1;
                if in_list {
                    return out;
                }
            }
            '(' => {
                *at += 1;
                if depth >= MAX_DEPTH {
                    continue;
                }
                let inner = tokens_in(chars, at, depth + 1, true);
                out.push(Token::List(inner));
            }
            '"' => {
                *at += 1;
                let mut s = String::new();
                while *at < chars.len() {
                    match chars[*at] {
                        '"' => {
                            *at += 1;
                            break;
                        }
                        '\\' if *at + 1 < chars.len() => {
                            s.push(chars[*at + 1]);
                            *at += 2;
                        }
                        c => {
                            s.push(c);
                            *at += 1;
                        }
                    }
                }
                out.push(Token::Quoted(s));
            }
            '{' => {
                let end = chars[*at..].iter().position(|&c| c == '}');
                let literal = end.and_then(|e| {
                    let inner: String = chars[*at + 1..*at + e].iter().collect();
                    inner.trim_end_matches('+').parse::<usize>().ok()
                });
                match (end, literal) {
                    (Some(e), Some(n)) => {
                        *at += e + 1;
                        out.push(Token::Literal(n));
                    }
                    _ => {
                        *at += 1;
                    }
                }
            }
            _ => {
                let start = *at;
                let mut depth_square = 0u32;
                while *at < chars.len() {
                    match chars[*at] {
                        '[' => depth_square += 1,
                        ']' => depth_square = depth_square.saturating_sub(1),
                        ' ' | '(' | ')' | '"' | '{' if depth_square == 0 && *at > start => break,
                        ' ' | ')' | '(' | '"' if depth_square == 0 => break,
                        _ => {}
                    }
                    // `BODY[HEADER.FIELDS (FROM TO)]` is one atom.
                    *at += 1;
                }
                if *at == start {
                    *at += 1;
                    continue;
                }
                let atom: String = chars[start..*at].iter().collect();
                if atom.eq_ignore_ascii_case("NIL") {
                    out.push(Token::Nil);
                } else {
                    out.push(Token::Atom(atom));
                }
            }
        }
    }
    out
}

/// How a command ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    No,
    Bad,
}

/// What one line of a server says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// `A3 OK Done`
    Tagged { tag: String, status: Status, text: String },
    /// `+ go ahead`
    Continue(String),
    /// `* …`
    Untagged(Untagged),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Untagged {
    Capability(Vec<String>),
    /// `(flags) "delimiter" name`, the name as the server wrote it.
    List { flags: Vec<String>, delimiter: Option<char>, name: String },
    Exists(u32),
    Search(Vec<u32>),
    Status { name: String, items: Vec<(String, u64)> },
    Fetch { sequence: u32, items: Vec<Token> },
    /// `* OK [UIDVALIDITY 3] …`, `* OK [CAPABILITY …]`
    Code { code: String, args: Vec<String> },
    Bye(String),
    /// Anything else (flags, recent, expunge, text).
    Other,
}

/// What the server's `line` is; `None` for an empty line.
pub fn response(line: &[u8]) -> Option<Response> {
    let text = String::from_utf8_lossy(line);
    let text = text.trim_end();
    if text.is_empty() {
        return None;
    }
    if let Some(rest) = text.strip_prefix('+') {
        return Some(Response::Continue(String::from(rest.trim())));
    }
    if let Some(rest) = text.strip_prefix("* ") {
        return Some(Response::Untagged(untagged(rest, line)));
    }
    let (tag, rest) = text.split_once(' ')?;
    let (status, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    let status = if status.eq_ignore_ascii_case("OK") {
        Status::Ok
    } else if status.eq_ignore_ascii_case("NO") {
        Status::No
    } else if status.eq_ignore_ascii_case("BAD") {
        Status::Bad
    } else {
        return None;
    };
    Some(Response::Tagged {
        tag: String::from(tag),
        status,
        text: String::from(rest),
    })
}

fn untagged(rest: &str, raw: &[u8]) -> Untagged {
    let (first, tail) = rest.split_once(' ').unwrap_or((rest, ""));
    let upper: String = first.chars().map(|c| c.to_ascii_uppercase()).collect();
    if let Ok(number) = first.parse::<u32>() {
        let (word, after) = tail.split_once(' ').unwrap_or((tail, ""));
        if word.eq_ignore_ascii_case("EXISTS") {
            return Untagged::Exists(number);
        }
        if word.eq_ignore_ascii_case("FETCH") {
            // The line as bytes: after `* n FETCH `.
            let start = raw.len().saturating_sub(after.len());
            let items = tokens(&raw[start..]);
            let items = match items.into_iter().next() {
                Some(Token::List(items)) => items,
                _ => Vec::new(),
            };
            return Untagged::Fetch { sequence: number, items };
        }
        return Untagged::Other;
    }
    match upper.as_str() {
        "CAPABILITY" => Untagged::Capability(tail.split_whitespace().map(String::from).collect()),
        "LIST" | "LSUB" => list(tail),
        "SEARCH" => Untagged::Search(tail.split_whitespace().filter_map(|n| n.parse().ok()).collect()),
        "STATUS" => status_line(tail),
        "BYE" => Untagged::Bye(String::from(tail.trim())),
        "OK" | "NO" | "BAD" | "PREAUTH" => code(tail),
        _ => Untagged::Other,
    }
}

/// `* OK [CODE args] text`.
fn code(tail: &str) -> Untagged {
    let Some(inner) = tail.strip_prefix('[') else {
        return Untagged::Other;
    };
    let Some((inner, _)) = inner.split_once(']') else {
        return Untagged::Other;
    };
    let mut words = inner.split_whitespace();
    let code: String = words
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| c.to_ascii_uppercase())
        .collect();
    Untagged::Code {
        code,
        args: words.map(String::from).collect(),
    }
}

fn list(tail: &str) -> Untagged {
    let tokens = tokens(tail.as_bytes());
    let mut it = tokens.iter();
    let flags = match it.next() {
        Some(Token::List(flags)) => flags.iter().filter_map(|t| t.text().map(String::from)).collect(),
        _ => return Untagged::Other,
    };
    let delimiter = match it.next() {
        Some(Token::Quoted(d)) => d.chars().next(),
        _ => None,
    };
    let name = match it.next() {
        Some(token) => match token.text() {
            Some(name) => String::from(name),
            None => return Untagged::Other,
        },
        None => return Untagged::Other,
    };
    Untagged::List { flags, delimiter, name }
}

fn status_line(tail: &str) -> Untagged {
    let tokens = tokens(tail.as_bytes());
    let name = tokens.first().and_then(Token::text).map(String::from).unwrap_or_default();
    let mut items = Vec::new();
    if let Some(Token::List(list)) = tokens.get(1) {
        for pair in list.chunks(2) {
            if let [key, value] = pair
                && let (Some(key), Some(value)) = (key.text(), value.number())
            {
                let upper: String = key.chars().map(|c| c.to_ascii_uppercase()).collect();
                items.push((upper, value));
            }
        }
    }
    Untagged::Status { name, items }
}

/// The value after `key` in the items of a FETCH (`UID 42`, `RFC822.SIZE
/// 1000`).
pub fn fetch_number(items: &[Token], key: &str) -> Option<u64> {
    let at = items
        .iter()
        .position(|t| t.text().is_some_and(|k| k.eq_ignore_ascii_case(key)))?;
    items.get(at + 1)?.number()
}

/// The text after `key` in the items of a FETCH (`INTERNALDATE "…"`).
pub fn fetch_text<'a>(items: &'a [Token], key: &str) -> Option<&'a str> {
    let at = items
        .iter()
        .position(|t| t.text().is_some_and(|k| k.eq_ignore_ascii_case(key)))?;
    items.get(at + 1)?.text()
}

/// The flags of a FETCH (`\Seen`, …).
pub fn fetch_flags(items: &[Token]) -> Vec<String> {
    items
        .iter()
        .position(|t| t.text().is_some_and(|k| k.eq_ignore_ascii_case("FLAGS")))
        .and_then(|at| items.get(at + 1))
        .and_then(Token::list)
        .map(|flags| flags.iter().filter_map(|f| f.text().map(String::from)).collect())
        .unwrap_or_default()
}

/// The size of the literal a FETCH's body part ends with: the line's last
/// token is `{n}`.
pub fn literal_at_end(line: &[u8]) -> Option<usize> {
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    let line = line.strip_suffix(b"}")?;
    let open = line.iter().rposition(|&b| b == b'{')?;
    let inner = &line[open + 1..];
    let inner = inner.strip_suffix(b"+").unwrap_or(inner);
    if inner.is_empty() || !inner.iter().all(u8::is_ascii_digit) {
        return None;
    }
    core::str::from_utf8(inner).ok()?.parse().ok()
}

/// `text` as an IMAP quoted string (control characters become spaces).
pub fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The name of a mailbox as the reader sees it: Modified UTF-7 (RFC 3501
/// 5.1.3) decoded (`&AOQ-` is ä); a name that is not valid stays as it is.
pub fn mailbox_display(name: &str) -> String {
    if !name.contains('&') {
        return String::from(name);
    }
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('-') else {
            return String::from(name);
        };
        let body = &after[..end];
        if body.is_empty() {
            out.push('&');
        } else {
            let mut standard = String::from(body);
            standard = standard.replace(',', "/");
            let bytes = crate::codec::decode_base64(standard.as_bytes());
            if !bytes.len().is_multiple_of(2) {
                return String::from(name);
            }
            let units: Vec<u16> = bytes.chunks(2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
            for c in char::decode_utf16(units) {
                out.push(c.unwrap_or('\u{fffd}'));
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The flags of a message as one letter each: S seen, F flagged, A
/// answered, D deleted, R draft.
pub fn flag_letters(flags: &[String]) -> String {
    let mut out = String::new();
    for flag in flags {
        let letter = match flag.to_ascii_lowercase().as_str() {
            "\\seen" => 'S',
            "\\flagged" => 'F',
            "\\answered" => 'A',
            "\\deleted" => 'D',
            "\\draft" => 'R',
            _ => continue,
        };
        if !out.contains(letter) {
            out.push(letter);
        }
    }
    out
}
