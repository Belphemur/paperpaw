//! The links of the open note, as a list: a note's links to other notes
//! (`[[wikilinks]]` and links to `.md` files) listed from the reader's menu,
//! each opening its note in the reader, *Back* returning to the list and
//! to the note. Tapping a link in the text does not lead to another file
//! with this reader; this list does.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Params, Rect, RefreshMode,
    Screen, Storage,
};

use crate::lines;
use crate::markdown::wiki::{self, Vault};
use crate::note;

/// Parameter: the note whose links are listed.
pub(crate) const NOTE: &str = params::PATH;

/// Links listed at most.
const MAX_LINKS: usize = 100;
/// The longest a link's address is, in bytes.
const MAX_ADDRESS: usize = 512;

/// A link of a note to another note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Link {
    /// What is shown: the link's words, else the note's name.
    pub label: String,
    /// The note it names: a name as a vault writes it, or a path as a file
    /// link writes it (`file`).
    pub name: String,
    pub file: bool,
}

/// The links of the note at `path` to other notes, each once, in the order
/// the note has them: its wikilinks and its links to Markdown files,
/// pictures and other kinds of file left out.
pub(crate) fn read(storage: &dyn Storage, path: &str) -> Vec<Link> {
    let mut found: Vec<Link> = Vec::new();
    let _ = lines::each(storage, path, &mut |line| {
        scan(line, &mut found);
        found.len() < MAX_LINKS
    });
    found
}

fn add(found: &mut Vec<Link>, link: Link) {
    let same = |l: &Link| l.file == link.file && l.name.eq_ignore_ascii_case(&link.name);
    if found.len() < MAX_LINKS && !found.iter().any(same) {
        found.push(link);
    }
}

/// Whether `name` is a note's name without an ending, or a note file.
fn is_note_name(name: &str) -> bool {
    match name.rsplit_once('.') {
        Some((stem, ending)) if !stem.is_empty() && ending.len() <= 8 && !ending.contains(' ') => {
            note::is_note(name)
        }
        _ => true,
    }
}

fn scan(line: &str, found: &mut Vec<Link>) {
    let bytes = line.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        // `![[picture]]` and `![](picture)`: not links to notes.
        let embed = at > 0 && bytes[at - 1] == b'!';
        if line[at..].starts_with("[[") {
            if let Some((link, taken)) = wiki::parse(&line[at..]) {
                if !embed && !link.target.is_empty() && is_note_name(link.target) {
                    let mut label = String::new();
                    link.shown(&mut label);
                    add(
                        found,
                        Link {
                            label,
                            name: link.target.to_string(),
                            file: false,
                        },
                    );
                }
                at += taken;
                continue;
            }
        } else if line[at..].starts_with('[')
            && !embed
            && let Some((label, address, taken)) = file_link(&line[at..])
        {
            add(
                found,
                Link {
                    label: label.to_string(),
                    name: address,
                    file: true,
                },
            );
            at += taken;
            continue;
        }
        // On to the next character.
        at += line[at..].chars().next().map_or(1, char::len_utf8);
    }
}

/// The link `[words](address)` that begins `text`, to a Markdown file of
/// the vault: its words, its address without a `#heading` and with `%20`
/// read as a space, and how many bytes it takes.
fn file_link(text: &str) -> Option<(&str, String, usize)> {
    let inner = text.strip_prefix('[')?;
    let close = inner.find("](")?;
    let label = &inner[..close];
    if label.contains('[') || label.contains(']') {
        return None;
    }
    let rest = &inner[close + 2..];
    let end = rest.find(')')?;
    let taken = 1 + close + 2 + end + 1;
    // An address may carry a title after a space.
    let address = rest[..end].split(' ').next().unwrap_or_default();
    let address = address.trim_matches(|c| c == '<' || c == '>');
    let address = address.split('#').next().unwrap_or_default();
    if address.is_empty()
        || address.len() > MAX_ADDRESS
        || address.contains("://")
        || address.starts_with("mailto:")
        || address.starts_with('/')
        || !note::is_note(address)
    {
        return None;
    }
    let label = if label.trim().is_empty() {
        address.rsplit('/').next().unwrap_or(address)
    } else {
        label.trim()
    };
    Some((label, address.replace("%20", " "), taken))
}

/// The note `link` leads to from the note at `from`: its place on the
/// storage, where it is.
pub(crate) fn resolve(storage: &dyn Storage, from: &str, link: &Link) -> Option<String> {
    if link.file {
        return file_path(storage, from, &link.name);
    }
    let vault = Vault::new(storage, from);
    if note::is_note(&link.name) {
        return vault.find(&link.name);
    }
    // A name without its ending: `.md`, else `.markdown`.
    vault
        .find(&alloc::format!("{}.md", link.name))
        .or_else(|| vault.find(&alloc::format!("{}.markdown", link.name)))
}

/// `address`, a path from the folder of the note at `from`, as a path of
/// the storage if there is a file there.
fn file_path(storage: &dyn Storage, from: &str, address: &str) -> Option<String> {
    let mut parts: Vec<&str> = from
        .rsplit_once('/')
        .map_or("", |(dir, _)| dir)
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    for part in address.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    let path = parts.join("/");
    storage.exists(&path).then_some(path)
}

#[allow(missing_debug_implementations, reason = "a screen")]
pub(crate) struct LinksScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    note: String,
    title: String,
    links: Vec<Link>,
}

impl LinksScreen {
    pub(crate) fn new(env: &Env<'_>, params: &Params) -> Self {
        let note = params.get(NOTE).unwrap_or_default().to_string();
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        let list = ListView::new(area, u32::from(style.row())).focus_on_keys(env);
        let links = read(env.storage(), &note);
        Self {
            title: env.i18n().tr("notes-vault.links").to_string(),
            style,
            header,
            list,
            note,
            links,
        }
    }

    fn activate(&mut self, index: usize, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let Some(link) = self.links.get(index) else {
            return;
        };
        match resolve(env.storage(), &self.note, link) {
            Some(path) => cx.command(paperpaw_sdk::readers::open(path)),
            None => {
                let text = env.i18n().format(
                    "notes-vault.links.missing",
                    &[("name", Arg::Str(&link.label))],
                );
                cx.command(Command::Notify(text));
            }
        }
    }
}

impl Screen for LinksScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.links.len();
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, &self.title, None, self.list.paging(len));
        if len == 0 {
            let text = ui::tr_input(env, "notes-vault.links.none");
            ui.empty(self.list.area(), "note", "", text);
            return;
        }
        let this = &*self;
        this.list.draw(&mut ui, len, |ui, index, rect, focused| {
            if let Some(link) = this.links.get(index) {
                ui::Row::value(&link.label, "›")
                    .focused(focused)
                    .draw(ui, rect);
            }
        });
    }

    fn on_action(&mut self, action: &paperpaw_sdk::Id, cx: &mut Cx<'_, '_>) -> Handled {
        let rows = self.list.rows() as isize;
        let delta = if let Some(delta) = ui::list_step(action, rows) {
            delta
        } else if *action == actions::CONFIRM {
            self.activate(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        let len = self.links.len();
        self.list.move_focus(delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let len = self.links.len();
        let paging = self.list.paging(len);
        if let Some(action) = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        match self.list.hit(gesture.start, len) {
            Some(index) => {
                self.list.set_focus(index);
                self.activate(index, cx);
                cx.request_refresh(RefreshMode::Partial);
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(line: &str) -> Vec<(String, String, bool)> {
        let mut found = Vec::new();
        scan(line, &mut found);
        found
            .into_iter()
            .map(|l| (l.label, l.name, l.file))
            .collect()
    }

    #[test]
    fn wikilinks_and_file_links_are_found_and_pictures_are_not() {
        let found = links(
            "See [[Plan]], [[Ideas#Later|the later ones]], ![[map.png]], ![pic](a.png), \
             [site](https://example.org/a.md) and [Notes](Sub/My%20Notes.md#top) [[Plan]]",
        );
        assert_eq!(
            found,
            [
                ("Plan".into(), "Plan".into(), false),
                ("the later ones".into(), "Ideas".into(), false),
                ("Notes".into(), "Sub/My Notes.md".into(), true),
            ]
        );
    }

    #[test]
    fn a_file_that_is_no_note_and_a_heading_of_this_note_are_not_links_to_notes() {
        assert!(links("[[#Heading]] [[report.pdf]] [x](report.pdf) [y](#top)").is_empty());
    }

    #[test]
    fn a_line_of_odd_characters_does_not_cut_a_character() {
        assert_eq!(links("é [[Ünï]] ö [[]] [a](ü.md)").len(), 2);
    }
}
