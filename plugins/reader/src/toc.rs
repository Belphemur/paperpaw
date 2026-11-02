//! The table of contents screen.
//!
//! Lists the document's chapters with the current one focused; choosing an
//! entry emits `core.goto`, which the reader (or any other
//! plugin) handles, and closes the list. Only the visible page of entries
//! is held in memory, so long tables of contents work on small devices.
//!
//! A book without a table of contents (or with a single entry) gets one
//! made from its headings: found once, a few blocks at a time while the
//! list says so, and kept in the book's data folder.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::document::{Document, Locator, TocEntry};
use paperpaw_sdk::ui::{self, ListChange, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, RefreshMode, Screen,
};

use paperpaw_sdk::builtin::events;

/// Parameters carrying a position.
pub(crate) const SECTION: &str = "section";
pub(crate) const OFFSET: &str = "offset";

pub(crate) fn locator_params(path: &str, at: Locator) -> Params {
    Params::new()
        .with(params::PATH, path.to_string())
        .with(SECTION, alloc::format!("{}", at.section))
        .with(OFFSET, alloc::format!("{}", at.offset))
}

pub(crate) fn locator_of(params: &Params) -> Option<Locator> {
    Some(Locator::new(
        params.get(SECTION)?.parse().ok()?,
        params.get(OFFSET)?.parse().ok()?,
    ))
}

/// Headings kept at most for a table of contents.
const MAX_HEADINGS: usize = 500;
/// Blocks read in one step of finding the headings.
const STEP_BLOCKS: usize = 60;
/// Longest heading kept.
const MAX_TITLE: usize = 120;

fn headings_path(path: &str) -> String {
    alloc::format!("{}/headings.txt", crate::screen::book_dir(path))
}

/// The headings found before, if the book is still the same file.
fn load_headings(env: &Env<'_>, path: &str) -> Option<Vec<TocEntry>> {
    let size = env.storage().size(path).ok()?;
    let data = paperpaw_sdk::read_file(env.storage(), &headings_path(path), 256 * 1024).ok()?;
    let text = core::str::from_utf8(&data).ok()?;
    let mut lines = text.lines();
    if lines.next()? != alloc::format!("size {size}") {
        return None;
    }
    Some(
        lines
            .filter_map(|line| {
                let mut f = line.splitn(4, '\t');
                let section = f.next()?.parse().ok()?;
                let offset = f.next()?.parse().ok()?;
                let level = f.next()?.parse().ok()?;
                Some(TocEntry {
                    at: Locator::new(section, offset),
                    level,
                    title: f.next()?.to_string(),
                })
            })
            .collect(),
    )
}

fn save_headings(env: &Env<'_>, path: &str, headings: &[TocEntry]) {
    let size = env.storage().size(path).unwrap_or_default();
    let mut out = alloc::format!("size {size}\n");
    for h in headings {
        out.push_str(&alloc::format!(
            "{}\t{}\t{}\t{}\n",
            h.at.section,
            h.at.offset,
            h.level,
            h.title
        ));
    }
    #[expect(
        clippy::let_underscore_must_use,
        reason = "the headings kept as a cache, made again where missing"
    )]
    let _ = env.storage().write(&headings_path(path), out.as_bytes());
}

/// Headings at their levels below the highest one found (0 for it).
fn normalized(mut headings: Vec<TocEntry>) -> Vec<TocEntry> {
    let top = headings.iter().map(|h| h.level).min().unwrap_or(0);
    for h in &mut headings {
        h.level = (h.level - top).min(4);
    }
    headings
}

pub(crate) struct TocScreen {
    path: String,
    document: Option<Box<dyn Document>>,
    /// The table of contents made from the headings, if the book's own
    /// is missing or too short.
    headings: Option<Vec<TocEntry>>,
    /// Finding the headings: where to read on, and those found so far.
    scan: Option<(Locator, Vec<TocEntry>)>,
    len: usize,
    /// Index of the first entry in `visible`.
    first: usize,
    /// The entries of the visible page.
    visible: Vec<TocEntry>,
    style: UiStyle,
    header: ui::Header,
    list: ListView,
}

impl TocScreen {
    /// The style, the header and the list placed for `env`'s screen.
    fn frame(env: &Env<'_>) -> (UiStyle, ui::Header, ListView) {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        let list = ListView::new(area, u32::from(style.row())).focus_on_keys(env);
        (style, header, list)
    }

    pub(crate) fn open(env: &Env<'_>, params: &Params) -> Self {
        let path = params.get(params::PATH).unwrap_or_default().to_string();
        let options = crate::screen::open_options(env);
        let document = env.formats().open_with(env.storage(), &path, &options).ok();
        let len = document
            .as_ref()
            .map_or(0, |d| d.toc_len(env.storage()) as usize);
        let (style, header, list) = Self::frame(env);
        // Few entries: the headings may do better.
        let (headings, scan) = if len <= 1 && document.is_some() {
            match load_headings(env, &path) {
                Some(found) => (Some(found), None),
                None => (None, Some((Locator::START, Vec::new()))),
            }
        } else {
            (None, None)
        };
        let headings = headings.filter(|h| h.len() > len);
        let len = headings.as_ref().map_or(len, Vec::len);
        let mut screen = Self {
            path,
            document,
            headings,
            scan,
            len,
            first: 0,
            visible: Vec::new(),
            style,
            header,
            list,
        };
        if let Some(current) = locator_of(params) {
            screen.list.set_focus(screen.current(env, current));
        }
        screen.load_page(env);
        screen
    }

    /// The last entry at or before `at`. Entries are in reading order, so
    /// only those in `at`'s chapter need their exact position.
    fn current(&self, env: &Env<'_>, at: Locator) -> usize {
        if let Some(headings) = &self.headings {
            return headings.iter().rposition(|h| h.at <= at).unwrap_or(0);
        }
        let Some(document) = &self.document else {
            return 0;
        };
        let mut focus = 0;
        for index in 0..self.len {
            let Some(entry) = document.toc_entry(env.storage(), index as u32) else {
                continue;
            };
            let before = match entry.at.section.cmp(&at.section) {
                core::cmp::Ordering::Less => true,
                core::cmp::Ordering::Equal => document
                    .toc_target(env.storage(), index as u32)
                    .is_some_and(|target| target <= at),
                core::cmp::Ordering::Greater => false,
            };
            if !before {
                break;
            }
            focus = index;
        }
        focus
    }

    /// Reads the entries of the page the focus is on.
    fn load_page(&mut self, env: &Env<'_>) {
        let rows = self.list.rows();
        self.first = self.list.page() * rows;
        let end = (self.first + rows).min(self.len);
        self.visible.clear();
        if let Some(headings) = &self.headings {
            self.visible.extend(
                headings[self.first.min(headings.len())..end]
                    .iter()
                    .cloned(),
            );
        } else if let Some(document) = &self.document {
            for index in self.first..end {
                let entry = document.toc_entry(env.storage(), index as u32);
                self.visible.push(entry.unwrap_or(TocEntry {
                    title: String::from("?"),
                    level: 0,
                    at: Locator::START,
                }));
            }
        }
    }

    fn choose(&self, index: usize, cx: &mut Cx<'_, '_>) {
        let Some(document) = &self.document else {
            return;
        };
        let target = match &self.headings {
            Some(headings) => headings.get(index).map(|h| h.at),
            None => document.toc_target(cx.env().storage(), index as u32),
        };
        if let Some(at) = target {
            cx.command(Command::Back);
            cx.command(events::GOTO.emit(events::Goto::to(self.path.clone(), at)));
        }
    }

    fn moved(&mut self, change: ListChange, cx: &mut Cx<'_, '_>) {
        if change == ListChange::Page {
            let env = *cx.env();
            self.load_page(&env);
        }
        change.request(cx);
    }
}

impl TocScreen {
    /// Reads on looking for headings; `true` when done.
    fn scan_step(&mut self, env: &Env<'_>) -> bool {
        let (Some(document), Some((next, found))) = (&self.document, self.scan.as_mut()) else {
            return true;
        };
        let mut at = *next;
        for _ in 0..STEP_BLOCKS {
            if at.section >= document.sections() || found.len() >= MAX_HEADINGS {
                return true;
            }
            match document.block_at(env.storage(), at) {
                Ok(Some(block)) => {
                    if let paperpaw_sdk::document::BlockKind::Heading(level) = block.kind {
                        let title: String =
                            block.text.split_whitespace().collect::<Vec<_>>().join(" ");
                        let title: String = title.chars().take(MAX_TITLE).collect();
                        if !title.is_empty() {
                            found.push(TocEntry {
                                title,
                                level,
                                at: Locator::new(at.section, block.start),
                            });
                        }
                    }
                    at = Locator::new(at.section, block.end.max(at.offset + 1));
                }
                Ok(None) | Err(_) => at = Locator::new(at.section + 1, 0),
            }
        }
        *next = at;
        false
    }
}

impl Screen for TocScreen {
    /// Turned or resized: laid out anew for the size, the focus on its
    /// entry, the entries of the page read again; the headings still
    /// being looked for go on.
    fn on_resize(&mut self, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let (style, header, fresh) = Self::frame(&env);
        // The focus, whether it shows and the page stay with the list.
        self.list.set_area(fresh.area());
        self.list.set_row(fresh.row());
        (self.style, self.header) = (style, header);
        self.load_page(&env);
        cx.request_redraw();
        Handled::Yes
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        if self.scan.is_some() {
            let now = cx.env().now();
            cx.wake_at(now);
        }
    }

    fn on_timer(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if !self.scan_step(&env) {
            cx.wake_at(env.now().add_millis(5));
            return;
        }
        let Some((_, found)) = self.scan.take() else {
            return;
        };
        let found = normalized(found);
        save_headings(&env, &self.path, &found);
        if found.len() > self.len {
            self.len = found.len();
            self.headings = Some(found);
            self.list.set_focus(0);
            self.load_page(&env);
        }
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.len;
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(
            &mut ui,
            env.i18n().tr("reader.toc"),
            None,
            self.list.paging(len),
        );
        if len == 0 || self.scan.is_some() && self.headings.is_none() && len <= 1 {
            let message = if self.scan.is_some() {
                "reader.toc.finding"
            } else {
                "reader.no-toc"
            };
            ui.message(self.list.area(), env.i18n().tr(message));
            return;
        }
        let Some(font) = style.font(env) else {
            return;
        };
        let (first, visible) = (self.first, &self.visible);
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let Some(entry) = index.checked_sub(first).and_then(|i| visible.get(i)) else {
                return;
            };
            let color = style.focus_ink(focused);
            let indent = style.margin() as f32 * (1.0 + 2.0 * f32::from(entry.level.min(4)));
            let width = f32::from(rect.width) - indent - style.margin() as f32;
            let title = ui::fit(&font, style.text(), &entry.title, width);
            ui::Label::new(&title)
                .px(style.text())
                .color(color)
                .draw_from(ui, indent, rect);
        });
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.len;
        let rows = self.list.rows() as isize;
        let a = action;
        if *a == actions::UP || *a == actions::LEFT || *a == actions::PREVIOUS {
            let change = self.list.move_focus(-1, len);
            self.moved(change, cx);
        } else if *a == actions::DOWN || *a == actions::RIGHT || *a == actions::NEXT {
            let change = self.list.move_focus(1, len);
            self.moved(change, cx);
        } else if *a == actions::NEXT_PAGE {
            let change = self.list.move_focus(rows, len);
            self.moved(change, cx);
        } else if *a == actions::PREV_PAGE {
            let change = self.list.move_focus(-rows, len);
            self.moved(change, cx);
        } else if *a == actions::CONFIRM {
            self.choose(self.list.focus(), cx);
        } else {
            return Handled::No;
        }
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let env = *cx.env();
        let paging = self.list.paging(self.len);
        if let Some(action) = self.header.tap(&env, &self.style, gesture.start, paging) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        match self.list.hit(gesture.start, self.len) {
            Some(index) => {
                self.choose(index, cx);
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}
