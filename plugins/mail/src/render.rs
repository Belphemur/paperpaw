//! A message (RFC 5322 and MIME) as text for the screen: the text part
//! (plain, else HTML made text) decoded and in UTF-8, the attachments
//! named. It reads the message from storage a piece at a time and writes
//! the text to storage, so a message of any size takes the same little
//! memory; what does not fit the limit is cut, and said.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::{Storage, StorageError};

use crate::codec::{self, Base64, Charset, Decoder};
use crate::header::{self, ContentType, Headers};
use crate::html::HtmlText;

/// The most of a header block that is read (a longer one is cut).
const MAX_HEADERS: usize = 16 * 1024;
/// The longest line taken whole; a longer one comes as several.
const MAX_LINE: usize = 8 * 1024;
/// Parts inside parts, at most.
const MAX_DEPTH: usize = 8;
/// The most parts looked at.
const MAX_PARTS: usize = 256;

/// What the headers and the structure of a message say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rendered {
    pub from: Vec<header::Address>,
    pub to: Vec<header::Address>,
    pub cc: Vec<header::Address>,
    pub reply_to: Vec<header::Address>,
    pub subject: String,
    /// Seconds since 1970, where the Date header could be read.
    pub date: Option<i64>,
    pub message_id: String,
    pub references: String,
    /// `name (type)` of each attachment.
    pub attachments: Vec<String>,
    /// Whether the text was cut at the limit.
    pub truncated: bool,
    /// Bytes of text written.
    pub text_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Identity,
    Qp,
    Base64,
}

#[derive(Debug)]
struct Frame {
    /// `--boundary`
    boundary: Vec<u8>,
    alternative: bool,
    plain_seen: bool,
}

#[derive(Debug)]
struct Body {
    html: Option<HtmlText>,
    encoding: Encoding,
    base64: Base64,
    decoder: Decoder,
}

#[derive(Debug)]
enum Mode {
    /// The headers of a part, as far as they came.
    Headers(Vec<u8>),
    /// Before the first boundary of a multipart, and after the last.
    Skip,
    Body(Body),
}

/// The walk through a message, line by line.
#[derive(Debug)]
pub struct Walker {
    mode: Mode,
    frames: Vec<Frame>,
    top: bool,
    parts: usize,
    limit: usize,
    /// The text so far, not yet taken.
    text: String,
    written: usize,
    /// Whether text of a part came already (a second is set apart).
    any: bool,
    pub info: Rendered,
}

impl Walker {
    pub fn new(limit: usize) -> Self {
        Self {
            mode: Mode::Headers(Vec::new()),
            frames: Vec::new(),
            top: true,
            parts: 0,
            limit,
            text: String::new(),
            written: 0,
            any: false,
            info: Rendered::default(),
        }
    }

    /// The text made since the last call.
    pub fn take_text(&mut self) -> String {
        core::mem::take(&mut self.text)
    }

    /// Whether the text reached the limit: the rest is not needed.
    pub fn full(&self) -> bool {
        self.info.truncated
    }

    /// One line of the message, without its line break.
    pub fn line(&mut self, line: &[u8]) {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if let Some((frame, kind)) = self.boundary_of(line) {
            self.end_part();
            match kind {
                Boundary::Part => {
                    self.frames.truncate(frame + 1);
                    self.mode = Mode::Headers(Vec::new());
                }
                Boundary::End => {
                    self.frames.truncate(frame);
                    self.mode = Mode::Skip;
                }
            }
            return;
        }
        match &mut self.mode {
            Mode::Skip => {}
            Mode::Headers(raw) => {
                if line.is_empty() {
                    let raw = core::mem::take(raw);
                    self.begin_part(&raw);
                } else if raw.len() < MAX_HEADERS {
                    raw.extend_from_slice(line);
                    raw.push(b'\n');
                }
            }
            Mode::Body(_) => self.body_line(line),
        }
    }

    /// The message ends: what is open is closed.
    pub fn finish(&mut self) {
        // A message of only headers (no empty line after them).
        if let Mode::Headers(raw) = &mut self.mode {
            let raw = core::mem::take(raw);
            if !raw.is_empty() {
                self.begin_part(&raw);
            }
        }
        self.end_part();
        self.mode = Mode::Skip;
    }

    /// Which multipart's boundary `line` is (the innermost that has it),
    /// and whether it starts a part or ends the multipart.
    fn boundary_of(&self, line: &[u8]) -> Option<(usize, Boundary)> {
        if !line.starts_with(b"--") {
            return None;
        }
        let (index, frame) = self
            .frames
            .iter()
            .enumerate()
            .rev()
            .find(|(_, f)| line.starts_with(&f.boundary))?;
        let rest = &line[frame.boundary.len()..];
        if rest.starts_with(b"--") {
            Some((index, Boundary::End))
        } else if rest.iter().all(|b| matches!(b, b' ' | b'\t' | b'\r')) {
            Some((index, Boundary::Part))
        } else {
            None
        }
    }

    /// The headers of a part are complete: what the part is.
    fn begin_part(&mut self, raw: &[u8]) {
        let headers = Headers::parse(raw);
        let top = core::mem::replace(&mut self.top, false);
        if top {
            self.top_headers(&headers);
        }
        self.parts += 1;
        if self.parts > MAX_PARTS {
            self.mode = Mode::Skip;
            return;
        }
        let kind = ContentType::parse(headers.get("content-type"));
        if kind.is_multipart() {
            let boundary = kind.param("boundary").filter(|b| !b.is_empty());
            match boundary {
                Some(boundary) if self.frames.len() < MAX_DEPTH => {
                    let mut line = Vec::with_capacity(boundary.len() + 2);
                    line.extend_from_slice(b"--");
                    line.extend_from_slice(boundary.as_bytes());
                    self.frames.push(Frame {
                        boundary: line,
                        alternative: kind.kind == "multipart/alternative",
                        plain_seen: false,
                    });
                    self.mode = Mode::Skip;
                }
                _ => self.mode = Mode::Skip,
            }
            return;
        }
        let disposition = headers.get("content-disposition");
        let attached = disposition.is_some_and(|d| {
            let lower: String = d.chars().take(10).map(|c| c.to_ascii_lowercase()).collect();
            lower.starts_with("attachment")
        });
        let is_text = kind.kind == "text/plain" || kind.kind == "text/html";
        if !is_text || attached {
            self.attachment(&headers, &kind, disposition);
            self.mode = Mode::Skip;
            return;
        }
        let html = kind.kind == "text/html";
        // Of a pair of the same text, the plain one is read.
        if let Some(frame) = self.frames.last_mut()
            && frame.alternative
        {
            if html && frame.plain_seen {
                self.mode = Mode::Skip;
                return;
            }
            if !html {
                frame.plain_seen = true;
            }
        }
        let encoding = match headers.get("content-transfer-encoding").map(str::trim) {
            Some(e) if e.eq_ignore_ascii_case("quoted-printable") => Encoding::Qp,
            Some(e) if e.eq_ignore_ascii_case("base64") => Encoding::Base64,
            _ => Encoding::Identity,
        };
        let charset = kind
            .param("charset")
            .and_then(Charset::of)
            .unwrap_or(Charset::Utf8);
        if self.any {
            self.emit("\n\n");
        }
        self.any = true;
        self.mode = Mode::Body(Body {
            html: html.then(HtmlText::new),
            encoding,
            base64: Base64::default(),
            decoder: Decoder::new(charset),
        });
    }

    fn top_headers(&mut self, headers: &Headers) {
        let info = &mut self.info;
        let list = |name: &str| header::addresses(headers.get(name).unwrap_or(""));
        info.from = list("from");
        info.to = list("to");
        info.cc = list("cc");
        info.reply_to = list("reply-to");
        info.subject = String::from(headers.get("subject").unwrap_or("").trim());
        info.date = headers.get("date").and_then(header::date_seconds);
        info.message_id = String::from(headers.get("message-id").unwrap_or("").trim());
        info.references = String::from(headers.get("references").unwrap_or("").trim());
    }

    fn attachment(&mut self, headers: &Headers, kind: &ContentType, disposition: Option<&str>) {
        let from_disposition = disposition
            .map(|d| ContentType::parse(Some(&alloc::format!("x/x;{}", d.split_once(';').map_or("", |(_, p)| p)))));
        let name = from_disposition
            .as_ref()
            .and_then(|d| d.param("filename"))
            .or_else(|| kind.param("name"))
            .map(String::from)
            .unwrap_or_default();
        let _ = headers;
        let mut label = if name.is_empty() {
            String::from("(no name)")
        } else {
            name
        };
        label.push_str(" (");
        label.push_str(&kind.kind);
        label.push(')');
        if self.info.attachments.len() < 32 {
            self.info.attachments.push(label);
        }
    }

    fn body_line(&mut self, line: &[u8]) {
        let Mode::Body(body) = &mut self.mode else {
            return;
        };
        let mut out = String::new();
        match body.encoding {
            Encoding::Identity => {
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                body.decoder.push(line, &mut out);
                body.decoder.push(b"\n", &mut out);
            }
            Encoding::Qp => {
                let mut with_break = Vec::with_capacity(line.len() + 1);
                with_break.extend_from_slice(line.strip_suffix(b"\r").unwrap_or(line));
                with_break.push(b'\n');
                let bytes = codec::decode_qp(&with_break, false);
                body.decoder.push(&bytes, &mut out);
            }
            Encoding::Base64 => {
                let mut bytes = Vec::new();
                body.base64.push(line, &mut bytes);
                body.decoder.push(&bytes, &mut out);
            }
        }
        if let Some(html) = &mut body.html {
            let mut text = String::new();
            html.push(&out, &mut text);
            out = text;
        }
        self.emit(&out);
    }

    /// A part ends: what the decoders held back comes out.
    fn end_part(&mut self) {
        let Mode::Body(body) = &mut self.mode else {
            return;
        };
        let mut out = String::new();
        if body.encoding == Encoding::Base64 {
            let mut bytes = Vec::new();
            body.base64.finish(&mut bytes);
            body.decoder.push(&bytes, &mut out);
        }
        body.decoder.finish(&mut out);
        if let Some(html) = &mut body.html {
            let mut text = String::new();
            html.push(&out, &mut text);
            html.finish(&mut text);
            out = text;
        }
        self.emit(&out);
        self.mode = Mode::Skip;
    }

    /// Adds `text`, up to the limit.
    fn emit(&mut self, text: &str) {
        if self.info.truncated || text.is_empty() {
            return;
        }
        let room = self.limit.saturating_sub(self.written);
        if text.len() <= room {
            self.text.push_str(text);
            self.written += text.len();
            return;
        }
        let mut cut = room;
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        self.text.push_str(&text[..cut]);
        self.written += cut;
        self.info.truncated = true;
    }
}

#[derive(Debug, Clone, Copy)]
enum Boundary {
    Part,
    End,
}

/// The pieces a message is read in.
const CHUNK: usize = 1024;

/// The message in `source` as text in `out` (replaced), at most `limit`
/// bytes of it, and what its headers say. The message is read and the text
/// written in pieces.
pub fn render(
    storage: &dyn Storage,
    source: &str,
    out: &str,
    limit: usize,
) -> Result<Rendered, StorageError> {
    let mut walker = Walker::new(limit);
    storage.write(out, b"")?;
    let mut offset = 0u64;
    let mut line: Vec<u8> = Vec::new();
    let mut buffer = [0u8; CHUNK];
    'read: loop {
        let n = storage.read_at(source, offset, &mut buffer)?;
        if n == 0 {
            break;
        }
        offset += n as u64;
        for &byte in &buffer[..n] {
            if byte == b'\n' {
                walker.line(&line);
                line.clear();
            } else if line.len() < MAX_LINE {
                line.push(byte);
            } else {
                walker.line(&line);
                line.clear();
                line.push(byte);
            }
        }
        flush(storage, out, &mut walker)?;
        if walker.full() {
            break 'read;
        }
    }
    if !line.is_empty() {
        walker.line(&line);
    }
    walker.finish();
    flush(storage, out, &mut walker)?;
    let mut info = walker.info;
    info.text_len = walker.written;
    Ok(info)
}

fn flush(storage: &dyn Storage, out: &str, walker: &mut Walker) -> Result<(), StorageError> {
    let text = walker.take_text();
    if !text.is_empty() {
        storage.append(out, text.as_bytes())?;
    }
    Ok(())
}
