//! What a note is: a Markdown file. Its title is its file name.

use alloc::string::String;

/// Whether `name` is a note's file name (`.md`, `.markdown`).
pub(crate) fn is_note(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(stem, ending)| {
        !stem.is_empty()
            && (ending.eq_ignore_ascii_case("md") || ending.eq_ignore_ascii_case("markdown"))
    })
}

/// Whether a folder of the vault is left out: those of a program's own
/// (`.obsidian`, `.trash`, `.git`), as any that begins with a dot.
pub(crate) fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// The title of the note at `path`: its file name without the ending.
pub(crate) fn title(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// The folder of `path`, within the vault `root`, as written for
/// showing: empty for a note in the vault's own folder.
pub(crate) fn folder_in<'a>(path: &'a str, root: &str) -> &'a str {
    let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let root = root.trim_matches('/');
    let below = dir
        .strip_prefix(root)
        .filter(|rest| rest.is_empty() || rest.starts_with('/'))
        .unwrap_or(dir);
    below.trim_start_matches('/')
}

/// Most of a note's beginning is shown, in characters.
const EXCERPT: usize = 140;

/// The beginning of a note's text, as a line of plain words: past the
/// front matter, the headings and what is no text (rules, code, tables),
/// the first paragraph with its markup taken off.
pub(crate) fn excerpt(text: &str) -> String {
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        lines.next();
        let mut closed = false;
        let mut held = alloc::vec::Vec::new();
        for line in lines.by_ref() {
            if matches!(line.trim_end(), "---" | "...") {
                closed = true;
                break;
            }
            held.push(line);
        }
        if !closed {
            // No front matter: all of it is text.
            return excerpt(
                &text
                    .lines()
                    .skip(1)
                    .collect::<alloc::vec::Vec<_>>()
                    .join("\n"),
            );
        }
    }
    let mut out = String::new();
    let mut code = false;
    for line in lines {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            code = !code;
            continue;
        }
        let skip = code
            || line.is_empty()
            || line.starts_with('#') && line[1..].starts_with([' ', '#'])
            || line.starts_with('|')
            || line.chars().all(|c| matches!(c, '-' | '*' | '_' | ' '));
        if skip {
            // A paragraph ends at a blank line.
            if line.is_empty() && !out.is_empty() {
                break;
            }
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&plain(line));
        if out.chars().count() >= EXCERPT {
            break;
        }
    }
    let mut cut: String = out.chars().take(EXCERPT).collect();
    if out.chars().count() > EXCERPT {
        cut.push('…');
    }
    cut
}

/// The words of a line of Markdown without its signs: links as their
/// labels, emphasis and code marks and list and quote signs gone.
fn plain(line: &str) -> String {
    let line = line
        .trim_start_matches(['>', ' '])
        .trim_start_matches("- ")
        .trim_start_matches("* ");
    let mut out = String::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        if let Some(inner) = rest.strip_prefix("[[")
            && let Some(end) = inner.find("]]")
        {
            let link = &inner[..end];
            let shown = link.rsplit_once('|').map_or(link, |(_, label)| label);
            out.push_str(shown.split('#').next().unwrap_or(shown));
            rest = &inner[end + 2..];
        } else if c == '['
            && let Some((label, after)) = rest[1..].split_once("](")
            && let Some((_, tail)) = after.split_once(')')
        {
            out.push_str(label);
            rest = tail;
        } else {
            if !matches!(c, '*' | '_' | '`' | '~' | '=') {
                out.push(c);
            }
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The beginning of the note at `path`, as [`excerpt`] makes it.
pub(crate) fn excerpt_of_file(storage: &dyn paperpaw_sdk::Storage, path: &str) -> String {
    let mut chunk = [0u8; 2048];
    match storage.read_at(path, 0, &mut chunk) {
        Ok(n) if n > 0 => excerpt(&String::from_utf8_lossy(&chunk[..n])),
        _ => String::new(),
    }
}

pub(crate) fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        String::from(name)
    } else {
        alloc::format!("{dir}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_are_markdown_files() {
        assert!(is_note("a.md") && is_note("A.MD") && is_note("b c.markdown"));
        assert!(!is_note(".md") && !is_note("a.txt") && !is_note("md") && !is_note("a.mdx"));
    }

    #[test]
    fn the_excerpt_is_the_first_paragraph_as_plain_words() {
        let note = "---\ntags: [a]\n---\n# Title\n\n> **Bold** and [[Other|a link]] with [x](https://y.org) `code`.\nMore of it.\n\nSecond paragraph.\n";
        assert_eq!(excerpt(note), "Bold and a link with x code. More of it.");
        assert_eq!(excerpt("# Only a heading\n\n---\n"), "");
        assert_eq!(excerpt("```\ncode\n```\ntext"), "text");
        let long = "word ".repeat(100);
        assert!(excerpt(&long).ends_with('…'));
        assert_eq!(excerpt("---\nno end\nreally"), "no end really");
        assert_eq!(excerpt(&"[[".repeat(1000)).chars().count().min(1), 1);
    }

    #[test]
    fn titles_and_folders() {
        assert_eq!(title("v/Projects/Plan.md"), "Plan");
        assert_eq!(title("Plan.v2.md"), "Plan.v2");
        assert_eq!(title("noending"), "noending");
        assert_eq!(folder_in("v/Projects/Plan.md", "v"), "Projects");
        assert_eq!(folder_in("v/Plan.md", "v/"), "");
        assert_eq!(folder_in("other/Plan.md", "v"), "other");
        assert_eq!(folder_in("vault2/Plan.md", "vault"), "vault2");
    }
}
