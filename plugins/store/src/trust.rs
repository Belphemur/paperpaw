//! Whether a plugin downloaded may be installed (ADR 0024). A data
//! plugin may. A code plugin runs with full access to the device, so its
//! code is checked whole first ([`check_file`]): made for this PaperPaw,
//! and signed by its author, whose key is remembered at the first install
//! ([`native::author`]). At the first install the key must be the
//! one the library names for the plugin; later, an update or a new
//! install after a removal, it must be the one remembered: one signed by
//! another is refused, with a short warning, whichever library offers it
//! (trust on first install; the key stays known after a removal).

use alloc::format;
use alloc::string::String;

use paperpaw_sdk::install::marks::{From, Marks};
use paperpaw_sdk::install::{self, Progress};
use paperpaw_sdk::native::{self, Author, check_file};
use paperpaw_sdk::{Commands, Env, Storage};

use crate::INSTALLER;

/// Why a plugin downloaded is not installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Its code is damaged, or no plugin file this PaperPaw reads.
    Damaged,
    /// Its code was built for another PaperPaw or device.
    OtherBuild,
    /// Its code is of an older plugin file's format, which an older
    /// PaperPaw made: it needs a new build.
    Older,
    /// Its code is not signed.
    Unsigned,
    /// Its code is signed by another key than the library names.
    NotTheAuthor,
    /// This PaperPaw loads no code plugins.
    Unsupported,
    /// The device has a plugin of its id it cannot replace: one built in,
    /// or one of the other kind.
    Taken,
    /// Its code does not fit in the room left for code plugins.
    NoRoom,
    /// Signed by another key than the one this device knows for the
    /// plugin, from its first install.
    KeyChanged,
}

impl Refusal {
    /// What the reader is told, a translation key taking `{name}`.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Refusal::Damaged => "store.refused.damaged",
            Refusal::OtherBuild => "store.refused.other-build",
            Refusal::Older => "store.refused.older",
            Refusal::Unsigned => "store.refused.unsigned",
            Refusal::NotTheAuthor => "store.refused.not-the-author",
            Refusal::Unsupported => "store.refused.unsupported",
            Refusal::Taken => "store.refused.taken",
            Refusal::NoRoom => "store.refused.no-room",
            Refusal::KeyChanged => "store.refused.key-changed",
        }
    }
}

/// What the reader is told of `refusal`: for one for want of room, where
/// the web flasher gives more, with that way too.
pub(crate) fn refusal_key(env: &Env<'_>, refusal: Refusal) -> &'static str {
    if refusal == Refusal::NoRoom
        && crate::room::Room::of(env, None)
            .code
            .as_ref()
            .is_some_and(crate::room::stock_flash)
    {
        return "store.refused.no-room-flash";
    }
    refusal.key()
}

/// What to do with a plugin all of whose files are here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Install it; for a code plugin, with its author's key and its
    /// code's digest to remember.
    Install {
        code: Option<Code>,
    },
    Refused(Refusal),
}

/// A code plugin's code checked: its author's key, and its file's digest
/// (its header's SHA-256), which only the file installed has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Code {
    pub(crate) author: [u8; 32],
    pub(crate) sha256: [u8; 32],
}

/// The key the library names for `plugin`'s author, kept from when its
/// download started until it is installed.
fn expected_path() -> String {
    format!("{}/author.txt", Env::plugin_dir(crate::PLUGIN.as_str()))
}

/// Keeps the key the library names for the author of `plugin`, which is
/// being downloaded (`None` for a data plugin).
pub(crate) fn expect(storage: &dyn Storage, plugin: &str, key: Option<&[u8; 32]>) {
    match key {
        Some(key) => {
            let mut text = String::from(plugin);
            text.push('\t');
            for byte in key {
                text.push_str(&format!("{byte:02x}"));
            }
            let _ = storage.write(&expected_path(), text.as_bytes());
        }
        None => {
            let _ = storage.remove(&expected_path());
        }
    }
}

fn expected(storage: &dyn Storage, plugin: &str) -> Option<[u8; 32]> {
    let text = paperpaw_sdk::read_file(storage, &expected_path(), 256).ok()?;
    let text = core::str::from_utf8(&text).ok()?;
    let (id, hex) = text.trim().split_once('\t')?;
    if id != plugin || hex.len() != 64 {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let pair = core::str::from_utf8(pair).ok()?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(key)
}

/// Whether the plugin downloaded into `folder` may be installed.
pub(crate) fn verdict(env: &Env<'_>, plugin: &str, folder: &str) -> Verdict {
    let storage = env.storage();
    let code = paperpaw_sdk::native::CODE_FILES
        .iter()
        .map(|name| format!("{folder}/{name}"))
        .find(|path| storage.exists(path));
    if !crate::catalog::may_install(env.plugins(), plugin, code.is_some()) {
        return Verdict::Refused(Refusal::Taken);
    }
    let Some(code) = code else {
        return Verdict::Install { code: None };
    };
    let Some(native) = env.native().filter(|native| code.ends_with(&*native.file)) else {
        return Verdict::Refused(Refusal::Unsupported);
    };
    let (header, signature) = match check_file(storage, &code) {
        Ok(checked) => checked,
        Err(paperpaw_sdk::native::Problem::OlderFormat) => {
            return Verdict::Refused(Refusal::Older);
        }
        Err(paperpaw_sdk::native::Problem::NewerFormat) => {
            return Verdict::Refused(Refusal::OtherBuild);
        }
        Err(_) => return Verdict::Refused(Refusal::Damaged),
    };
    // Of this firmware's fingerprint and compiler.
    if !native.fits(&header) {
        return Verdict::Refused(Refusal::OtherBuild);
    }
    // Where the room for code is known, it must fit beside the others
    // (in place of its version before).
    let room = crate::room::Room::of(env, Some(plugin));
    if let Some(code_room) = room.code {
        let need = code_room.needs(u64::from(header.image_len), header.align);
        if !code_room.fits(room.taken, room.plugins, need) {
            return Verdict::Refused(Refusal::NoRoom);
        }
    }
    let Some(signature) = signature else {
        return Verdict::Refused(Refusal::Unsigned);
    };
    let key = signature.key;
    let code = Code {
        author: key,
        sha256: header.sha256,
    };
    match native::author(storage, plugin, &key) {
        Author::Known => Verdict::Install { code: Some(code) },
        Author::Other => Verdict::Refused(Refusal::KeyChanged),
        Author::New if expected(storage, plugin) == Some(key) => {
            Verdict::Install { code: Some(code) }
        }
        Author::New => Verdict::Refused(Refusal::NotTheAuthor),
        _ => Verdict::Refused(Refusal::NotTheAuthor),
    }
}

/// Installs the complete job's plugin; once it is in place (see
/// [`install::marks`]), its author's key is remembered, its code named as
/// installed, and it is the store's, which offers its updates. `None`
/// where that
/// cannot be written (its code would not run): not installed, the job
/// left for another try.
pub(crate) fn install(env: &Env<'_>, code: Option<Code>, out: &mut Commands) -> Option<Progress> {
    let folder = INSTALLER.folder(env.storage())?;
    install::adopt(env);
    let from = match install::folder_version(env.storage(), &folder) {
        Some(version) => From::Store(version),
        None => From::Local,
    };
    let mut marks = Marks::new(from);
    if let Some(code) = code {
        marks = marks.code(code.sha256).author(code.author);
    }
    if install::marks::write(env.storage(), &folder, &marks).is_err() {
        return None;
    }
    let _ = env.storage().remove(&expected_path());
    INSTALLER.install(env, out)
}

/// Whether the reader was told once what code plugins are, before the
/// first one is installed.
fn told_path() -> String {
    format!("{}/code-told", Env::plugin_dir(crate::PLUGIN.as_str()))
}

pub(crate) fn told(storage: &dyn Storage) -> bool {
    storage.exists(&told_path())
}

pub(crate) fn tell(storage: &dyn Storage) {
    let _ = storage.create_dir(&Env::plugin_dir(crate::PLUGIN.as_str()));
    let _ = storage.write(&told_path(), b"");
}
