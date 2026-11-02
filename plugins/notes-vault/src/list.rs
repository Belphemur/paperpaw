//! The lists of notes: the folders of the vault, the latest changes, the
//! notes of a tag, the results of a search. Each is shown a window at a
//! time (see `page`), with *Earlier* and *More* rows at its ends where
//! the list goes on; a note opens in the reader, a folder in a list of its
//! own.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Params, Rect, RefreshMode, Screen,
};

use crate::note::folder_in;
use crate::page::{self, Key, Order, Row};
use crate::source::Source;

/// Parameter: what the list shows (`folder`, `recent`, `tag`, `search`).
pub(crate) const MODE: &str = params::MODE;
/// Parameter: the folder (`folder`), the tag (`tag`) or the words
/// searched for (`search`).
pub(crate) const WHAT: &str = params::TEXT;

/// What a row of the list is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// *Earlier notes*: the window before.
    Earlier,
    Entry(usize),
    /// *More notes*: the window after.
    More,
}

#[allow(missing_debug_implementations, reason = "a screen")]
pub(crate) struct ListScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    source: Source,
    order: Order,
    /// The vault's folder, to show where a note is in it.
    root: String,
    title: String,
    /// What it says where there is nothing to list.
    empty: &'static str,
    rows: Vec<Row>,
    more: bool,
    /// What each window before this began after, from the first.
    before: Vec<Option<Key>>,
    /// What this window began after.
    after: Option<Key>,
    /// There is an index, where the list needs one.
    indexed: bool,
}

impl ListScreen {
    pub(crate) fn new(env: &Env<'_>, p: &Params) -> Self {
        let root = crate::vault(env);
        let what = p.get(WHAT).unwrap_or_default().to_string();
        let (source, order, title, empty) = match p.get(MODE) {
            Some("recent") => (
                Source::Notes {
                    folder: what,
                    query: String::new(),
                },
                Order::Recent,
                env.i18n().tr("notes-vault.recent").to_string(),
                "notes-vault.empty.recent",
            ),
            Some("tag") => (
                Source::Tag(what.clone()),
                Order::Name,
                alloc::format!("#{what}"),
                "notes-vault.empty.tag",
            ),
            Some("search") => (
                Source::Notes {
                    folder: String::new(),
                    query: what.clone(),
                },
                Order::Name,
                env.i18n().format(
                    "notes-vault.search.title",
                    &[("query", paperpaw_sdk::Arg::Str(&what))],
                ),
                "notes-vault.empty.search",
            ),
            _ => {
                let dir = if what.is_empty() { root.clone() } else { what };
                let name = dir.rsplit('/').next().unwrap_or(&dir);
                let title = if dir == root {
                    env.i18n().tr("notes-vault.folders").to_string()
                } else {
                    name.to_string()
                };
                (
                    Source::Folder(dir),
                    Order::Name,
                    title,
                    "notes-vault.empty.folder",
                )
            }
        };
        let (style, header, list) = Self::frame(env);
        let mut screen = Self {
            style,
            header,
            list,
            indexed: matches!(source, Source::Folder(_))
                || crate::index::kept(env.storage(), &root).is_some(),
            source,
            order,
            root,
            title,
            empty,
            rows: Vec::new(),
            more: false,
            before: Vec::new(),
            after: None,
        };
        screen.load(env);
        screen
    }

    fn frame(env: &Env<'_>) -> (UiStyle, ui::Header, ListView) {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        let list = ListView::new(area, style.noted_row()).focus_on_keys(env);
        (style, header, list)
    }

    /// Reads the window that follows `self.after`.
    fn load(&mut self, env: &Env<'_>) {
        if !self.indexed {
            return;
        }
        let window = page::window(self.order, self.after.as_ref(), &mut |emit| {
            self.source.feed(env.storage(), emit);
        });
        self.rows = window.rows;
        self.more = window.more;
    }

    fn len(&self) -> usize {
        self.rows.len() + usize::from(!self.before.is_empty()) + usize::from(self.more)
    }

    fn item(&self, index: usize) -> Option<Item> {
        let earlier = !self.before.is_empty();
        if earlier && index == 0 {
            return Some(Item::Earlier);
        }
        let at = index - usize::from(earlier);
        match at.cmp(&self.rows.len()) {
            core::cmp::Ordering::Less => Some(Item::Entry(at)),
            core::cmp::Ordering::Equal if self.more => Some(Item::More),
            _ => None,
        }
    }

    fn redraw(cx: &mut Cx<'_, '_>) {
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }

    fn activate(&mut self, index: usize, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match self.item(index) {
            Some(Item::Entry(at)) => {
                let Some(row) = self.rows.get(at) else {
                    return;
                };
                if row.folder {
                    cx.command(Command::OpenScreen(
                        crate::LIST_SCREEN,
                        Params::new()
                            .with(MODE, "folder")
                            .with(WHAT, row.path.clone()),
                    ));
                } else {
                    cx.command(paperpaw_sdk::readers::open(row.path.clone()));
                }
            }
            Some(Item::More) => {
                let last = self.rows.last().map(|r| self.order.key(r));
                self.before.push(self.after.take());
                self.after = last;
                self.load(&env);
                self.list.set_focus(0);
                Self::redraw(cx);
            }
            Some(Item::Earlier) => {
                self.after = self.before.pop().flatten();
                self.load(&env);
                self.list.set_focus(0);
                Self::redraw(cx);
            }
            None => {}
        }
    }

    /// The line under a note's title: where it is in the vault.
    fn note_line(&self, env: &Env<'_>, row: &Row) -> String {
        if row.folder {
            return env.i18n().tr("notes-vault.folder").to_string();
        }
        let folder = folder_in(&row.path, &self.root);
        if folder.is_empty() {
            env.i18n().tr("notes-vault.in-vault").to_string()
        } else {
            folder.to_string()
        }
    }
}

impl Screen for ListScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.len();
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, &self.title, None, self.list.paging(len));
        if !self.indexed {
            let text = env.i18n().tr("notes-vault.no-index");
            ui.message(self.list.area(), text);
            return;
        }
        if len == 0 {
            let text = ui::tr_input(env, self.empty);
            ui.empty(self.list.area(), "note", "", text);
            return;
        }
        let this = &*self;
        this.list.draw(&mut ui, len, |ui, index, rect, focused| {
            match this.item(index) {
                Some(Item::Entry(at)) => {
                    let row = &this.rows[at];
                    let line = this.note_line(env, row);
                    ui::Row::value(&row.title, if row.folder { "›" } else { "" })
                        .note(&line)
                        .focused(focused)
                        .draw(ui, rect);
                }
                Some(Item::Earlier) => {
                    ui::Row::value(env.i18n().tr("notes-vault.earlier"), "‹")
                        .focused(focused)
                        .draw(ui, rect);
                }
                Some(Item::More) => {
                    ui::Row::value(env.i18n().tr("notes-vault.more"), "›")
                        .focused(focused)
                        .draw(ui, rect);
                }
                None => {}
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
        let len = self.len();
        self.list.move_focus(delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let len = self.len();
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
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}
