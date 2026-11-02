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
pub(super) struct Wikilink<'a> {
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
pub(super) fn parse(text: &str) -> Option<(Wikilink<'_>, usize)> {
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
pub(super) struct Vault<'a> {
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

    fn look(&self, name: &str) -> Option<String> {
        for base in [self.dir.as_str(), self.root.as_str()] {
            let path = join(base, name);
            if self.storage.exists(&path) {
                return Some(path);
            }
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

    #[test]
    fn picture_endings() {
        assert!(is_picture("a.PNG") && is_picture("a.b.jpeg"));
        assert!(!is_picture("a.md") && !is_picture("png"));
    }
}
