//! What the small parts of a Word document say: styles, lists, the
//! relationships (where pictures and links lead) and the properties (title,
//! author, language); and small helpers for its XML.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::Storage;
use paperpaw_sdk::document::DocumentError;
use paperpaw_sdk::xml::{Attrs, Token, Tokenizer, decode, local_name};
use paperpaw_sdk::zip::{Entry, ZipArchive};

use crate::{
    BOLD, ITALIC, MAX_ADDRESS, MAX_LISTS, MAX_NAME, MAX_REL_BYTES, MAX_RELS, MAX_SMALL, MAX_STYLES,
    MAX_TITLE,
};

/// The error for a damaged or failing archive.
pub(crate) fn error(e: paperpaw_sdk::zip::ZipError) -> DocumentError {
    match e {
        paperpaw_sdk::zip::ZipError::Storage(e) => DocumentError::Storage(e),
        paperpaw_sdk::zip::ZipError::OutOfMemory => DocumentError::OutOfMemory,
        other => DocumentError::Malformed(Cow::Owned(other.to_string())),
    }
}

pub(crate) fn malformed(message: &'static str) -> DocumentError {
    DocumentError::Malformed(Cow::Borrowed(message))
}

/// The value of attribute `name` without its prefix (`w:val` as `val`).
pub(crate) fn attr<'a>(attrs: &Attrs<'a>, name: &str) -> Option<Cow<'a, str>> {
    attrs
        .iter()
        .find(|(n, _)| local_name(n) == name)
        .map(|(_, value)| decode(value))
}

/// Streams the XML part `entry` through `handle`.
pub(crate) fn xml_part(
    storage: &dyn Storage,
    archive: &ZipArchive,
    entry: &Entry,
    handle: &mut dyn FnMut(Token<'_>),
) -> Result<(), DocumentError> {
    let mut tokenizer = Tokenizer::new();
    let mut failed = None;
    archive
        .stream(
            storage,
            entry,
            &mut |chunk| match tokenizer.try_feed(chunk, &mut *handle) {
                Ok(()) => true,
                Err(_) => {
                    failed = Some(DocumentError::OutOfMemory);
                    false
                }
            },
        )
        .map_err(error)?;
    tokenizer.finish(handle);
    failed.map_or(Ok(()), Err)
}

/// The small part `name`, if there and not too large, streamed through
/// `handle`; a part that cannot be read is as if it was not there.
pub(crate) fn small_part(
    storage: &dyn Storage,
    archive: &ZipArchive,
    name: &str,
    handle: &mut dyn FnMut(Token<'_>),
) -> Result<(), DocumentError> {
    let Ok(Some(entry)) = archive.entry(storage, name) else {
        return Ok(());
    };
    if entry.size > MAX_SMALL {
        return Ok(());
    }
    match xml_part(storage, archive, &entry, handle) {
        Err(DocumentError::OutOfMemory) => Err(DocumentError::OutOfMemory),
        // A part that is damaged leaves what was read of it.
        _ => Ok(()),
    }
}

/// What a style says: how a paragraph in it is shown.
#[derive(Debug, Default)]
pub(crate) struct Style {
    pub(crate) id: String,
    /// The heading level (1 to 6), or 0.
    pub(crate) level: u8,
    /// Its `BOLD` and `ITALIC`.
    pub(crate) format: u8,
    /// The list its paragraphs are in (a list's number and level).
    pub(crate) list: Option<(u32, u8)>,
}

/// A relationship: where `id` points.
#[derive(Debug)]
pub(crate) struct Rel {
    pub(crate) id: String,
    pub(crate) target: String,
    pub(crate) external: bool,
    pub(crate) image: bool,
}

/// What the small parts say.
#[derive(Debug, Default)]
pub(crate) struct Parts {
    pub(crate) styles: Vec<Style>,
    /// The abstract list each list is of.
    pub(crate) numbers: Vec<(u32, u32)>,
    /// Whether the level of an abstract list is numbered: (list, level,
    /// numbered).
    pub(crate) levels: Vec<(u32, u8, bool)>,
    pub(crate) rels: Vec<Rel>,
    pub(crate) rel_bytes: usize,
    pub(crate) title: String,
    pub(crate) author: String,
    pub(crate) language: String,
}

impl Parts {
    pub(crate) fn style(&self, id: &str) -> Option<&Style> {
        self.styles.iter().find(|s| s.id == id)
    }

    pub(crate) fn rel(&self, id: &str) -> Option<&Rel> {
        self.rels.iter().find(|r| r.id == id)
    }

    /// Whether list `number` at `level` is numbered (not bulleted).
    pub(crate) fn numbered(&self, number: u32, level: u8) -> bool {
        let abstract_list = self.numbers.iter().find(|(n, _)| *n == number);
        abstract_list
            .and_then(|(_, a)| {
                self.levels
                    .iter()
                    .find(|(list, l, _)| list == a && *l == level)
            })
            .is_some_and(|(_, _, numbered)| *numbered)
    }
}

/// The heading level a style's name stands for (`heading 2`, `Title`).
pub(crate) fn heading_of(name: &str) -> u8 {
    let name = name.to_ascii_lowercase();
    if name == "title" {
        return 1;
    }
    name.strip_prefix("heading ")
        .and_then(|n| n.trim().parse::<u8>().ok())
        .filter(|n| (1..=9).contains(n))
        .map_or(0, |n| n.min(6))
}

pub(crate) fn flag(attrs: &Attrs<'_>) -> bool {
    !attr(attrs, "val").is_some_and(|v| matches!(v.as_ref(), "0" | "false" | "off" | "none"))
}

pub(crate) fn read_styles(
    storage: &dyn Storage,
    archive: &ZipArchive,
    parts: &mut Parts,
) -> Result<(), DocumentError> {
    let mut current = false;
    let (mut in_ppr, mut in_rpr) = (false, false);
    small_part(
        storage,
        archive,
        "word/styles.xml",
        &mut |token| match token {
            Token::Start { name, attrs, .. } => match local_name(name) {
                "style" if parts.styles.len() < MAX_STYLES => {
                    let id = attr(&attrs, "styleId").unwrap_or_default();
                    current = id.len() <= MAX_NAME && !id.is_empty();
                    if current {
                        parts.styles.push(Style {
                            id: id.into_owned(),
                            ..Style::default()
                        });
                    }
                }
                "name" if current => {
                    if let Some(style) = parts.styles.last_mut() {
                        let name = attr(&attrs, "val").unwrap_or_default();
                        style.level = style.level.max(heading_of(&name));
                        match name.to_ascii_lowercase().as_str() {
                            "strong" => style.format |= BOLD,
                            "emphasis" => style.format |= ITALIC,
                            _ => {}
                        }
                    }
                }
                "outlineLvl" if current => {
                    let level = attr(&attrs, "val").and_then(|v| v.parse::<u8>().ok());
                    if let (Some(style), Some(level)) = (parts.styles.last_mut(), level)
                        && level < 6
                        && style.level == 0
                    {
                        style.level = level + 1;
                    }
                }
                "numId" if current && in_ppr => {
                    if let (Some(style), Some(n)) = (
                        parts.styles.last_mut(),
                        attr(&attrs, "val").and_then(|v| v.parse::<u32>().ok()),
                    ) {
                        let level = style.list.map_or(0, |(_, l)| l);
                        style.list = Some((n, level));
                    }
                }
                "ilvl" if current && in_ppr => {
                    if let (Some(style), Some(l)) = (
                        parts.styles.last_mut(),
                        attr(&attrs, "val").and_then(|v| v.parse::<u8>().ok()),
                    ) {
                        let number = style.list.map_or(0, |(n, _)| n);
                        style.list = Some((number, l));
                    }
                }
                "pPr" => in_ppr = true,
                "rPr" => in_rpr = true,
                "b" | "i" if current && in_rpr && !in_ppr && flag(&attrs) => {
                    if let Some(style) = parts.styles.last_mut() {
                        style.format |= if local_name(name) == "b" {
                            BOLD
                        } else {
                            ITALIC
                        };
                    }
                }
                _ => {}
            },
            Token::End { name } => match local_name(name) {
                "style" => current = false,
                "pPr" => in_ppr = false,
                "rPr" => in_rpr = false,
                _ => {}
            },
            _ => {}
        },
    )
}

pub(crate) fn read_numbering(
    storage: &dyn Storage,
    archive: &ZipArchive,
    parts: &mut Parts,
) -> Result<(), DocumentError> {
    let (mut abstract_id, mut level, mut number): (Option<u32>, Option<u8>, Option<u32>) =
        (None, None, None);
    small_part(
        storage,
        archive,
        "word/numbering.xml",
        &mut |token| match token {
            Token::Start { name, attrs, .. } => {
                let value = || attr(&attrs, "val");
                match local_name(name) {
                    "abstractNum" => {
                        abstract_id = attr(&attrs, "abstractNumId").and_then(|v| v.parse().ok());
                    }
                    "lvl" => level = attr(&attrs, "ilvl").and_then(|v| v.parse().ok()),
                    "numFmt" => {
                        if let (Some(a), Some(l), Some(v)) = (abstract_id, level, value())
                            && parts.levels.len() < MAX_LISTS
                            && !parts.levels.iter().any(|e| e.0 == a && e.1 == l)
                        {
                            parts
                                .levels
                                .push((a, l, !matches!(v.as_ref(), "bullet" | "none")));
                        }
                    }
                    "num" => number = attr(&attrs, "numId").and_then(|v| v.parse().ok()),
                    "abstractNumId" if abstract_id.is_none() || number.is_some() => {
                        if let (Some(n), Some(a)) = (number, value().and_then(|v| v.parse().ok()))
                            && parts.numbers.len() < MAX_LISTS
                        {
                            parts.numbers.push((n, a));
                        }
                    }
                    _ => {}
                }
            }
            Token::End { name } => match local_name(name) {
                "abstractNum" => (abstract_id, level) = (None, None),
                "lvl" => level = None,
                "num" => number = None,
                _ => {}
            },
            _ => {}
        },
    )
}

pub(crate) fn read_rels(
    storage: &dyn Storage,
    archive: &ZipArchive,
    parts: &mut Parts,
) -> Result<(), DocumentError> {
    small_part(
        storage,
        archive,
        "word/_rels/document.xml.rels",
        &mut |token| {
            if let Token::Start { name, attrs, .. } = token
                && local_name(name) == "Relationship"
                && parts.rels.len() < MAX_RELS
                && let (Some(id), Some(target)) = (attr(&attrs, "Id"), attr(&attrs, "Target"))
                && id.len() <= MAX_NAME
                && target.len() <= MAX_ADDRESS
                && parts.rel_bytes + id.len() + target.len() <= MAX_REL_BYTES
            {
                parts.rel_bytes += id.len() + target.len();
                let kind = attr(&attrs, "Type").unwrap_or_default();
                parts.rels.push(Rel {
                    id: id.into_owned(),
                    target: target.into_owned(),
                    external: attr(&attrs, "TargetMode").is_some_and(|m| m == "External"),
                    image: kind.ends_with("/image"),
                });
            }
        },
    )
}

pub(crate) fn read_properties(
    storage: &dyn Storage,
    archive: &ZipArchive,
    parts: &mut Parts,
) -> Result<(), DocumentError> {
    let mut field: Option<u8> = None;
    small_part(
        storage,
        archive,
        "docProps/core.xml",
        &mut |token| match token {
            Token::Start { name, .. } => {
                field = match local_name(name) {
                    "title" => Some(0),
                    "creator" => Some(1),
                    "language" => Some(2),
                    _ => None,
                };
            }
            Token::End { .. } => field = None,
            Token::Text(text) => {
                let into = match field {
                    Some(0) => &mut parts.title,
                    Some(1) => &mut parts.author,
                    Some(2) => &mut parts.language,
                    _ => return,
                };
                if into.len() < MAX_TITLE {
                    into.push_str(&decode(&text));
                }
            }
            _ => {}
        },
    )
}

/// Where the path `target` of a relationship of the document is in the
/// archive (`media/a.png` beside `document.xml`, `/word/x` from the root,
/// `../x` up).
pub(crate) fn archive_path(target: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    let rooted = target.starts_with('/');
    if !rooted {
        parts.push("word");
    }
    for piece in target.split('/') {
        match piece {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            piece => parts.push(piece),
        }
    }
    parts.join("/")
}

/// The picture kind a file name says (the ones that can be shown).
pub(crate) fn picture_kind(target: &str) -> Option<&'static str> {
    let (_, ext) = target.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "gif" => Some("gif"),
        _ => None,
    }
}

/// Whether `target` is an address to follow from a link.
pub(crate) fn is_address(target: &str) -> bool {
    ["http://", "https://", "mailto:"].iter().any(|scheme| {
        target.len() >= scheme.len() && target[..scheme.len()].eq_ignore_ascii_case(scheme)
    })
}

/// A bookmark's name as the name its links use: letters, digits, `_`, `-`
/// and `.`.
pub(crate) fn clean_id(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .take(crate::MAX_NAME)
        .collect()
}
