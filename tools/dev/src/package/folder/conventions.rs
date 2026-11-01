//! The conventions of `docs/plugin-conventions.md` that a plugin's files
//! show: folder names, pictures, texts for people. Warnings only: the
//! installer and the plugin library accept these, but plugins that keep
//! them are alike, which is the point.

use paperpaw_sdk::native::CODE_FILES;

use super::{File, Folder, TOP_FILES, extension, is_test};
use crate::report::Finding;

/// The folders at the top of a plugin's folder: the kernel's (`i18n`,
/// `fonts`, `points`), the plugin library's (`store`) and the conventions'
/// (`how`, `pictures`, `data`).
const FOLDERS: [&str; 7] = [
    "i18n", "how", "fonts", "points", "store", "pictures", "data",
];
/// The most pixels a picture needs: the X4's screen.
const SCREEN: (u32, u32) = (480, 800);
/// The most bytes of a picture over a whole screen.
const MAX_PICTURE: u64 = 60 * 1000;
/// The most bytes of a logo or an icon.
const MAX_SMALL_PICTURE: u64 = 10 * 1000;

pub fn check(folder: &Folder, add: &mut dyn FnMut(Finding)) {
    folders(folder, add);
    pictures(folder, add);
    how(folder, add);
    languages(folder, add);
    code_with_data(folder, add);
}

/// A folder at the top that is none of the conventions' names.
fn folders(folder: &Folder, add: &mut dyn FnMut(Finding)) {
    let mut seen = Vec::new();
    for file in &folder.files {
        let Some((top, _)) = file.path.split_once('/') else {
            continue;
        };
        if FOLDERS.contains(&top) || seen.contains(&top) || is_test(&file.path) {
            continue;
        }
        seen.push(top);
        add(Finding::warning(
            "package.folder",
            format!("{top}/"),
            format!(
                "`{top}/` is none of the folders a plugin has ({}); pictures go in `pictures/`, other files a plugin reads in `data/` (docs/plugin-conventions.md).",
                FOLDERS.join(", ")
            ),
        ));
    }
}

/// A PNG's width, height and whether it is interlaced, from its header.
fn png_header(bytes: &[u8]) -> Option<(u32, u32, bool)> {
    let header = bytes.get(..29)?;
    if !header.starts_with(b"\x89PNG\r\n\x1a\n") || &header[12..16] != b"IHDR" {
        return None;
    }
    let number = |at: usize| {
        u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
    };
    Some((number(16), number(20), header[28] != 0))
}

/// Whether the name of `file` says it is a logo or an icon.
fn is_small(file: &File) -> bool {
    let name = file.path.rsplit('/').next().unwrap_or(&file.path);
    name.contains("logo") || name.contains("icon")
}

/// Pictures: in `pictures/`, not interlaced, no larger than the screen,
/// and small.
fn pictures(folder: &Folder, add: &mut dyn FnMut(Finding)) {
    for file in &folder.files {
        let path = file.path.as_str();
        if extension(path) != "png" || path.starts_with("store/") || is_test(path) {
            continue;
        }
        if !path.starts_with("pictures/") {
            add(Finding::warning(
                "package.picture-place",
                path,
                "Pictures go in `pictures/` (docs/plugin-conventions.md).",
            ));
        }
        let Some((width, height, interlaced)) = png_header(&file.bytes) else {
            continue;
        };
        if interlaced {
            add(Finding::warning(
                "package.picture",
                path,
                "An interlaced PNG: PaperPaw cannot decode it, and shows no picture. Save it without interlacing.",
            ));
        }
        let fits = |w: u32, h: u32| (width <= w && height <= h) || (width <= h && height <= w);
        if !fits(SCREEN.0, SCREEN.1) {
            add(Finding::warning(
                "package.picture",
                path,
                format!(
                    "{width} x {height} pixels, more than the {} x {} of the X4's screen; PaperPaw scales it down while it reads, so the file only takes room.",
                    SCREEN.0, SCREEN.1
                ),
            ));
        }
        let (most, what) = if is_small(file) {
            (MAX_SMALL_PICTURE, "a logo or an icon")
        } else {
            (MAX_PICTURE, "a picture")
        };
        if file.size > most {
            add(Finding::warning(
                "package.picture",
                path,
                format!(
                    "{} KB, over the {} KB meant for {what}; gray and fewer pixels make it smaller.",
                    file.size.div_ceil(1000),
                    most / 1000
                ),
            ));
        }
    }
}

/// `how/en.txt` and `how/de.txt`, or per plugin in `how/<plugin-id>/`.
fn how(folder: &Folder, add: &mut dyn FnMut(Finding)) {
    let mut places: Vec<&str> = Vec::new();
    for file in &folder.files {
        let Some(rest) = file.path.strip_prefix("how/") else {
            continue;
        };
        if extension(rest) != "txt" {
            continue;
        }
        let place = rest.rsplit_once('/').map_or("how", |(dir, _)| dir);
        if !places.contains(&place) {
            places.push(place);
        }
    }
    for place in places {
        let dir = if place == "how" {
            "how/".to_string()
        } else {
            format!("how/{place}/")
        };
        for language in ["en", "de"] {
            let path = format!("{dir}{language}.txt");
            if folder.file(&path).is_none() {
                add(Finding::warning(
                    "package.how",
                    path,
                    format!(
                        "A plugin explains itself in English and German; `{dir}{language}.txt` is missing."
                    ),
                ));
            }
        }
    }
}

/// English with German: a catalog of English alone is not translated.
fn languages(folder: &Folder, add: &mut dyn FnMut(Finding)) {
    if folder.file("i18n/en.ini").is_some() && folder.file("i18n/de.ini").is_none() {
        add(Finding::warning(
            "package.languages",
            "i18n/de.ini",
            "The plugin has English texts and no German ones; PaperPaw's plugins have both (docs/plugin-conventions.md).",
        ));
    }
}

/// A folder holding a plugin's code and data files of the kernel as well:
/// the kernel reads both, and the code usually embeds the data.
fn code_with_data(folder: &Folder, add: &mut dyn FnMut(Finding)) {
    if !folder
        .files
        .iter()
        .any(|f| CODE_FILES.contains(&f.path.as_str()))
    {
        return;
    }
    for file in &folder.files {
        if file.path != "plugin.ini" && TOP_FILES.contains(&file.path.as_str()) {
            add(Finding::warning(
                "package.code-data",
                file.path.as_str(),
                "A plugin with code also holds a data file the kernel reads; if the code embeds the same file, it is registered twice.",
            ));
        }
    }
}
