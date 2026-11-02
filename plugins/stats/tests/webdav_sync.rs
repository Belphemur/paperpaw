//! Syncing the statistics with KOReader's through a WebDAV folder, with
//! the real kernel and its HTTP requests answered in the test as a
//! server would: the first sync (nothing there yet), a newer file there,
//! someone faster (412), a wrong password, a missing folder, no network,
//! and a file there that is no database; the password of earlier
//! versions moving to the passwords and keys; taking over KOReader's
//! statistics on the same device.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{App, Card, DAY, DIR, TOUCH, X4};
use paperpaw_core::{HttpCall, HttpResponse};
use paperpaw_sdk::event::EventData;
use paperpaw_sdk::replies::Typed;
use paperpaw_sdk::secrets::{FILE, SecretChange, Secrets};
use paperpaw_sdk::{Id, Instant, MemoryStorage, Params, Storage};

/// 14 October 2026, 10:41 in Berlin (UTC+2).
const NOW: i64 = 1_791_967_260_000;
const TODAY: i64 = 20_740;
const FOLDER: &str = "[stats]\nwebdav-url = dav.example/koreader/\nwebdav-user = me\n";
const REMOTE: &str = "https://dav.example/koreader/statistics.sqlite3";
const SECRET: &str = "stats.webdav";

/// A device that read `book` (an id and title) for `minutes` today,
/// starting `hour` (UTC), its password kept.
fn device(books: &[(u16, &str)], hour: i64, minutes: u32) -> MemoryStorage {
    let mut card = Card::new();
    for &(id, title) in books {
        card = card.book(id, title, 300, TODAY * DAY).read(
            id,
            TODAY * DAY + hour * 3600 + i64::from(id) * 7200,
            minutes,
        );
    }
    let storage = card.build();
    let mut secrets = Secrets::default();
    secrets.apply(SecretChange::keep(SECRET, "stats.secret", "pw"));
    storage.write(FILE, secrets.encode().as_bytes()).unwrap();
    storage
}

fn app(storage: MemoryStorage) -> App {
    App::new(X4, storage, Some(NOW), FOLDER, true)
}

/// The statistics' files: the log and the books.
fn statistics(app: &App) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    (
        app.storage.get(&format!("{DIR}/pages.bin")),
        app.storage.get(&format!("{DIR}/books.ini")),
    )
}

/// The titles of the books.
fn titles(app: &App) -> Vec<String> {
    let books = app
        .storage
        .get(&format!("{DIR}/books.ini"))
        .unwrap_or_default();
    String::from_utf8(books)
        .unwrap()
        .lines()
        .filter_map(|l| l.strip_prefix("title = "))
        .map(|t| t.trim_matches('"').to_string())
        .collect()
}

/// The files kept of the last uploads (`statistics.sqlite3.<key>.sync`,
/// one for the folder and user set).
fn last_uploads(app: &App) -> Vec<String> {
    app.storage
        .files()
        .into_iter()
        .filter(|f| f.starts_with("statistics.sqlite3.") && f.ends_with(".sync"))
        .collect()
}

fn header<'a>(call: &'a HttpCall, name: &str) -> Option<&'a str> {
    call.headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

impl App {
    /// *Sync now*, by buttons: the request it makes.
    fn sync_now(&mut self) -> Vec<HttpCall> {
        // From the start of the page: one open is gone back to as it was.
        if self.screen() == "stats.sync" {
            self.press("back");
        }
        self.open("stats.sync", Params::new());
        for _ in 0..3 {
            self.press("down");
        }
        assert!(self.shown().contains(">Sync now\n"), "{}", self.shown());
        self.press("confirm");
        self.requests()
    }

    /// The server's answer to `call`: what follows.
    fn answer(
        &mut self,
        call: &HttpCall,
        status: u16,
        body: &[u8],
        etag: Option<&str>,
    ) -> Vec<HttpCall> {
        self.now += 100;
        let response = HttpResponse {
            id: call.id,
            status,
            body: body.to_vec(),
            etag: etag.map(str::to_string),
            ..HttpResponse::default()
        };
        let effects = self.kernel.http_response(Instant(self.now), response);
        self.settle(effects);
        self.requests()
    }

    /// The file a PUT sends.
    fn body(&self, put: &HttpCall) -> Vec<u8> {
        let path = put.body_from.as_deref().expect("a file to send");
        self.storage.get(path).expect("the file to send is there")
    }
}

/// The first sync of device A: what it uploaded.
fn first_upload() -> Vec<u8> {
    let mut a = app(device(&[(1, "Anna Karenina"), (2, "Effi Briest")], 6, 10));
    let get = a.sync_now();
    let put = a.answer(&get[0], 404, b"Not Found", None);
    let body = a.body(&put[0]);
    assert!(a.answer(&put[0], 201, b"", None).is_empty());
    body
}

#[test]
fn the_first_sync_uploads_what_is_here() {
    let mut app = app(device(&[(1, "Anna Karenina"), (2, "Effi Briest")], 6, 10));
    let before = common::stays(&app.storage);
    assert_eq!(before.len(), 20);
    let requests = app.sync_now();
    assert_eq!(requests.len(), 1);
    let get = &requests[0];
    assert_eq!((get.method.as_str(), get.url.as_str()), ("GET", REMOTE));
    assert_eq!(header(get, "Authorization"), Some("Basic bWU6cHc="));
    assert!(get.save_to.is_some());
    assert!(app.shown().contains("Syncing …"), "{}", app.shown());
    // Pressed again while it runs: nothing more.
    app.press("confirm");
    assert!(app.requests().is_empty());

    // Nothing there yet: what is here goes up, unless a file came there
    // meanwhile.
    let requests = app.answer(get, 404, b"Not Found", None);
    assert_eq!(requests.len(), 1);
    let put = &requests[0];
    assert_eq!((put.method.as_str(), put.url.as_str()), ("PUT", REMOTE));
    assert_eq!(header(put, "If-Match"), None);
    assert_eq!(header(put, "If-None-Match"), Some("*"));
    assert_eq!(
        header(put, "Content-Type"),
        Some("application/octet-stream")
    );
    assert_eq!(header(put, "Authorization"), Some("Basic bWU6cHc="));
    let body = app.body(put);
    assert!(body.starts_with(b"SQLite format 3\0"));

    assert!(app.answer(put, 201, b"", None).is_empty());
    assert!(app.shown().contains("Synced: 2 books."), "{}", app.shown());
    // The statistics here are what they were; what went up is kept to
    // compare with next time, for this folder and user.
    assert_eq!(common::stays(&app.storage), before);
    assert_eq!(titles(&app), ["Anna Karenina", "Effi Briest"]);
    let kept = last_uploads(&app);
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(
        app.storage.get(&format!("{DIR}/{}", kept[0])).as_deref(),
        Some(body.as_slice())
    );
}

#[test]
fn a_sync_leaves_no_working_files_of_the_merge() {
    let remote = first_upload();
    let mut app = app(device(&[(1, "Anna Karenina")], 6, 10));
    let get = app.sync_now();
    let put = app.answer(&get[0], 200, &remote, Some("\"v1\""));
    app.answer(&put[0], 201, b"", None);
    let files = app.storage.files();
    assert_eq!(files.len(), 3, "{files:?}");
    assert_eq!(files[..2], ["books.ini", "pages.bin"], "{files:?}");
    assert_eq!(last_uploads(&app).len(), 1, "{files:?}");
}

/// The kernel writes a download's answer to the file asked for, also an
/// error page; with nothing there yet (404) or a refused download, that
/// page is not left in the plugin's folder.
#[test]
fn a_download_that_finds_nothing_leaves_no_file() {
    let mut app = app(device(&[(1, "Anna Karenina")], 6, 10));
    let get = app.sync_now();
    let put = app.answer(&get[0], 404, b"Not Found", None);
    app.answer(&put[0], 201, b"", None);
    assert!(!app.storage.0.exists(&format!("{DIR}/incoming.sqlite3")));
    let get = app.sync_now();
    app.answer(&get[0], 401, b"Unauthorized", None);
    assert!(!app.storage.0.exists(&format!("{DIR}/incoming.sqlite3")));
}

#[test]
fn a_newer_file_there_is_merged_and_uploaded_on_condition() {
    let remote = first_upload();
    // Device B read another book, and one of A's.
    let mut b = app(device(&[(1, "Anna Karenina"), (5, "Faust")], 12, 5));
    let get = b.sync_now();
    // A weak validator is sent as a strong one, as KOReader does.
    let put = b.answer(&get[0], 200, &remote, Some("W/\"v1\""));
    assert_eq!(put.len(), 1);
    assert_eq!(header(&put[0], "If-Match"), Some("\"v1\""));
    assert!(b.answer(&put[0], 204, b"", None).is_empty());
    assert!(b.shown().contains("Synced: 3 books."), "{}", b.shown());
    let mut titles = titles(&b);
    titles.sort();
    assert_eq!(titles, ["Anna Karenina", "Effi Briest", "Faust"]);
    // A's twenty minutes and B's ten: Anna Karenina was read on both
    // devices, at other times.
    assert_eq!(common::stays(&b.storage).len(), 30);
}

/// A file gone from the folder this device uploaded to (deleted there,
/// or a server's mistake) is not replaced with this device's statistics
/// alone at once: the sync says so, and the next one uploads.
#[test]
fn a_file_gone_from_the_folder_is_not_replaced_at_once() {
    let mut app = app(device(&[(1, "Anna Karenina")], 6, 10));
    let get = app.sync_now();
    let put = app.answer(&get[0], 404, b"Not Found", None);
    assert!(app.answer(&put[0], 201, b"", None).is_empty());
    let get = app.sync_now();
    assert!(app.answer(&get[0], 404, b"Not Found", None).is_empty());
    assert!(
        app.shown().contains("is no longer in the folder"),
        "{}",
        app.shown()
    );
    assert_eq!(titles(&app), ["Anna Karenina"]);
    let get = app.sync_now();
    let put = app.answer(&get[0], 404, b"Not Found", None);
    assert_eq!(put.len(), 1);
    assert_eq!(header(&put[0], "If-None-Match"), Some("*"));
}

/// What a download without an ETag lacks is not taken for deleted there:
/// the upload could replace a file changed meanwhile, without telling.
#[test]
fn without_an_etag_nothing_counts_as_deleted_there() {
    let three = [(1, "Anna Karenina"), (2, "Effi Briest"), (3, "Faust")];
    // There, the third book was deleted since this device's upload.
    let mut other = app(device(&three[..2], 6, 10));
    let get = other.sync_now();
    let put = other.answer(&get[0], 404, b"Not Found", None);
    let two = other.body(&put[0]);
    for (etag, books) in [(None, 3), (Some("\"v2\""), 2)] {
        let mut a = app(device(&three, 6, 10));
        let get = a.sync_now();
        let put = a.answer(&get[0], 404, b"Not Found", None);
        assert!(a.answer(&put[0], 201, b"", None).is_empty());
        let get = a.sync_now();
        let put = a.answer(&get[0], 200, &two, etag);
        assert_eq!(put.len(), 1, "{etag:?}");
        assert_eq!(titles(&a).len(), books, "{etag:?}");
    }
}

/// The file in the folder empty (replaced by that of a device that
/// started anew, or all statistics reset in KOReader): nothing here goes
/// and nothing is uploaded, the user is asked. *Keep* uploads them again.
#[test]
fn an_empty_file_there_deletes_nothing_unless_the_user_says_so() {
    let mut fresh = app(device(&[], 6, 10));
    let get = fresh.sync_now();
    let put = fresh.answer(&get[0], 404, b"Not Found", None);
    let empty = fresh.body(&put[0]);
    let mut a = app(device(&[(1, "Anna Karenina"), (2, "Effi Briest")], 6, 10));
    let get = a.sync_now();
    let put = a.answer(&get[0], 404, b"Not Found", None);
    assert!(a.answer(&put[0], 201, b"", None).is_empty());
    let before = common::stays(&a.storage);
    let get = a.sync_now();
    assert!(a.answer(&get[0], 200, &empty, Some("\"v2\"")).is_empty());
    // Asked, the focus on Keep.
    let shown = a.shown();
    // The question wraps over lines as the width lets it.
    assert!(shown.contains("here too?"), "{shown}");
    assert!(shown.contains(">Keep"), "{shown}");
    assert_eq!(common::stays(&a.storage), before);
    a.press("confirm");
    let get = a.requests();
    assert_eq!(get.len(), 1, "{get:?}");
    let put = a.answer(&get[0], 200, &empty, Some("\"v2\""));
    assert_eq!(put.len(), 1);
    assert!(a.answer(&put[0], 204, b"", None).is_empty());
    assert!(a.shown().contains("Synced: 2 books."), "{}", a.shown());
    assert_eq!(titles(&a), ["Anna Karenina", "Effi Briest"]);
    assert_eq!(common::stays(&a.storage), before);
}

/// Most statistics removed in KOReader since this device's upload (the
/// books read last): whether they were deleted there or a backup was
/// restored cannot be told, so the user is asked. *Remove* removes them
/// here too, and is remembered for the file there as it is: a failed
/// upload does not ask again.
#[test]
fn statistics_removed_there_go_here_when_the_user_says_so() {
    let three = [(1, "Anna Karenina"), (2, "Effi Briest"), (3, "Faust")];
    let mut other = app(device(&three[..1], 6, 10));
    let get = other.sync_now();
    let put = other.answer(&get[0], 404, b"Not Found", None);
    let one = other.body(&put[0]);
    let mut a = app(device(&three, 6, 10));
    let get = a.sync_now();
    let put = a.answer(&get[0], 404, b"Not Found", None);
    assert!(a.answer(&put[0], 201, b"", None).is_empty());
    let get = a.sync_now();
    assert!(a.answer(&get[0], 200, &one, Some("\"v2\"")).is_empty());
    // Asked, the focus on Keep.
    let shown = a.shown();
    // The question wraps over lines as the width lets it.
    assert!(shown.contains("here too?"), "{shown}");
    assert!(shown.contains(">Keep"), "{shown}");
    assert_eq!(titles(&a).len(), 3);
    a.press("left");
    assert!(a.shown().contains(">Remove"), "{}", a.shown());
    a.press("confirm");
    let get = a.requests();
    assert_eq!(get.len(), 1, "{get:?}");
    let put = a.answer(&get[0], 200, &one, Some("\"v2\""));
    assert_eq!(put.len(), 1);
    assert_eq!(titles(&a), ["Anna Karenina"]);
    assert!(a.answer(&put[0], 500, b"", None).is_empty());
    let get = a.sync_now();
    let put = a.answer(&get[0], 200, &one, Some("\"v2\""));
    assert_eq!(put.len(), 1, "not asked again");
    assert!(a.answer(&put[0], 204, b"", None).is_empty());
    assert!(a.shown().contains("Synced: 1 book."), "{}", a.shown());
    assert_eq!(common::stays(&a.storage).len(), 10);
}

#[test]
fn someone_faster_makes_it_start_over_three_times_at_most() {
    let remote = first_upload();
    let mut b = app(device(&[(5, "Faust")], 12, 5));
    let before = statistics(&b);
    let mut get = b.sync_now();
    for attempt in 1..=3 {
        assert_eq!(get.len(), 1, "attempt {attempt}");
        assert_eq!(get[0].method, "GET");
        let put = b.answer(&get[0], 200, &remote, Some(&format!("\"v{attempt}\"")));
        assert_eq!(
            header(&put[0], "If-Match"),
            Some(format!("\"v{attempt}\"").as_str())
        );
        // Changed in between.
        get = b.answer(&put[0], 412, b"Precondition Failed", None);
    }
    assert!(get.is_empty());
    assert!(
        b.shown().contains("The server reported an error (412)."),
        "{}",
        b.shown()
    );
    // What was merged stays merged (the same again next time).
    assert_ne!(statistics(&b), before);
    assert_eq!(common::stays(&b.storage).len(), 25);
}

#[test]
fn a_wrong_password_says_so_and_changes_nothing() {
    for status in [401, 403] {
        let mut app = app(device(&[(1, "Anna Karenina")], 6, 10));
        let before = statistics(&app);
        let stays = common::stays(&app.storage);
        let get = app.sync_now();
        assert!(
            app.answer(&get[0], status, b"Unauthorized", None)
                .is_empty()
        );
        assert!(
            app.shown().contains("User name or password is wrong"),
            "{}",
            app.shown()
        );
        assert_eq!(statistics(&app), before);
        // The upload refused: the same.
        let get = app.sync_now();
        let put = app.answer(&get[0], 404, b"", None);
        assert!(app.answer(&put[0], status, b"", None).is_empty());
        assert!(app.shown().contains("User name or password is wrong"));
        assert_eq!(common::stays(&app.storage), stays);
        assert_eq!(titles(&app), ["Anna Karenina"]);
        assert!(last_uploads(&app).is_empty());
    }
}

#[test]
fn what_else_can_fail_is_said_in_words() {
    for (on_put, status, said) in [
        (false, 0, "The server cannot be reached."),
        (false, 500, "The server reported an error (500)."),
        (false, 409, "The folder does not exist on the server."),
        (true, 404, "The folder does not exist on the server."),
        (true, 409, "The folder does not exist on the server."),
        (true, 507, "The server reported an error (507)."),
        (true, 0, "The server cannot be reached."),
    ] {
        let mut app = app(device(&[(1, "Anna Karenina")], 6, 10));
        let get = app.sync_now();
        let last = if on_put {
            let put = app.answer(&get[0], 404, b"", None);
            app.answer(&put[0], status, b"", None)
        } else {
            app.answer(&get[0], status, b"", None)
        };
        assert!(last.is_empty(), "{status}");
        assert!(app.shown().contains(said), "{status}: {}", app.shown());
        // And it can be tried again.
        assert_eq!(app.sync_now().len(), 1, "{status}");
    }
}

#[test]
fn without_a_network_or_a_folder_nothing_is_asked() {
    let mut offline = App::new(X4, device(&[(1, "A")], 6, 10), Some(NOW), FOLDER, false);
    assert!(offline.sync_now().is_empty());
    assert!(
        offline
            .shown()
            .contains("This device has no network connection.")
    );
    let mut no_folder = App::new(X4, device(&[(1, "A")], 6, 10), Some(NOW), "", true);
    assert!(no_folder.sync_now().is_empty());
    assert!(no_folder.shown().contains("Enter the folder first."));
}

#[test]
fn a_file_there_that_is_no_database_leaves_the_statistics_here() {
    let remote = first_upload();
    let broken: [(&str, Vec<u8>); 4] = [
        ("a web page", b"<html><body>Login</body></html>".to_vec()),
        ("empty", Vec::new()),
        ("cut off", remote[..remote.len() / 3].to_vec()),
        ("the header only", remote[..100].to_vec()),
    ];
    for (what, body) in broken {
        let mut b = app(device(&[(5, "Faust")], 12, 5));
        let before = common::stays(&b.storage);
        let get = b.sync_now();
        let put = b.answer(&get[0], 200, &body, Some("\"v1\""));
        // Nothing here lost or changed; of the file there at most what
        // could be read of it.
        let after = common::stays(&b.storage);
        for stay in &before {
            assert!(after.contains(stay), "{what}: {stay:?}");
        }
        if let Some(put) = put.first() {
            b.answer(put, 204, b"", None);
        }
        assert!(
            !b.storage.0.exists(&format!("{DIR}/incoming.sqlite3")),
            "{what}"
        );
    }
}

/// KOReader, finding a file there it cannot read, stops the sync rather
/// than replace the file; so does this device, saying why, or the other
/// devices' statistics in it would be gone for the next device that
/// syncs.
#[test]
fn a_file_there_that_is_no_database_is_not_replaced() {
    let mut b = app(device(&[(5, "Faust")], 12, 5));
    let get = b.sync_now();
    let put = b.answer(
        &get[0],
        200,
        b"<html><body>Login</body></html>",
        Some("\"v1\""),
    );
    assert!(put.is_empty(), "uploaded over a file it could not read");
    assert!(b.shown().contains("cannot be read"), "{}", b.shown());
}

/// The device started again with the files of `app`'s that matter here:
/// the secrets and the old password file.
fn restart(app: &App) -> App {
    let mut storage = MemoryStorage::new();
    for path in [FILE.to_string(), format!("{DIR}/webdav.ini")] {
        if let Some(data) = app.storage.get(&path) {
            storage = storage.with(&path, data);
        }
    }
    App::new(X4, storage, Some(NOW), "", false)
}

/// The old file goes once the password is in the secrets, at the next
/// start: until then it is the only copy (the secrets might not have
/// been saved).
#[test]
fn the_password_of_earlier_versions_moves_to_the_passwords() {
    let v1 = include_str!("../testdata/webdav.ini.v1");
    let storage = MemoryStorage::new().with(&format!("{DIR}/webdav.ini"), v1);
    let app = App::new(X4, storage, Some(NOW), "", false);
    assert_eq!(app.kernel.secrets().get(SECRET), Some("secret 1"));
    assert!(app.storage.0.exists(&format!("{DIR}/webdav.ini")));
    let app = restart(&app);
    assert_eq!(app.kernel.secrets().get(SECRET), Some("secret 1"));
    assert!(!app.storage.0.exists(&format!("{DIR}/webdav.ini")));
}

#[test]
fn a_password_kept_already_is_not_replaced_by_an_old_one() {
    let storage =
        device(&[], 0, 0).with(&format!("{DIR}/webdav.ini"), "[webdav]\npassword = older\n");
    let app = App::new(X4, storage, Some(NOW), "", false);
    assert_eq!(app.kernel.secrets().get(SECRET), Some("pw"));
    assert!(!app.storage.0.exists(&format!("{DIR}/webdav.ini")));
}

#[test]
fn an_old_file_without_a_password_just_goes() {
    for text in [
        &b""[..],
        b"[webdav]\nurl = https://x\n",
        b"\xff\xfe password = x",
    ] {
        let storage = MemoryStorage::new().with(&format!("{DIR}/webdav.ini"), text.to_vec());
        let app = App::new(X4, storage, Some(NOW), "", false);
        assert_eq!(app.kernel.secrets().get(SECRET), None, "{text:?}");
        assert!(!app.storage.0.exists(&format!("{DIR}/webdav.ini")));
    }
    // Passwords are kept as typed, also one of spaces.
    let storage = MemoryStorage::new().with(
        &format!("{DIR}/webdav.ini"),
        "[webdav]\npassword = \"  \"\n",
    );
    let app = restart(&App::new(X4, storage, Some(NOW), "", false));
    assert_eq!(app.kernel.secrets().get(SECRET), Some("  "));
    assert!(!app.storage.0.exists(&format!("{DIR}/webdav.ini")));
}

#[test]
fn the_folder_user_and_password_are_entered_on_the_page() {
    let mut app = App::new(TOUCH, MemoryStorage::new(), Some(NOW), "", true);
    app.open("stats.sync", Params::new());
    let typed = |text: &str| Typed::new(text.to_string(), Params::new()).to_params();
    for (event, text) in [
        ("stats.url-entered", " cloud.example.org/dav/KOReader/ "),
        ("stats.user-entered", "  anna "),
        ("stats.password-entered", " secret "),
    ] {
        app.now += 100;
        let effects = app
            .kernel
            .send_event(Instant(app.now), Id::new_static(event), typed(text));
        app.settle(effects);
    }
    let settings = app.kernel.settings();
    assert_eq!(
        settings.text("stats.webdav-url"),
        "https://cloud.example.org/dav/KOReader"
    );
    assert_eq!(settings.text("stats.webdav-user"), "anna");
    // Passwords are kept as typed: spaces may be part of them.
    assert_eq!(app.kernel.secrets().get(SECRET), Some(" secret "));
    let shown = app.shown();
    assert!(
        shown.contains("Folder (WebDAV), cloud.example.org/dav/KOReader"),
        "{shown}"
    );
    assert!(shown.contains("User name, anna"), "{shown}");
    // An empty password removes it.
    app.now += 100;
    let effects = app.kernel.send_event(
        Instant(app.now),
        Id::new_static("stats.password-entered"),
        typed(""),
    );
    app.settle(effects);
    assert_eq!(app.kernel.secrets().get(SECRET), None);
}

#[test]
fn koreaders_statistics_on_the_device_are_taken_over() {
    let koreader = first_upload();
    let path = "koreader/settings/statistics.sqlite3";
    // Without them, no way to take them over.
    let mut app = App::new(TOUCH, device(&[(5, "Faust")], 12, 5), Some(NOW), "", false);
    app.open("stats.sync", Params::new());
    assert!(!app.shown().contains("Take over from KOReader"));

    // KOReader still running: its newest pages are not in the file yet.
    let storage = device(&[(5, "Faust")], 12, 5)
        .with(path, koreader.clone())
        .with(&format!("{path}-wal"), vec![1u8; 32]);
    let mut app = App::new(TOUCH, storage, Some(NOW), "", false);
    let before = statistics(&app);
    app.open("stats.sync", Params::new());
    app.tap_on("Take over from KOReader");
    assert!(
        app.shown().contains("KOReader seems to be running"),
        "{}",
        app.shown()
    );
    assert_eq!(statistics(&app), before);

    let storage = device(&[(5, "Faust")], 12, 5).with(path, koreader);
    let mut app = App::new(TOUCH, storage, Some(NOW), "", false);
    app.open("stats.sync", Params::new());
    app.tap_on("Take over from KOReader");
    assert!(
        app.shown()
            .contains("Taken over: now 3 books, 25 pages read in all."),
        "{}",
        app.shown()
    );
    // Taking them over again adds nothing.
    app.tap_on("Take over from KOReader");
    assert!(
        app.shown()
            .contains("Taken over: now 3 books, 25 pages read in all."),
        "{}",
        app.shown()
    );
    assert_eq!(common::stays(&app.storage).len(), 25);
}
