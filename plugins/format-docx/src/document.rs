//! A Word document as a [`Document`]: converted once into the cache (see
//! [`crate::cache`]), read from there.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::document::{
    Block, BlockKind, Document, DocumentError, ImageRef, Link, Locator, Metadata, TocEntry,
};
use paperpaw_sdk::image::Fit;
use paperpaw_sdk::zip::ZipArchive;
use paperpaw_sdk::{Env, OpenOptions, Storage, read_file};

use crate::cache::{self, Kind};
use crate::parts::{
    Parts, error, malformed, read_numbering, read_properties, read_rels, read_styles, xml_part,
};
use crate::writer::Writer;
use crate::{MAX_TEXT, PLUGIN, images};

/// Changes when what is made of a document changes, so they are made anew.
const VERSION: u32 = 1;
const INFO: &str = "info.txt";

/// An open Word document.
#[derive(Debug)]
pub struct DocxDocument {
    path: String,
    /// The document's folder.
    dir: String,
    metadata: Metadata,
    /// The text's length, as places are counted.
    length: u32,
    fit: Fit,
    color: bool,
}

/// What tells the file from another of its name: its size and a hash of
/// its first and last 4 KB.
fn fingerprint(storage: &dyn Storage, path: &str) -> Result<String, DocumentError> {
    let size = storage.size(path)?;
    let head = read_file(storage, path, 4096).unwrap_or_default();
    let mut tail = alloc::vec![0u8; 4096];
    let n = storage.read_at(path, size.saturating_sub(4096), &mut tail)?;
    tail.truncate(n);
    let hash = paperpaw_sdk::digest::fnv1a64([head.as_slice(), tail.as_slice()].concat());
    Ok(format!("{size}:{hash:016x}"))
}

impl DocxDocument {
    /// Opens the document at `path` (converting it if it is not yet).
    pub fn open(
        storage: &dyn Storage,
        path: &str,
        options: &OpenOptions,
    ) -> Result<Self, DocumentError> {
        let dir = format!(
            "{}/{}",
            Env::plugin_dir(PLUGIN.as_str()),
            paperpaw_sdk::library::key(path)
        );
        let stamp = format!("{VERSION}\n{}\n", fingerprint(storage, path)?);
        let done = format!("{dir}/{}", cache::DONE);
        if read_file(storage, &done, 256).ok().as_deref() != Some(stamp.as_bytes()) {
            if let Err(e) = Self::convert(storage, path, &dir) {
                // Nothing half made stays.
                storage.remove_dir_all(&dir).ok();
                return Err(e);
            }
            storage.write(&done, stamp.as_bytes())?;
        }
        let info = read_file(storage, &format!("{dir}/{INFO}"), 4096)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default();
        let mut lines = info.lines();
        let length = lines.next().and_then(|l| l.parse().ok()).unwrap_or(0);
        let authors = lines.next().unwrap_or("");
        let title = lines.next().unwrap_or("");
        let language = lines.next().unwrap_or("");
        let mut metadata = Metadata::default();
        metadata.title = Some(if title.is_empty() {
            let name = path.rsplit('/').next().unwrap_or(path);
            name.rsplit_once('.').map_or(name, |(s, _)| s).to_string()
        } else {
            title.to_string()
        });
        if !authors.is_empty() {
            metadata.authors = alloc::vec![authors.to_string()];
        }
        if !language.is_empty() {
            metadata.language = Some(language.to_string());
        }
        Ok(Self {
            path: path.to_string(),
            dir,
            metadata,
            length,
            fit: options.image_fit,
            color: options.color,
        })
    }

    /// Makes the cache of the document: its text read a piece at a time.
    fn convert(storage: &dyn Storage, path: &str, dir: &str) -> Result<(), DocumentError> {
        storage.remove_dir_all(dir).ok();
        storage.create_dir(dir)?;
        let archive = ZipArchive::open(storage, path).map_err(error)?;
        let Some(document) = archive.entry(storage, "word/document.xml").map_err(error)? else {
            return Err(malformed("not a Word document"));
        };
        if document.size > MAX_TEXT {
            return Err(malformed("the document is too large"));
        }
        let mut parts = Parts::default();
        read_properties(storage, &archive, &mut parts)?;
        read_styles(storage, &archive, &mut parts)?;
        read_numbering(storage, &archive, &mut parts)?;
        read_rels(storage, &archive, &mut parts)?;
        let mut writer = Writer::new(storage, dir, &parts);
        xml_part(storage, &archive, &document, &mut |token| {
            writer.token(token);
        })?;
        writer.finish();
        if let Some(e) = writer.error() {
            return Err(e);
        }
        let length = writer.length();
        let pictures = core::mem::take(&mut writer.pictures);
        drop(writer);
        storage.write(
            &format!("{dir}/{}", images::LIST),
            pictures.join("\n").as_bytes(),
        )?;
        let one_line = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        let language: String = parts
            .language
            .trim()
            .chars()
            .take(32)
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let info = format!(
            "{length}\n{}\n{}\n{language}\n",
            one_line(&parts.author),
            one_line(&parts.title)
        );
        storage.write(&format!("{dir}/{INFO}"), info.as_bytes())?;
        Ok(())
    }
}

impl Document for DocxDocument {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn sections(&self) -> u32 {
        1
    }

    fn section_len(&self, section: u32) -> u32 {
        if section == 0 { self.length } else { 0 }
    }

    fn block_at(&self, storage: &dyn Storage, at: Locator) -> Result<Option<Block>, DocumentError> {
        if at.section != 0 {
            return Ok(None);
        }
        let Some(loaded) = cache::block_at(storage, &self.dir, at.offset) else {
            return Ok(None);
        };
        let kind = match loaded.kind {
            Kind::Paragraph => BlockKind::Paragraph,
            Kind::Heading(level) => BlockKind::Heading(level),
            Kind::Image => BlockKind::Image,
        };
        let picture = loaded.image.map(|(index, _, _)| {
            images::picture(storage, &self.path, &self.dir, index, self.fit, self.color)
        });
        let (kind, text) = match (&picture, loaded.kind) {
            // A picture that cannot be shown shows what it says it is.
            (Some(None), _) => (
                BlockKind::Paragraph,
                if loaded.text.is_empty() {
                    String::from("[…]")
                } else {
                    format!("[{}]", loaded.text)
                },
            ),
            _ => (kind, loaded.text),
        };
        let mut block = Block::new(kind, loaded.start, loaded.end, text)
            .continues(loaded.continues)
            .continued(loaded.continued)
            .map(loaded.map)
            .runs(loaded.runs)
            .align(loaded.align);
        if let Some(Some(image)) = picture {
            block = block.image(image);
        }
        Ok(Some(block))
    }

    fn toc_len(&self, storage: &dyn Storage) -> u32 {
        cache::toc_len(storage, &self.dir)
    }

    fn toc_entry(&self, storage: &dyn Storage, index: u32) -> Option<TocEntry> {
        let (at, level, title) = cache::toc_entry(storage, &self.dir, index)?;
        Some(TocEntry {
            title,
            level,
            at: Locator::new(0, at),
        })
    }

    fn prepare_image(&self, storage: &dyn Storage, image: &ImageRef) -> Option<ImageRef> {
        images::prepare(storage, &self.dir, image, self.fit, self.color)
    }

    fn links(&self, storage: &dyn Storage, section: u32, from: u32, to: u32) -> Vec<Link> {
        if section != 0 {
            return Vec::new();
        }
        cache::links(storage, &self.dir, from, to)
    }

    fn link_target(&self, storage: &dyn Storage, href: &str) -> Option<Locator> {
        let hash: u32 = href.strip_prefix('#')?.parse().ok()?;
        cache::mark(storage, &self.dir, hash).map(|at| Locator::new(0, at))
    }
}

/// A document opened for the library.
pub(crate) fn opener(
    storage: &dyn Storage,
    path: &str,
    options: &OpenOptions,
) -> Result<Box<dyn Document>, DocumentError> {
    DocxDocument::open(storage, path, options).map(|d| Box::new(d) as Box<dyn Document>)
}
