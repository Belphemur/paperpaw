//! How much room is left for plugins: free space on the storage for
//! their files and, where code plugins run from a room of their own (the
//! X4's flash, the X4 Pro's PSRAM), how much of it the code plugins
//! installed take. Each is said where the device can tell, and left out
//! where it cannot.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::native::{self, CodeRoom};
use paperpaw_sdk::{Arg, Env, files};

use crate::catalog::Entry;

/// The room this device has for plugins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Room {
    /// Bytes free on the storage, where it can tell.
    pub free: Option<u64>,
    /// The room code plugins run from, where PaperPaw loads them and its
    /// shell says.
    pub code: Option<CodeRoom>,
    /// Bytes the code plugins installed take there, and how many they
    /// are (those waiting for a restart with them): what the next start
    /// places, so those switched off, which it skips, are not in it.
    pub taken: u64,
    pub plugins: u32,
    /// Bytes the code plugins switched off would take, and how many they
    /// are: free for now, taken again where they are switched on.
    pub off: u64,
    pub off_plugins: u32,
}

/// Whether a full flash with the web flasher gives `room` much more: the
/// shell says so where the device runs a smaller layout than PaperPaw's
/// own (the X4 after an SD install from CrossPoint).
pub(crate) fn stock_flash(room: &CodeRoom) -> bool {
    room.larger.is_some()
}

impl Room {
    /// The room as it is now, `plugin`'s own code left out (an update
    /// takes the place of the version before).
    pub(crate) fn of(env: &Env<'_>, plugin: Option<&str>) -> Room {
        let code = env.native().and_then(|_| native::code_room(env.storage()));
        let mut installed = Installed::default();
        if let (Some(room), Some(native)) = (&code, env.native()) {
            installed = Installed::of(env, room, &native.file);
            // Its own code makes room for the update, where it is on.
            if let Some(own) = plugin
                .filter(|id| !switched_off(env, id))
                .and_then(|id| own_need(env, room, id))
            {
                installed.taken = installed.taken.saturating_sub(own);
                installed.plugins = installed.plugins.saturating_sub(1);
            }
        }
        Room {
            free: env.storage().free_space(),
            code,
            taken: installed.taken,
            plugins: installed.plugins,
            off: installed.off,
            off_plugins: installed.off_plugins,
        }
    }

    /// Bytes left for code plugins.
    pub(crate) fn code_left(&self) -> Option<u64> {
        self.code.map(|room| room.left(self.taken))
    }

    /// Whether `plugin` fits, where it can be told: its files on the
    /// storage, and its code (`image_len` bytes, where the library says)
    /// in the room for code. What cannot be told is taken to fit: the
    /// download and the start check again.
    pub(crate) fn fits(&self, plugin: &Entry, image_len: Option<u64>) -> bool {
        let files = self.free.is_none_or(|free| plugin.size <= free);
        let code = match (plugin.code, self.code, image_len) {
            (true, Some(room), Some(len)) => {
                room.fits(self.taken, self.plugins, room.needs(len, 0))
            }
            _ => true,
        };
        files && code
    }

    /// Whether `plugin` fits only as long as the code plugins that are
    /// switched off stay off: the next start skips them, so it fits, but
    /// switching them on again would not.
    pub(crate) fn fits_while_off(&self, plugin: &Entry, image_len: Option<u64>) -> bool {
        let (true, Some(room), Some(len)) = (plugin.code, self.code, image_len) else {
            return false;
        };
        self.off_plugins > 0
            && self.fits(plugin, image_len)
            && !room.fits(
                self.taken.saturating_add(self.off),
                self.plugins.saturating_add(self.off_plugins),
                room.needs(len, 0),
            )
    }

    /// The note beside the store's title: the free space, where known
    /// (what is left for code does not fit beside the title on the X4).
    pub(crate) fn short(&self, env: &Env<'_>) -> Option<String> {
        let free = size(env, self.free?);
        Some(
            env.i18n()
                .format("store.room", &[("free", Arg::Str(&free))]),
        )
    }

    /// The line under the installed plugins: the free space, and how
    /// much of the room for code is taken; `None` where neither is known.
    pub(crate) fn line(&self, env: &Env<'_>) -> Option<String> {
        let mut parts = alloc::vec::Vec::new();
        if let Some(free) = self.free {
            let free = size(env, free);
            parts.push(
                env.i18n()
                    .format("store.room", &[("free", Arg::Str(&free))]),
            );
        }
        if let Some(room) = self.code {
            let (used, total) = (size(env, self.taken), size(env, room.total));
            let key = if room.shared {
                "store.room.code-about"
            } else {
                "store.room.code-used"
            };
            let args = [("used", Arg::Str(&used)), ("total", Arg::Str(&total))];
            parts.push(env.i18n().format(key, &args));
        }
        let mut line = parts.join(" · ");
        if let Some(flash) = self.flash(env) {
            if !line.is_empty() {
                line.push_str(". ");
            }
            line.push_str(&flash);
        }
        (!line.is_empty()).then_some(line)
    }

    /// Where the room is the stock layout's (the X4 after an SD install
    /// from CrossPoint): that a full install via USB gives more, with the
    /// room it would give; `None` elsewhere.
    pub(crate) fn flash(&self, env: &Env<'_>) -> Option<String> {
        let larger = self.code.filter(stock_flash)?.larger?;
        let larger = size(env, larger);
        Some(
            env.i18n()
                .format("store.room.flash", &[("larger", Arg::Str(&larger))]),
        )
    }

    /// The note of the first-run setup: the room for code plugins now and
    /// that a full install gives more; `None` except on the stock layout.
    pub(crate) fn setup_note(&self, env: &Env<'_>) -> Option<String> {
        let flash = self.flash(env)?;
        let left = size(env, self.code_left()?);
        let now = env
            .i18n()
            .format("store.room.now", &[("left", Arg::Str(&left))]);
        Some(alloc::format!("{now} {flash}"))
    }

    /// What `plugin` needs and what is left, for its page; `None` where
    /// nothing is known of what is left.
    pub(crate) fn needs(
        &self,
        env: &Env<'_>,
        plugin: &Entry,
        image_len: Option<u64>,
    ) -> Option<String> {
        if let (true, Some(left)) = (plugin.code, self.code_left()) {
            let need = size(env, image_len.unwrap_or(plugin.size));
            let left = size(env, left);
            let key = if self.fits_while_off(plugin, image_len) {
                "store.needs.code-off"
            } else if self.fits(plugin, image_len) {
                "store.needs.code"
            } else if self.code.as_ref().is_some_and(stock_flash) {
                "store.no-room.code-flash"
            } else {
                "store.no-room.code"
            };
            let args = [("size", Arg::Str(&need)), ("left", Arg::Str(&left))];
            return Some(env.i18n().format(key, &args));
        }
        let free = size(env, self.free?);
        let need = size(env, plugin.size);
        let key = if self.fits(plugin, image_len) {
            "store.needs"
        } else {
            "store.no-room.files"
        };
        let args = [("size", Arg::Str(&need)), ("free", Arg::Str(&free))];
        Some(env.i18n().format(key, &args))
    }
}

/// What the code plugins installed take of the room at the next start.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Installed {
    taken: u64,
    plugins: u32,
    off: u64,
    off_plugins: u32,
}

impl Installed {
    /// Each folder in `plugins` with a code file named `file`, by its
    /// header (as `native::installed_code_need` measures them): those
    /// installed since the start count (loaded only after a restart),
    /// those switched off do not (the loader skips their code at the next
    /// start and the room it would have placed them in is free), and
    /// folders that begin with a dot (a download being put together) are
    /// no plugins.
    fn of(env: &Env<'_>, room: &CodeRoom, file: &str) -> Self {
        let mut folders: Vec<String> = Vec::new();
        let _ = env
            .storage()
            .list(paperpaw_sdk::builtin::PLUGIN_DIR, &mut |entry| {
                if entry.is_dir && !entry.name.starts_with('.') {
                    folders.push(String::from(entry.name));
                }
            });
        let mut installed = Self::default();
        for folder in folders {
            let path = alloc::format!("{}/{folder}/{file}", paperpaw_sdk::builtin::PLUGIN_DIR);
            let Some(header) = native::read_header(env.storage(), &path) else {
                continue;
            };
            let need = room.needs(u64::from(header.image_len), header.align);
            if switched_off(env, &folder) {
                installed.off = installed.off.saturating_add(need);
                installed.off_plugins += 1;
            } else {
                installed.taken = installed.taken.saturating_add(need);
                installed.plugins += 1;
            }
        }
        installed
    }
}

/// Whether the plugin in folder `id` is switched off (a plugin the kernel
/// does not list is taken to be on: it runs once installed).
fn switched_off(env: &Env<'_>, id: &str) -> bool {
    env.plugins().get(id).is_some_and(|info| !info.enabled)
}

/// The room the installed code of plugin `id` takes, by its header.
fn own_need(env: &Env<'_>, room: &CodeRoom, id: &str) -> Option<u64> {
    let native = env.native()?;
    let dir = paperpaw_sdk::builtin::PLUGIN_DIR;
    let path = alloc::format!("{dir}/{id}/{}", native.file);
    let header = native::read_header(env.storage(), &path)?;
    Some(room.needs(u64::from(header.image_len), header.align))
}

fn size(env: &Env<'_>, bytes: u64) -> String {
    files::shown_size(env, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIB: u64 = 1024;

    #[test]
    fn the_flasher_is_named_only_where_it_gives_more_room() {
        // The X4 in the stock layout's slot (6.25 MiB), and in its own.
        let room = CodeRoom::new(640 * KIB, 0, 4096).with_plugins(0, Some(16));
        assert!(stock_flash(&room.in_slot(0x64_0000, 0x7F_0000)));
        assert!(!stock_flash(&room.in_slot(0x7F_0000, 0x7F_0000)));
        // Not said (shared memory, other devices): not named.
        assert!(!stock_flash(&room));
        assert!(!stock_flash(&room.shared()));
    }
}
