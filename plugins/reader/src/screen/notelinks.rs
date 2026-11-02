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

/// Folders looked into at most while a note is looked for.
const MAX_FOLDERS: usize = 1000;
/// Entries read at most while a note is looked for.
const MAX_ENTRIES: usize = 30_000;

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
/// is none.
pub(super) fn find(storage: &dyn Storage, from: &str, link: &NoteLink) -> Option<String> {
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
    for base in [dir, root.as_str()] {
        for ending in ["", ".md", ".markdown"] {
            let path = join(base, &alloc::format!("{name}{ending}"));
            if storage.exists(&path) {
                return Some(path);
            }
        }
    }
    // By its name alone, anywhere in the vault: the nearest the top.
    let wanted = name.rsplit('/').next().unwrap_or(name).to_lowercase();
    let is_it = |file: &str| {
        let file = file.to_lowercase();
        file == wanted
            || file
                .strip_suffix(".md")
                .or_else(|| file.strip_suffix(".markdown"))
                .is_some_and(|stem| stem == wanted)
    };
    // Beside the note first, whatever the case.
    let mut beside = None;
    let listed = storage.list(dir, &mut |entry| {
        if beside.is_none() && !entry.is_dir && is_it(entry.name) {
            beside = Some(join(dir, entry.name));
        }
    });
    if listed.is_ok() && beside.is_some() {
        return beside;
    }
    let (mut folders, mut next, mut seen) = (alloc::vec![root], 0, 0);
    while let Some(folder) = folders.get(next).cloned() {
        next += 1;
        let (mut hit, mut below) = (None, Vec::new());
        let listed = storage.list(&folder, &mut |entry| {
            seen += 1;
            if seen > MAX_ENTRIES || entry.name.starts_with('.') {
                return;
            }
            if entry.is_dir {
                below.push(join(&folder, entry.name));
            } else if hit.is_none() && is_it(entry.name) {
                hit = Some(join(&folder, entry.name));
            }
        });
        if listed.is_ok() && hit.is_some() || seen > MAX_ENTRIES {
            return hit;
        }
        let room = MAX_FOLDERS.saturating_sub(folders.len());
        folders.extend(below.into_iter().take(room));
    }
    None
}

/// The vault's folder: the nearest folder above `dir` (or `dir`) with an
/// `.obsidian` folder in it, else `dir`.
fn vault_root(storage: &dyn Storage, dir: &str) -> String {
    let mut up = dir;
    for _ in 0..8 {
        if paperpaw_sdk::files::is_dir(storage, &join(up, ".obsidian")) {
            return up.to_string();
        }
        match up.rsplit_once('/') {
            Some((above, _)) => up = above,
            None if up.is_empty() => break,
            None => up = "",
        }
    }
    dir.to_string()
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
            find(&storage, from, &link("plan")).as_deref(),
            Some("v/Projects/Plan.md")
        );
        assert_eq!(
            find(&storage, from, &link("TOP")).as_deref(),
            Some("v/Top.md")
        );
        // Two of a name: the one nearest the top.
        assert_eq!(
            find(&storage, from, &link("Idea")).as_deref(),
            Some("v/Ideas/Idea.md")
        );
        // With its folder.
        assert_eq!(
            find(&storage, from, &link("Projects/Deep/Idea")).as_deref(),
            Some("v/Projects/Deep/Idea.md")
        );
        // Beside the note first.
        let beside = MemoryStorage::new()
            .with("v/.obsidian/a", b"x".to_vec())
            .with("v/A/Idea.md", b"x".to_vec())
            .with("v/B/Idea.md", b"x".to_vec());
        assert_eq!(
            find(&beside, "v/B/n.md", &link("idea")).as_deref(),
            Some("v/B/Idea.md")
        );
        // Not found; not in hidden folders; not past the vault.
        assert_eq!(find(&storage, from, &link("Missing")), None);
        assert_eq!(find(&storage, from, &link("Secret")), None);
        assert_eq!(find(&storage, from, &link("")), None);
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
            find(&storage, "v/a.md", &path("v/Top.md")).as_deref(),
            Some("v/Top.md")
        );
        assert_eq!(
            find(&storage, "v/a.md", &path("v/Top")).as_deref(),
            Some("v/Top.md")
        );
        assert_eq!(find(&storage, "v/a.md", &path("v/Nope.md")), None);
        assert_eq!(find(&storage, "v/a.md", &path("")), None);
    }
}
