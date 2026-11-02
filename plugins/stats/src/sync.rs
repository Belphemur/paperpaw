//! Sharing statistics with KOReader: through the same cloud folder
//! (WebDAV, e.g. Nextcloud) KOReader syncs its statistics to, and by
//! taking over the statistics of KOReader on the same device.
//!
//! A sync downloads `statistics.sqlite3` from the folder, merges it (see
//! [`merge`]), uploads the result (only if nobody changed the file in
//! between: `If-Match` with its ETag, or `If-None-Match: *` where there
//! was none, else it starts over) and keeps what it uploaded, to tell
//! later what was deleted there; for that folder and user only. KOReader
//! does the same, so either may sync first. A downloaded file that cannot
//! be read ends the sync: nothing is merged and nothing uploaded.
//!
//! Nothing counts as deleted there where the download came without an
//! ETag: the upload could then replace a file changed meanwhile. A file
//! gone from a folder this device uploaded to is not replaced at once (a
//! server may say so by mistake): the sync says so, and the next one
//! uploads. Where the file there lacks much and it cannot be told whether
//! that was deleted there (see [`merge`]), nothing is uploaded: the user
//! is asked whether to remove it here too, and the sync starts again with
//! the answer, kept for that file until an upload changes it.

use alloc::borrow::Cow;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::{Confirmed, HttpReply, Typed};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Commands, Cx, Env, Gesture, GestureKind, Handled, HttpRequest, Id,
    Params, Rect, RefreshMode, Screen,
};

use paperpaw_sdk::secrets::SecretChange;

use crate::merge::{self, Failure, Outcome};
use paperpaw_sdk::setting::{Setting, Text};

/// The folder (its address) and the user; the password is kept with the
/// other passwords and keys (see `paperpaw_sdk::secrets`) as [`SECRET`].
pub const URL: Setting<Text> = Setting::new("stats.webdav-url");
pub const USER: Setting<Text> = Setting::new("stats.webdav-user");
pub(crate) const SCREEN: Id = Id::new_static("stats.sync");
/// The password.
pub(crate) const SECRET: &str = "stats.webdav";

const DOWNLOADED: Event<HttpReply> = Event::new("stats.downloaded");
const UPLOADED: Event<HttpReply> = Event::new("stats.uploaded");
/// Emitted with a message (`text`) when a sync or import ends.
pub(crate) const DONE: Event<Finished> = Event::new("stats.sync-done");

paperpaw_sdk::event_data! {
    /// How a sync ended ([`DONE`]), in words.
    pub(crate) struct Finished {
        pub(crate) text: String,
    }
}
const URL_ENTERED: Event<Typed> = Event::new("stats.url-entered");
/// Whether to remove here what was removed there; with the file's ETag
/// as [`ETAG`].
const DECIDED: Event<Confirmed> = Event::new("stats.removed-there");
/// The message read in a dialog (nothing to do then).
const READ: Event<Confirmed> = Event::new("stats.sync-message-read");
const ETAG: &str = "etag";

/// The WebDAV folder and its user, both set on the sync page.
static ON_PAGE: paperpaw_sdk::SettingExtra = paperpaw_sdk::SettingExtra::new().editor(SCREEN);
static SETTINGS: [paperpaw_sdk::SettingSpec; 2] = [
    paperpaw_sdk::SettingSpec::text(URL.static_str(), "stats.setting.sync", "", crate::GROUP)
        .with_extra(&ON_PAGE)
        .kept_on_reset()
        .description("stats.setting.sync.help"),
    paperpaw_sdk::SettingSpec::text(USER.static_str(), "stats.sync.user", "", crate::GROUP)
        .with_extra(&ON_PAGE)
        .kept_on_reset()
        .advanced()
        .description("stats.sync.user.help"),
];
const USER_ENTERED: Event<Typed> = Event::new("stats.user-entered");
const PASSWORD_ENTERED: Event<Typed> = Event::new("stats.password-entered");

/// The file's name in the folder, as KOReader names it.
const FILE: &str = "statistics.sqlite3";
/// Tries when the file changes during a sync.
const ATTEMPTS: u8 = 3;
/// Where KOReader keeps its statistics on a device: on Android's shared
/// storage, and at the root of a memory card.
const KOREADER_FILES: [&str; 2] = [
    "storage/koreader/settings/statistics.sqlite3",
    "koreader/settings/statistics.sqlite3",
];

fn dir() -> String {
    Env::plugin_dir(crate::PLUGIN.as_str())
}

fn file(name: &str) -> String {
    format!("{}/{name}", dir())
}

/// What a sync in progress knows.
#[derive(Debug, Default)]
pub(crate) struct Sync {
    attempt: u8,
    running: bool,
    /// The last upload to the folder and user of this sync
    /// ([`last_upload`]).
    last_upload: String,
    /// The file and the user of this sync, as set when it started.
    url: String,
    user: String,
    /// The books there are after the merge.
    books: usize,
    /// Whether what the file there lacked was kept ([`Outcome::kept`]).
    kept: bool,
    /// What the user decided where the file there lacked much and it
    /// could not be told why ([`Outcome::unclear`]): for that file (its
    /// address and ETag), whether to remove it here too. It holds until
    /// an upload changes the file.
    decided: Option<(String, String, bool)>,
}

pub(crate) type SharedSync = Rc<RefCell<Sync>>;

/// The folder as a base URL: `https://` unless another scheme is given,
/// no `/` at the end.
pub(crate) fn folder(text: &str) -> String {
    paperpaw_sdk::web::base_url(text)
}

fn password<'a>(env: &Env<'a>) -> &'a str {
    env.secrets().get(SECRET).unwrap_or_default()
}

/// Keeps the password as typed (spaces too), or removes it if empty.
fn save_password(password: &str) -> Command {
    Command::set_secret(if password.is_empty() {
        SecretChange::remove(SECRET)
    } else {
        SecretChange::keep(SECRET, "stats.secret", password)
    })
}

/// Moves a password kept in the plugin's folder by earlier versions to
/// the other passwords and keys. The old file goes only once the
/// secrets have a password (at a later start): until then it is the only
/// copy, e.g. where the secrets could not be saved.
pub(crate) fn move_password(env: &Env<'_>, out: &mut Commands) {
    let path = file("webdav.ini");
    let Ok(data) = paperpaw_sdk::read_file(env.storage(), &path, 4096) else {
        return;
    };
    let old = old_password(core::str::from_utf8(&data).unwrap_or_default());
    if !old.is_empty() && env.secrets().get(SECRET).is_none() {
        out.push(save_password(&old));
        return;
    }
    #[expect(
        clippy::let_underscore_must_use,
        reason = "the password is in the secrets (or there was none); a file left behind is read once more and changes nothing"
    )]
    let _ = env.storage().remove(&path);
}

/// The password in the `webdav.ini` of earlier versions.
fn old_password(text: &str) -> String {
    paperpaw_sdk::config::parse(text)
        .filter_map(Result::ok)
        .find(|e| e.key == "password")
        .map(|e| e.value.as_str().into_owned())
        .unwrap_or_default()
}

impl Sync {
    /// A request for the file of this sync.
    fn request(&self, env: &Env<'_>, method: &str, reply: &Event<HttpReply>) -> HttpRequest {
        let request = HttpRequest::new(method, &self.url, reply);
        if self.user.is_empty() {
            return request;
        }
        let auth = paperpaw_sdk::web::basic_auth(&self.user, password(env));
        request.header(String::from("Authorization"), auth)
    }
}

fn done(out: &mut Commands, message: String) {
    out.push(DONE.emit(Finished { text: message }));
}

/// The file this device last uploaded to the folder and user set now:
/// the next sync there compares with it. One is kept for one folder and
/// user only: another one's tells nothing about what was deleted here.
fn last_upload(env: &Env<'_>) -> String {
    let target = format!(
        "{}\n{}",
        folder(env.settings().read(&URL)),
        env.settings().read(&USER)
    );
    let key = paperpaw_sdk::digest::fnv1a64(target);
    file(&format!("{FILE}.{key:016x}.sync"))
}

/// Removes the last uploads kept but `keep`: those of other folders or
/// users, and the one of earlier versions (`statistics.sqlite3.sync`, of
/// whichever folder was set then).
fn forget_other_uploads(env: &Env<'_>, keep: &str) {
    let mut others = Vec::new();
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a folder that cannot be listed has no old uploads to forget"
    )]
    let _ = env.storage().list(&dir(), &mut |entry| {
        let path = file(entry.name);
        if entry.name.starts_with(FILE) && entry.name.ends_with(".sync") && path != keep {
            others.push(path);
        }
    });
    for path in others {
        #[expect(
            clippy::let_underscore_must_use,
            reason = "an old upload left behind only takes room"
        )]
        let _ = env.storage().remove(&path);
    }
}

impl Sync {
    /// Starts a sync, or says why it cannot.
    pub(crate) fn start(&mut self, env: &Env<'_>, out: &mut Commands) {
        if !env.caps().network {
            return done(out, env.i18n().tr("stats.sync.no-network").to_string());
        }
        if env.settings().read(&URL).trim().is_empty() {
            return done(out, env.i18n().tr("stats.sync.no-folder").to_string());
        }
        self.running = true;
        self.attempt += 1;
        self.last_upload = last_upload(env);
        self.url = format!("{}/{FILE}", folder(env.settings().read(&URL)));
        self.user = env.settings().read(&USER).to_string();
        let mut get = self.request(env, "GET", &DOWNLOADED);
        get = get.save_to(file("incoming.sqlite3"));
        out.push(Command::http(get));
    }

    /// The other side's file is here (or not): merge, then upload.
    pub(crate) fn downloaded(&mut self, env: &Env<'_>, reply: &HttpReply, out: &mut Commands) {
        if !self.running {
            return;
        }
        let incoming = file("incoming.sqlite3");
        let uploaded_before = env.storage().exists(&self.last_upload);
        let income = match reply.status {
            200 => Some(incoming.as_str()),
            // Gone from where this device uploaded to (or a server's
            // mistake): not replaced with this device's alone. Without
            // the last upload, the next sync uploads.
            404 if uploaded_before => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "a download left behind is written over by the next sync"
                )]
                let _ = env.storage().remove(&incoming);
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "a last upload that cannot be removed only makes the next sync say the same"
                )]
                let _ = env.storage().remove(&self.last_upload);
                self.running = false;
                self.attempt = 0;
                return done(out, env.i18n().tr("stats.sync.missing").to_string());
            }
            // None yet: this is the first.
            404 => None,
            status => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "a download left behind is written over by the next sync"
                )]
                let _ = env.storage().remove(&incoming);
                return self.fail(env, &status.to_string(), out);
            }
        };
        // Without an ETag the upload replaces the file whatever came
        // meanwhile: what it lacks then is not taken for deleted there.
        let cached = (uploaded_before && reply.etag.is_some()).then_some(self.last_upload.as_str());
        let upload = file(FILE);
        let decided = self.decided_for(reply.etag.as_deref());
        let result = merge::merge_as(
            env.storage(),
            income,
            cached,
            Some(&upload),
            &dir(),
            decided,
        );
        #[expect(
            clippy::let_underscore_must_use,
            reason = "a download left behind is written over by the next sync"
        )]
        let _ = env.storage().remove(&incoming);
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(failure) => {
                self.running = false;
                self.attempt = 0;
                let key = match failure {
                    Failure::Income(_) => "stats.sync.unreadable",
                    Failure::Here(_) => "stats.sync.failed",
                };
                let reason = format!("{failure}");
                return done(
                    out,
                    env.i18n().format(key, &[("reason", Arg::Str(&reason))]),
                );
            }
        };
        if outcome.unclear {
            return self.ask(env, reply.etag.as_deref().unwrap_or_default(), out);
        }
        self.books = outcome.books;
        self.kept = outcome.kept;
        let mut put = self.request(env, "PUT", &UPLOADED).body_from(upload);
        match (income, &reply.etag) {
            // There was none: only if there still is none.
            (None, _) => put = put.header(String::from("If-None-Match"), String::from("*")),
            (Some(_), Some(etag)) => {
                // A weak validator does not match strongly (as KOReader does).
                let etag = etag.trim_start_matches("W/").trim_start_matches("w/");
                put = put.header(String::from("If-Match"), etag.to_string());
            }
            (Some(_), None) => {}
        }
        let put = put.header(
            String::from("Content-Type"),
            String::from("application/octet-stream"),
        );
        out.push(Command::http(put));
    }

    /// What the user decided for the file there with `etag`, if anything.
    fn decided_for(&self, etag: Option<&str>) -> Option<bool> {
        let (url, decided_etag, remove) = self.decided.as_ref()?;
        (*url == self.url && Some(decided_etag.as_str()) == etag).then_some(*remove)
    }

    /// The file there (with `etag`) lacks much, and it cannot be told
    /// whether that was deleted there: nothing is uploaded, and the user
    /// is asked whether to remove it here too. Going back asks again with
    /// the next sync.
    fn ask(&mut self, env: &Env<'_>, etag: &str, out: &mut Commands) {
        #[expect(
            clippy::let_underscore_must_use,
            reason = "an upload left behind is written over by the next sync"
        )]
        let _ = env.storage().remove(&file(FILE));
        self.running = false;
        self.attempt = 0;
        done(out, env.i18n().tr("stats.sync.asked").to_string());
        out.push(
            paperpaw_sdk::ask::confirm(env.i18n().tr("stats.sync.ask"), &DECIDED)
                .text(env.i18n().tr("stats.sync.ask.text"))
                .yes(env.i18n().tr("stats.sync.remove"))
                .no(env.i18n().tr("stats.sync.keep"))
                .with(ETAG, etag)
                .command(),
        );
    }

    /// The user decided: the sync starts again, with the decision for the
    /// file asked about.
    pub(crate) fn decide(&mut self, env: &Env<'_>, answer: &Confirmed, out: &mut Commands) {
        if self.running {
            return;
        }
        let etag = answer.context.get(ETAG).unwrap_or_default().to_string();
        self.decided = Some((self.url.clone(), etag, answer.yes));
        self.start(env, out);
    }

    /// The merged file is up (or someone was faster).
    pub(crate) fn uploaded(&mut self, env: &Env<'_>, reply: &HttpReply, out: &mut Commands) {
        if !self.running {
            return;
        }
        let status = reply.status.to_string();
        let upload = file(FILE);
        match reply.status {
            200..=299 => {
                // What was uploaded is what the next sync compares with.
                // Where it cannot be kept, the one before stays: it can
                // only tell less, never more, as deleted there.
                if paperpaw_sdk::files::move_to(env.storage(), &upload, &self.last_upload).is_err()
                {
                    #[expect(
                        clippy::let_underscore_must_use,
                        reason = "an upload left behind is written over by the next sync"
                    )]
                    let _ = env.storage().remove(&upload);
                }
                forget_other_uploads(env, &self.last_upload);
                // The file there changed: a decision for it holds no more.
                self.decided = None;
                self.running = false;
                self.attempt = 0;
                let key = if self.kept {
                    "stats.sync.done-kept"
                } else {
                    "stats.sync.done"
                };
                let text = env.i18n().format(
                    key,
                    &[(
                        "books",
                        Arg::Str(
                            &env.i18n()
                                .plural("stats.books-count", self.books as i64, &[]),
                        ),
                    )],
                );
                done(out, text);
            }
            // Changed in between: again, from the download.
            412 if self.attempt < ATTEMPTS => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "an upload left behind is written over by the next sync"
                )]
                let _ = env.storage().remove(&upload);
                self.running = false;
                self.start(env, out);
            }
            _ => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "an upload left behind is written over by the next sync"
                )]
                let _ = env.storage().remove(&upload);
                self.fail(env, &status, out);
            }
        }
    }

    fn fail(&mut self, env: &Env<'_>, status: &str, out: &mut Commands) {
        self.running = false;
        self.attempt = 0;
        let key = match status {
            "0" => "stats.sync.unreachable",
            "401" | "403" => "stats.sync.denied",
            "404" | "409" => "stats.sync.no-such-folder",
            _ => "stats.sync.server-error",
        };
        done(out, env.i18n().format(key, &[("status", Arg::Str(status))]));
    }
}

/// KOReader's statistics on this device, if any.
pub(crate) fn koreader_file(env: &Env<'_>) -> Option<&'static str> {
    KOREADER_FILES.into_iter().find(|f| env.storage().exists(f))
}

/// Takes over KOReader's statistics on this device.
fn import(env: &Env<'_>) -> String {
    let Some(path) = koreader_file(env) else {
        return env.i18n().tr("stats.import.none").to_string();
    };
    // Written by KOReader while it runs: its newest pages may still be
    // in the log beside the file.
    let wal = format!("{path}-wal");
    if env.storage().size(&wal).is_ok_and(|s| s > 0) {
        return env.i18n().tr("stats.import.busy").to_string();
    }
    match merge::merge(env.storage(), Some(path), None, None, &dir()) {
        Ok(Outcome { books, stays, .. }) => env.i18n().format(
            "stats.import.done",
            &[
                (
                    "books",
                    Arg::Str(&env.i18n().plural("stats.books-count", books as i64, &[])),
                ),
                ("stays", Arg::Int(stays as i64)),
            ],
        ),
        Err(failure) => {
            let key = match failure {
                Failure::Income(_) => "stats.import.unreadable",
                Failure::Here(_) => "stats.sync.failed",
            };
            let reason = format!("{failure}");
            env.i18n().format(key, &[("reason", Arg::Str(&reason))])
        }
    }
}

/// Registers the sync's settings, events and screen.
pub(crate) fn register(
    r: &mut paperpaw_sdk::Registrar,
    sync: SharedSync,
) -> Result<(), paperpaw_sdk::RegisterError> {
    use paperpaw_sdk::ScreenSpec;
    r.on(paperpaw_sdk::builtin::events::STARTED, |env, (), out| {
        move_password(env, out);
    })?;
    r.add_settings(&SETTINGS)?;
    let s = sync.clone();
    r.on(DOWNLOADED, move |env, reply, out| {
        s.borrow_mut().downloaded(env, reply, out);
    })?;
    let s = sync.clone();
    r.on(UPLOADED, move |env, reply, out| {
        s.borrow_mut().uploaded(env, reply, out);
    })?;
    let s = sync.clone();
    r.on(DECIDED, move |env, answer, out| {
        s.borrow_mut().decide(env, answer, out);
    })?;
    r.add_screen(
        ScreenSpec::new(
            SCREEN,
            "stats.sync.title",
            alloc::boxed::Box::new(move |env, _| {
                alloc::boxed::Box::new(SyncScreen::new(env, sync.clone()))
            }),
        )
        .instances(paperpaw_sdk::Instances::SINGLE),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Folder,
    User,
    Password,
    SyncNow,
    Import,
    /// The message whole, in a dialog, where it does not fit below.
    More,
}

/// The sync's screen: folder, user and password, *Sync now*, and
/// *Take over from KOReader* when KOReader's statistics are here.
pub(crate) struct SyncScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    note: Rect,
    rows: Vec<Row>,
    sync: SharedSync,
    message: String,
    /// Offline mode: the offer to go online, and whether *Sync now* waits
    /// for it.
    online: ui::offline::OfflineOffer,
    waiting: bool,
}

impl SyncScreen {
    fn new(env: &Env<'_>, sync: SharedSync) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let mut rows = Vec::from([Row::Folder, Row::User, Row::Password, Row::SyncNow]);
        if koreader_file(env).is_some() {
            rows.push(Row::Import);
        }
        let top = header.area().height + style.gap();
        let mut screen = Self {
            list: ListView::new(
                Rect::new(0, top, display.width, style.row()),
                u32::from(style.row()),
            )
            .focus_on_keys(env),
            note: Rect::default(),
            rows,
            sync,
            message: env.i18n().tr("stats.sync.help").to_string(),
            online: ui::offline::OfflineOffer::new(),
            waiting: false,
            style,
            header,
        };
        screen.place(env);
        screen
    }

    /// The list and the message below it: the message whole, the list
    /// paging where the rest does not hold its rows (the largest text),
    /// two rows high at least; where the message does not fit even so, a
    /// last row shows it.
    fn place(&mut self, env: &Env<'_>) {
        let (style, display) = (&self.style, env.device().display);
        let top = self.list.area().y;
        let row = style.row();
        let note = ui::message_height(env, style, display.width, &self.message);
        let rest = display.height.saturating_sub(top + style.gap());
        self.rows.retain(|r| *r != Row::More);
        if note > rest.saturating_sub(row * 2) {
            self.rows.push(Row::More);
        }
        if self.list.focus() >= self.rows.len() {
            self.list.set_focus(self.rows.len() - 1);
        }
        let note = if self.more() { 0 } else { note };
        let room = rest.saturating_sub(note);
        let height = (row * self.rows.len() as u16)
            .min(room.max(row * 2))
            .min(display.height.saturating_sub(top));
        self.list.set_area(Rect::new(0, top, display.width, height));
        let note_top = top + height + style.gap();
        self.note = Rect::new(
            0,
            note_top,
            display.width,
            display.height.saturating_sub(note_top),
        );
    }

    /// Whether the message shows in a row's dialog, not below.
    fn more(&self) -> bool {
        self.rows.last() == Some(&Row::More)
    }

    /// Whether the offer to go online shows in place of the list.
    fn offering(&self, env: &Env<'_>) -> bool {
        self.waiting && self.online.shows(env)
    }

    /// What the reader chose on the offer: the sync starts, or is told to
    /// need the network.
    fn offered(&mut self, chosen: Option<ui::offline::Offer>, cx: &mut Cx<'_, '_>) {
        let Some(chosen) = chosen else {
            return;
        };
        self.waiting = false;
        match chosen {
            ui::offline::Offer::Online => self.activate(Row::SyncNow, cx),
            _ => {
                let message = cx.env().i18n().tr("stats.sync.no-network").to_string();
                self.show(message, cx);
            }
        }
    }

    fn show(&mut self, message: String, cx: &mut Cx<'_, '_>) {
        self.message = message;
        self.place(cx.env());
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }

    fn activate(&mut self, row: Row, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        // The address typed as it is, the password as dots.
        let keyboard = |title: &str, text: String, reply: &Event<Typed>, password: bool| {
            let question = paperpaw_sdk::ask::text(env.i18n().tr(title), reply).initial(text);
            if password {
                question.password().command()
            } else if title == "stats.sync.folder" {
                question.url().command()
            } else {
                question.command()
            }
        };
        match row {
            Row::Folder => cx.command(keyboard(
                "stats.sync.folder",
                env.settings().read(&URL).to_string(),
                &URL_ENTERED,
                false,
            )),
            Row::User => cx.command(keyboard(
                "stats.sync.user",
                env.settings().read(&USER).to_string(),
                &USER_ENTERED,
                false,
            )),
            Row::Password => cx.command(keyboard(
                "stats.sync.password",
                String::new(),
                &PASSWORD_ENTERED,
                true,
            )),
            Row::SyncNow => {
                if self.sync.borrow().running {
                    return;
                }
                // Offline mode: asked first, once.
                if env.caps().network
                    && !env.settings().read(&URL).trim().is_empty()
                    && self.online.shows(&env)
                {
                    self.waiting = true;
                    cx.request_redraw();
                    cx.request_refresh(RefreshMode::Fast);
                    return;
                }
                let mut commands = paperpaw_sdk::Commands::new();
                self.sync.borrow_mut().start(&env, &mut commands);
                for command in commands {
                    cx.command(command);
                }
                if self.sync.borrow().running {
                    self.show(env.i18n().tr("stats.sync.running").to_string(), cx);
                }
            }
            Row::Import => {
                let message = import(&env);
                self.show(message, cx);
            }
            Row::More => cx.command(
                paperpaw_sdk::ask::confirm(env.i18n().tr("stats.sync.title"), &READ)
                    .text(self.message.clone())
                    .only_yes()
                    .command(),
            ),
        }
    }
}

impl Screen for SyncScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let title = env.i18n().tr("stats.sync.title");
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(&mut ui, title, None, None);
        if self.offering(env) {
            let top = self.list.area().y;
            let area = Rect::new(
                0,
                top,
                env.device().display.width,
                env.device().display.height.saturating_sub(top),
            );
            return self.online.draw(&mut ui, area);
        }
        let rows = &self.rows;
        let has_password = !password(env).is_empty();
        self.list
            .draw(&mut ui, rows.len(), |ui, index, rect, focused| {
                let (title, value) = match rows[index] {
                    Row::Folder => {
                        let url = folder(env.settings().read(&URL));
                        let shown = url.strip_prefix("https://").unwrap_or(&url).to_string();
                        ("stats.sync.folder", shown)
                    }
                    Row::User => ("stats.sync.user", env.settings().read(&USER).to_string()),
                    Row::Password => (
                        "stats.sync.password",
                        String::from(if has_password {
                            "••••••"
                        } else {
                            ""
                        }),
                    ),
                    Row::SyncNow => ("stats.sync.now", String::from("›")),
                    Row::Import => ("stats.import", String::from("›")),
                    Row::More => ("stats.sync.more", String::from("›")),
                };
                ui::Row::value(env.i18n().tr(title), &value)
                    .focused(focused)
                    .draw(ui, rect);
            });
        if !self.more() {
            ui.message(self.note, &self.message);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let a = action;
        if *a != actions::BACK && self.offering(cx.env()) {
            let chosen = self.online.on_action(a, cx).done();
            self.offered(chosen, cx);
            return Handled::Yes;
        }
        let delta = if [actions::UP, actions::LEFT, actions::PREVIOUS].contains(a) {
            -1
        } else if [actions::DOWN, actions::RIGHT, actions::NEXT].contains(a) {
            1
        } else if *a == actions::CONFIRM {
            self.activate(self.rows[self.list.focus().min(self.rows.len() - 1)], cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list
            .move_focus_in(cx.env(), delta, self.rows.len())
            .request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        if let Some(action) = self.header.tap(cx.env(), &self.style, gesture.start, None) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        if self.offering(cx.env()) {
            let chosen = self.online.on_gesture(&gesture, cx).done();
            self.offered(chosen, cx);
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, self.rows.len()) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        cx.request_redraw_area(self.list.area());
        self.activate(self.rows[index], cx);
        Handled::Yes
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(typed) = URL_ENTERED.read(event, params) {
            cx.command(Command::set(&URL, Cow::Owned(folder(&typed.text))));
            self.list.set_focus(1);
        } else if let Some(typed) = USER_ENTERED.read(event, params) {
            cx.command(Command::set(
                &USER,
                Cow::Owned(typed.text.trim().to_string()),
            ));
            self.list.set_focus(2);
        } else if let Some(typed) = PASSWORD_ENTERED.read(event, params) {
            cx.command(save_password(&typed.text));
            self.list.set_focus(3);
        } else if let Some(finished) = DONE.read(event, params) {
            return self.show(finished.text, cx);
        } else {
            return;
        }
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credentials_and_folders() {
        assert_eq!(
            folder(" cloud.example.org/dav/KOReader/ "),
            "https://cloud.example.org/dav/KOReader"
        );
        assert_eq!(folder("http://nas:8080/dav"), "http://nas:8080/dav");
    }

    /// A password is kept as typed: spaces at either end belong to it.
    #[test]
    fn passwords_are_kept_as_typed() {
        let Command::SetSecret(change) = save_password(" secret ") else {
            panic!("no secret kept");
        };
        assert_eq!(change.value.as_deref(), Some(" secret "));
        let Command::SetSecret(change) = save_password("") else {
            panic!("no secret removed");
        };
        assert_eq!(change.value, None);
    }

    /// What version 1 wrote stays readable (see `docs/data-files.md`).
    #[test]
    fn the_password_of_version_1_is_found() {
        let text = include_str!("../testdata/webdav.ini.v1");
        assert_eq!(super::old_password(text), "secret 1");
    }
}
