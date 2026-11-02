//! What a notes vault adds to Markdown: `[[wikilinks]]` (`[[note]]`,
//! `[[note|label]]`, `[[note#heading]]`, `[[#heading]]`), embeds
//! (`![[picture.png]]`, `![[note]]`), and the places they lead to.
//!
//! A link to another note is written as `[[note|heading]]` in the HTML's
//! `href`: it is no web address and no place in this file, so the reader
//! finds the note by its name when it is followed (see the reader's
//! `follow`), not now, when the note may not be there yet.

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::RefCell;

use paperpaw_sdk::Storage;
use paperpaw_sdk::xml::escape;

/// The longest a wikilink is, in bytes, between the brackets.
const MAX_LINK: usize = 512;

/// Folders visited at most while a picture is looked for.
const MAX_FOLDERS: usize = 200;
/// Entries read at most while a picture is looked for.
const MAX_ENTRIES: usize = 4000;
/// Pictures looked for at most in one file: the rest are shown as text.
const MAX_LOOKUPS: usize = 64;

/// The picture formats that are shown.
const PICTURES: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

/// A wikilink read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Wikilink<'a> {
    /// The note or file it names, without its folder's path (maybe empty:
    /// a heading of this note).
    pub target: &'a str,
    /// The heading (or block) after `#`.
    pub fragment: Option<&'a str>,
    /// The words shown instead of the name, after `|`.
    pub label: Option<&'a str>,
}

impl Wikilink<'_> {
    /// What is shown for the link: its label, else the note's name, with
    /// the heading after it.
    pub fn shown(&self, out: &mut String) {
        match (self.label, self.fragment) {
            (Some(label), _) => out.push_str(label),
            (None, None) => out.push_str(self.target),
            (None, Some(fragment)) => {
                if !self.target.is_empty() {
                    out.push_str(self.target);
                    out.push_str(" > ");
                }
                out.push_str(fragment);
            }
        }
    }

    /// The `href` it leads by, to go from the converted HTML to the
    /// storage's root, where the reader finds it (`pictures` leads from
    /// the HTML to the note's folder): `[[note|heading]]`, the heading as
    /// [`slug`] writes it; this note's heading alone as `#heading`.
    pub fn href(&self, pictures: &str, out: &mut String) {
        let mut href = String::new();
        let fragment = self.fragment.map(slug).filter(|s| !s.is_empty());
        if self.target.is_empty() {
            href.push('#');
            href.push_str(fragment.as_deref().unwrap_or_default());
        } else {
            href.push_str(&"../".repeat(pictures.matches("../").count()));
            href.push_str("[[");
            // The converter reads `%20` as a space.
            href.push_str(&self.target.replace('%', "%25"));
            if let Some(fragment) = fragment {
                href.push('|');
                href.push_str(&fragment);
            }
            href.push_str("]]");
        }
        escape(&href, out);
    }
}

/// The wikilink that begins `text` (at its `[[`) and how many bytes it
/// takes; `None` where there is none (no `]]` on the line, nothing between
/// the brackets, or more than [`MAX_LINK`] bytes).
pub(crate) fn parse(text: &str) -> Option<(Wikilink<'_>, usize)> {
    let rest = text.strip_prefix("[[")?;
    let end = rest
        .char_indices()
        .take_while(|&(at, _)| at <= MAX_LINK)
        .find(|&(at, _)| rest[at..].starts_with("]]"))
        .map(|(at, _)| at)?;
    let inner = &rest[..end];
    if inner.contains("[[") {
        return None;
    }
    // A pipe in a table is written `\|`.
    let (name, label) = match inner.split_once('|') {
        Some((name, label)) => (name.trim_end_matches('\\'), Some(label.trim())),
        None => (inner, None),
    };
    let (target, fragment) = match name.split_once('#') {
        Some((target, fragment)) => (target.trim(), Some(fragment.trim())),
        None => (name.trim(), None),
    };
    if target.is_empty() && fragment.is_none_or(str::is_empty)
        || target.split('/').any(|p| p == "..")
    {
        return None;
    }
    Some((
        Wikilink {
            target,
            fragment: fragment.filter(|f| !f.is_empty()),
            label: label.filter(|l| !l.is_empty()),
        },
        end + 4,
    ))
}

/// A heading as an anchor: letters and digits in lower case, the rest
/// between them one `-`.
pub(super) fn slug(heading: &str) -> String {
    let mut slug = String::new();
    let mut apart = false;
    for c in heading.chars().take(200) {
        if c.is_alphanumeric() {
            if apart && !slug.is_empty() {
                slug.push('-');
            }
            apart = false;
            slug.extend(c.to_lowercase());
        } else {
            apart = true;
        }
    }
    slug
}

/// Whether `name` is a picture that is shown, by its ending.
pub(super) fn is_picture(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(_, ending)| PICTURES.iter().any(|p| ending.eq_ignore_ascii_case(p)))
}

/// The vault a note is in: where pictures it names are looked for.
pub(crate) struct Vault<'a> {
    storage: &'a dyn Storage,
    /// The vault's folder: the nearest above the note with an `.obsidian`
    /// folder, else the note's own.
    root: String,
    /// The note's folder.
    dir: String,
    /// What was looked for, and where it was found.
    found: RefCell<Vec<(String, Option<String>)>>,
}

impl<'a> Vault<'a> {
    pub fn new(storage: &'a dyn Storage, note: &str) -> Self {
        let dir = note.rsplit_once('/').map_or("", |(dir, _)| dir);
        let mut root = dir;
        let mut up = dir;
        // The folder with `.obsidian` in it, within a few folders above.
        for _ in 0..8 {
            let marker = join(up, ".obsidian");
            if paperpaw_sdk::files::is_dir(storage, &marker) {
                root = up;
                break;
            }
            match up.rsplit_once('/') {
                Some((above, _)) => up = above,
                None if up.is_empty() => break,
                None => up = "",
            }
        }
        Self {
            storage,
            root: root.to_string(),
            dir: dir.to_string(),
            found: RefCell::new(Vec::new()),
        }
    }

    /// The place on the storage of the file `name` (with its folders, if
    /// it has some): beside the note, else the vault's folder, else the
    /// first of that name in the vault, the one nearest its top.
    pub fn find(&self, name: &str) -> Option<String> {
        if name.is_empty() || name.contains("..") {
            return None;
        }
        {
            let found = self.found.borrow();
            if let Some((_, path)) = found.iter().find(|(n, _)| n == name) {
                return path.clone();
            }
            if found.len() >= MAX_LOOKUPS {
                return None;
            }
        }
        let path = self.look(name);
        self.found
            .borrow_mut()
            .push((name.to_string(), path.clone()));
        path
    }

    /// Looks for the pictures the note at `note` embeds (`![[name]]`) in
    /// one go: those not beside it or at the vault's top are looked for in
    /// one walk of the vault, not one walk each. What was looked for is
    /// kept, as [`find`](Self::find) keeps it.
    pub fn prefetch(&self, note: &str) {
        let mut names = embedded_pictures(self.storage, note);
        names.retain(|n| self.find_here(n).is_none());
        if names.is_empty() {
            return;
        }
        let wanted: Vec<String> = names
            .iter()
            .map(|n| n.rsplit('/').next().unwrap_or(n).to_lowercase())
            .collect();
        let mut hits: Vec<Option<String>> = alloc::vec![None; names.len()];
        let (mut folders, mut seen, mut next) = (alloc::vec![self.root.clone()], 0, 0);
        while let Some(folder) = folders.get(next).cloned() {
            next += 1;
            let mut below = Vec::new();
            let _ = self.storage.list(&folder, &mut |entry| {
                seen += 1;
                if seen > MAX_ENTRIES || entry.name.starts_with('.') {
                    return;
                }
                if entry.is_dir {
                    if folders.len() + below.len() < MAX_FOLDERS {
                        below.push(join(&folder, entry.name));
                    }
                } else {
                    let lower = entry.name.to_lowercase();
                    for (at, w) in wanted.iter().enumerate() {
                        if hits[at].is_none() && *w == lower {
                            hits[at] = Some(join(&folder, entry.name));
                        }
                    }
                }
            });
            if seen > MAX_ENTRIES || hits.iter().all(Option::is_some) {
                break;
            }
            folders.append(&mut below);
        }
        let mut found = self.found.borrow_mut();
        for (name, hit) in names.into_iter().zip(hits) {
            if found.len() < MAX_LOOKUPS {
                found.push((name, hit));
            }
        }
    }

    /// The file `name`, beside the note or at the vault's top.
    fn find_here(&self, name: &str) -> Option<String> {
        [self.dir.as_str(), self.root.as_str()]
            .into_iter()
            .map(|base| join(base, name))
            .find(|path| self.storage.exists(path))
    }

    fn look(&self, name: &str) -> Option<String> {
        if let Some(path) = self.find_here(name) {
            return Some(path);
        }
        // By its name, anywhere below the vault's folder.
        let wanted = name.rsplit('/').next().unwrap_or(name).to_lowercase();
        let (mut folders, mut seen) = (alloc::vec![self.root.clone()], 0);
        let mut next = 0;
        while let Some(folder) = folders.get(next).cloned() {
            next += 1;
            let mut hit = None;
            let mut below = Vec::new();
            let _ = self.storage.list(&folder, &mut |entry| {
                seen += 1;
                if seen > MAX_ENTRIES || entry.name.starts_with('.') {
                    return;
                }
                if entry.is_dir {
                    below.push(join(&folder, entry.name));
                } else if hit.is_none() && entry.name.to_lowercase() == wanted {
                    hit = Some(join(&folder, entry.name));
                }
            });
            if hit.is_some() {
                return hit;
            }
            if seen > MAX_ENTRIES {
                return None;
            }
            for sub in below {
                if folders.len() < MAX_FOLDERS {
                    folders.push(sub);
                }
            }
        }
        None
    }
}

/// The pictures the file at `path` embeds (`![[name]]`), each once, at
/// most [`MAX_LOOKUPS`]: read a piece at a time, as the file is converted.
fn embedded_pictures(storage: &dyn Storage, path: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    // What of the last piece may be the beginning of an embed.
    let mut window: Vec<u8> = Vec::new();
    let (mut chunk, mut at) = ([0u8; 1024], 0u64);
    while names.len() < MAX_LOOKUPS {
        let Ok(n) = storage.read_at(path, at, &mut chunk) else {
            break;
        };
        if n == 0 {
            break;
        }
        at += n as u64;
        window.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&window).into_owned();
        let mut from = 0;
        while let Some(found) = text[from..].find("![[") {
            let start = from + found;
            match parse(&text[start + 1..]) {
                Some((link, taken)) => {
                    if is_picture(link.target)
                        && names.len() < MAX_LOOKUPS
                        && !names.iter().any(|n| n.eq_ignore_ascii_case(link.target))
                    {
                        names.push(link.target.to_string());
                    }
                    from = start + 1 + taken;
                }
                None if text.len() - start <= MAX_LINK + 4 => {
                    from = start;
                    break;
                }
                None => from = start + 3,
            }
        }
        // Keep what may begin an embed: from the last `![` on.
        let mut keep = text[from..]
            .rfind("![")
            .map_or(text.len().saturating_sub(1), |k| from + k);
        while !text.is_char_boundary(keep) {
            keep += 1;
        }
        window = text.as_bytes()[keep..].to_vec();
        if window.len() > MAX_LINK + 8 {
            window.clear();
        }
    }
    names
}

impl core::fmt::Debug for Vault<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vault")
            .field("root", &self.root)
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        alloc::format!("{dir}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::MemoryStorage;

    #[test]
    fn links_are_read_in_all_their_forms() {
        let read = |t: &str| {
            parse(t).map(|(l, n)| {
                let own = |s: Option<&str>| s.map(String::from);
                (l.target.to_string(), own(l.fragment), own(l.label), n)
            })
        };
        let some = |s: &str| Some(String::from(s));
        assert_eq!(read("[[Note]] rest"), Some(("Note".into(), None, None, 8)));
        assert_eq!(
            read("[[Note|the note]]"),
            Some(("Note".into(), None, some("the note"), 17))
        );
        assert_eq!(
            read("[[Note#Part two]]"),
            Some(("Note".into(), some("Part two"), None, 17))
        );
        assert_eq!(
            read("[[#Here|there]]"),
            Some((String::new(), some("Here"), some("there"), 15))
        );
        assert_eq!(read("[[a\\|b]]").map(|r| r.0), Some("a".into()));
        for bad in ["[[]]", "[[ ]]", "[[no end", "[[a [[b]]", "[[#]]", "[ [x]]"] {
            assert_eq!(read(bad), None, "{bad}");
        }
        // Not read past the limit, nor past a line.
        assert_eq!(read(&alloc::format!("[[{}]]", "x".repeat(600))), None);
    }

    #[test]
    fn what_is_shown_and_where_it_leads() {
        let (link, _) = parse("[[Note#Big Part]]").unwrap();
        let (mut shown, mut href) = (String::new(), String::new());
        link.shown(&mut shown);
        link.href("../../notes/", &mut href);
        assert_eq!(
            (shown.as_str(), href.as_str()),
            ("Note > Big Part", "../../[[Note|big-part]]")
        );
        let (link, _) = parse("[[#Big Part|up]]").unwrap();
        let (mut shown, mut href) = (String::new(), String::new());
        link.shown(&mut shown);
        link.href("", &mut href);
        assert_eq!((shown.as_str(), href.as_str()), ("up", "#big-part"));
    }

    #[test]
    fn headings_make_anchors() {
        assert_eq!(slug("A big -- Part, 2!"), "a-big-part-2");
        assert_eq!(slug("Ärger über Öl"), "ärger-über-öl");
        assert_eq!(slug("!!!"), "");
        assert!(slug(&"ab ".repeat(500)).len() <= 200);
    }

    #[test]
    fn pictures_are_found_where_obsidian_finds_them() {
        let storage = MemoryStorage::new()
            .with("v/.obsidian/app.json", b"{}".to_vec())
            .with("v/Notes/a.md", b"x".to_vec())
            .with("v/Files/deep/Cat.PNG", b"x".to_vec())
            .with("v/Notes/dog.png", b"x".to_vec())
            .with("v/top.png", b"x".to_vec());
        let vault = Vault::new(&storage, "v/Notes/a.md");
        assert_eq!(vault.find("dog.png").as_deref(), Some("v/Notes/dog.png"));
        assert_eq!(vault.find("top.png").as_deref(), Some("v/top.png"));
        assert_eq!(
            vault.find("cat.png").as_deref(),
            Some("v/Files/deep/Cat.PNG")
        );
        assert_eq!(vault.find("none.png"), None);
        assert_eq!(vault.find("../x.png"), None);
        assert_eq!(vault.find(""), None);
    }

    /// A storage that counts the folders listed.
    struct Counting(MemoryStorage, core::cell::Cell<usize>);

    impl Storage for Counting {
        fn read_at(
            &self,
            path: &str,
            offset: u64,
            buf: &mut [u8],
        ) -> Result<usize, paperpaw_sdk::StorageError> {
            self.0.read_at(path, offset, buf)
        }
        fn size(&self, path: &str) -> Result<u64, paperpaw_sdk::StorageError> {
            self.0.size(path)
        }
        fn write(&self, path: &str, data: &[u8]) -> Result<(), paperpaw_sdk::StorageError> {
            self.0.write(path, data)
        }
        fn append(&self, path: &str, data: &[u8]) -> Result<(), paperpaw_sdk::StorageError> {
            self.0.append(path, data)
        }
        fn list(
            &self,
            dir: &str,
            visit: &mut dyn FnMut(paperpaw_sdk::DirEntry<'_>),
        ) -> Result<(), paperpaw_sdk::StorageError> {
            self.1.set(self.1.get() + 1);
            self.0.list(dir, visit)
        }
        fn remove(&self, path: &str) -> Result<(), paperpaw_sdk::StorageError> {
            self.0.remove(path)
        }
        fn exists(&self, path: &str) -> bool {
            self.0.exists(path)
        }
        fn rename(&self, from: &str, to: &str) -> Result<(), paperpaw_sdk::StorageError> {
            self.0.rename(from, to)
        }
        fn create_dir(&self, path: &str) -> Result<(), paperpaw_sdk::StorageError> {
            self.0.create_dir(path)
        }
    }

    #[test]
    fn the_pictures_of_a_note_are_looked_for_in_one_walk() {
        // 40 pictures in folders below the vault's top, one embed of each,
        // one straddling the first piece read, and one missing.
        let mut note = String::from("![[none.png]] ");
        let mut storage = MemoryStorage::new().with("v/.obsidian/app.json", b"{}".to_vec());
        for n in 0..40 {
            storage = storage.with(&alloc::format!("v/A{n:02}/deep/p{n}.png"), b"x".to_vec());
            note.push_str(&alloc::format!("![[p{n}.png]] and ![[P{n}.PNG|200]] "));
            if n == 5 {
                note.push_str(&" ".repeat(1024 - note.len() % 1024 - 5));
            }
        }
        let storage = Counting(storage.with("v/n.md", note.into_bytes()), 0.into());
        let vault = Vault::new(&storage, "v/n.md");
        assert_eq!(embedded_pictures(&storage, "v/n.md").len(), 41);
        vault.prefetch("v/n.md");
        // One walk: the vault's folder, then each folder, each level;
        // the 80 embeds did not make 80 walks.
        let listed = storage.1.get();
        assert!(listed <= 1 + 40 + 40 + 1, "{listed} folders listed");
        for n in 0..40 {
            assert_eq!(
                vault.find(&alloc::format!("p{n}.png")),
                Some(alloc::format!("v/A{n:02}/deep/p{n}.png"))
            );
        }
        assert_eq!(vault.find("none.png"), None);
        // Answered from what was kept: nothing more is listed.
        assert_eq!(storage.1.get(), listed);
    }

    #[test]
    fn picture_endings() {
        assert!(is_picture("a.PNG") && is_picture("a.b.jpeg"));
        assert!(!is_picture("a.md") && !is_picture("png"));
    }
}
