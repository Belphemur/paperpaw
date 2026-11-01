//! What is made of a book when it is first opened, kept in the book's
//! folder of the plugin (moved and removed with the book): its blocks of
//! text in `blocks.bin`, where they are in `index.bin` (a start, an end
//! and a place in `blocks.bin`, 12 bytes each, in order, so a block is
//! found by halving), its links in `links.bin` (a start, an end, the
//! position it leads to) and its table of contents in `toc.idx` and
//! `toc.bin`, the addresses of its links to the web in `urls.idx` and
//! `urls.bin`, and the places of its bookmarks in `marks.bin` (a hash of the
//! name, a position, 8 bytes each). `done` is written last: a folder
//! without it is made anew.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::document::{Align, DocumentError, Link, OffsetMap, Run, TextStyle};
use paperpaw_sdk::{Storage, StorageError};

pub(crate) const BLOCKS: &str = "blocks.bin";
pub(crate) const INDEX: &str = "index.bin";
pub(crate) const LINKS: &str = "links.bin";
pub(crate) const TOC_INDEX: &str = "toc.idx";
pub(crate) const TOC: &str = "toc.bin";
pub(crate) const URLS_INDEX: &str = "urls.idx";
pub(crate) const URLS: &str = "urls.bin";
pub(crate) const MARKS: &str = "marks.bin";
pub(crate) const DONE: &str = "done";
/// A link's value with this bit set is the number of an address in
/// `urls.bin`; without it, the hash of a bookmark's name.
pub(crate) const EXTERNAL: u32 = 1 << 31;
/// Most addresses and bookmarks kept.
const MOST_URLS: u32 = 4096;
const MOST_MARKS: u32 = 20_000;

/// The most the cache files may hold (bytes of `blocks.bin`).
const MOST_BLOCKS: usize = 96 * 1024 * 1024;
/// Written to the card when this many bytes wait.
const FLUSH: usize = 6 * 1024;
/// The most a block's text may be: the block's own length field is 16 bits.
const MOST_TEXT: usize = 60_000;
/// Most runs or checkpoints a block holds.
const MOST_PARTS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Paragraph,
    /// A heading of level 1 to 6.
    Heading(u8),
    Image,
}

impl Kind {
    fn code(self) -> u8 {
        match self {
            Kind::Paragraph => 0,
            Kind::Heading(level) => level.clamp(1, 6),
            Kind::Image => 7,
        }
    }

    fn of(code: u8) -> Kind {
        match code {
            1..=6 => Kind::Heading(code),
            7 => Kind::Image,
            _ => Kind::Paragraph,
        }
    }
}

/// A block to write.
pub(crate) struct Block<'a> {
    pub kind: Kind,
    pub align: Align,
    pub continues: bool,
    pub continued: bool,
    pub start: u32,
    pub end: u32,
    pub text: &'a str,
    /// Text indexes `from..to` in a style (the bits of a `TextStyle`).
    pub runs: &'a [(u16, u16, u16)],
    /// Where the text and the source part from each other: a text index
    /// and the source offset it is at.
    pub checks: &'a [(u16, u32)],
    /// A picture: its record counted from the first picture, and the size
    /// it is decoded to.
    pub image: Option<(u32, u32, u32)>,
}

/// What is written, waiting to be put on the card.
#[derive(Debug, Default)]
pub(crate) struct Output {
    blocks: Vec<u8>,
    index: Vec<u8>,
    links: Vec<u8>,
    toc_index: Vec<u8>,
    toc: Vec<u8>,
    urls_index: Vec<u8>,
    urls: Vec<u8>,
    marks: Vec<u8>,
    written: usize,
    toc_written: usize,
    urls_written: usize,
    urls_count: u32,
    marks_count: u32,
    error: Option<StorageError>,
    full: bool,
    pub(crate) count: u32,
}

impl Output {
    pub(crate) fn failed(&self) -> bool {
        self.error.is_some() || self.full
    }

    pub(crate) fn block(&mut self, block: &Block<'_>) {
        let text = block.text.as_bytes();
        let text = &text[..text.len().min(MOST_TEXT)];
        let at = self.written + self.blocks.len();
        if at + text.len() + 1024 > MOST_BLOCKS {
            self.full = true;
            return;
        }
        self.index.extend_from_slice(&block.start.to_le_bytes());
        self.index.extend_from_slice(&block.end.to_le_bytes());
        self.index.extend_from_slice(&(at as u32).to_le_bytes());
        self.count += 1;
        let flags =
            u8::from(block.continues) | u8::from(block.continued) << 1 | block.align.bits() << 4;
        let out = &mut self.blocks;
        out.push(block.kind.code());
        out.push(flags);
        out.extend_from_slice(&(text.len() as u16).to_le_bytes());
        out.extend_from_slice(&(block.runs.len().min(MOST_PARTS) as u16).to_le_bytes());
        out.extend_from_slice(&(block.checks.len().min(MOST_PARTS) as u16).to_le_bytes());
        if let Some((record, width, height)) = block.image {
            for n in [record, width, height] {
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        out.extend_from_slice(text);
        for &(from, to, style) in block.runs.iter().take(MOST_PARTS) {
            for n in [from, to, style] {
                out.extend_from_slice(&n.to_le_bytes());
            }
        }
        for &(index, source) in block.checks.iter().take(MOST_PARTS) {
            out.extend_from_slice(&index.to_le_bytes());
            out.extend_from_slice(&source.to_le_bytes());
        }
    }

    pub(crate) fn link(&mut self, start: u32, end: u32, position: u32) {
        for n in [start, end, position] {
            self.links.extend_from_slice(&n.to_le_bytes());
        }
    }

    /// Keeps the address `url`; its number for a link.
    pub(crate) fn url(&mut self, url: &str) -> Option<u32> {
        if self.urls_count >= MOST_URLS {
            return None;
        }
        let place = (self.urls_written + self.urls.len()) as u32;
        self.urls_index.extend_from_slice(&place.to_le_bytes());
        self.urls
            .extend_from_slice(&(url.len() as u16).to_le_bytes());
        self.urls.extend_from_slice(url.as_bytes());
        self.urls_count += 1;
        Some(self.urls_count - 1)
    }

    /// The bookmark with hash `hash` is at `position`.
    pub(crate) fn mark(&mut self, hash: u32, position: u32) {
        if self.marks_count < MOST_MARKS {
            self.marks.extend_from_slice(&hash.to_le_bytes());
            self.marks.extend_from_slice(&position.to_le_bytes());
            self.marks_count += 1;
        }
    }

    pub(crate) fn toc(&mut self, at: u32, level: u8, title: &str) {
        let title = title.as_bytes();
        let mut len = title.len().min(255);
        while !core::str::from_utf8(&title[..len]).is_ok() {
            len -= 1;
        }
        let place = (self.toc_written + self.toc.len()) as u32;
        self.toc_index.extend_from_slice(&place.to_le_bytes());
        self.toc.extend_from_slice(&at.to_le_bytes());
        self.toc.push(level);
        self.toc.push(len as u8);
        self.toc.extend_from_slice(&title[..len]);
    }

    /// Puts what waits on the card, in the folder `dir`; all of it when
    /// `all`, else once enough waits.
    pub(crate) fn flush(&mut self, storage: &dyn Storage, dir: &str, all: bool) {
        if self.error.is_some() {
            return;
        }
        let waiting = self.blocks.len()
            + self.index.len()
            + self.links.len()
            + self.toc.len()
            + self.toc_index.len()
            + self.urls.len()
            + self.urls_index.len()
            + self.marks.len();
        if !all && waiting < FLUSH {
            return;
        }
        let mut result = Ok(());
        for (name, bytes) in [
            (BLOCKS, &mut self.blocks),
            (INDEX, &mut self.index),
            (LINKS, &mut self.links),
            (TOC_INDEX, &mut self.toc_index),
            (TOC, &mut self.toc),
            (URLS_INDEX, &mut self.urls_index),
            (URLS, &mut self.urls),
            (MARKS, &mut self.marks),
        ] {
            if bytes.is_empty() {
                continue;
            }
            let path = alloc::format!("{dir}/{name}");
            result = result.and(storage.append(&path, bytes));
            match name {
                BLOCKS => self.written += bytes.len(),
                TOC => self.toc_written += bytes.len(),
                URLS => self.urls_written += bytes.len(),
                _ => {}
            }
            bytes.clear();
        }
        if let Err(error) = result {
            self.error = Some(error);
        }
    }

    pub(crate) fn error(&mut self) -> Option<DocumentError> {
        if let Some(error) = self.error.take() {
            return Some(error.into());
        }
        self.full
            .then_some(DocumentError::Malformed(alloc::borrow::Cow::Borrowed(
                "the book is too long",
            )))
    }
}

/// A block read from the cache.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Loaded {
    pub kind: Kind,
    pub align: Align,
    pub continues: bool,
    pub continued: bool,
    pub start: u32,
    pub end: u32,
    pub text: String,
    pub runs: Vec<Run>,
    pub map: OffsetMap,
    pub image: Option<(u32, u32, u32)>,
}

fn u32_at(storage: &dyn Storage, path: &str, at: u64) -> Option<u32> {
    let mut bytes = [0u8; 4];
    (storage.read_at(path, at, &mut bytes).ok()? == 4).then(|| u32::from_le_bytes(bytes))
}

/// How many blocks the cache holds.
pub(crate) fn count(storage: &dyn Storage, dir: &str) -> u32 {
    storage
        .size(&alloc::format!("{dir}/{INDEX}"))
        .map_or(0, |size| (size / 12) as u32)
}

/// The block that has source offset `at`, or the first after it.
pub(crate) fn block_at(storage: &dyn Storage, dir: &str, at: u32) -> Option<Loaded> {
    let index = alloc::format!("{dir}/{INDEX}");
    let (mut low, mut high) = (0u64, u64::from(count(storage, dir)));
    // The first block that ends after `at`.
    while low < high {
        let mid = (low + high) / 2;
        let end = u32_at(storage, &index, mid * 12 + 4)?;
        if end > at {
            high = mid;
        } else {
            low = mid + 1;
        }
    }
    let entry = low * 12;
    let (start, end, place) = (
        u32_at(storage, &index, entry)?,
        u32_at(storage, &index, entry + 4)?,
        u32_at(storage, &index, entry + 8)?,
    );
    load(storage, dir, start, end, place)
}

fn load(storage: &dyn Storage, dir: &str, start: u32, end: u32, place: u32) -> Option<Loaded> {
    let path = alloc::format!("{dir}/{BLOCKS}");
    let mut head = [0u8; 20];
    let n = storage.read_at(&path, u64::from(place), &mut head).ok()?;
    if n < 8 {
        return None;
    }
    let kind = Kind::of(head[0]);
    let flags = head[1];
    let len = usize::from(u16::from_le_bytes([head[2], head[3]]));
    let n_runs = usize::from(u16::from_le_bytes([head[4], head[5]]));
    let n_checks = usize::from(u16::from_le_bytes([head[6], head[7]]));
    if n_runs > MOST_PARTS || n_checks > MOST_PARTS {
        return None;
    }
    let mut at = u64::from(place) + 8;
    let image = if kind == Kind::Image {
        if n < 20 {
            return None;
        }
        at += 12;
        let word = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        Some((word(8), word(12), word(16)))
    } else {
        None
    };
    let mut body = alloc::vec![0u8; len + n_runs * 6 + n_checks * 6];
    let n = storage.read_at(&path, at, &mut body).ok()?;
    if n < body.len() {
        return None;
    }
    let text = String::from_utf8_lossy(&body[..len]).into_owned();
    let word = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
    let runs = (0..n_runs)
        .map(|i| {
            let at = len + i * 6;
            Run::new(
                u32::from(word(at)),
                u32::from(word(at + 2)),
                TextStyle::from_bits(u32::from(word(at + 4))),
            )
        })
        .filter(|r| r.start < r.end && r.end as usize <= text.len())
        .collect();
    let mut map = OffsetMap::default();
    for i in 0..n_checks {
        let at = len + n_runs * 6 + i * 6;
        let source = u32::from_le_bytes([body[at + 2], body[at + 3], body[at + 4], body[at + 5]]);
        map.checkpoint(usize::from(word(at)), source);
    }
    Some(Loaded {
        kind,
        align: Align::from_bits(flags >> 4),
        continues: flags & 1 != 0,
        continued: flags & 2 != 0,
        start,
        end,
        text,
        runs,
        map,
        image,
    })
}

/// The links that overlap the source offsets `from..to`, in order.
pub(crate) fn links(storage: &dyn Storage, dir: &str, from: u32, to: u32) -> Vec<Link> {
    let path = alloc::format!("{dir}/{LINKS}");
    let count = storage.size(&path).map_or(0, |size| size / 12);
    // The first link that ends after `from`: links are in order.
    let (mut low, mut high) = (0u64, count);
    while low < high {
        let mid = (low + high) / 2;
        match u32_at(storage, &path, mid * 12 + 4) {
            Some(end) if end > from => high = mid,
            Some(_) => low = mid + 1,
            None => return Vec::new(),
        }
    }
    let mut found = Vec::new();
    for i in low..count.min(low + 256) {
        let (Some(start), Some(end), Some(position)) = (
            u32_at(storage, &path, i * 12),
            u32_at(storage, &path, i * 12 + 4),
            u32_at(storage, &path, i * 12 + 8),
        ) else {
            break;
        };
        if start >= to {
            break;
        }
        let href = if position & EXTERNAL != 0 {
            let Some(url) = url_at(storage, dir, position & !EXTERNAL) else {
                continue;
            };
            url
        } else {
            alloc::format!("#{position}")
        };
        found.push(Link::new(start, end, href));
    }
    found
}

/// How many entries the table of contents has.
pub(crate) fn toc_len(storage: &dyn Storage, dir: &str) -> u32 {
    storage
        .size(&alloc::format!("{dir}/{TOC_INDEX}"))
        .map_or(0, |size| (size / 4) as u32)
}

/// Table of contents entry `index`: where it is, its level and title.
pub(crate) fn toc_entry(storage: &dyn Storage, dir: &str, index: u32) -> Option<(u32, u8, String)> {
    let place = u64::from(u32_at(
        storage,
        &alloc::format!("{dir}/{TOC_INDEX}"),
        u64::from(index) * 4,
    )?);
    let path = alloc::format!("{dir}/{TOC}");
    let mut head = [0u8; 6];
    if storage.read_at(&path, place, &mut head).ok()? < 6 {
        return None;
    }
    let mut title = alloc::vec![0u8; usize::from(head[5])];
    if storage.read_at(&path, place + 6, &mut title).ok()? < title.len() {
        return None;
    }
    Some((
        u32::from_le_bytes([head[0], head[1], head[2], head[3]]),
        head[4],
        String::from_utf8_lossy(&title).into_owned(),
    ))
}

/// Address number `index` of the links to the web.
fn url_at(storage: &dyn Storage, dir: &str, index: u32) -> Option<String> {
    let place = u64::from(u32_at(
        storage,
        &alloc::format!("{dir}/{URLS_INDEX}"),
        u64::from(index) * 4,
    )?);
    let path = alloc::format!("{dir}/{URLS}");
    let mut head = [0u8; 2];
    if storage.read_at(&path, place, &mut head).ok()? < 2 {
        return None;
    }
    let mut url = alloc::vec![0u8; usize::from(u16::from_le_bytes(head))];
    if storage.read_at(&path, place + 2, &mut url).ok()? < url.len() {
        return None;
    }
    String::from_utf8(url).ok()
}

/// Where the bookmark with hash `hash` is: the first of that name.
pub(crate) fn mark(storage: &dyn Storage, dir: &str, hash: u32) -> Option<u32> {
    let path = alloc::format!("{dir}/{MARKS}");
    let mut chunk = [0u8; 512];
    let mut at = 0u64;
    loop {
        let n = storage.read_at(&path, at, &mut chunk).ok()?;
        if n < 8 {
            return None;
        }
        for pair in chunk[..n - n % 8].as_chunks::<8>().0 {
            if u32::from_le_bytes([pair[0], pair[1], pair[2], pair[3]]) == hash {
                return Some(u32::from_le_bytes([pair[4], pair[5], pair[6], pair[7]]));
            }
        }
        at += n as u64;
    }
}
