//! The library's authors, series and tags.
//!
//! They are read from the library's catalog
//! ([`paperpaw_sdk::library::catalog`]), which the kernel makes anew in
//! the background (after PaperPaw starts, after books came, went or moved
//! or the library folder changed); a view shows the catalog there was
//! until the new one is complete, then reads it again
//! ([`events::CATALOG_CHANGED`]).
//!
//! A view lists the authors (or series, or tags) with how many books each
//! has; choosing one lists those books with their covers and states
//! ([`crate::list::shelf`]), a series by its numbers. Only the names are
//! held in memory, never the whole catalog.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, events, params};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, RefreshMode, Screen,
};

use crate::list::shelf::{self as books, Kind};
use paperpaw_sdk::library::{
    self,
    catalog::{self, Record, State},
};

/// The view of authors (`mode` = `authors`), series (`series`) or tags
/// (`tags`).
pub(crate) const MODE_AUTHORS: &str = "authors";
pub(crate) const MODE_SERIES: &str = "series";
pub(crate) const MODE_TAGS: &str = "tags";

/// What books are listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shelf {
    Authors,
    Series,
    Tags,
}

impl Shelf {
    fn of(mode: Option<&str>) -> Shelf {
        match mode {
            Some(MODE_SERIES) => Shelf::Series,
            Some(MODE_TAGS) => Shelf::Tags,
            _ => Shelf::Authors,
        }
    }

    /// The list of books one of its names opens.
    fn books(self) -> Kind {
        match self {
            Shelf::Authors => Kind::Author,
            Shelf::Series => Kind::Series,
            Shelf::Tags => Kind::Tag,
        }
    }
}
/// Names held at most.
const MAX_NAMES: usize = 2000;
/// What a book is listed under in a view: its author, its series, or
/// each of its tags.
fn keys(record: &Record, shelf: Shelf) -> Vec<&str> {
    match shelf {
        Shelf::Authors => alloc::vec![record.author.as_str()],
        Shelf::Series => alloc::vec![record.series.as_str()],
        Shelf::Tags => record.tags().collect(),
    }
}

/// The names of the authors, series or tags and how many books each
/// has, alphabetically.
fn names(books: catalog::Books<'_>, shelf: Shelf) -> Vec<(String, usize)> {
    let mut names: Vec<(String, usize)> = Vec::new();
    books(&mut |_, line| {
        for key in keys(&line, shelf) {
            // Books without series are no series; those without author
            // are listed under "Unknown author".
            if shelf == Shelf::Series && key.is_empty() {
                continue;
            }
            if let Some((_, count)) = names.iter_mut().find(|(n, _)| n == key) {
                *count += 1;
            } else if names.len() < MAX_NAMES {
                names.push((key.to_string(), 1));
            }
        }
    });
    names.sort_by_cached_key(|(n, _)| n.to_lowercase());
    names
}

pub(crate) struct ShelfScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    shelf: Shelf,
    names: Vec<(String, usize)>,
    /// No catalog yet: the kernel is making the first.
    collecting: bool,
    /// The kernel could not make the catalog (`CATALOG_FAILED`).
    failed: bool,
}

impl ShelfScreen {
    /// The style, the header and the list placed for `env`'s screen.
    fn frame(env: &Env<'_>) -> (UiStyle, ui::Header, ListView) {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        let list = ListView::new(area, style.noted_row()).focus_on_keys(env);
        (style, header, list)
    }

    pub(crate) fn new(env: &Env<'_>, params: &Params) -> Self {
        let (style, header, list) = Self::frame(env);
        let shelf = Shelf::of(params.get(params::MODE));
        Self {
            list,
            names: names(&mut |f| library::service(env).each(env, f), shelf),
            collecting: false,
            failed: false,
            shelf,
            style,
            header,
        }
    }

    fn redraw(&self, cx: &mut Cx<'_, '_>) {
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }

    /// Opens the books of name `index`.
    fn activate(&mut self, index: usize, cx: &mut Cx<'_, '_>) {
        if let Some((name, _)) = self.names.get(index) {
            cx.command(books::open(self.shelf.books(), name));
        }
    }

    fn title(&self) -> &'static str {
        match self.shelf {
            Shelf::Authors => "files.by-author",
            Shelf::Series => "files.by-series",
            Shelf::Tags => "files.by-tag",
        }
    }
}

impl Screen for ShelfScreen {
    /// Turned or resized: laid out anew, the focus on its name.
    fn on_resize(&mut self, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let (style, header, fresh) = Self::frame(&env);
        // The focus, whether it shows and the page stay with the list.
        self.list.set_area(fresh.area());
        self.list.set_row(fresh.row());
        (self.style, self.header) = (style, header);
        cx.request_redraw();
        Handled::Yes
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.names.len();
        let title = env.i18n().tr(self.title());
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, title, None, self.list.paging(len));
        if len == 0 {
            let area = self.list.area();
            // An empty view says what would be in it.
            let (icon, text) = match self.shelf {
                _ if self.failed => {
                    let title = env.i18n().tr("files.list.failed.title");
                    let text = env.i18n().tr("files.list.failed");
                    ui.empty(area, "books", title, text);
                    return;
                }
                _ if self.collecting => {
                    let message = env.i18n().tr("files.list.collecting");
                    ui.message(area, message);
                    return;
                }
                Shelf::Series => ("books", "files.no-series"),
                Shelf::Tags => ("tag", "files.no-tags"),
                Shelf::Authors => ("books", "files.no-books"),
            };
            let text = ui::tr_input(env, text);
            ui.empty(area, icon, "", text);
            return;
        }
        let names = &self.names;
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let (name, count) = &names[index];
            let name = if name.is_empty() {
                env.i18n().tr("files.unknown-author")
            } else {
                name.as_str()
            };
            let note = env.i18n().plural("files.books", *count as i64, &[]);
            ui::Row::value(name, "›")
                .note(&note)
                .focused(focused)
                .draw(ui, rect);
        });
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let state = library::service(&env).catalog_state(&env);
        self.collecting = crate::list::collecting(&env);
        if state == State::Missing {
            // No catalog yet: the kernel makes it, then it shows.
            cx.command(library::request_catalog());
        }
    }

    fn on_event(&mut self, event: &Id, _: &Params, cx: &mut Cx<'_, '_>) {
        // The folder could not be read: said until a catalog is made.
        if *event == events::CATALOG_FAILED {
            self.failed = true;
            self.collecting = false;
            self.redraw(cx);
            return;
        }
        // The new catalog, or a book gone (noted beside it): the names
        // again, keeping the place.
        if [
            events::CATALOG_CHANGED.id(),
            events::BOOK_REMOVED.id(),
            events::BOOK_MOVED.id(),
        ]
        .contains(&event)
        {
            let env = *cx.env();
            self.collecting = false;
            self.failed = false;
            self.names = names(&mut |f| library::service(&env).each(&env, f), self.shelf);
            self.list.set_len(self.names.len());
            self.redraw(cx);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let a = action;
        let rows = self.list.rows() as isize;
        let delta = if let Some(delta) = ui::list_step(a, rows) {
            delta
        } else if *a == actions::CONFIRM {
            self.activate(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        let change = self.list.move_focus(delta, self.names.len());
        change.request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let len = self.names.len();
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

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::MemoryStorage;
    use paperpaw_sdk::library;

    #[test]
    fn catalog_lines_and_views() {
        let storage = MemoryStorage::new();
        let books = [
            (
                "b/2.epub",
                "Ann",
                "Second",
                "Tales",
                "2",
                &["Fantasy", "Kids"][..],
            ),
            ("b/1.epub", "Ann", "First", "Tales", "1", &["Fantasy"]),
            ("b/x.epub", "bob", "Alone", "", "", &[]),
            ("b/y.txt", "", "Nobody's", "", "", &[]),
        ];
        let records: Vec<Record> = books
            .iter()
            .map(|(path, author, title, series, number, tags)| {
                let some = |s: &str| (!s.is_empty()).then(|| String::from(s));
                let book = library::Book::default()
                    .author(some(author))
                    .title(some(title))
                    .series(some(series))
                    .series_index(some(number))
                    .tags(tags.iter().map(|t| String::from(*t)).collect());
                Record::of(path, &book)
            })
            .collect();
        catalog::write(&storage, &records).unwrap();
        let authors = names(&mut |f| catalog::each_at(&storage, f), Shelf::Authors);
        assert_eq!(
            authors,
            [
                (String::new(), 1),
                (String::from("Ann"), 2),
                (String::from("bob"), 1)
            ]
        );
        assert_eq!(
            names(&mut |f| catalog::each_at(&storage, f), Shelf::Series),
            [(String::from("Tales"), 2)]
        );
        assert_eq!(
            names(&mut |f| catalog::each_at(&storage, f), Shelf::Tags),
            [(String::from("Fantasy"), 2), (String::from("Kids"), 1)]
        );
        // Searching: every word, anywhere, in any case.
        let found: Vec<String> = catalog::search(&storage, "ann TALES")
            .into_iter()
            .map(|l| l.title)
            .collect();
        assert_eq!(found, ["First", "Second"]);
        assert_eq!(catalog::search(&storage, "kids")[0].title, "Second");
        assert!(catalog::search(&storage, "nothing").is_empty());
    }
}
