//! The two lists: the books that have notes (with *Export* at the end),
//! and the notes of one book. Choosing a note opens the book there.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::annotations::{self, Annotation, Kind};
use paperpaw_sdk::builtin::{actions, events, params};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, Screen,
};

use crate::scan::{self, Counted};
use crate::{BOOK, clippings};

/// What the screen lists.
enum Rows {
    /// The books with notes, and whether more were left out.
    Books(Vec<Counted>, bool),
    /// The notes of one book.
    Notes {
        path: String,
        title: String,
        all: Vec<Annotation>,
    },
}

#[allow(missing_debug_implementations, reason = "a screen")]
pub(crate) struct NotesScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    rows: Rows,
}

impl NotesScreen {
    /// The books with notes.
    pub(crate) fn books(env: &Env<'_>) -> Self {
        let (books, more) = scan::books(env);
        Self::with(env, Rows::Books(books, more))
    }

    /// The notes of the book at the parameter `path`.
    pub(crate) fn of_book(env: &Env<'_>, params: &Params) -> Self {
        let path = params.get(params::PATH).unwrap_or_default().to_string();
        let title = paperpaw_sdk::library::book(env.storage(), &path)
            .map(|book| book.shown_title(&path).into_owned())
            .unwrap_or_else(|| paperpaw_sdk::files::name(&path).to_string());
        let all = annotations::load(env.storage(), &path);
        Self::with(env, Rows::Notes { path, title, all })
    }

    fn with(env: &Env<'_>, rows: Rows) -> Self {
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        Self {
            list: ListView::new(area, style.noted_row()).focus_on_keys(env),
            style,
            header,
            rows,
        }
    }

    /// The rows: the books and *Export*, or the notes.
    fn len(&self) -> usize {
        match &self.rows {
            Rows::Books(books, _) => match books.len() {
                0 => 0,
                n => n + 1,
            },
            Rows::Notes { all, .. } => all.len(),
        }
    }

    fn export(cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let message = match clippings::export(&env) {
            Ok(done) if done.notes > 0 => {
                let path = format!("{}/{}", clippings::folder(&env), clippings::NAME);
                env.i18n().plural(
                    "notes-overview.exported",
                    i64::from(done.notes),
                    &[("path", Arg::Str(&path))],
                )
            }
            Ok(_) => env.i18n().tr("notes-overview.none").to_string(),
            Err(_) => env.i18n().tr("notes-overview.export-failed").to_string(),
        };
        cx.command(Command::Notify(message));
    }

    fn open_entry(&self, index: usize, cx: &mut Cx<'_, '_>) {
        match &self.rows {
            Rows::Books(books, _) => {
                if let Some(book) = books.get(index) {
                    cx.command(Command::OpenScreen(
                        BOOK,
                        Params::new().with(params::PATH, book.path.clone()),
                    ));
                } else if index == books.len() && index > 0 {
                    Self::export(cx);
                }
            }
            Rows::Notes { path, all, .. } => {
                if let Some(note) = all.get(index) {
                    // The book first, then its place.
                    cx.command(paperpaw_sdk::readers::open(path));
                    cx.command(events::GOTO.emit(events::Goto::to(path.clone(), note.mark.start)));
                }
            }
        }
    }
}

/// `3 highlights · 1 bookmark`.
fn counts(env: &Env<'_>, book: &Counted) -> String {
    let mut parts: Vec<String> = Vec::new();
    for (key, n) in [
        ("notes-overview.highlights", book.highlights),
        ("notes-overview.bookmarks", book.bookmarks),
    ] {
        if n > 0 {
            parts.push(env.i18n().plural(key, i64::from(n), &[]));
        }
    }
    parts.join(" · ")
}

impl Screen for NotesScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.len();
        let mut ui = ui::Ui::new(canvas, env, &style);
        let title = match &self.rows {
            Rows::Books(..) => env.i18n().tr("notes-overview.title").to_string(),
            Rows::Notes { title, .. } => title.clone(),
        };
        self.header
            .draw(&mut ui, &title, None, self.list.paging(len));
        if len == 0 {
            ui.empty(
                self.list.area(),
                "bookmarks",
                env.i18n().tr("notes-overview.empty.title"),
                env.i18n().tr("notes-overview.empty"),
            );
            return;
        }
        let rows = &self.rows;
        self.list
            .draw(&mut ui, len, |ui, index, rect, focused| match rows {
                Rows::Books(books, more) => {
                    let Some(book) = books.get(index) else {
                        let help = if *more {
                            "notes-overview.export.help.more"
                        } else {
                            "notes-overview.export.help"
                        };
                        ui::Row::value(env.i18n().tr("notes-overview.export"), "›")
                            .note(env.i18n().tr(help))
                            .focused(focused)
                            .draw(ui, rect);
                        return;
                    };
                    ui::Row::plain(&book.title)
                        .note(&counts(env, book))
                        .focused(focused)
                        .draw(ui, rect);
                }
                Rows::Notes { all, .. } => {
                    let Some(a) = all.get(index) else { return };
                    let (kind, text) = match a.mark.kind {
                        Kind::Bookmark => ("notes-overview.bookmark", a.text.clone()),
                        _ => (
                            "notes-overview.highlight",
                            env.i18n()
                                .format("notes-overview.quoted", &[("text", Arg::Str(&a.text))]),
                        ),
                    };
                    let what = if a.note.is_empty() {
                        env.i18n().tr(kind)
                    } else {
                        a.note.as_str()
                    };
                    let note = format!("{what} · {} %", a.percent);
                    ui::Row::plain(&text)
                        .note(&note)
                        .focused(focused)
                        .draw(ui, rect);
                }
            });
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.len();
        let rows = self.list.rows() as isize;
        let delta = if let Some(delta) = ui::list_step(action, rows) {
            delta
        } else if *action == actions::CONFIRM {
            self.open_entry(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list.move_focus(delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let env = *cx.env();
        let paging = self.list.paging(self.len());
        if let Some(action) = self.header.tap(&env, &self.style, gesture.start, paging) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        match self.list.hit(gesture.start, self.len()) {
            Some(index) => {
                self.list.set_focus(index);
                self.open_entry(index, cx);
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}
