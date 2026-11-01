//! The pictures of a document: taken out of the archive into the document's
//! folder when their page shows, then decoded a few rows at a time into
//! gray (or colour) files.

use alloc::format;
use alloc::string::String;

use paperpaw_sdk::document::ImageRef;
use paperpaw_sdk::image::{
    ColorWriter, Fit, Format, GrayWriter, ImageError, ImageFile, Source, decode, dimensions,
};
use paperpaw_sdk::zip::{ZipArchive, ZipError};
use paperpaw_sdk::{Storage, read_file};

use crate::MAX_IMAGE;

/// Bytes of a picture read to tell its size.
const HEAD: usize = 4096;
/// The list of the pictures' places in the archive, one a line.
pub(crate) const LIST: &str = "pictures.txt";

/// The file picture `index` is taken out into.
fn media(dir: &str, index: u32) -> String {
    format!("{dir}/media-{index}")
}

/// The file picture `index` is decoded into.
pub(crate) fn file(dir: &str, index: u32, color: bool) -> String {
    format!("{dir}/img-{index}.{}", if color { "rgb" } else { "gray" })
}

/// The marker of a picture that cannot be shown.
fn failed(dir: &str, index: u32) -> String {
    format!("{dir}/img-{index}.fail")
}

/// The picture number in the name of a file made by [`file`].
fn index_of(path: &str) -> Option<u32> {
    let name = path.rsplit('/').next()?;
    let (stem, _) = name.strip_prefix("img-")?.rsplit_once('.')?;
    stem.parse().ok()
}

/// Where picture `index` is in the archive.
fn place(storage: &dyn Storage, dir: &str, index: u32) -> Option<String> {
    let list = read_file(storage, &format!("{dir}/{LIST}"), 100 * 300).ok()?;
    let list = String::from_utf8(list).ok()?;
    list.lines().nth(index as usize).map(String::from)
}

/// A whole file as a source of image bytes.
struct Whole<'a> {
    storage: &'a dyn Storage,
    path: &'a str,
    offset: u64,
}

impl Source for Whole<'_> {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ImageError> {
        let n = self
            .storage
            .read_at(self.path, self.offset, buf)
            .map_err(|_| ImageError::Io)?;
        self.offset += n as u64;
        Ok(n)
    }
}

/// Takes picture `index` out of the archive `path`, unless it is out.
fn take_out(storage: &dyn Storage, path: &str, dir: &str, index: u32) -> Option<String> {
    let to = media(dir, index);
    if storage.size(&to).is_ok_and(|size| size > 0) {
        return Some(to);
    }
    let from = place(storage, dir, index)?;
    let archive = ZipArchive::open(storage, path).ok()?;
    let entry = archive.entry(storage, &from).ok()??;
    if entry.size > MAX_IMAGE {
        return None;
    }
    storage.write(&to, &[]).ok()?;
    let mut failed = false;
    let streamed = archive.stream(storage, &entry, &mut |chunk| {
        failed = storage.append(&to, chunk).is_err();
        !failed
    });
    if failed || matches!(streamed, Err(ZipError::OutOfMemory) | Err(_)) {
        storage.remove(&to).ok();
        return None;
    }
    Some(to)
}

/// The picture `index` as it will be shown: its size, no larger than
/// `fit`, in the file it is decoded into (which is made by
/// [`prepare`]).
pub(crate) fn picture(
    storage: &dyn Storage,
    path: &str,
    dir: &str,
    index: u32,
    fit: Fit,
    color: bool,
) -> Option<ImageRef> {
    let to = file(dir, index, color);
    if let Some(done) = ImageFile::open(storage, &to) {
        return Some(ImageRef {
            path: done.path,
            width: done.width,
            height: done.height,
        });
    }
    if storage.exists(&failed(dir, index)) {
        return None;
    }
    let mut head = [0u8; HEAD];
    let from = take_out(storage, path, dir, index);
    let n = from
        .as_ref()
        .and_then(|from| storage.read_at(from, 0, &mut head).ok());
    let shown = n.and_then(|n| {
        Format::detect(&head[..n])?;
        let (width, height) = dimensions(&head[..n])?;
        let (width, height) = fit.size(width, height);
        (width > 0 && height > 0).then_some(ImageRef {
            path: to,
            width,
            height,
        })
    });
    if shown.is_none() {
        storage.write(&failed(dir, index), b"").ok();
    }
    shown
}

/// Decodes the picture of `image` into its file; `None` where it is there
/// already or cannot be.
pub(crate) fn prepare(
    storage: &dyn Storage,
    dir: &str,
    image: &ImageRef,
    fit: Fit,
    color: bool,
) -> Option<ImageRef> {
    if ImageFile::open(storage, &image.path).is_some() {
        return None;
    }
    let index = index_of(&image.path)?;
    let from = media(dir, index);
    let mut head = [0u8; 8];
    let n = storage.read_at(&from, 0, &mut head).ok()?;
    let format = Format::detect(&head[..n])?;
    let mut source = Whole {
        storage,
        path: &from,
        offset: 0,
    };
    let to = &image.path;
    let result = if color {
        let mut writer = ColorWriter::new(storage, to);
        decode(format, &mut source, fit, &mut writer).and_then(|()| writer.finish())
    } else {
        let mut writer = GrayWriter::new(storage, to);
        decode(format, &mut source, fit, &mut writer).and_then(|()| writer.finish())
    };
    let decoded = result
        .ok()
        .and_then(|()| ImageFile::open(storage, to))
        .filter(|f| f.width > 0 && f.height > 0);
    let Some(file) = decoded else {
        storage.remove(to).ok();
        storage.write(&failed(dir, index), b"").ok();
        return None;
    };
    Some(ImageRef {
        path: file.path,
        width: file.width,
        height: file.height,
    })
}
