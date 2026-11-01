//! The pictures of a book: its cover and the pictures in its text, JPEG,
//! PNG or GIF records decoded a few rows at a time into gray (or colour)
//! files of the book's folder, when their page shows.

use alloc::format;
use alloc::string::String;

use paperpaw_sdk::Storage;
use paperpaw_sdk::document::ImageRef;
use paperpaw_sdk::image::{
    ColorWriter, Fit, Format, GrayWriter, ImageError, ImageFile, Source, decode, dimensions,
};

use crate::header::{Header, record_range};
use crate::text::Pictures;

/// Bytes of a picture read to tell its size.
const HEAD: usize = 4096;
/// The largest picture record decoded.
const MOST_PICTURE: u64 = 8 * 1024 * 1024;

/// A part of a file as a source of image bytes.
struct Range<'a> {
    storage: &'a dyn Storage,
    path: &'a str,
    offset: u64,
    end: u64,
}

impl Source for Range<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ImageError> {
        let room = (self.end - self.offset).min(buf.len() as u64) as usize;
        if room == 0 {
            return Ok(0);
        }
        let n = self
            .storage
            .read_at(self.path, self.offset, &mut buf[..room])
            .map_err(|_| ImageError::Io)?;
        self.offset += n as u64;
        Ok(n)
    }
}

/// Where the picture of record `index` (the first picture's is 1) is in
/// the file, if it is a picture the plugin decodes.
pub(crate) struct Book<'a> {
    pub storage: &'a dyn Storage,
    pub path: &'a str,
    pub header: &'a Header,
    pub size: u64,
    pub fit: Fit,
}

impl Book<'_> {
    fn record(&self, record: u32) -> Option<(u64, u64)> {
        let (start, end) = record_range(
            self.storage,
            self.path,
            self.header.records,
            self.size,
            record,
        )?;
        (end > start && end - start <= MOST_PICTURE).then_some((start, end))
    }

    /// The record of picture `index` counted from the first picture.
    pub(crate) fn picture_record(&self, index: u32) -> Option<u32> {
        let first = self.header.first_image;
        (first > 0 && index > 0)
            .then(|| first.checked_add(index - 1))
            .flatten()
    }

    /// Decodes `record` into the file `to`, no larger than `fit`.
    pub(crate) fn decode(&self, record: u32, to: &str, fit: Fit, color: bool) -> Option<ImageRef> {
        let (start, end) = self.record(record)?;
        let mut head = [0u8; 8];
        let n = self.storage.read_at(self.path, start, &mut head).ok()?;
        let format = Format::detect(&head[..n])?;
        let mut source = Range {
            storage: self.storage,
            path: self.path,
            offset: start,
            end,
        };
        let result = if color {
            let mut writer = ColorWriter::new(self.storage, to);
            decode(format, &mut source, fit, &mut writer).and_then(|()| writer.finish())
        } else {
            let mut writer = GrayWriter::new(self.storage, to);
            decode(format, &mut source, fit, &mut writer).and_then(|()| writer.finish())
        };
        if result.is_err() {
            self.storage.remove(to).ok();
            return None;
        }
        ImageFile::open(self.storage, to)
            .filter(|file| file.width > 0 && file.height > 0)
            .map(|file| ImageRef {
                path: file.path,
                width: file.width,
                height: file.height,
            })
    }
}

impl Pictures for Book<'_> {
    fn size(&mut self, index: u32) -> Option<(u32, u32)> {
        let record = self.picture_record(index)?;
        let (start, end) = self.record(record)?;
        let mut head = [0u8; HEAD];
        let room = (end - start).min(HEAD as u64) as usize;
        let n = self
            .storage
            .read_at(self.path, start, &mut head[..room])
            .ok()?;
        Format::detect(&head[..n])?;
        let (width, height) = dimensions(&head[..n])?;
        let (width, height) = self.fit.size(width, height);
        (width > 0 && height > 0).then_some((width, height))
    }
}

/// The file the picture of record number `index` is decoded into.
pub(crate) fn file(dir: &str, index: u32, color: bool) -> String {
    format!("{dir}/img-{index}.{}", if color { "rgb" } else { "gray" })
}

/// The marker of a picture that could not be decoded.
pub(crate) fn failed(dir: &str, index: u32) -> String {
    format!("{dir}/img-{index}.fail")
}

/// The picture number in the name of a file made by [`file`].
pub(crate) fn index_of(path: &str) -> Option<u32> {
    let name = path.rsplit('/').next()?;
    let (stem, _) = name.strip_prefix("img-")?.rsplit_once('.')?;
    stem.parse().ok()
}
