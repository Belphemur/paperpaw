//! Code plugins fetched for an update before it is installed (ADR 0024):
//! a code plugin runs only on the firmware it was built for, so the
//! update server names, with an update, the builds for it of the code
//! plugins this device has, and of plugins built in now that the update
//! has as files instead. Once the update is downloaded, they are
//! downloaded one after another (`paperpaw_sdk::install`), checked (the
//! author known, or the one the server names for a plugin new here), and
//! put where the updated firmware installs them at its first start
//! ([`native::fetched_folder`]). Settings and data belong to a plugin's
//! id, so they stay.
//!
//! The answer's `code` lists them:
//!
//! ```json
//! "code": [{"id": "stats", "name": "Statistics", "version": "0.5.0",
//!           "author_key": "…64 hex digits…",
//!           "files": [{"path": "plugin.ini", "url": "…", "size": 120, "sha256": "…"},
//!                     {"path": "code.ppn", "url": "…", "size": 412345, "sha256": "…"}]}]
//! ```
//!
//! What is left to fetch is kept in the plugin's folder, so it goes on
//! while the page is closed; the plugin being fetched stays first until
//! it is done. A restart (on the X4, every sleep) cuts its download off:
//! it is begun anew when the update page shows again ([`resume`]), and
//! what is left for an update installed meanwhile, or no longer newer, is
//! let go at the start ([`interrupted`]). So the update never waits for
//! its plugins for good: each one fetched or failed, it is ready.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::event::Event;
use paperpaw_sdk::firmware::{Firmware, compare_versions};
use paperpaw_sdk::install::{self, Digest, Installer, Package, PackageFile, Received};
use paperpaw_sdk::json::stream::{Event as JsonEvent, Parser};
use paperpaw_sdk::native::{self, Author};
use paperpaw_sdk::replies::HttpReply;
use paperpaw_sdk::{Arg, Command, Commands, Env, PluginKind, Storage};

use crate::{PLUGIN, file};

/// A file of a plugin fetched.
pub(crate) const FETCHED: Event<HttpReply> = Event::new("update.fetched");
const INSTALLER: Installer = Installer::new(PLUGIN, FETCHED);
/// The plugins left to fetch, the update's version first.
const QUEUE: &str = "fetch.txt";
/// The plugin being fetched and the author the server names for it.
const CURRENT: &str = "fetch-current.txt";
/// The names of those that could not be fetched, a line each.
const FAILED: &str = "fetch-failed.txt";
/// The fingerprint and compiler the update's code plugins must have, where
/// the server named them: `<16 hex digits> <8 hex digits>`.
const WANTED: &str = "fetch-native.txt";
/// A crate of a first-party plugin, named after it.
const CRATE_PREFIX: &str = "paperpaw-plugin-";

/// A plugin's build for an update.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Build {
    pub package: Package,
    pub version: String,
    /// The author's key the server names (64 hex digits), for a plugin
    /// whose author this device does not know yet.
    pub author: String,
}

/// The builds the answer `body` names (`code`), those that can be
/// fetched.
pub(crate) fn parse(body: &str) -> Vec<Build> {
    let mut builds: Vec<Build> = Vec::new();
    let mut parser = Parser::default();
    parser.feed(body.as_bytes(), &mut |event| match event {
        JsonEvent::Open(path) if path == ["code", "[]"] => builds.push(Build::default()),
        JsonEvent::Open(path) if path == ["code", "[]", "files", "[]"] => {
            if let Some(build) = builds.last_mut() {
                build.package.files.push(PackageFile::default());
            }
        }
        JsonEvent::Value(path, text) => {
            let Some(build) = builds.last_mut() else {
                return;
            };
            match path {
                [code, item, key] if code == "code" && item == "[]" => match key.as_str() {
                    "id" => build.package.plugin = text.to_string(),
                    "name" => build.package.name = text.to_string(),
                    "version" => build.version = text.to_string(),
                    "author_key" => build.author = text.to_ascii_lowercase(),
                    _ => {}
                },
                [code, item, files, entry, key]
                    if code == "code" && item == "[]" && files == "files" && entry == "[]" =>
                {
                    let Some(file) = build.package.files.last_mut() else {
                        return;
                    };
                    match key.as_str() {
                        "path" => file.path = text.to_string(),
                        "url" => file.url = text.to_string(),
                        "size" => file.size = text.parse().unwrap_or(0),
                        key => {
                            file.digest.take(key, text);
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    });
    builds.retain(|b| b.package.is_valid() && !b.version.contains(char::is_whitespace));
    builds
}

/// The builds as kept with an offer: a line `build <id> <version>
/// <author or -> <name>` and a line `file <size> <digest> <path> <url>`
/// for each of its files.
pub(crate) fn to_text(builds: &[Build]) -> String {
    let mut text = String::new();
    for build in builds {
        let author = if build.author.is_empty() {
            "-"
        } else {
            &build.author
        };
        text.push_str(&format!(
            "build {} {} {author} {}\n",
            build.package.plugin,
            build.version,
            build.package.name.replace(['\n', '\r'], " ")
        ));
        for f in &build.package.files {
            let digest = f.digest.text();
            text.push_str(&format!("file {} {digest} {} {}\n", f.size, f.path, f.url));
        }
    }
    text
}

/// What [`to_text`] wrote, from its lines (others are passed over).
pub(crate) fn from_lines<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<Build> {
    let mut builds: Vec<Build> = Vec::new();
    for line in lines {
        if let Some(rest) = line.strip_prefix("build ") {
            let mut parts = rest.splitn(4, ' ');
            let (Some(id), Some(version), Some(author)) =
                (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            builds.push(Build {
                package: Package::new(
                    id.to_string(),
                    parts.next().unwrap_or(id).trim().to_string(),
                ),
                version: version.to_string(),
                author: if author == "-" {
                    String::new()
                } else {
                    author.to_string()
                },
            });
        } else if let Some(rest) = line.strip_prefix("file ")
            && let Some(build) = builds.last_mut()
        {
            let mut parts = rest.splitn(4, ' ');
            let (Some(size), Some(digest), Some(path), Some(url)) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let digest = match digest.split_once(':') {
                Some(("sha256", hex)) => Digest::Sha256(hex.to_string()),
                Some((_, hex)) => Digest::Sha1(hex.to_string()),
                None => continue,
            };
            build.package.files.push(PackageFile::new(
                path.to_string(),
                url.trim().to_string(),
                size.parse().unwrap_or(0),
                digest,
            ));
        }
    }
    builds.retain(|b| b.package.is_valid());
    builds
}

/// The plugin a crate of the image's list is (`paperpaw-plugin-stats`:
/// `stats`).
pub(crate) fn plugin_of(crate_name: &str) -> &str {
    crate_name.strip_prefix(CRATE_PREFIX).unwrap_or(crate_name)
}

/// Whether the plugin of `crate_name`, which an update leaves out, comes
/// back as a file: among the builds this device takes (`wanted`, see
/// [`wanted`]), or fetched already and checked.
pub(crate) fn comes_back(storage: &dyn Storage, wanted: &[Build], crate_name: &str) -> bool {
    let id = plugin_of(crate_name);
    wanted.iter().any(|b| b.package.plugin == id)
        || (native::fetched_code(storage, id).is_some()
            && storage.exists(&format!("{}/plugin.ini", native::fetched_folder(id))))
}

/// Of `builds`, those that fit the room for code beside the code plugins
/// that stay (where the room is known: an X4 has little), in order; and
/// the names of those that do not. A build's file is taken as its size,
/// at most what it needs. Code plugins switched off take no room: the
/// loader skips their code at the next start; nor does a build of one.
fn fitting(env: &Env<'_>, builds: Vec<Build>) -> (Vec<Build>, Vec<String>) {
    let (Some(native_build), Some(room)) = (env.native(), native::code_room(env.storage())) else {
        return (builds, Vec::new());
    };
    let storage = env.storage();
    let installed = native::installed_code_need(storage, &room, &native_build.file, &|id| {
        switched_off(env, id)
    });
    let (mut taken, mut count) = (installed.taken, installed.plugins);
    // Code plugins these builds replace make room for them.
    for build in builds
        .iter()
        .filter(|b| !switched_off(env, &b.package.plugin))
    {
        let path = native_build.path(&paperpaw_sdk::files::join(
            paperpaw_sdk::builtin::PLUGIN_DIR,
            &build.package.plugin,
        ));
        if let Some(header) = native::read_header(storage, &path) {
            taken = taken.saturating_sub(room.needs(u64::from(header.image_len), header.align));
            count = count.saturating_sub(1);
        }
    }
    let (mut fit, mut left_out) = (Vec::new(), Vec::new());
    for build in builds {
        if switched_off(env, &build.package.plugin) {
            fit.push(build);
            continue;
        }
        let size = build
            .package
            .files
            .iter()
            .find(|f| f.path == native_build.file)
            .map_or(0, |f| f.size);
        let need = room.needs(size, 0);
        if room.fits(taken, count, need) {
            taken = taken.saturating_add(need);
            count += 1;
            fit.push(build);
        } else {
            left_out.push(build.package.name);
        }
    }
    (fit, left_out)
}

/// Whether the plugin `id` is switched off (one the kernel does not list
/// is taken to be on: it runs once installed).
fn switched_off(env: &Env<'_>, id: &str) -> bool {
    env.plugins().get(id).is_some_and(|info| !info.enabled)
}

/// Which of `builds` this device takes: where it loads code plugins, those
/// of code plugins it has and of plugins built in that are switched on
/// (one switched off is not fetched; its settings stay). Not one the
/// store did not install ([`install::origin`]): it is the reader's own,
/// never put in place by itself.
pub(crate) fn wanted(env: &Env<'_>, builds: &[Build]) -> Vec<Build> {
    if env.native().is_none() {
        return Vec::new();
    }
    // Those installed before origins were kept: the store's.
    install::adopt(env);
    // Read once for all of them, where one is not built in.
    let origins = core::cell::OnceCell::new();
    builds
        .iter()
        .filter(|b| {
            env.plugins().get(&b.package.plugin).is_some_and(|p| {
                let version = p.update.unwrap_or(p.manifest.version);
                let local = p.kind != PluginKind::BuiltIn
                    && origins
                        .get_or_init(|| install::Origins::read(env.storage()))
                        .of(p.manifest.id.as_str(), version)
                        == install::Origin::Local;
                (p.kind == PluginKind::Code || p.enabled) && !local
            })
        })
        .cloned()
        .collect()
}

/// The update whose plugins are being fetched, if any are left.
pub(crate) fn fetching(storage: &dyn Storage) -> Option<String> {
    let text = paperpaw_sdk::read_file(storage, &file(QUEUE), 64 * 1024).ok()?;
    let text = core::str::from_utf8(&text).ok()?;
    let mut lines = text.lines();
    let version = lines.next()?.trim().to_string();
    (!from_lines(lines).is_empty() || INSTALLER.progress(storage).is_some()).then_some(version)
}

/// Starts fetching `builds` for the update `version`; `false` if there
/// are none.
pub(crate) fn start(
    env: &Env<'_>,
    version: &str,
    builds: &[Build],
    native: Option<(u64, u32)>,
    out: &mut Commands,
) -> bool {
    if builds.is_empty() {
        return false;
    }
    let storage = env.storage();
    let _ = storage.create_dir(&paperpaw_sdk::Env::plugin_dir(PLUGIN.as_str()));
    let _ = storage.remove(&file(FAILED));
    let _ = match native {
        Some((abi, compiler)) => storage.write(
            &file(WANTED),
            format!("{abi:016x} {compiler:08x}").as_bytes(),
        ),
        None => storage.remove(&file(WANTED)),
    };
    let (builds, no_room) = fitting(env, builds.to_vec());
    for name in no_room {
        failed(storage, &name);
    }
    keep(storage, version, &builds);
    next(env, version, out);
    true
}

fn keep(storage: &dyn Storage, version: &str, builds: &[Build]) {
    let text = format!("{version}\n{}", to_text(builds));
    let _ = storage.write(&file(QUEUE), text.as_bytes());
}

/// The plugins left to fetch, the one being fetched first.
fn left(storage: &dyn Storage) -> Vec<Build> {
    paperpaw_sdk::read_file(storage, &file(QUEUE), 64 * 1024)
        .ok()
        .and_then(|t| String::from_utf8(t).ok())
        .map(|t| from_lines(t.lines().skip(1)))
        .unwrap_or_default()
}

/// The plugin being fetched is done, or failed: the next one's turn.
fn done_with(storage: &dyn Storage, version: &str, plugin: &str) {
    let mut left = left(storage);
    if left.first().is_some_and(|b| b.package.plugin == plugin) {
        left.remove(0);
    }
    keep(storage, version, &left);
}

/// At the start: a download the restart cut off is let go (begun anew by
/// [`resume`]); what is left for an update that is no longer newer than
/// `firmware` (installed meanwhile, or another chosen) is let go whole.
pub(crate) fn interrupted(storage: &dyn Storage, firmware: &Firmware) {
    if INSTALLER.progress(storage).is_some() {
        INSTALLER.cancel(storage);
    }
    let version = paperpaw_sdk::read_file(storage, &file(QUEUE), 64 * 1024)
        .ok()
        .and_then(|t| String::from_utf8(t).ok())
        .and_then(|t| t.lines().next().map(|v| v.trim().to_string()));
    if version.is_some_and(|v| !compare_versions(&v, &firmware.version).is_gt()) {
        for name in [QUEUE, CURRENT, FAILED] {
            let _ = storage.remove(&file(name));
        }
    }
}

/// Goes on fetching (from the update page): after a restart cut it off.
pub(crate) fn resume(env: &Env<'_>, out: &mut Commands) {
    if INSTALLER.progress(env.storage()).is_some() {
        return;
    }
    if let Some(version) = fetching(env.storage()) {
        next(env, &version, out);
    }
}

/// Fetches the next plugin left; when none is, says the update is ready.
fn next(env: &Env<'_>, version: &str, out: &mut Commands) {
    let storage = env.storage();
    let mut left = left(storage);
    while !left.is_empty() {
        let build = left[0].clone();
        let current = format!("{} {}\n", build.package.plugin, build.author);
        let _ = storage.write(&file(CURRENT), current.as_bytes());
        if INSTALLER.start(env, &build.package, out) {
            return;
        }
        failed(storage, &build.package.name);
        left.remove(0);
        keep(storage, version, &left);
    }
    let _ = storage.remove(&file(QUEUE));
    let _ = storage.remove(&file(CURRENT));
    out.push(Command::Notify(env.i18n().format(
        "update.downloaded",
        &[(
            "version",
            Arg::Str(&crate::shown_version(env.i18n(), version)),
        )],
    )));
    if let Some(names) = failed_names(storage) {
        out.push(Command::Notify(
            env.i18n()
                .format("update.not-fetched", &[("plugins", Arg::Str(&names))]),
        ));
    }
}

fn failed(storage: &dyn Storage, name: &str) {
    let mut names = failed_names(storage).unwrap_or_default();
    if !names.is_empty() {
        names.push_str(", ");
    }
    names.push_str(name);
    let _ = storage.write(&file(FAILED), names.as_bytes());
}

/// The plugins that could not be fetched for the update, by name.
pub(crate) fn failed_names(storage: &dyn Storage) -> Option<String> {
    let text = paperpaw_sdk::read_file(storage, &file(FAILED), 4096).ok()?;
    let text = String::from_utf8(text).ok()?;
    (!text.trim().is_empty()).then(|| text.trim().to_string())
}

/// A download of a plugin's file done: the next one asked for, or the
/// plugin checked and put in its place, and the next plugin fetched.
pub(crate) fn received(env: &Env<'_>, reply: &HttpReply, out: &mut Commands) {
    let storage = env.storage();
    let Some(version) = paperpaw_sdk::read_file(storage, &file(QUEUE), 64 * 1024)
        .ok()
        .and_then(|t| String::from_utf8(t).ok())
        .and_then(|t| t.lines().next().map(str::to_string))
    else {
        return;
    };
    match INSTALLER.received(env, reply, out) {
        Received::Other | Received::Next(_) => return,
        Received::Complete(progress) => {
            if !place(env, &progress.plugin) {
                failed(storage, &progress.name);
            }
            INSTALLER.cancel(storage);
            done_with(storage, &version, &progress.plugin);
        }
        Received::Failed(progress) => {
            failed(storage, &progress.name);
            done_with(storage, &version, &progress.plugin);
        }
        _ => return,
    }
    next(env, &version, out);
}

/// Puts the complete download of `plugin` where the updated firmware
/// installs it, its code named as fetched: only by the author known (or,
/// for one new here, the author the server names).
fn place(env: &Env<'_>, plugin: &str) -> bool {
    let storage = env.storage();
    let (Some(native_build), Some(folder)) = (env.native(), INSTALLER.folder(storage)) else {
        return false;
    };
    let Ok((header, Some(signature))) = native::check_file(storage, &native_build.path(&folder))
    else {
        return false;
    };
    // Of the update's fingerprint and compiler, as the server names them,
    // else of the running firmware's: never a file the update would not
    // load.
    let mut wanted = native_build.clone();
    if let Some((abi, compiler)) = wanted_native(storage) {
        wanted.fingerprint = native::Fingerprint(abi);
        wanted.compiler = compiler;
    }
    if !wanted.fits(&header) {
        return false;
    }
    let author_ok = match native::author(storage, plugin, &signature.key) {
        Author::Known => true,
        Author::Other => false,
        Author::New => {
            author_named(env, plugin, &signature.key)
                && native::remember_author(storage, plugin, &signature.key)
        }
        _ => false,
    };
    if !author_ok || !native::remember_fetched(storage, plugin, &header.sha256) {
        return false;
    }
    // The store's at the version fetched too: it is still once the new
    // firmware puts it in place.
    let version = install::folder_release(storage, &folder);
    let target = native::fetched_folder(plugin);
    let _ = storage.remove_dir_all(&target);
    if storage.rename(&folder, &target).is_err() {
        return false;
    }
    if let Some(version) = version {
        install::remember_store_install(storage, plugin, version);
    }
    true
}

/// The fingerprint and compiler the update's code plugins must have, as
/// [`start`] kept them.
fn wanted_native(storage: &dyn Storage) -> Option<(u64, u32)> {
    let text = paperpaw_sdk::read_file(storage, &file(WANTED), 64).ok()?;
    let text = core::str::from_utf8(&text).ok()?;
    let mut parts = text.split_whitespace();
    let abi = u64::from_str_radix(parts.next()?, 16).ok()?;
    let compiler = u32::from_str_radix(parts.next()?, 16).ok()?;
    Some((abi, compiler))
}

/// Whether the server named `key` as the author of `plugin`, new here.
fn author_named(env: &Env<'_>, plugin: &str, key: &[u8; 32]) -> bool {
    let Some(current) = paperpaw_sdk::read_file(env.storage(), &file(CURRENT), 1024)
        .ok()
        .and_then(|t| String::from_utf8(t).ok())
    else {
        return false;
    };
    let mut parts = current.split_whitespace();
    parts.next() == Some(plugin)
        && parts
            .next()
            .is_some_and(|named| paperpaw_sdk::digest::hex::encode(key).eq_ignore_ascii_case(named))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANSWER: &str = r#"{"version": "0.5.0", "url": "u", "code": [
        {"id": "stats", "name": "Statistics", "version": "0.5.0",
         "author_key": "AB01ab01ab01ab01ab01ab01ab01ab01ab01ab01ab01ab01ab01ab01ab01ab01",
         "files": [
            {"path": "plugin.ini", "url": "https://p/stats/plugin.ini", "size": 120,
             "sha1": "0123456789012345678901234567890123456789"},
            {"path": "code.ppn", "url": "https://p/stats/x4/code.ppn", "size": 412345,
             "sha256": "0123456789012345678901234567890123456789012345678901234567890123"}]},
        {"id": "broken", "version": "1.0.0", "files": []}]}"#;

    #[test]
    fn builds_are_read_from_the_answer() {
        let builds = parse(ANSWER);
        assert_eq!(builds.len(), 1, "the one without files is passed over");
        let stats = &builds[0];
        assert_eq!(stats.package.plugin, "stats");
        assert_eq!(stats.package.name, "Statistics");
        assert_eq!(stats.version, "0.5.0");
        assert!(stats.author.starts_with("ab01"));
        assert_eq!(stats.package.files.len(), 2);
        assert_eq!(stats.package.files[1].size, 412_345);
        assert!(matches!(stats.package.files[1].digest, Digest::Sha256(_)));
    }

    #[test]
    fn builds_are_kept_as_text() {
        let builds = parse(ANSWER);
        let text = to_text(&builds);
        assert_eq!(from_lines(text.lines()), builds);
    }

    #[test]
    fn plugins_left_out_come_back_as_files() {
        let builds = parse(ANSWER);
        let storage = paperpaw_sdk::MemoryStorage::new()
            .with(
                "plugins/.widgets.next/plugin.ini",
                "[plugin]\nname = Widgets\n",
            )
            .with(
                "plugins/.weather.next/plugin.ini",
                "[plugin]\nname = Weather\n",
            );
        // Fetched and checked: its code named as fetched; weather's not.
        assert!(native::remember_fetched(&storage, "widgets", &[1; 32]));
        assert!(comes_back(&storage, &builds, "paperpaw-plugin-stats"));
        assert!(comes_back(&storage, &builds, "paperpaw-plugin-widgets"));
        assert!(!comes_back(&storage, &builds, "paperpaw-plugin-weather"));
        // Not among those this device takes (switched off, say).
        assert!(!comes_back(&storage, &[], "paperpaw-plugin-stats"));
        assert!(!comes_back(&storage, &builds, "chess-for-paperpaw"));
    }
}
