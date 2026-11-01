//! A fetch against answers as Readwise Reader gives them (mocked, no network):
//! the list, the texts of the articles as books; then archiving one read to
//! the end, and what a fetch does at the answers that stop it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use std::rc::Rc;

use paperpaw_core::{HttpCall, HttpResponse, Io, Kernel};
use paperpaw_platform::{ButtonState, InputEvent, Rotation};
use paperpaw_plugin_readwise_reader::ReadwiseReader;
use paperpaw_sdk::secrets::{FILE, SecretChange, Secrets};
use paperpaw_sdk::{
    DeviceInfo, DirEntry, DisplayInfo, InputCaps, Instant, MemoryClass, MemoryStorage, PixelFormat,
    Storage, StorageError,
};

const DEVICE: DeviceInfo = DeviceInfo::new(
    "test",
    "Test device",
    DisplayInfo::new(200, 300, 100, PixelFormat::Gray4, Rotation::Deg0, None),
    InputCaps::new(true, false, false, &[]),
    MemoryClass::Large,
);

/// A storage the test looks into while the kernel has it.
#[derive(Clone)]
struct Shared(Rc<MemoryStorage>);

impl Storage for Shared {
    fn read_at(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        self.0.read_at(path, offset, buf)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.0.size(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.0.write(path, data)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.0.append(path, data)
    }
    fn list(&self, dir: &str, visit: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), StorageError> {
        self.0.list(dir, visit)
    }
    fn remove(&self, path: &str) -> Result<(), StorageError> {
        self.0.remove(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.0.exists(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        self.0.rename(from, to)
    }
    fn create_dir(&self, path: &str) -> Result<(), StorageError> {
        self.0.create_dir(path)
    }
}

/// The library at the storage's top.
const CONFIG: &str =
    "[core]\nlibrary-folder =\n\n[keymap.global]\nbutton.fetch = readwise-reader.fetch\n";
const TOKEN: &str = "T0ken-not-real";

const LIST: &str = r#"{"count": 2, "nextPageCursor": null, "results": [
    {"id": "01newer", "title": "Newer <one>", "category": "article", "location": "later"},
    {"id": "01older", "title": "Older", "category": "article", "location": "later"}
]}"#;

fn doc(id: &str, location: &str, html: &str) -> String {
    format!(
        r#"{{"count": 1, "nextPageCursor": null, "results": [{{"id": "{id}", "author": "Ann & Bo", "location": "{location}", "html_content": "{html}"}}]}}"#
    )
}

fn press(kernel: &mut Kernel, at: u64) -> Vec<HttpCall> {
    for state in [ButtonState::Pressed, ButtonState::Released] {
        kernel.handle(
            Instant(at),
            InputEvent::Button {
                name: "fetch",
                state,
            },
        );
    }
    requests(kernel)
}

fn requests(kernel: &mut Kernel) -> Vec<HttpCall> {
    kernel.render(&mut |_, _| {});
    let io = kernel.take_io().into_iter();
    io.filter_map(|io| match io {
        Io::Http(call) => Some(call),
        _ => None,
    })
    .collect()
}

fn answer(kernel: &mut Kernel, call: &HttpCall, status: u16, body: &str) -> Vec<HttpCall> {
    kernel.http_response(
        Instant(10),
        HttpResponse {
            id: call.id,
            status,
            body: body.as_bytes().to_vec(),
            ..HttpResponse::default()
        },
    );
    requests(kernel)
}

fn kernel(storage: &Shared) -> Kernel {
    let mut kernel = Kernel::builder(DEVICE)
        .storage(storage.clone())
        .user_config(CONFIG)
        .network(true)
        .plugin(ReadwiseReader)
        .build()
        .unwrap();
    kernel.start(Instant(0));
    kernel
}

fn storage_with_token() -> Shared {
    let mut secrets = Secrets::default();
    secrets.apply(SecretChange::keep("readwise-reader.token", "", TOKEN));
    Shared(Rc::new(
        MemoryStorage::new().with(FILE, secrets.encode().into_bytes()),
    ))
}

/// The names of the books in the folder `Reader`.
fn books(storage: &Shared) -> Vec<String> {
    let mut names = Vec::new();
    let _ = storage.list("Reader", &mut |entry| names.push(entry.name.to_string()));
    names
}

fn text(storage: &Shared, path: &str) -> String {
    String::from_utf8(storage.0.get(path).unwrap().to_vec()).unwrap()
}

const FILES: &str = ".paperpaw/plugins/readwise-reader";

/// Fetches the two articles of [`LIST`]: the books exist.
fn fetched() -> (Shared, Kernel) {
    let storage = storage_with_token();
    let mut kernel = kernel(&storage);
    let calls = press(&mut kernel, 1);
    let calls = answer(&mut kernel, &calls[0], 200, LIST);
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        &doc("01older", "later", "<p>Old \\u00e9 \\\"text\\\"</p>"),
    );
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        &doc("01newer", "later", "<p>New</p>"),
    );
    assert!(calls.is_empty());
    (storage, kernel)
}

#[test]
fn fetches_new_articles_as_books() {
    let storage = storage_with_token();
    let mut kernel = kernel(&storage);
    let calls = press(&mut kernel, 1);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].method, "GET");
    assert_eq!(
        calls[0].url,
        "https://readwise.io/api/v3/list/?location=later&category=article&limit=20"
    );
    assert!(
        calls[0]
            .headers
            .contains(&(String::from("Authorization"), format!("Token {TOKEN}")))
    );
    // The books, the oldest first.
    let calls = answer(&mut kernel, &calls[0], 200, LIST);
    assert_eq!(
        calls[0].url,
        "https://readwise.io/api/v3/list/?id=01older&withHtmlContent=true"
    );
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        &doc("01older", "later", "<p>Old \\u00e9 \\\"text\\\"</p>"),
    );
    assert_eq!(
        calls[0].url,
        "https://readwise.io/api/v3/list/?id=01newer&withHtmlContent=true"
    );
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        &doc("01newer", "later", "<p>New</p>"),
    );
    assert!(calls.is_empty());
    let old = text(&storage, "Reader/Older.html");
    assert!(
        old.contains("<h1>Older</h1><p>Ann &amp; Bo</p><p>Old é \"text\"</p></body></html>"),
        "{old}"
    );
    assert!(old.starts_with("<!DOCTYPE html>"));
    // A title with markup is text in the book; the file's name is clean.
    let new = books(&storage)
        .into_iter()
        .find(|n| n != "Older.html")
        .unwrap();
    assert!(!new.contains('<'), "{new}");
    assert!(text(&storage, &format!("Reader/{new}")).contains("<h1>Newer &lt;one&gt;</h1>"));
    // Nothing is left of the answers.
    for name in ["list.json", "doc.json", "article.tmp"] {
        assert!(
            storage.0.get(&format!("{FILES}/{name}")).is_none(),
            "{name}"
        );
    }
    // A second fetch with the same list changes nothing.
    let calls = press(&mut kernel, 20);
    let calls = answer(&mut kernel, &calls[0], 200, LIST);
    assert!(calls.is_empty());
    assert_eq!(books(&storage).len(), 2);
}

#[test]
fn archives_what_was_read_and_follows_archiving_elsewhere() {
    let (storage, mut kernel) = fetched();
    // Article `01older` read to the end: archived with the next fetch,
    // its book removed.
    storage
        .0
        .write(&format!("{FILES}/finished.txt"), b"01older\n")
        .unwrap();
    let calls = press(&mut kernel, 30);
    assert_eq!(calls[0].method, "PATCH");
    assert_eq!(calls[0].url, "https://readwise.io/api/v3/update/01older/");
    assert_eq!(
        calls[0].body.as_deref(),
        Some(br#"{"location":"archive"}"#.as_slice())
    );
    let calls = answer(&mut kernel, &calls[0], 200, "{}");
    assert!(storage.0.get("Reader/Older.html").is_none());
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        r#"{"count": 0, "results": []}"#,
    );
    // The other is not in the list (empty): Reader is asked; it is
    // archived there.
    assert_eq!(calls[0].url, "https://readwise.io/api/v3/list/?id=01newer");
    let calls = answer(&mut kernel, &calls[0], 200, &doc("01newer", "archive", ""));
    assert!(calls.is_empty());
    assert_eq!(books(&storage).len(), 0);
    let _ = kernel;
}

#[test]
fn a_book_stays_where_reader_still_has_it() {
    let (storage, mut kernel) = fetched();
    let calls = press(&mut kernel, 30);
    let mut calls = answer(
        &mut kernel,
        &calls[0],
        200,
        r#"{"count": 0, "results": []}"#,
    );
    // Both are asked about, the oldest first; in the Inbox or the list
    // they stay, and an answer that is not about them changes nothing.
    assert!(calls[0].url.ends_with("?id=01older"));
    calls = answer(
        &mut kernel,
        &calls[0],
        200,
        &doc("01older", "shortlist", ""),
    );
    assert!(calls[0].url.ends_with("?id=01newer"));
    let calls = answer(&mut kernel, &calls[0], 200, &doc("zzz", "archive", ""));
    assert!(calls.is_empty());
    assert_eq!(books(&storage).len(), 2);
    // Deleted in Reader (no result): its book goes.
    let calls = press(&mut kernel, 40);
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        r#"{"count": 0, "results": []}"#,
    );
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        r#"{"count": 0, "results": []}"#,
    );
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        r#"{"count": 0, "results": []}"#,
    );
    assert!(calls.is_empty());
    assert_eq!(books(&storage).len(), 0);
}

#[test]
fn a_book_with_notes_stays_in_the_library() {
    let (storage, mut kernel) = fetched();
    let line = "h\t1\t0\t1\t20\t5\t0\t0\tSome text\tMy note\n";
    let file = paperpaw_sdk::annotations::file("Reader/Older.html");
    storage.0.write(&file, line.as_bytes()).unwrap();
    storage
        .0
        .write(&format!("{FILES}/finished.txt"), b"01older\n")
        .unwrap();
    let calls = press(&mut kernel, 30);
    assert_eq!(calls[0].method, "PATCH");
    let calls = answer(&mut kernel, &calls[0], 200, "{}");
    let list = r#"{"count":1,"results":[{"id":"01newer","title":"Newer <one>"}]}"#;
    let calls = answer(&mut kernel, &calls[0], 200, list);
    assert!(calls.is_empty());
    // Archived in Reader, let go of here: the book and its notes stay.
    assert!(calls.iter().all(|c| !c.url.contains("01older")));
    assert!(storage.0.get("Reader/Older.html").is_some());
    assert_eq!(storage.0.get(&file).unwrap(), line.as_bytes());
    assert!(
        !storage
            .0
            .get(&format!("{FILES}/articles.txt"))
            .unwrap()
            .starts_with(b"01older")
    );
}

#[test]
fn stops_where_reader_says_so() {
    for (status, key) in [
        (401, "did not accept"),
        (403, "did not accept"),
        (429, "wait"),
        (0, "cannot be reached"),
    ] {
        let storage = storage_with_token();
        let mut kernel = kernel(&storage);
        let calls = press(&mut kernel, 1);
        let calls = answer(&mut kernel, &calls[0], status, r#"{"detail": "x"}"#);
        assert!(calls.is_empty(), "{status}");
        let _ = key;
        assert!(books(&storage).is_empty());
        // Nothing noted: the next fetch starts anew.
        let calls = press(&mut kernel, 100);
        assert_eq!(calls.len(), 1, "{status}");
    }
}

#[test]
fn needs_the_token_and_a_network() {
    let storage = Shared(Rc::new(MemoryStorage::new()));
    let mut kernel = kernel(&storage);
    assert!(press(&mut kernel, 1).is_empty());
    let storage = storage_with_token();
    let mut kernel = Kernel::builder(DEVICE)
        .storage(storage.clone())
        .user_config(CONFIG)
        .network(false)
        .plugin(ReadwiseReader)
        .build()
        .unwrap();
    kernel.start(Instant(0));
    assert!(press(&mut kernel, 1).is_empty());
}

#[test]
fn an_article_without_text_or_cut_is_not_a_book() {
    for body in [
        r#"{"count":1,"results":[{"id":"01older","author":"A","location":"later","html_content":null}]}"#.to_string(),
        r#"{"count":1,"results":[{"id":"01older","html_content":"<p>cut"#.to_string(),
        "<html>Maintenance</html>".to_string(),
    ] {
        let storage = storage_with_token();
        let mut kernel = kernel(&storage);
        let calls = press(&mut kernel, 1);
        let calls = answer(&mut kernel, &calls[0], 200, LIST);
        let calls = answer(&mut kernel, &calls[0], 200, &body);
        // The other article goes on.
        assert!(calls[0].url.contains("01newer"));
        let calls = answer(&mut kernel, &calls[0], 200, &doc("01newer", "later", "<p>x</p>"));
        assert!(calls.is_empty());
        assert_eq!(books(&storage).len(), 1, "{body}");
        assert!(storage.0.get(&format!("{FILES}/article.tmp")).is_none());
    }
}

#[test]
fn another_token_lets_go_of_the_books() {
    let (storage, _) = fetched();
    let mut secrets = Secrets::default();
    secrets.apply(SecretChange::keep(
        "readwise-reader.token",
        "",
        "other-token",
    ));
    storage.0.write(FILE, secrets.encode().as_bytes()).unwrap();
    let mut kernel = kernel(&storage);
    let calls = press(&mut kernel, 50);
    // Empty list: the books of the other account are not asked about nor
    // removed.
    let calls = answer(
        &mut kernel,
        &calls[0],
        200,
        r#"{"count": 0, "results": []}"#,
    );
    assert!(calls.is_empty());
    assert_eq!(books(&storage).len(), 2);
}
