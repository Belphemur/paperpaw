//! The headers of a message (RFC 5322): folded lines unfolded, their
//! values as text, the addresses and the date in them, and the
//! Content-Type of a part.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec;

/// The headers of a message or a part: name (lower case) and value
/// (unfolded, encoded words decoded), in the order they came.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers {
    fields: Vec<(String, String)>,
}

impl Headers {
    /// The headers in `raw` (up to an empty line, if there is one). A
    /// line that is no header (no colon, a continuation without a header)
    /// is skipped.
    pub fn parse(raw: &[u8]) -> Self {
        let mut fields: Vec<(String, Vec<u8>)> = Vec::new();
        for line in raw.split(|&b| b == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                break;
            }
            if matches!(line[0], b' ' | b'\t') {
                if let Some((_, value)) = fields.last_mut() {
                    value.push(b' ');
                    value.extend_from_slice(trim(line));
                }
                continue;
            }
            let Some(colon) = line.iter().position(|&b| b == b':') else {
                continue;
            };
            let name = &line[..colon];
            if name.is_empty() || name.iter().any(|b| !(33..=126).contains(b)) {
                continue;
            }
            let mut lower = String::with_capacity(name.len());
            lower.extend(name.iter().map(|b| char::from(b.to_ascii_lowercase())));
            fields.push((lower, trim(&line[colon + 1..]).to_vec()));
        }
        Self {
            fields: fields
                .into_iter()
                .map(|(name, value)| {
                    let text = if name == "content-type" || name == "content-disposition" {
                        // Parameters are parsed from the raw text (see
                        // `ContentType`), not as encoded words.
                        String::from_utf8_lossy(&value).into_owned()
                    } else {
                        codec::decode_words(&value)
                    };
                    (name, text)
                })
                .collect(),
        }
    }

    /// The first header `name` (lower case).
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

fn trim(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|b| !matches!(b, b' ' | b'\t')).unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .map_or(start, |p| p + 1);
    &bytes[start..end.max(start)]
}

/// A Content-Type (or Content-Disposition) value: the type, in lower
/// case, and its parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentType {
    /// `text/plain`, `multipart/alternative`; `text/plain` where the
    /// header is missing (RFC 2045).
    pub kind: String,
    params: Vec<(String, String)>,
}

impl ContentType {
    /// The value of a Content-Type header (`None`: the default).
    pub fn parse(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self {
                kind: String::from("text/plain"),
                params: Vec::new(),
            };
        };
        let mut parts = split_params(value).into_iter();
        let mut kind = String::new();
        kind.extend(
            parts
                .next()
                .unwrap_or_default()
                .trim()
                .chars()
                .map(|c| c.to_ascii_lowercase()),
        );
        if !kind.contains('/') {
            kind = String::from("text/plain");
        }
        let mut params: Vec<(String, String)> = Vec::new();
        for part in parts {
            let Some((name, value)) = part.split_once('=') else {
                continue;
            };
            let mut name = String::from(name.trim());
            name.make_ascii_lowercase();
            let value = unquote(value.trim());
            // RFC 2231: `name*=utf-8''%C3%A4` and pieces `name*0=…`.
            if let Some((base, rest)) = name.split_once('*') {
                let extended = rest.ends_with('*') || rest.is_empty();
                let value = if extended { percent_decoded(&value) } else { value };
                let base = String::from(base);
                match params.iter_mut().find(|(n, _)| *n == base) {
                    Some((_, existing)) if !rest.is_empty() && !rest.starts_with('0') => {
                        existing.push_str(&value)
                    }
                    Some(_) => {}
                    None => params.push((base, value)),
                }
                continue;
            }
            params.push((name, value));
        }
        Self { kind, params }
    }

    /// A parameter, e.g. `boundary` or `charset`.
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Whether it is `multipart/…`.
    pub fn is_multipart(&self) -> bool {
        self.kind.starts_with("multipart/")
    }
}

/// `value` cut at the `;` outside quotes.
fn split_params(value: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (0, false);
    for (i, c) in value.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                out.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&value[start..]);
    out
}

fn unquote(value: &str) -> String {
    let Some(inner) = value.strip_prefix('"') else {
        return String::from(value);
    };
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => break,
            '\\' => out.extend(chars.next()),
            c => out.push(c),
        }
    }
    out
}

/// RFC 2231's `utf-8''%C3%A4`: the text with its percent signs undone.
fn percent_decoded(value: &str) -> String {
    let (charset, rest) = match value.split_once('\'') {
        Some((charset, rest)) => (charset, rest.split_once('\'').map_or(rest, |(_, text)| text)),
        None => ("", value),
    };
    let mut bytes = Vec::with_capacity(rest.len());
    let raw = rest.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%'
            && let (Some(h), Some(l)) = (
                raw.get(i + 1).and_then(|b| char::from(*b).to_digit(16)),
                raw.get(i + 2).and_then(|b| char::from(*b).to_digit(16)),
            )
        {
            bytes.push((h * 16 + l) as u8);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    codec::text_of(codec::Charset::of(charset).unwrap_or(codec::Charset::Utf8), &bytes)
}

/// An address as a header gives it: `Ada <ada@example.org>`,
/// `"Lovelace, Ada" <ada@example.org>`, `ada@example.org`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Address {
    /// The name shown, empty where there is none.
    pub name: String,
    /// `ada@example.org`.
    pub address: String,
}

impl Address {
    /// The name, else the address.
    pub fn shown(&self) -> &str {
        if self.name.is_empty() {
            &self.address
        } else {
            &self.name
        }
    }
}

/// The addresses of a header's value, comma apart (a comma in a quoted
/// name or a group does not part them). Comments in parentheses go.
pub fn addresses(value: &str) -> Vec<Address> {
    let mut out = Vec::new();
    let (mut start, mut quoted, mut angle, mut comment) = (0, false, false, 0u32);
    let mut pieces = Vec::new();
    for (i, c) in value.char_indices() {
        match c {
            '"' if comment == 0 => quoted = !quoted,
            '<' if !quoted && comment == 0 => angle = true,
            '>' if !quoted && comment == 0 => angle = false,
            '(' if !quoted => comment += 1,
            ')' if !quoted && comment > 0 => comment -= 1,
            ',' | ';' if !quoted && !angle && comment == 0 => {
                pieces.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    pieces.push(&value[start..]);
    for piece in pieces {
        if let Some(address) = one_address(piece) {
            out.push(address);
        }
    }
    out
}

fn one_address(piece: &str) -> Option<Address> {
    let piece = piece.trim();
    // `Group: a@b` leaves the group's name out.
    let piece = match piece.split_once(':') {
        Some((_, rest)) if !piece.contains('<') || piece.find(':') < piece.find('<') => rest.trim(),
        _ => piece,
    };
    if let Some(open) = piece.rfind('<') {
        let close = piece[open..].find('>').map_or(piece.len(), |c| open + c);
        let address = piece[open + 1..close].trim();
        if !address.contains('@') {
            return None;
        }
        let name = unquote(piece[..open].trim());
        return Some(Address {
            name: String::from(name.trim()),
            address: String::from(address),
        });
    }
    // Without brackets: the address, maybe with a comment `(Ada)`.
    let mut bare = String::new();
    let mut comment = None;
    let mut depth = 0u32;
    let mut note = String::new();
    for c in piece.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    comment = Some(note.clone());
                }
            }
            c if depth > 0 => note.push(c),
            c => bare.push(c),
        }
    }
    let bare = bare.trim();
    if !bare.contains('@') || bare.contains(char::is_whitespace) {
        return None;
    }
    Some(Address {
        name: comment.unwrap_or_default().trim().into(),
        address: String::from(bare),
    })
}

/// The date of a Date header (RFC 5322: `Tue, 3 Nov 2026 09:15:00 +0100`)
/// as seconds since 1970; `None` where it cannot be read.
pub fn date_seconds(value: &str) -> Option<i64> {
    let value = value.trim();
    // Drop a weekday and a comment `(CET)`.
    let value = value.split_once(',').map_or(value, |(_, rest)| rest);
    let value = value.split('(').next().unwrap_or(value);
    let mut words = value.split_whitespace();
    let day: i64 = words.next()?.parse().ok()?;
    let month = month_of(words.next()?)?;
    let mut year: i64 = words.next()?.parse().ok()?;
    // Two digits: RFC 5322's rule.
    if year < 50 {
        year += 2000;
    } else if year < 100 {
        year += 1900;
    }
    let time = words.next().unwrap_or("00:00:00");
    let mut clock = time.split(':');
    let hour: i64 = clock.next()?.parse().ok()?;
    let minute: i64 = clock.next().unwrap_or("0").parse().ok()?;
    let second: i64 = clock.next().unwrap_or("0").parse().ok()?;
    let zone = words.next().unwrap_or("+0000");
    let offset = zone_seconds(zone);
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

fn month_of(name: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let lower: String = name.chars().take(3).map(|c| c.to_ascii_lowercase()).collect();
    MONTHS.iter().position(|m| *m == lower).map(|i| i as i64 + 1)
}

fn zone_seconds(zone: &str) -> i64 {
    let named = match zone {
        "GMT" | "UT" | "UTC" | "Z" => Some(0),
        "EST" => Some(-5),
        "EDT" => Some(-4),
        "CST" => Some(-6),
        "CDT" => Some(-5),
        "MST" => Some(-7),
        "MDT" => Some(-6),
        "PST" => Some(-8),
        "PDT" => Some(-7),
        "CET" => Some(1),
        "CEST" => Some(2),
        _ => None,
    };
    if let Some(hours) = named {
        return hours * 3600;
    }
    let (sign, digits) = match zone.as_bytes().first() {
        Some(b'+') => (1, &zone[1..]),
        Some(b'-') => (-1, &zone[1..]),
        _ => return 0,
    };
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return 0;
    }
    let number: i64 = digits.parse().unwrap_or(0);
    sign * ((number / 100) * 3600 + (number % 100) * 60)
}

/// Days from 1970-01-01 to the civil date (proleptic Gregorian).
pub fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date (year, month, day) of `days` since 1970-01-01.
pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `subject` for a reply: `Re: ` once, whatever case a former one had.
pub fn reply_subject(subject: &str) -> String {
    let subject = subject.trim();
    let lower: String = subject.chars().take(3).map(|c| c.to_ascii_lowercase()).collect();
    if lower == "re:" {
        String::from(subject)
    } else {
        let mut out = String::from("Re: ");
        out.push_str(subject);
        out
    }
}
