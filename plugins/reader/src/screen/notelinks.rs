//! Links from one note to another: `[[Note|heading]]` (a wikilink, as the
//! Markdown reader writes it) and the path of a note's file. The note is
//! found by its name in the vault when the link is followed, and opens
//! over this one, at the heading if the link names one; *Back* returns.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::Storage;

use paperpaw_sdk::builtin::params;
use paperpaw_sdk::{Command, Env, Params};

/// Opens the note at `path` as books are opened (with the reading app
/// chosen for it), with what is asked of the reader that opens it: the
/// heading to open at, the notes to go back to. Those are the reader's
/// own parameters, which opening a book does not carry (see
/// `paperpaw_sdk::readers::resolve`): added to where it leads.
pub(super) fn open(env: &Env<'_>, path: &str, anchor: Option<&str>, back: &[String]) -> Command {
    let extra = |mut opened: Params| {
        if let Some(anchor) = anchor {
            opened = opened.with(crate::ANCHOR, anchor.to_string());
        }
        if !back.is_empty() {
            opened = opened.with(crate::NOTES_BACK, back.join("\n"));
        }
        opened
    };
    let base = Params::new().with(params::PATH, path.to_string());
    match paperpaw_sdk::readers::resolve(env, &base) {
        Command::OpenSlot(slot, opened) => Command::OpenSlot(slot, extra(opened)),
        Command::OpenInSlot(slot, screen, opened) => {
            Command::OpenInSlot(slot, screen, extra(opened))
        }
        other => other,
    }
}

/// How far a note is looked for in a vault: the folders looked into and
/// the entries read at most in one tap (it runs in a key handler, with
/// the book open, so less where memory is small).
#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    folders: usize,
    entries: usize,
}

impl Limits {
    pub(super) fn of(env: &Env<'_>) -> Self {
        if env.device().memory == paperpaw_sdk::MemoryClass::Small {
            Self {
                folders: 100,
                entries: 2000,
            }
        } else {
            Self {
                folders: 200,
                entries: 4000,
            }
        }
    }
}

/// What a link to another note says.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct NoteLink {
    /// A note's name, or the path of its file.
    pub what: String,
    /// Whether `what` is a name (`[[…]]`), not a path.
    pub by_name: bool,
    /// The anchor of the heading to open at.
    pub anchor: Option<String>,
}

/// Reads `href` as a link to another note; `None` where it is not one (a
/// web address, an anchor in this note, nothing).
pub(super) fn read(href: &str) -> Option<NoteLink> {
    if paperpaw_sdk::document::is_external(href) || href.starts_with('#') || href.is_empty() {
        return None;
    }
    if let Some(inner) = href.strip_prefix("[[").and_then(|h| h.strip_suffix("]]")) {
        let (name, anchor) = match inner.split_once('|') {
            Some((name, anchor)) => (name, Some(anchor.to_string())),
            None => (inner, None),
        };
        return (!name.is_empty()).then(|| NoteLink {
            what: name.to_string(),
            by_name: true,
            anchor: anchor.filter(|a| !a.is_empty()),
        });
    }
    let (path, anchor) = match href.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor.to_string())),
        None => (href, None),
    };
    Some(NoteLink {
        what: path.to_string(),
        by_name: false,
        anchor: anchor.filter(|a| !a.is_empty()),
    })
}

/// The file `link` leads to, from the note at `from`: `None` where there
/// is none. A name is looked for beside the note and, in a vault (a folder
/// with `.obsidian` above the note), anywhere in it, within `limits`.
pub(super) fn find(
    storage: &dyn Storage,
    from: &str,
    link: &NoteLink,
    limits: Limits,
) -> Option<String> {
    let dir = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    if !link.by_name {
        // A path from the root; a note without its ending too.
        return [link.what.clone(), alloc::format!("{}.md", link.what)]
            .into_iter()
            .find(|p| storage.exists(p) && !link.what.is_empty());
    }
    let name = link.what.trim_matches('/');
    let root = vault_root(storage, dir);
    // As written, from the note's folder or the vault's (`Folder/Note`).
    let bases = [Some(dir), root.as_deref()];
    for base in bases.into_iter().flatten() {
        for ending in ["", ".md", ".markdown"] {
            let path = join(base, &alloc::format!("{name}{ending}"));
            if storage.exists(&path) {
                return Some(path);
            }
        }
    }
    // By its name alone: beside the note first, whatever the case.
    let wanted = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    let is_it = |file: &str| {
        let file = file.to_lowercase();
        file == wanted
            || file
                .strip_suffix(".md")
                .or_else(|| file.strip_suffix(".markdown"))
                .is_some_and(|stem| stem == wanted)
    };
    let mut beside = None;
    let listed = storage.list(dir, &mut |entry| {
        if beside.is_none() && !entry.is_dir && is_it(entry.name) {
            beside = Some(join(dir, entry.name));
        }
    });
    if listed.is_ok() && beside.is_some() {
        return beside;
    }
    // Anywhere in the vault, the nearest the top. Not outside a vault: any
    // book's link could walk the whole card.
    walk(storage, &root?, limits, is_it)
}

/// The first file `is_it` says is the one, in `root`'s folders level by
/// level, reading `limits.entries` entries and keeping `limits.folders`
/// folders to look into at most.
fn walk(
    storage: &dyn Storage,
    root: &str,
    limits: Limits,
    is_it: impl Fn(&str) -> bool,
) -> Option<String> {
    let (mut folders, mut next, mut seen) = (alloc::vec![root.to_string()], 0, 0);
    while let Some(folder) = folders.get(next).cloned() {
        next += 1;
        let (mut hit, mut below) = (None, Vec::new());
        let listed = storage.list(&folder, &mut |entry| {
            seen += 1;
            if seen > limits.entries || entry.name.starts_with('.') {
                return;
            }
            if entry.is_dir {
                if folders.len() + below.len() < limits.folders {
                    below.push(join(&folder, entry.name));
                }
            } else if hit.is_none() && is_it(entry.name) {
                hit = Some(join(&folder, entry.name));
            }
        });
        if listed.is_ok() && hit.is_some() || seen > limits.entries {
            return hit;
        }
        folders.append(&mut below);
    }
    None
}

/// The vault's folder: the nearest folder above `dir` (or `dir`) with an
/// `.obsidian` folder in it; `None` where there is none.
fn vault_root(storage: &dyn Storage, dir: &str) -> Option<String> {
    let mut up = dir;
    for _ in 0..8 {
        if paperpaw_sdk::files::is_dir(storage, &join(up, ".obsidian")) {
            return Some(up.to_string());
        }
        match up.rsplit_once('/') {
            Some((above, _)) => up = above,
            None if up.is_empty() => break,
            None => up = "",
        }
    }
    None
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

    const LIMITS: Limits = Limits {
        folders: 200,
        entries: 4000,
    };

    fn vault() -> MemoryStorage {
        MemoryStorage::new()
            .with("v/.obsidian/app.json", b"{}".to_vec())
            .with("v/Daily/2026-01-02.md", b"x".to_vec())
            .with("v/Projects/Plan.md", b"x".to_vec())
            .with("v/Projects/Deep/Idea.md", b"x".to_vec())
            .with("v/Ideas/Idea.md", b"x".to_vec())
            .with("v/Top.md", b"x".to_vec())
            .with("v/.hidden/Secret.md", b"x".to_vec())
    }

    #[test]
    fn links_are_read() {
        let read = |h: &str| read(h).map(|l| (l.what, l.by_name, l.anchor));
        assert_eq!(
            read("[[Plan|big-part]]"),
            Some(("Plan".into(), true, Some("big-part".into())))
        );
        assert_eq!(read("[[Plan]]"), Some(("Plan".into(), true, None)));
        assert_eq!(
            read("v/Other.md#sec"),
            Some(("v/Other.md".into(), false, Some("sec".into())))
        );
        for none in ["", "#here", "https://x.org", "mailto:a@b.c", "[[]]"] {
            assert_eq!(read(none), None, "{none}");
        }
    }

    #[test]
    fn a_name_finds_its_note_as_a_vault_does() {
        let storage = vault();
        let link = |n: &str| NoteLink {
            what: n.into(),
            by_name: true,
            anchor: None,
        };
        let from = "v/Daily/2026-01-02.md";
        // Case does not matter; the ending is left out.
        assert_eq!(
            find(&storage, from, &link("plan"), LIMITS).as_deref(),
            Some("v/Projects/Plan.md")
        );
        assert_eq!(
            find(&storage, from, &link("TOP"), LIMITS).as_deref(),
            Some("v/Top.md")
        );
        // Two of a name: the one nearest the top.
        assert_eq!(
            find(&storage, from, &link("Idea"), LIMITS).as_deref(),
            Some("v/Ideas/Idea.md")
        );
        // With its folder.
        assert_eq!(
            find(&storage, from, &link("Projects/Deep/Idea"), LIMITS).as_deref(),
            Some("v/Projects/Deep/Idea.md")
        );
        // Beside the note first.
        let beside = MemoryStorage::new()
            .with("v/.obsidian/a", b"x".to_vec())
            .with("v/A/Idea.md", b"x".to_vec())
            .with("v/B/Idea.md", b"x".to_vec());
        assert_eq!(
            find(&beside, "v/B/n.md", &link("idea"), LIMITS).as_deref(),
            Some("v/B/Idea.md")
        );
        // Not found; not in hidden folders; not past the vault.
        assert_eq!(find(&storage, from, &link("Missing"), LIMITS), None);
        assert_eq!(find(&storage, from, &link("Secret"), LIMITS), None);
        assert_eq!(find(&storage, from, &link(""), LIMITS), None);
    }

    #[test]
    fn a_path_finds_its_file() {
        let storage = vault();
        let path = |p: &str| NoteLink {
            what: p.into(),
            by_name: false,
            anchor: None,
        };
        assert_eq!(
            find(&storage, "v/a.md", &path("v/Top.md"), LIMITS).as_deref(),
            Some("v/Top.md")
        );
        assert_eq!(
            find(&storage, "v/a.md", &path("v/Top"), LIMITS).as_deref(),
            Some("v/Top.md")
        );
        assert_eq!(find(&storage, "v/a.md", &path("v/Nope.md"), LIMITS), None);
        assert_eq!(find(&storage, "v/a.md", &path(""), LIMITS), None);
    }

    #[test]
    fn outside_a_vault_a_name_is_looked_for_beside_the_note_only() {
        let link = NoteLink {
            what: "Deep".into(),
            by_name: true,
            anchor: None,
        };
        // A library: no `.obsidian`, the file far below.
        let books = MemoryStorage::new()
            .with("books/a/b/c/Deep.md", b"x".to_vec())
            .with("books/Near.md", b"x".to_vec())
            .with("books/n.epub", b"x".to_vec());
        assert_eq!(find(&books, "books/n.epub", &link, LIMITS), None);
        let near = NoteLink {
            what: "near".into(),
            ..link
        };
        assert_eq!(
            find(&books, "books/n.epub", &near, LIMITS).as_deref(),
            Some("books/Near.md")
        );
    }

    #[test]
    fn the_walk_of_a_vault_ends_within_its_limits() {
        // 2,000 folders side by side, one note in each, none the one.
        let mut storage = MemoryStorage::new().with("v/.obsidian/app.json", b"{}".to_vec());
        for n in 0..2000 {
            storage = storage.with(&alloc::format!("v/f{n:04}/n.md"), b"x".to_vec());
        }
        let tested = core::cell::Cell::new(0);
        let found = walk(&storage, "v", LIMITS, |_| {
            tested.set(tested.get() + 1);
            false
        });
        assert_eq!(found, None);
        // At most the folders it keeps were looked into.
        assert!(tested.get() <= LIMITS.folders, "{}", tested.get());
        // A note in the last folder is not found: the walk is bounded, as
        // `wiki.rs`'s is.
        storage = storage.with("v/f1999/Wanted.md", b"x".to_vec());
        let link = NoteLink {
            what: "Wanted".into(),
            by_name: true,
            anchor: None,
        };
        assert_eq!(find(&storage, "v/a.md", &link, LIMITS), None);
        // More entries than the room for them in one folder: it ends.
        let mut many = MemoryStorage::new().with("v/.obsidian/app.json", b"{}".to_vec());
        for n in 0..10_000 {
            many = many.with(&alloc::format!("v/g{n:05}/x"), b"x".to_vec());
        }
        let tested = core::cell::Cell::new(0);
        assert_eq!(
            walk(&many, "v", LIMITS, |_| {
                tested.set(tested.get() + 1);
                false
            }),
            None
        );
        assert!(tested.get() <= LIMITS.folders);
    }
}
