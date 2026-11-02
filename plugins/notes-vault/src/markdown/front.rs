//! Front matter, the block of properties a note may begin with
//! (`---` … `---`): not shown, but for its tags, which are shown at the
//! top as the `#tags` they are. A block that is never closed is no front
//! matter: it is read as the text it is.

use alloc::string::String;
use alloc::vec::Vec;

/// Lines of front matter held at most, while it is looked at.
const MAX_LINES: usize = 100;
/// Bytes of front matter held at most.
const MAX_BYTES: usize = 8 * 1024;
/// Tags shown at most.
const MAX_TAGS: usize = 32;
/// The longest a tag is, in bytes.
const MAX_TAG: usize = 64;

/// Where the file's beginning is read.
#[derive(Debug, Default)]
pub(super) enum Front {
    /// Nothing read yet: a first line `---` opens front matter.
    #[default]
    Start,
    /// Inside it: the lines held, to be shown as text should it never
    /// close.
    Open { lines: Vec<String>, bytes: usize },
    /// Past it, or none.
    Done,
}

/// What one more line does to the front matter.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// Held or dropped: nothing for the line to show.
    Taken,
    /// Front matter ended: its tags to show, if there are some.
    Closed(Vec<String>),
    /// Not front matter after all: these lines (the opening `---`, those
    /// held, and the line itself) are text.
    Text(Vec<String>),
    /// Not in front matter: the line is read as usual.
    Past,
}

impl Front {
    /// Reads `raw`, a line of the file.
    pub fn feed(&mut self, raw: &str) -> Step {
        match self {
            Front::Done => Step::Past,
            Front::Start => {
                if raw.trim_end() == "---" {
                    *self = Front::Open {
                        lines: Vec::new(),
                        bytes: 0,
                    };
                    Step::Taken
                } else {
                    *self = Front::Done;
                    Step::Past
                }
            }
            Front::Open { lines, bytes } => {
                let closing = matches!(raw.trim_end(), "---" | "...");
                if closing {
                    let tags = tags(lines);
                    *self = Front::Done;
                    return Step::Closed(tags);
                }
                if lines.len() >= MAX_LINES || *bytes + raw.len() > MAX_BYTES {
                    return self.give_up(Some(raw));
                }
                *bytes += raw.len();
                lines.push(String::from(raw));
                Step::Taken
            }
        }
    }

    /// The file ended: front matter still open was none.
    pub fn end(&mut self) -> Step {
        if matches!(self, Front::Open { .. }) {
            self.give_up(None)
        } else {
            Step::Past
        }
    }

    fn give_up(&mut self, last: Option<&str>) -> Step {
        let Front::Open { lines, .. } = core::mem::replace(self, Front::Done) else {
            return Step::Past;
        };
        let mut text = Vec::with_capacity(lines.len() + 2);
        text.push(String::from("---"));
        text.extend(lines);
        text.extend(last.map(String::from));
        Step::Text(text)
    }
}

/// The tags the properties name: `tags: a, b`, `tags: [a, b]`, or a list
/// under `tags:` (the key is `tag` too).
fn tags(lines: &[String]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let mut listing = false;
    for line in lines {
        let item = line.trim_start().strip_prefix("- ");
        if listing && let Some(item) = item {
            add(&mut found, item);
            continue;
        }
        listing = false;
        // A key at the left edge only.
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
        if value.is_empty() {
            listing = true;
        }
        for part in value.split([',', ' ']) {
            add(&mut found, part);
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
        && found.len() < MAX_TAGS
        && !found.contains(&clean)
    {
        found.push(clean);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(lines: &[&str]) -> Vec<Step> {
        let mut front = Front::default();
        let mut steps: Vec<Step> = lines.iter().map(|l| front.feed(l)).collect();
        steps.push(front.end());
        steps
    }

    #[test]
    fn front_matter_is_taken_and_its_tags_kept() {
        let steps = run(&["---", "title: A", "tags: [x, \"y z\", #w]", "---", "# Body"]);
        assert_eq!(
            steps,
            [
                Step::Taken,
                Step::Taken,
                Step::Taken,
                Step::Closed(alloc::vec!["x".into(), "y".into(), "z".into(), "w".into()]),
                Step::Past,
                Step::Past,
            ]
        );
    }

    #[test]
    fn tags_as_a_list() {
        let steps = run(&["---", "tags:", "  - one", "  - two/sub", "other: x", "---"]);
        assert_eq!(
            steps[5],
            Step::Closed(alloc::vec!["one".into(), "two/sub".into()])
        );
    }

    #[test]
    fn a_block_never_closed_is_text() {
        let steps = run(&["---", "no end", "more"]);
        assert_eq!(
            steps[3],
            Step::Text(alloc::vec!["---".into(), "no end".into(), "more".into()])
        );
        // Too long to be front matter: text, and the rest as usual.
        let many: Vec<String> = (0..150).map(|n| alloc::format!("k{n}: v")).collect();
        let mut lines = alloc::vec!["---"];
        lines.extend(many.iter().map(String::as_str));
        let steps = run(&lines);
        assert!(
            matches!(&steps[101], Step::Text(t) if t.len() == 102),
            "{:?}",
            steps[101]
        );
        assert_eq!(steps[102], Step::Past);
    }

    #[test]
    fn a_file_without_front_matter_is_past_at_once() {
        assert_eq!(run(&["# Title", "---", "x", "---"])[1], Step::Past);
        assert_eq!(run(&[" ---"])[0], Step::Past);
    }
}
