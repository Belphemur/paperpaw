//! A Mobipocket book as a [`Document`]: converted once into the cache (see
//! [`crate::cache`]), read from there.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::document::{
    Block, BlockKind, Document, DocumentError, ImageRef, Link, Locator, Metadata, TocEntry,
};
use paperpaw_sdk::image::Fit;
use paperpaw_sdk::{Env, OpenOptions, Storage, read_file};

use crate::PLUGIN;
use crate::cache::{self, Kind, Output};
use crate::header::{Header, fingerprint, record_range};
use crate::images::{self, Book};
use crate::palmdoc;
use crate::text::Converter;

/// Changes when what is made of a book changes, so books are made anew.
const VERSION: u32 = 1;

/// An open Mobipocket book.
#[derive(Debug)]
pub struct MobiDocument {
    path: String,
    header: Header,
    size: u64,
    /// The book's folder.
    dir: String,
    metadata: Metadata,
    fit: Fit,
    cover_fit: Fit,
    color: bool,
}

fn malformed(why: &'static str) -> DocumentError {
    DocumentError::Malformed(Cow::Borrowed(why))
}

impl MobiDocument {
    /// Opens the book at `path` (converting it if it is not yet).
    pub fn open(
        storage: &dyn Storage,
        path: &str,
        options: &OpenOptions,
    ) -> Result<Self, DocumentError> {
        let header = Header::read(storage, path)?;
        let size = storage.size(path)?;
        let dir = paperpaw_sdk::library::kept_path(&Env::plugin_dir(PLUGIN.as_str()), "", path);
        let stamp = format!(
            "{VERSION}\n{}\n{}x{}\n{}\n",
            fingerprint(storage, path)?,
            options.image_fit.short,
            options.image_fit.long,
            options.color
        );
        let current = read_file(storage, &format!("{dir}/{}", cache::DONE), 256).ok();
        if current.as_deref() != Some(stamp.as_bytes()) {
            Self::convert(storage, path, &header, size, &dir, options)?;
            storage.write(&format!("{dir}/{}", cache::DONE), stamp.as_bytes())?;
        }
        let mut metadata = header.metadata.clone();
        if metadata.title.is_none() {
            let name = path.rsplit('/').next().unwrap_or(path);
            metadata.title = Some(name.rsplit_once('.').map_or(name, |(s, _)| s).to_string());
        }
        Ok(Self {
            path: path.to_string(),
            header,
            size,
            dir,
            metadata,
            fit: options.image_fit,
            cover_fit: options.cover_fit,
            color: options.color,
        })
    }

    /// Makes the cache of the book: its text read a record at a time.
    fn convert(
        storage: &dyn Storage,
        path: &str,
        header: &Header,
        size: u64,
        dir: &str,
        options: &OpenOptions,
    ) -> Result<(), DocumentError> {
        storage.remove_dir_all(dir).ok();
        storage.create_dir(dir)?;
        let mut pictures = Book {
            storage,
            path,
            header,
            size,
            fit: options.image_fit,
        };
        let mut converter = Converter::new(header.encoding, &mut pictures, Output::default());
        let record_size = header.record_size as usize;
        let mut raw: Vec<u8> = Vec::new();
        let mut text: Vec<u8> = Vec::new();
        let mut fed = 0usize;
        for record in 1..=header.text_records {
            let Some((start, end)) = record_range(storage, path, header.records, size, record)
            else {
                return Err(malformed("the text is cut off"));
            };
            let want = (end - start).min(2 * record_size as u64 + 64) as usize;
            raw.clear();
            raw.resize(want, 0);
            let n = storage.read_at(path, start, &mut raw)?;
            raw.truncate(n);
            let len = palmdoc::text_len(&raw, header.extra_flags);
            text.clear();
            if header.compressed {
                // A faulty record keeps the text it gave before the fault.
                palmdoc::decompress(&raw[..len], &mut text, record_size).ok();
            } else {
                text.extend_from_slice(&raw[..len.min(record_size)]);
            }
            let last = record == header.text_records;
            let expected = if last {
                (header.text_length as usize).saturating_sub(fed)
            } else {
                record_size
            };
            text.truncate(expected);
            // Offsets are those of the text as it should be: a record that
            // is short is made up with spaces.
            if !last && text.len() < expected {
                text.resize(expected, b' ');
            }
            fed += text.len();
            converter.feed(&text);
            converter.out.flush(storage, dir, false);
            if let Some(error) = converter.out.error() {
                return Err(error);
            }
        }
        converter.finish();
        converter.out.flush(storage, dir, true);
        match converter.out.error() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn book<'a>(&'a self, storage: &'a dyn Storage) -> Book<'a> {
        Book {
            storage,
            path: &self.path,
            header: &self.header,
            size: self.size,
            fit: self.fit,
        }
    }
}

impl Document for MobiDocument {
    fn metadata(&self) -> &Metadata {
        &self.metadata
    }

    fn sections(&self) -> u32 {
        1
    }

    fn section_len(&self, section: u32) -> u32 {
        if section == 0 {
            self.header.text_length
        } else {
            0
        }
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
        let mut block = Block::new(kind, loaded.start, loaded.end, loaded.text)
            .continues(loaded.continues)
            .continued(loaded.continued)
            .map(loaded.map)
            .runs(loaded.runs)
            .align(loaded.align);
        if let Some((index, width, height)) = loaded.image {
            block = block.image(ImageRef {
                path: images::file(&self.dir, index, self.color),
                width,
                height,
            });
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
        if paperpaw_sdk::image::ImageFile::open(storage, &image.path).is_some() {
            return None;
        }
        let index = images::index_of(&image.path)?;
        let failed = images::failed(&self.dir, index);
        if storage.exists(&failed) {
            return None;
        }
        let book = self.book(storage);
        let record = book.picture_record(index)?;
        let to = images::file(&self.dir, index, self.color);
        let decoded = book.decode(record, &to, self.fit, self.color);
        if decoded.is_none() {
            storage.write(&failed, b"").ok();
        }
        decoded
    }

    fn cover(&self, storage: &dyn Storage) -> Result<Option<ImageRef>, DocumentError> {
        let Some(record) = self.header.cover else {
            return Ok(None);
        };
        let to = format!(
            "{}/cover-{}.{}",
            self.dir,
            self.cover_fit.long,
            if self.color { "rgb" } else { "gray" }
        );
        if let Some(file) = paperpaw_sdk::image::ImageFile::open(storage, &to) {
            return Ok(Some(ImageRef {
                path: file.path,
                width: file.width,
                height: file.height,
            }));
        }
        Ok(self
            .book(storage)
            .decode(record, &to, self.cover_fit, self.color))
    }

    fn links(&self, storage: &dyn Storage, section: u32, from: u32, to: u32) -> Vec<Link> {
        if section != 0 {
            return Vec::new();
        }
        cache::links(storage, &self.dir, from, to)
    }

    fn link_target(&self, _: &dyn Storage, href: &str) -> Option<Locator> {
        let position: u32 = href.strip_prefix('#')?.parse().ok()?;
        (position < self.header.text_length).then(|| Locator::new(0, position))
    }
}

/// A book opened for the library.
pub(crate) fn opener(
    storage: &dyn Storage,
    path: &str,
    options: &OpenOptions,
) -> Result<Box<dyn Document>, DocumentError> {
    MobiDocument::open(storage, path, options).map(|d| Box::new(d) as Box<dyn Document>)
}
