//! The pictures of the moods: one's own from the folder
//! [`PICTURES`](crate::PICTURES) names, else PaperPaw's cats, carried in
//! the plugin. Each is decoded once into the plugin's folder (a few rows
//! at a time, never whole in memory) and drawn or made small from there.

use alloc::format;
use alloc::string::String;

use paperpaw_sdk::Env;
use paperpaw_sdk::document::ImageRef;
use paperpaw_sdk::image::{Fit, ImageFile};

use crate::{Mood, PICTURES, data};

/// Where the cats are on the card, installed with the plugin: one PNG a
/// mood, 16 grays, at most 400 pixels a side, white around them.
const FOLDER: &str = "plugins/reading-goal/pictures";

/// The same cats carried in the code, for where the plugin is part of the
/// app (a shell with room: there is no installed folder to read from).
/// A plugin file for a reader leaves them out (`default-features = false`
/// in tools/plugin-files): its package brings the cats on the card, and
/// 134 KB of the X4's plugin room stay free.
#[cfg(feature = "embedded-pictures")]
const CATS: [&[u8]; 4] = [
    include_bytes!("../pictures/sad.png"),
    include_bytes!("../pictures/meh.png"),
    include_bytes!("../pictures/content.png"),
    include_bytes!("../pictures/happy.png"),
];

/// The largest side a picture is decoded to: as large as it is drawn on
/// a screen.
const MOST: u32 = 800;

/// The picture of `mood`.
pub(crate) fn of(env: &Env<'_>, mood: Mood) -> Option<ImageRef> {
    own(env, mood).or_else(|| cat(env, mood))
}

fn opened(env: &Env<'_>, file: &str) -> Option<ImageRef> {
    ImageFile::open(env.storage(), file).map(|f| ImageRef {
        path: f.path,
        width: f.width,
        height: f.height,
    })
}

/// One's own picture of `mood`, if the folder has one PaperPaw reads.
fn own(env: &Env<'_>, mood: Mood) -> Option<ImageRef> {
    let folder: String = env
        .settings()
        .text(PICTURES.as_str())
        .trim_matches('/')
        .into();
    if folder.is_empty() {
        return None;
    }
    for extension in ["png", "jpg", "jpeg", "PNG", "JPG", "JPEG"] {
        let path = format!("{folder}/{}.{extension}", mood.name());
        let Ok(size) = env.storage().size(&path) else {
            continue;
        };
        // Made again where the picture changed (another size, mostly).
        let file = data(&format!("own-{}-{size}.gray", mood.name()));
        if let Some(image) = opened(env, &file) {
            return Some(image);
        }
        let fit = Fit::side(MOST);
        if let Some(image) =
            paperpaw_sdk::image::picture_from_file(env.storage(), &path, &file, fit, false)
        {
            return Some(image);
        }
    }
    None
}

/// PaperPaw's cat of `mood`: the card's, else the code's where it carries them.
fn cat(env: &Env<'_>, mood: Mood) -> Option<ImageRef> {
    let path = format!("{FOLDER}/{}.png", mood.name());
    if let Ok(size) = env.storage().size(&path) {
        // Named by its size: a cat drawn anew in an update is decoded anew.
        let file = data(&format!("cat-{}-{size}.gray", mood.name()));
        let from_card = opened(env, &file).or_else(|| {
            paperpaw_sdk::image::picture_from_file(
                env.storage(),
                &path,
                &file,
                Fit::side(MOST),
                false,
            )
        });
        if from_card.is_some() {
            return from_card;
        }
    }
    carried(env, mood)
}

#[cfg(feature = "embedded-pictures")]
fn carried(env: &Env<'_>, mood: Mood) -> Option<ImageRef> {
    let bytes = CATS[mood as usize];
    let file = data(&format!("cat-{}-{}.gray", mood.name(), bytes.len()));
    opened(env, &file).or_else(|| {
        paperpaw_sdk::image::picture_from_bytes(env.storage(), bytes, &file, Fit::side(MOST), false)
    })
}

/// A plugin file for a reader carries no cats: without the card's, the
/// mood has no picture (the screens draw its words).
#[cfg(not(feature = "embedded-pictures"))]
fn carried(_env: &Env<'_>, _mood: Mood) -> Option<ImageRef> {
    None
}
