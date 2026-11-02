//! The tags of a note: those its front matter names and the `#tags` in
//! its first part, read once for the whole vault (see `index`).

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::Storage;

/// How much of a note is read for its tags, in bytes.
pub(crate) const READ: usize = 8 * 1024;
/// Tags kept of one note.
pub(crate) const PER_NOTE: usize = 16;
/// The longest a tag is, in bytes.
const MAX_TAG: usize = 48;

/// The tags in the beginning of the note at `path`, each once, in the
/// order they are met.
pub(crate) fn of_file(storage: &dyn Storage, path: &str) -> Vec<String> {
    let mut chunk = [0u8; 1024];
    let mut text: Vec<u8> = Vec::new();
    let mut at = 0u64;
    while text.len() < READ {
        match storage.read_at(path, at, &mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                at += n as u64;
                text.extend_from_slice(&chunk[..n]);
            }
        }
    }
    text.truncate(READ);
    of_text(&String::from_utf8_lossy(&text))
}

/// The tags of `text`: those of its front matter (`tags: [a, b]`, or a
/// list under `tags:`), and the words in it that begin with `#` and a
/// letter, outside code.
pub(crate) fn of_text(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut lines = text.lines().peekable();
    if lines.peek().is_some_and(|l| l.trim_end() == "---") {
        lines.next();
        let mut front: Vec<&str> = Vec::new();
        let mut closed = false;
        for line in lines.by_ref() {
            if matches!(line.trim_end(), "---" | "...") {
                closed = true;
                break;
            }
            front.push(line);
        }
        if closed {
            in_front_matter(&front, &mut found);
        } else {
            // Never closed: no front matter, all of it text.
            return inline_tags(text.lines());
        }
    }
    for tag in inline_tags(lines) {
        add(&mut found, &tag);
    }
    found
}

fn in_front_matter(lines: &[&str], found: &mut Vec<String>) {
    let mut listing = false;
    for line in lines {
        if listing && let Some(item) = line.trim_start().strip_prefix("- ") {
            add(found, item);
            continue;
        }
        listing = false;
        let Some((key, value)) = line
            .split_once(':')
            .filter(|_| !line.starts_with([' ', '\t']))
        else {
            continue;
        };
        if !matches!(key.trim().to_ascii_lowercase().as_str(), "tags" | "tag") {
            continue;
        }
        let value = value.trim().trim_start_matches('[').trim_end_matches(']');
        listing = value.is_empty();
        for part in value.split([',', ' ']) {
            add(found, part);
        }
    }
}

/// The `#tags` of lines of text, not in code and not headings.
fn inline_tags<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut found = Vec::new();
    let mut code = false;
    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            code = !code;
            continue;
        }
        if code || found.len() >= PER_NOTE {
            continue;
        }
        let mut before = ' ';
        let mut ticks = false;
        for (at, c) in line.char_indices() {
            if c == '`' {
                ticks = !ticks;
            } else if c == '#' && !ticks && before.is_whitespace() {
                let rest = &line[at + 1..];
                let end = rest
                    .find(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '/')))
                    .unwrap_or(rest.len());
                let word = &rest[..end];
                // A tag has a letter in it: `#12` is a number, `# ` a
                // heading.
                if word.chars().any(char::is_alphabetic) {
                    add(&mut found, word);
                }
            }
            before = c;
        }
    }
    found
}

fn add(found: &mut Vec<String>, tag: &str) {
    let tag = tag.trim().trim_matches(['"', '\'']).trim_start_matches('#');
    let clean: String = tag
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '/'))
        .collect();
    if !clean.is_empty()
        && clean.len() <= MAX_TAG
        && found.len() < PER_NOTE
        && !found.iter().any(|t| t.eq_ignore_ascii_case(&clean))
    {
        found.push(clean);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_come_from_front_matter_and_text() {
        let note =
            "---\ntitle: x\ntags: [a, \"b c\"]\n---\n# Heading\nText #inline and #nested/tag.\n";
        assert_eq!(of_text(note), ["a", "b", "c", "inline", "nested/tag"]);
        let list = "---\ntags:\n  - one\n  - two\nother: x\n---\nbody #three";
        assert_eq!(of_text(list), ["one", "two", "three"]);
    }

    #[test]
    fn what_is_no_tag_is_left_out() {
        let note = "# Heading\n#12 is a number, a#b is not, `#code`\n```\n#fenced\n```\n#Real\n";
        assert_eq!(of_text(note), ["Real"]);
        // Front matter never closed: all text.
        assert_eq!(of_text("---\ntags: x\n#t"), ["t"]);
        // Each tag once, whatever its case.
        assert_eq!(of_text("#A #a #b"), ["A", "b"]);
    }

    #[test]
    fn hostile_text_is_read_without_a_panic() {
        let many = "#t ".repeat(10_000);
        assert_eq!(of_text(&many), ["t"]);
        let distinct: String = (0..500).map(|n| alloc::format!("#t{n} ")).collect();
        assert_eq!(of_text(&distinct).len(), PER_NOTE);
        let _ = of_text(&"#".repeat(10_000));
        let _ = of_text(&"---\n".repeat(1000));
        assert!(of_text("#".repeat(60).as_str()).is_empty());
        let long = alloc::format!("#{}", "x".repeat(200));
        assert!(of_text(&long).is_empty());
    }
}
