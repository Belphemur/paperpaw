//! The room the code plugins installed take, by whether they are on.

use alloc::string::String;
use alloc::vec::Vec;
use paperpaw_platform::Storage;

use super::{CodeRoom, read_header};

/// What the code plugins installed take of the room for code, by plugin
/// state (see [`installed_code_need`]).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InstalledNeed {
    /// The bytes those that are on take, and how many they are.
    pub taken: u64,
    pub plugins: u32,
    /// The same for those switched off.
    pub off: u64,
    pub off_plugins: u32,
}

/// The bytes the code plugins installed take where `room` places them:
/// each folder in `plugins` with a code file named `file`
/// ([`super::Native::file`]), by its header. Those installed since the start
/// (loaded only after a restart) count; those `off` says are switched off
/// (by the folder's name) are kept apart, as the loader skips their code
/// at the next start and the room it would have placed them in is free.
/// Folders whose names begin with a dot (a download being put together,
/// e.g. `.clock.part`) are no plugins and do not count.
pub fn installed_code_need(
    storage: &dyn Storage,
    room: &CodeRoom,
    file: &str,
    off: &dyn Fn(&str) -> bool,
) -> InstalledNeed {
    let mut folders: Vec<String> = Vec::new();
    let _ = storage.list(crate::builtin::PLUGIN_DIR, &mut |entry| {
        if entry.is_dir && !entry.name.starts_with('.') {
            folders.push(String::from(entry.name));
        }
    });
    let mut installed = InstalledNeed::default();
    for folder in folders {
        let path = alloc::format!("{}/{folder}/{file}", crate::builtin::PLUGIN_DIR);
        let Some(header) = read_header(storage, &path) else {
            continue;
        };
        let need = room.needs(u64::from(header.image_len), header.align);
        if off(&folder) {
            installed.off = installed.off.saturating_add(need);
            installed.off_plugins += 1;
        } else {
            installed.taken = installed.taken.saturating_add(need);
            installed.plugins += 1;
        }
    }
    installed
}
