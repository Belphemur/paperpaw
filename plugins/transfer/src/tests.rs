use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use paperpaw_fat::fatfs::{
    self, FileSystem, FormatVolumeOptions, FsOptions, Seek, SeekFrom, Write,
};
use paperpaw_fat::{BLOCK, BlockDisk, Blocks, DiskError, FatStorage};
use paperpaw_i18n::{Catalog, check};
use paperpaw_sdk::{DirEntry, Instant, MemoryStorage, Storage, StorageError, files, read_file};

use crate::web::{self, Done, Refused, Request};
use paperpaw_sdk::served::{Guard, LOCKED_FOR, WRONG_KEYS};

#[test]
fn translations_are_complete() {
    let en = Catalog::parse(include_str!("../i18n/en.ini")).unwrap();
    let de = Catalog::parse(include_str!("../i18n/de.ini")).unwrap();
    let issues = check::compare(&en, &de);
    assert!(issues.is_empty(), "{issues:?}");
}

#[test]
fn the_page_takes_its_texts() {
    assert!(crate::PAGE.contains("/*STRINGS*/{}"));
}

/// The key of the tests.
const KEY: &str = "abc";

/// Answers one request at `at`, with `guard`.
fn ask_at(
    storage: &dyn Storage,
    guard: &mut Guard,
    at: u64,
    method: &str,
    target: &str,
    body_file: Option<&str>,
) -> (u16, Vec<u8>, Option<String>, Option<Done>) {
    let request = Request {
        number: 1,
        method,
        target,
        body_file,
        at: Instant(at),
    };
    let (response, done) = web::answer(storage, guard, "<page>", request);
    let file = response.body_file().map(String::from);
    (response.status(), response.body().to_vec(), file, done)
}

/// Answers one request with the key [`KEY`], none wrong before.
fn ask(
    storage: &dyn Storage,
    method: &str,
    target: &str,
    body_file: Option<&str>,
) -> (u16, Vec<u8>, Option<String>, Option<Done>) {
    let mut guard = Guard::new(KEY.to_string());
    ask_at(storage, &mut guard, 0, method, target, body_file)
}

#[test]
fn the_key_is_needed() {
    let storage = MemoryStorage::new();
    assert_eq!(ask(&storage, "GET", "/", None).0, 403);
    assert_eq!(ask(&storage, "GET", "/?k=abd", None).0, 403);
    assert_eq!(ask(&storage, "GET", "/?k=ab", None).0, 403);
    assert_eq!(ask(&storage, "GET", "/?k=abcd", None).0, 403);
    assert_eq!(ask(&storage, "GET", "/?k=", None).0, 403);
    let (status, body, ..) = ask(&storage, "GET", "/?k=abc", None);
    assert_eq!((status, body.as_slice()), (200, &b"<page>"[..]));
}

#[test]
fn wrong_keys_lock_for_a_while() {
    let storage = MemoryStorage::new();
    let mut guard = Guard::new(KEY.to_string());
    let mut ask = |at, target| ask_at(&storage, &mut guard, at, "GET", target, None).0;
    // Requests without a key guess nothing: not counted.
    for _ in 0..3 * WRONG_KEYS {
        assert_eq!(ask(10, "/apple-touch-icon.png"), 403);
    }
    assert_eq!(ask(10, "/?k=abc"), 200);
    for n in 0..WRONG_KEYS {
        assert_eq!(ask(20 + u64::from(n), "/?k=guess"), 403);
    }
    // Then nothing is answered, not even the right key.
    assert_eq!(ask(30, "/?k=abc"), 429);
    assert_eq!(ask(30, "/?k=guess"), 429);
    assert_eq!(ask(24 + LOCKED_FOR - 1, "/api/list?k=abc"), 429);
    // After a while, again; a new round of wrong keys locks again.
    assert_eq!(ask(24 + LOCKED_FOR, "/?k=abc"), 200);
    let later = 24 + LOCKED_FOR + 1;
    for _ in 0..WRONG_KEYS {
        assert_eq!(ask(later, "/?k=guess"), 403);
    }
    assert_eq!(ask(later, "/?k=abc"), 429);
    // The phone's page is guarded alike.
    use crate::phone::{Request, answer};
    let mut guard = Guard::new(KEY.to_string());
    let storage = MemoryStorage::new().with("body", b"text".to_vec());
    let mut send = |key: &str| {
        let request = Request {
            method: "POST",
            target: &format!("/send?k={key}"),
            body_file: Some("body"),
            at: Instant(5),
        };
        let (response, text) = answer(7, &mut guard, "<page>", &storage, &request);
        (response.status(), text)
    };
    for _ in 0..WRONG_KEYS {
        assert_eq!(send("guess"), (403, None));
    }
    assert_eq!(send(KEY), (429, None));
}

#[test]
fn files_are_listed_and_fetched() {
    let storage = MemoryStorage::new()
        .with("Books/a b.epub", b"book".to_vec())
        .with(".paperpaw/x", b"own".to_vec());
    let (status, body, ..) = ask(&storage, "GET", "/api/list?k=abc", None);
    assert_eq!(status, 200);
    assert_eq!(
        String::from_utf8(body).unwrap(),
        r#"{"entries":[{"name":"Books","dir":true,"size":0}]}"#
    );
    let (_, body, ..) = ask(&storage, "GET", "/api/list?k=abc&path=Books", None);
    assert_eq!(
        String::from_utf8(body).unwrap(),
        r#"{"entries":[{"name":"a b.epub","dir":false,"size":4}]}"#
    );
    let (status, _, from, done) = ask(
        &storage,
        "GET",
        "/api/file?k=abc&path=Books%2Fa%20b.epub",
        None,
    );
    assert_eq!(status, 200);
    assert_eq!(from.as_deref(), Some("Books/a b.epub"));
    assert_eq!(done, Some(Done::Sent(String::from("a b.epub"))));
    // PaperPaw's own data stays out of reach.
    let hidden = "/api/file?k=abc&path=.paperpaw/x";
    assert_eq!(ask(&storage, "GET", hidden, None).0, 400);
    let around = "/api/list?k=abc&path=Books/../.paperpaw";
    assert_eq!(ask(&storage, "GET", around, None).0, 400);
    for missing in ["nope", "Books/nope", "books/a b.epub", "Books/a b.epub/x"] {
        let target = format!("/api/file?k=abc&path={}", encoded(missing));
        assert_eq!(ask(&storage, "GET", &target, None).0, 404, "{missing}");
    }
    // A folder is no file, a file no folder.
    assert_eq!(
        ask(&storage, "GET", "/api/file?k=abc&path=Books", None).0,
        404
    );
    let file = "/api/list?k=abc&path=Books/a%20b.epub";
    assert_eq!(ask(&storage, "GET", file, None).0, 404);
}

#[test]
fn big_folders_are_listed_in_parts() {
    // 1,000 books with long names: about 130 KB as one answer.
    let storage = MemoryStorage::new();
    for i in 0..1000 {
        let name = format!("Books/{i:04} {}.epub", "A rather long title".repeat(5));
        storage.write(&name, b"x").unwrap();
    }
    storage.write("Books/.hidden", b"x").unwrap();
    let mut names = Vec::new();
    let mut from = None;
    let mut parts = 0;
    loop {
        let target = match from {
            Some(from) => format!("/api/list?k=abc&path=Books&from={from}"),
            None => String::from("/api/list?k=abc&path=Books"),
        };
        let (status, body, ..) = ask(&storage, "GET", &target, None);
        assert_eq!(status, 200);
        assert!(body.len() <= web::LIST_PART + 256, "{}", body.len());
        let body = String::from_utf8(body).unwrap();
        names.extend(
            body.split("\"name\":\"")
                .skip(1)
                .map(|rest| rest.split('"').next().unwrap().to_string()),
        );
        parts += 1;
        from = body
            .rsplit_once(",\"next\":")
            .map(|(_, next)| next.trim_end_matches('}').parse::<usize>().unwrap());
        if from.is_none() {
            break;
        }
    }
    assert!(parts > 10, "{parts}");
    assert_eq!(names.len(), 1000);
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 1000, "an entry listed twice");
    // Past the end: nothing more.
    let (_, body, ..) = ask(
        &storage,
        "GET",
        "/api/list?k=abc&path=Books&from=5000",
        None,
    );
    assert_eq!(body, br#"{"entries":[]}"#);
}

#[test]
fn the_settings_file_is_read_only() {
    for name in ["paperpaw.ini", "PaperPaw.INI"] {
        let storage = MemoryStorage::new().with(
            name,
            b"[core]
"
            .to_vec(),
        );
        // Fetched, e.g. to keep a copy.
        let fetch = format!("/api/file?k=abc&path={name}");
        assert_eq!(ask(&storage, "GET", &fetch, None).0, 200);
        // Neither deleted nor put back changed.
        let delete = format!("/api/delete?k=abc&path={name}");
        assert_eq!(ask(&storage, "POST", &delete, None).0, 403, "{name}");
        assert!(storage.exists(name));
        storage.remove(name).unwrap();
        storage
            .write(
                "upload",
                b"[kosync]
server = x
",
            )
            .unwrap();
        let put = format!("/api/file?k=abc&path={name}");
        let (status, .., done) = ask(&storage, "PUT", &put, Some("upload"));
        assert_eq!(status, 403, "{name}");
        assert_eq!(done, Some(Done::NotReceived(String::from(name))));
        assert!(!storage.exists(name));
        // Nor a folder in its place.
        let mkdir = format!("/api/mkdir?k=abc&path={name}");
        assert_eq!(ask(&storage, "POST", &mkdir, None).0, 403, "{name}");
        // Elsewhere, a file of that name is just a file.
        storage.write("upload", b"x").unwrap();
        let put = format!("/api/file?k=abc&path=Books/{name}");
        storage.write("Books/a.epub", b"x").unwrap();
        assert_eq!(ask(&storage, "PUT", &put, Some("upload")).0, 200);
    }
}

#[test]
fn files_come_and_go() {
    let storage = MemoryStorage::new().with("Books/a.epub", b"old".to_vec());
    storage.write(".paperpaw/tmp/request-1", b"new").unwrap();
    let (status, .., done) = ask(
        &storage,
        "PUT",
        "/api/file?k=abc&path=Books/a.epub",
        Some(".paperpaw/tmp/request-1"),
    );
    assert_eq!(status, 200);
    // What is there is kept.
    assert_eq!(done, Some(Done::Received(String::from("Books/a (2).epub"))));
    assert_eq!(read_file(&storage, "Books/a (2).epub", 16).unwrap(), b"new");
    assert_eq!(read_file(&storage, "Books/a.epub", 16).unwrap(), b"old");
    assert!(!storage.exists(".paperpaw/tmp/request-1"));
    // Not into a folder that is not there.
    let nowhere = "/api/file?k=abc&path=No/a.epub";
    assert_eq!(ask(&storage, "PUT", nowhere, None).0, 400);
    let (status, .., done) = ask(&storage, "POST", "/api/mkdir?k=abc&path=Books/Neu", None);
    assert_eq!(
        (status, done),
        (200, Some(Done::Folder(String::from("Neu"))))
    );
    let delete = "/api/delete?k=abc&path=Books/a.epub";
    let (status, .., done) = ask(&storage, "POST", delete, None);
    assert_eq!(
        (status, done),
        (200, Some(Done::Deleted(String::from("Books/a.epub"))))
    );
    assert!(!storage.exists("Books/a.epub"));
    // Not everything at once.
    assert_eq!(ask(&storage, "POST", "/api/delete?k=abc", None).0, 404);
    assert_eq!(
        ask(&storage, "POST", "/api/delete?k=abc&path=/", None).0,
        404
    );
}

#[test]
fn paths_are_found_as_listed() {
    let storage = MemoryStorage::new().with("Books/x.epub", b"x".to_vec());
    let found = |path, new| web::find(&storage, path, new);
    assert_eq!(found("/Books/x.epub/", false).unwrap().path, "Books/x.epub");
    assert_eq!(found("", false).unwrap().is_dir, Some(true));
    assert_eq!(found("Books/new.epub", true).unwrap().is_dir, None);
    assert_eq!(found("Books/new.epub", false), Err(Refused::NotFound));
    assert_eq!(found("New/new.epub", true), Err(Refused::NotFound));
    assert_eq!(found("Books/x.epub/y", true), Err(Refused::NotFound));
    for bad in [
        "a//b", "../x", ".hidden", "a/.b", "C:/x", "a\\b", " a", "a.", "a ",
    ] {
        assert_eq!(found(bad, true), Err(Refused::Bad), "{bad}");
    }
}

#[test]
fn text_typed_on_a_phone_comes_back() {
    use crate::phone::{MAX_TEXT, Request, answer};
    let page = "<html>page</html>";
    let storage = MemoryStorage::new()
        .with("body", "Schlüssel:fx".as_bytes().to_vec())
        .with("long", vec![b'x'; MAX_TEXT + 1]);
    let mut guard = Guard::new(KEY.to_string());
    let mut ask = |method, target, body_file: Option<&str>| {
        let request = Request {
            method,
            target,
            body_file,
            at: Instant(0),
        };
        let (response, text) = answer(7, &mut guard, page, &storage, &request);
        (response.status(), response.body().to_vec(), text)
    };
    // Without the key: nothing.
    assert_eq!(ask("GET", "/", None).0, 403);
    assert_eq!(ask("POST", "/send?k=wrong", Some("body")).2, None);
    // The page, then the text.
    let (status, body, text) = ask("GET", "/?k=abc", None);
    assert_eq!(
        (status, body.as_slice(), text),
        (200, page.as_bytes(), None)
    );
    // Too long: said so, and nothing goes to the keyboard (which would
    // take an empty text and close).
    let (status, _, text) = ask("POST", "/send?k=abc", Some("long"));
    assert_eq!((status, text), (413, None));
    let (status, _, text) = ask("POST", "/send?k=abc", Some("gone"));
    assert_eq!((status, text), (500, None));
    let (status, _, text) = ask("POST", "/send?k=abc", Some("body"));
    assert_eq!((status, text.as_deref()), (200, Some("Schlüssel:fx")));
    // An empty field sends no body: an empty text.
    let (status, _, text) = ask("POST", "/send?k=abc", None);
    assert_eq!((status, text.as_deref()), (200, Some("")));
    assert_eq!(ask("GET", "/other?k=abc", None).0, 404);
}

#[test]
fn the_phone_page_takes_its_texts() {
    assert!(include_str!("../data/phone.html").contains("/*STRINGS*/{}"));
}

// Hostile paths: requests that try to get at PaperPaw's own data
// (passwords, trusted firmware keys) by every spelling a storage might
// take for it, on storages that take none (memory), all (as Windows or
// a FAT driver that matches short names) and PaperPaw's FAT card.

/// `text` with every byte but letters and digits as `%XX`, as a browser
/// may send it.
fn encoded(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// A storage over another that takes names as a lax file system does:
/// without regard to case, without dots and spaces at the end (as
/// Windows), and by short names of the form `PAPERP~1` (as NTFS and FAT
/// keep beside long names).
struct Lax(MemoryStorage);

impl Lax {
    /// The short name a lax system keeps for `name`.
    fn short(name: &str) -> String {
        let stem: String = name
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(6)
            .collect();
        format!("{}~1", stem.to_ascii_uppercase())
    }

    /// `path` as the storage below has it.
    fn real(&self, path: &str) -> String {
        let mut real = String::new();
        for part in path.split('/') {
            let loose = |name: &str| name.trim_end_matches(['.', ' ']).to_lowercase();
            let mut found = None;
            let _ = self.0.list(&real, &mut |entry| {
                if found.is_none()
                    && (loose(entry.name) == loose(part)
                        || Self::short(entry.name) == part.to_ascii_uppercase())
                {
                    found = Some(entry.name.to_string());
                }
            });
            let part = found.unwrap_or_else(|| part.trim_end_matches(['.', ' ']).to_string());
            real = files::join(&real, &part);
        }
        real
    }
}

impl Storage for Lax {
    fn read_at(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        self.0.read_at(&self.real(path), offset, buf)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.0.size(&self.real(path))
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.0.write(&self.real(path), data)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.0.append(&self.real(path), data)
    }
    fn list(&self, dir: &str, visit: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), StorageError> {
        let dir = if dir.is_empty() {
            String::new()
        } else {
            self.real(dir)
        };
        self.0.list(&dir, visit)
    }
    fn remove(&self, path: &str) -> Result<(), StorageError> {
        self.0.remove(&self.real(path))
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        self.0.rename(&self.real(from), &self.real(to))
    }
    fn create_dir(&self, path: &str) -> Result<(), StorageError> {
        self.0.create_dir(&self.real(path))
    }
    fn remove_dir_all(&self, path: &str) -> Result<(), StorageError> {
        self.0.remove_dir_all(&self.real(path))
    }
}

/// A card of blocks in memory.
struct Card(Vec<u8>);

impl Blocks for Card {
    fn read_block(&mut self, index: u32, block: &mut [u8; BLOCK]) -> Result<(), DiskError> {
        let at = index as usize * BLOCK;
        block.copy_from_slice(self.0.get(at..at + BLOCK).ok_or(DiskError)?);
        Ok(())
    }
    fn write_block(&mut self, index: u32, block: &[u8; BLOCK]) -> Result<(), DiskError> {
        let at = index as usize * BLOCK;
        self.0
            .get_mut(at..at + BLOCK)
            .ok_or(DiskError)?
            .copy_from_slice(block);
        Ok(())
    }
    fn blocks(&mut self) -> Result<u32, DiskError> {
        Ok((self.0.len() / BLOCK) as u32)
    }
}

/// A FAT card of 4 MB, formatted, as PaperPaw reads the X4's.
fn fat() -> FatStorage<BlockDisk<Card>> {
    let mut disk = BlockDisk::new(Card(vec![0; 4 << 20])).unwrap();
    fatfs::format_volume(&mut disk, FormatVolumeOptions::new()).unwrap();
    disk.flush().unwrap();
    disk.seek(SeekFrom::Start(0)).unwrap();
    let fs = FileSystem::new(disk, FsOptions::new()).unwrap();
    FatStorage::new(Box::leak(Box::new(fs)))
}

/// Every file below `dir`, with its content.
fn files_below(storage: &dyn Storage, dir: &str) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut entries = Vec::new();
    let _ = storage.list(dir, &mut |e| entries.push((e.name.to_string(), e.is_dir)));
    for (name, is_dir) in entries {
        let path = files::join(dir, &name);
        if is_dir {
            out.extend(files_below(storage, &path));
        } else {
            out.push((path.clone(), read_file(storage, &path, 1024).unwrap()));
        }
    }
    out.sort();
    out
}

/// Hostile paths to PaperPaw's own data, or what is in it.
const HOSTILE: &[&str] = &[
    ".paperpaw",
    ".paperpaw/secrets.txt",
    ".paperpaw/firmware-keys.txt",
    ".PaperPaw/secrets.txt",
    ".PAPERPAW/firmware-keys.txt",
    "PAPERP~1",
    "PAPERP~1/secrets.txt",
    "paperp~1/secrets.txt",
    "PAPERP~1/firmware-keys.txt",
    "PAPERP~1/plugins",
    "PAPERP~1/plugins/wifi/networks.txt",
    ".paperpaw./secrets.txt",
    ".paperpaw /secrets.txt",
    ".paperpaw../secrets.txt",
    "PAPERP~1./secrets.txt",
    "PAPERP~1 /secrets.txt",
    "/.paperpaw/secrets.txt",
    "//PAPERP~1//secrets.txt",
    "./.paperpaw/secrets.txt",
    "../.paperpaw/secrets.txt",
    "Books/../.paperpaw/secrets.txt",
    "Books/../PAPERP~1/secrets.txt",
    "Books/..",
    "..",
    ".",
    "\\.paperpaw\\secrets.txt",
    "Books\\..\\PAPERP~1\\secrets.txt",
    "C:/.paperpaw/secrets.txt",
    "PAPERP~1:secrets.txt",
    ".paperpaw\0/secrets.txt",
    "PAPERP~1\0",
    "PAPERP~1\n/secrets.txt",
    "PAPERP~1/secrets.txt\t",
    " PAPERP~1/secrets.txt",
    "Books/a.epub/../../PAPERP~1",
];

/// Runs every request on every hostile path against `storage`, with
/// PaperPaw's data and a book on it: nothing of the data is read, and it
/// is all as it was after.
fn hostile_paths_reach_nothing_on(storage: &dyn Storage) {
    storage.write(".paperpaw/secrets.txt", b"password").unwrap();
    storage
        .write(".paperpaw/firmware-keys.txt", b"own key")
        .unwrap();
    storage
        .write(".paperpaw/plugins/wifi/networks.txt", b"home")
        .unwrap();
    storage.write("Books/a.epub", b"a book").unwrap();
    let own = files_below(storage, ".paperpaw");
    assert_eq!(own.len(), 3);
    let mut guard = Guard::new(KEY.to_string());
    for (n, hostile) in HOSTILE.iter().enumerate() {
        let path = encoded(hostile);
        let mut ask = |method, endpoint, body: Option<&str>| {
            let target = format!("{endpoint}?k={KEY}&path={path}");
            ask_at(storage, &mut guard, 0, method, &target, body)
        };
        // Nothing read: not listed, not fetched.
        let (status, body, ..) = ask("GET", "/api/list", None);
        assert!(matches!(status, 400 | 404), "list {hostile:?}: {status}");
        assert!(!String::from_utf8_lossy(&body).contains("secrets"));
        let (status, _, from, _) = ask("GET", "/api/file", None);
        assert!(
            matches!(status, 400 | 404),
            "file {hostile:?}: {status} {from:?}"
        );
        // Nothing changed: not uploaded, made or deleted. (Where the
        // storage takes no other spelling, some are names like others:
        // `PAPERP~1` is made at the top, and deleted.)
        let body = format!(".paperpaw/tmp/request-{n}");
        storage.write(&body, b"bad").unwrap();
        let _ = ask("PUT", "/api/file", Some(&body));
        let _ = storage.remove(&body);
        let _ = ask("POST", "/api/mkdir", None);
        let _ = ask("POST", "/api/delete", None);
        let now: Vec<_> = files_below(storage, ".paperpaw")
            .into_iter()
            .filter(|(path, _)| !path.starts_with(".paperpaw/tmp/"))
            .collect();
        assert_eq!(now, own, "after {hostile:?}");
        let mut folders = Vec::new();
        storage
            .list(".paperpaw", &mut |e| {
                if e.is_dir && e.name != "tmp" {
                    folders.push(e.name.to_string());
                }
            })
            .unwrap();
        assert_eq!(folders, ["plugins"], "after {hostile:?}");
    }
    // What a browser may reach still works.
    let list = format!("/api/list?k={KEY}&path=Books");
    let (status, body, ..) = ask_at(storage, &mut guard, 0, "GET", &list, None);
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&body).contains("a.epub"));
    let file = format!("/api/file?k={KEY}&path=Books/a.epub");
    let (status, _, from, _) = ask_at(storage, &mut guard, 0, "GET", &file, None);
    assert_eq!((status, from.as_deref()), (200, Some("Books/a.epub")));
}

#[test]
fn hostile_paths_reach_nothing_in_memory() {
    hostile_paths_reach_nothing_on(&MemoryStorage::new());
}

#[test]
fn hostile_paths_reach_nothing_on_a_lax_storage() {
    let lax = Lax(MemoryStorage::new());
    // It is lax indeed: the data is there by other spellings.
    lax.write(".paperpaw/secrets.txt", b"password").unwrap();
    assert_eq!(lax.size("PAPERP~1/secrets.txt"), Ok(8));
    assert_eq!(lax.size(".PaperPaw./SECRETS.TXT "), Ok(8));
    hostile_paths_reach_nothing_on(&lax);
}

#[test]
fn hostile_paths_reach_nothing_on_a_fat_card() {
    hostile_paths_reach_nothing_on(&fat());
}

/// Moving and renaming on a FAT card: a rename, never over what is there
/// (`name (2)`), never a folder into itself, never PaperPaw's settings.
#[test]
fn files_and_folders_move_and_are_renamed() {
    let storage = fat();
    for dir in ["Books", "Books/Sub", "Downloads"] {
        storage.create_dir(dir).unwrap();
    }
    storage.write("Books/a.epub", b"there").unwrap();
    storage.write("Downloads/a.epub", b"new").unwrap();
    storage.write("paperpaw.ini", b"[core]\n").unwrap();
    let post = |target: &str| ask(&storage, "POST", target, None);
    // Into a folder with a book of that name: beside it.
    let (status, .., done) = post("/api/move?k=abc&path=Downloads/a.epub&to=Books");
    assert_eq!(status, 200);
    let moved = Done::Moved(
        String::from("Downloads/a.epub"),
        String::from("Books/a (2).epub"),
    );
    assert_eq!(done, Some(moved));
    assert_eq!(read_file(&storage, "Books/a (2).epub", 16).unwrap(), b"new");
    assert_eq!(read_file(&storage, "Books/a.epub", 16).unwrap(), b"there");
    assert!(!storage.exists("Downloads/a.epub"));
    // Renamed where it is.
    let (status, .., done) = post("/api/move?k=abc&path=Books/a.epub&to=Books&name=c.epub");
    assert_eq!(status, 200);
    let renamed = Done::Moved(String::from("Books/a.epub"), String::from("Books/c.epub"));
    assert_eq!(done, Some(renamed));
    // A folder with what is in it.
    let (status, .., done) = post("/api/move?k=abc&path=Books/Sub&to=Downloads");
    assert_eq!((status, done.is_some()), (200, true));
    assert!(files::is_dir(&storage, "Downloads/Sub"));
    // Where it is, as it is: nothing done.
    let (status, .., done) = post("/api/move?k=abc&path=Books/c.epub&to=Books");
    assert_eq!((status, done), (200, None));
    // Never into itself, nor PaperPaw's settings, nor into a file.
    assert_eq!(
        post("/api/move?k=abc&path=Downloads&to=Downloads/Sub").0,
        400
    );
    assert_eq!(post("/api/move?k=abc&path=paperpaw.ini&to=Books").0, 403);
    assert_eq!(
        post("/api/move?k=abc&path=Books/c.epub&to=Books/a (2).epub").0,
        404
    );
    assert_eq!(
        post("/api/move?k=abc&path=Books/c.epub&to=Books&name=../x").0,
        400
    );
}
