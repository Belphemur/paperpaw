//! Books: the book being read with its cover and progress, what to read
//! next, a series and the favourites as covers, a highlight of one's own,
//! the library in numbers; and the battery.
//!
//! Each reads what the library service knows ([`library`]: the books
//! opened last, a book's details, the favourites, the catalog of all
//! books) and the annotations ([`annotations`]), never a reader's own
//! files, and shows only covers looked up before: no book is opened to
//! show it. Covers are made small for where they go once a widget is
//! placed ([`Covers`]), and drawn from memory in every band. What takes longer to read (the catalog's books, highlights) is
//! read when the widget shows and kept until it may have changed
//! ([`Generation`]); widgets of books are not read each minute.
//!
//! [`annotations`]: paperpaw_sdk::annotations

mod battery;
mod favourites;
mod highlight;
mod next;
mod numbers;
mod progress;
mod reading;
mod series;
mod shelf;
#[cfg(test)]
pub(crate) mod tests;

use alloc::borrow::Cow;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt::Write as _;

use paperpaw_sdk::builtin::events;
use paperpaw_sdk::document::ImageRef;
use paperpaw_sdk::library::{self, catalog};
use paperpaw_sdk::ui::{self, Thumbnail, UiStyle};
use paperpaw_sdk::widgets::{self, Align, WidgetLook, line_height};
use paperpaw_sdk::{
    Arg, Canvas, Color, Command, Env, FontHandle, Point, Rect, RegisterError, Registrar,
    SettingSpec, Storage,
};

pub(crate) use crate::{frame_height, today};

pub(crate) fn register(r: &mut Registrar) -> Result<(), RegisterError> {
    r.add_translations(Cow::Borrowed(include_str!("../i18n/books.en.ini")))?;
    r.add_translations(Cow::Borrowed(include_str!("../i18n/books.de.ini")))?;
    // What was read of the catalog is read again once it was made anew
    // (books came, went or moved), a book changed (finished, a favourite)
    // or its annotations did.
    let generation = Generation::default();
    for event in [
        events::CATALOG_CHANGED.id(),
        events::BOOK_CHANGED.id(),
        events::LIBRARY_CHANGED.id(),
        events::BOOK_MOVED.id(),
        events::BOOK_REMOVED.id(),
        events::BOOK_FINISHED.id(),
        events::ANNOTATIONS_CHANGED.id(),
    ] {
        let generation = generation.clone();
        r.subscribe(
            event.clone(),
            alloc::boxed::Box::new(move |_, _, _| generation.next()),
        )?;
    }
    progress::register(r)?;
    reading::register(r)?;
    next::register(r, &generation)?;
    series::register(r, &generation)?;
    favourites::register(r)?;
    highlight::register(r, &generation)?;
    numbers::register(r, &generation)?;
    battery::register(r)?;
    Ok(())
}

/// Counts the changes to the library (books came, went, moved, were
/// finished; annotations changed): what widgets keep of it is read again
/// when it moved on.
#[derive(Debug, Clone, Default)]
pub(crate) struct Generation(Rc<Cell<u32>>);

impl Generation {
    fn next(&self) {
        self.0.set(self.0.get().wrapping_add(1));
    }

    pub(crate) fn get(&self) -> u32 {
        self.0.get()
    }
}

// ---- Books ------------------------------------------------------------------

/// A book as the widgets show it, from what the library knows of it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Book {
    pub path: String,
    pub title: String,
    pub author: String,
    pub series: String,
    /// Its number in the series, as the book says it (`3`, `2.5`).
    pub number: String,
    /// Percent read, once opened.
    pub progress: Option<u8>,
    /// Its cover, if it was looked up and is there.
    pub cover: Option<ImageRef>,
}

impl Book {
    /// The book at `path`, if it is there.
    pub(crate) fn of(storage: &dyn Storage, path: &str) -> Option<Book> {
        let details = match library::book(storage, path) {
            Some(details) => details,
            None if storage.exists(path) => library::Book::default(),
            None => return None,
        };
        Some(Book {
            title: details.shown_title(path).into_owned(),
            path: path.to_string(),
            author: details.author.clone().unwrap_or_default(),
            series: details.series.clone().unwrap_or_default(),
            number: details.series_index.clone().unwrap_or_default(),
            progress: details.progress,
            cover: details.cover.filter(|c| storage.exists(&c.path)),
        })
    }

    /// Opens it for reading.
    pub(crate) fn open(&self) -> Command {
        open(&self.path)
    }

    /// Read to the end.
    pub(crate) fn finished(&self) -> bool {
        self.progress == Some(100)
    }

    /// Percent read, 0 before it was opened.
    pub(crate) fn percent(&self) -> u8 {
        self.progress.unwrap_or(0)
    }
}

/// Opens the book at `path` for reading.
pub(crate) fn open(path: &str) -> Command {
    paperpaw_sdk::readers::open(path)
}

/// The book being read: the one opened last that is still there.
pub(crate) fn current(storage: &dyn Storage) -> Option<Book> {
    library::opened_books(storage)
        .iter()
        .take(MAX_LOOKED_AT)
        .find_map(|path| Book::of(storage, path))
}

/// Books looked at, at most, to find one there.
const MAX_LOOKED_AT: usize = 8;

/// Whether the library made its catalog (with the books' series and
/// authors), which the kernel makes.
pub(crate) fn has_catalog(env: &Env<'_>) -> bool {
    matches!(
        library::service(env).catalog_state(env),
        catalog::State::Ready { .. }
    )
}

/// What tells the catalog changed, or the status of a book in it.
pub(crate) fn catalog_stamp(env: &Env<'_>) -> u64 {
    match library::service(env).catalog_state(env) {
        catalog::State::Ready { stamp } => stamp,
        _ => 0,
    }
}

/// Calls `f` with every book of the library: from the catalog where
/// there is one, else from the library folder (each book by its path and
/// file name alone). Nothing is held: the catalog is read in pieces.
pub(crate) fn each_book(env: &Env<'_>, f: &mut dyn FnMut(catalog::Record)) {
    if has_catalog(env) {
        return library::service(env).each(env, &mut |_, record| f(record));
    }
    let folder = library::folder(env);
    library::books_in(env.storage(), env.formats(), folder, &mut |path| {
        f(catalog::Record::of(path, &library::Book::default()));
    });
}

/// A number of a series as a number to sort by: `2`, `2.5`, `10`
/// (in thousandths, [`catalog::series_order`]; books without one last).
pub(crate) fn series_rank(number: &str) -> u64 {
    match catalog::series_order(number) {
        u32::MAX => u64::MAX,
        order => u64::from(order),
    }
}

// ---- Days and chance -----------------------------------------------------------

/// A number that looks random, the same for the same `seed`.
pub(crate) fn mix(seed: u64) -> u64 {
    let mut z = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The hash of a book's path, as the library names its files by.
pub(crate) fn path_hash(path: &str) -> u64 {
    u64::from_str_radix(&library::key(path), 16).unwrap_or(0)
}

// ---- Settings ------------------------------------------------------------------

/// Registers a widget's settings `group` with its settings, both kept in
/// the program (where its text sits, its frame, how large its text is,
/// the parts it shows and the rest).
pub(crate) fn add_settings(
    r: &mut Registrar,
    group: &'static paperpaw_sdk::SettingGroupSpec,
    settings: &'static [SettingSpec],
) -> Result<(), RegisterError> {
    r.add_setting_group(group)?;
    r.add_settings(settings)
}

// The value of widget `id`'s choice `key`.
pub(crate) fn chosen<'e>(env: &Env<'e>, id: &str, key: &str) -> &'e str {
    env.settings().text(&format!("{id}.{key}"))
}

/// Widget `id`'s number `key`, at least 1.
pub(crate) fn count(env: &Env<'_>, id: &str, key: &str) -> usize {
    env.settings().int(&format!("{id}.{key}")).max(1) as usize
}

/// The values of a thickness setting: thin, medium, thick.
pub(crate) const THICKNESSES: [&str; 3] = [
    "widgets.books.thin",
    "widgets.books.medium",
    "widgets.books.thick",
];

/// A ring's or a bar's thickness for something `size` pixels across, as
/// widget `id`'s thickness setting says.
pub(crate) fn thickness(env: &Env<'_>, id: &str, size: u16) -> u32 {
    let share = match chosen(env, id, "thickness") {
        "widgets.books.thin" => 16,
        "widgets.books.thick" => 6,
        _ => 10,
    };
    (u32::from(size) / share)
        .max(u32::from(UiStyle::new(env).line()) * 2)
        .max(2)
}

// ---- Drawing -----------------------------------------------------------------

/// The text size widget `id`'s size setting says (for "as large as
/// fits": the large one, where the widget works out nothing better).
pub(crate) fn text_size(env: &Env<'_>, id: &str) -> f32 {
    widgets::size(env, id).text(&UiStyle::new(env))
}

/// `area` less `by` pixels at the top.
pub(crate) fn below(area: Rect, by: u16) -> Rect {
    let by = by.min(area.height);
    Rect::new(area.x, area.y + by, area.width, area.height - by)
}

/// A line of text `size` pixels large at the top of `area`, placed as
/// `align` says and shortened to fit; returns its height.
#[allow(
    clippy::too_many_arguments,
    reason = "its inputs are separate things; a struct of them would serve this call alone"
)]
pub(crate) fn draw_line(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    font: &FontHandle<'_>,
    size: f32,
    area: Rect,
    align: Align,
    text: &str,
    color: Color,
) -> u16 {
    let height = line_height(env, size).min(area.height);
    if text.is_empty() || height == 0 {
        return 0;
    }
    let text = ui::fit(font, size, text, f32::from(area.width));
    let x = align.x(area, font.measure(size, &text));
    let row = Rect::new(area.x, area.y, area.width, height);
    ui::draw_label(canvas, font, size, x, row, &text, color);
    height
}

/// What a line of [`Lines`] is to its widget: how it gives way where
/// there is no room.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// Above the rest ("Next up"): shortened where it is wider, left out
    /// first where it is too high.
    Caption,
    /// The book's title: smaller where it is wider, down to the body
    /// size, then in a few lines broken between words where there is
    /// room, else shortened with "…".
    Title,
    /// What the widget is there for (a page, a percentage): always
    /// shown, smaller down to the body size, then shortened.
    Main,
    /// A detail (the author, the series): shortened where it is wider,
    /// left out after the caption and the title's other lines.
    Note,
}

/// A line of [`Lines`].
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Line<'t> {
    pub size: f32,
    pub text: Cow<'t, str>,
    pub color: Color,
    pub role: Role,
}

/// Lines of text, each with its size, colour and [`Role`], drawn one
/// below the other together in the middle of an area, a little room
/// between them (the lines of a title closer, as a heading's,
/// `widgets::heading_line_height`). What does not fit gives way as its
/// role says: no line breaks inside a word, a caption before the title's
/// other lines before a note.
pub(crate) struct Lines<'t> {
    pub lines: Vec<Line<'t>>,
    /// The title as given, its size and the lines it shows in, while it
    /// shows in more than one.
    title: Option<(f32, Cow<'t, str>, usize)>,
    /// Where made to fit is kept for the frames that follow, if the
    /// widget keeps it.
    memo: Option<&'t FitMemo>,
}

/// What [`Lines::fit`] made of lines, by the room they had and what they
/// said: the same lines in the same room need not be worked out in every
/// frame (measuring a title at one size after another, in more and more
/// lines, takes long on a small device). Kept by the widget that draws,
/// a few entries.
#[derive(Debug, Default)]
pub(crate) struct FitMemo(RefCell<Vec<Kept>>);

/// One entry of a [`FitMemo`].
#[derive(Debug)]
struct Kept {
    key: String,
    lines: Vec<Line<'static>>,
    title: Option<(f32, String, usize)>,
}

/// The most entries a [`FitMemo`] keeps: the rooms one frame fits lines
/// in (a layout tried, then drawn).
const MEMO_ENTRIES: usize = 6;

impl<'t> Lines<'t> {
    pub(crate) fn new() -> Self {
        Self {
            lines: Vec::new(),
            title: None,
            memo: None,
        }
    }

    /// Lines made to fit by way of `memo`.
    pub(crate) fn remembering(memo: &'t FitMemo) -> Self {
        Self {
            memo: Some(memo),
            ..Self::new()
        }
    }

    /// What tells that lines fit as before: the room, the sizes and the
    /// font they are measured by, and the lines.
    fn key(&self, env: &Env<'_>, width: u16, height: u16) -> String {
        let style = UiStyle::new(env);
        let mut key = String::new();
        let font = style.font(env).map(|f| f.id().clone());
        let _ = write!(
            key,
            "{width}x{height} {} {} {} {:?}",
            style.small().to_bits(),
            style.text().to_bits(),
            style.gap(),
            font.as_ref().map(paperpaw_sdk::Id::as_str)
        );
        for line in &self.lines {
            let _ = write!(
                key,
                "|{:?} {} {:?} {}",
                line.role,
                line.size.to_bits(),
                line.color,
                line.text
            );
        }
        key
    }

    /// Adds a line, if it says anything.
    pub(crate) fn add(
        &mut self,
        role: Role,
        size: f32,
        text: impl Into<Cow<'t, str>>,
        color: Color,
    ) {
        let text = text.into();
        if !text.is_empty() {
            self.lines.push(Line {
                size,
                text,
                color,
                role,
            });
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The height they take, the room between them included.
    pub(crate) fn height(&self, env: &Env<'_>) -> u16 {
        let Some(last) = self.lines.last() else {
            return 0;
        };
        let steps: u16 = (1..self.lines.len()).map(|i| self.step(env, i)).sum();
        steps + line_height(env, last.size)
    }

    /// From the top of line `i - 1` to the top of line `i`: a heading's
    /// line between the lines of a title, else a line and the room
    /// between lines.
    fn step(&self, env: &Env<'_>, i: usize) -> u16 {
        let (above, line) = (&self.lines[i - 1], &self.lines[i]);
        if above.role == Role::Title && line.role == Role::Title {
            widgets::heading_line_height(env, above.size)
        } else {
            line_height(env, above.size) + Self::gap(env)
        }
    }

    fn gap(env: &Env<'_>) -> u16 {
        UiStyle::new(env).gap() / 4
    }

    /// The lines of a title at `s` that `free` pixels hold, a heading's
    /// line apart.
    fn title_rows(env: &Env<'_>, s: f32, free: u16) -> usize {
        let room = free + Self::gap(env);
        let first = line_height(env, s) + Self::gap(env);
        let apart = widgets::heading_line_height(env, s).max(1);
        usize::from(room >= first) + usize::from(room.saturating_sub(first) / apart)
    }

    /// Makes them fit `width` × `height`: each line on one line, smaller
    /// down to the body size where it is wider, a title still too wide in
    /// a few lines broken between words where they fit with the rest,
    /// else shortened with "…"; then, while they are too high, the
    /// caption left out, the title back in one line, the notes from the
    /// last, the main lines from the last. The title and the first line
    /// stay. Made again with the same room, nothing changes.
    pub(crate) fn fit(&mut self, env: &Env<'_>, width: u16, height: u16) {
        let Some(memo) = self.memo else {
            return self.fit_now(env, width, height);
        };
        let key = self.key(env, width, height);
        let kept = memo.0.borrow();
        if let Some(entry) = kept.iter().find(|entry| entry.key == key) {
            self.lines = entry
                .lines
                .iter()
                .map(|line| Line {
                    size: line.size,
                    text: Cow::Owned(line.text.to_string()),
                    color: line.color,
                    role: line.role,
                })
                .collect();
            self.title = entry
                .title
                .as_ref()
                .map(|(size, text, count)| (*size, Cow::Owned(text.clone()), *count));
            return;
        }
        drop(kept);
        self.fit_now(env, width, height);
        let mut kept = memo.0.borrow_mut();
        if kept.len() >= MEMO_ENTRIES {
            kept.remove(0);
        }
        kept.push(Kept {
            key,
            lines: self
                .lines
                .iter()
                .map(|line| Line {
                    size: line.size,
                    text: Cow::Owned(line.text.to_string()),
                    color: line.color,
                    role: line.role,
                })
                .collect(),
            title: self
                .title
                .as_ref()
                .map(|(size, text, count)| (*size, text.to_string(), *count)),
        });
    }

    /// [`fit`](Self::fit), without asking what was kept.
    fn fit_now(&mut self, env: &Env<'_>, width: u16, height: u16) {
        let style = UiStyle::new(env);
        let Some(font) = style.font(env) else {
            return;
        };
        let room = f32::from(width);
        let fits = |size: f32, text: &str| font.measure(size, text) <= room;
        let mut two = None;
        for (i, line) in self.lines.iter_mut().enumerate() {
            if fits(line.size, &line.text) {
                continue;
            }
            // Smaller, down to the small size, before it is shortened.
            let least = line.size.min(style.small());
            let text = &line.text;
            if let Some(size) = widgets::largest_size(line.size, least, |s| fits(s, text)) {
                line.size = size;
            } else if line.role == Role::Title && self.title.is_none() && two.is_none() {
                two = Some(i);
            } else {
                line.text = Cow::Owned(ui::fit(&font, least, &line.text, room));
                line.size = least;
            }
        }
        if let Some(i) = two {
            let (size, text, color) = {
                let line = &self.lines[i];
                (line.size, line.text.clone(), line.color)
            };
            let least = size.min(style.small());
            // In lines broken between words, where they fit with the main
            // lines and the notes (the caption gives way to them).
            let others: u16 = self
                .lines
                .iter()
                .filter(|l| matches!(l.role, Role::Main | Role::Note))
                .map(|l| line_height(env, l.size) + Self::gap(env))
                .sum();
            let free = height.saturating_sub(others);
            let wrap = |s: f32, most: usize| {
                let rows = Self::title_rows(env, s, free);
                let lines = wrap_words(&font, s, &text, width, most.min(rows));
                (lines.len() > 1).then_some(lines)
            };
            // As few lines as can be, then as large as can be.
            let split = (2..=MOST_TITLE_LINES)
                .find_map(|most| {
                    widgets::largest_size(size, least, |s| wrap(s, most).is_some())
                        .and_then(|s| wrap(s, most).map(|lines| (s, lines)))
                })
                .or_else(|| {
                    // Not whole in any: in lines up to the word it has
                    // to shorten, where that shows more than one line.
                    let rows = Self::title_rows(env, least, free).min(MOST_TITLE_LINES);
                    let lines = cut_lines(&font, least, &text, width, rows);
                    (lines.len() > 1).then_some((least, lines))
                });
            match split {
                Some((s, lines)) => {
                    let count = lines.len();
                    self.lines.splice(
                        i..=i,
                        lines.into_iter().map(|text| Line {
                            size: s,
                            text: Cow::Owned(text),
                            color,
                            role: Role::Title,
                        }),
                    );
                    self.title = Some((size, text, count));
                }
                None => {
                    let line = &mut self.lines[i];
                    line.text = Cow::Owned(ui::fit(&font, least, &text, room));
                    line.size = least;
                }
            }
        }
        while self.lines.len() > 1 && self.height(env) > height {
            if let Some(i) = self.lines.iter().position(|l| l.role == Role::Caption) {
                self.lines.remove(i);
            } else if let Some((size, text, count)) = self.title.take() {
                // The title back in one line, shortened.
                if let Some(i) = self.lines.iter().position(|l| l.role == Role::Title) {
                    self.lines.drain(i + 1..(i + count).min(self.lines.len()));
                    let least = size.min(style.text());
                    let line = &mut self.lines[i];
                    line.text = Cow::Owned(ui::fit(&font, least, &text, room));
                    line.size = least;
                }
            } else if let Some(i) = self.lines.iter().rposition(|l| l.role == Role::Note) {
                self.lines.remove(i);
            } else if let Some(i) = self
                .lines
                .iter()
                .rposition(|l| l.role == Role::Main)
                .filter(|&i| i > 0)
            {
                self.lines.remove(i);
            } else {
                break;
            }
        }
    }

    /// Draws them in `area` as `align` says, made to fit it first
    /// ([`fit`](Self::fit)); returns where they went.
    pub(crate) fn draw(
        &mut self,
        canvas: &mut Canvas<'_>,
        env: &Env<'_>,
        area: Rect,
        align: Align,
    ) -> Rect {
        let style = UiStyle::new(env);
        let Some(font) = style.font(env) else {
            return area;
        };
        self.fit(env, area.width, area.height);
        let total = self.height(env).min(area.height);
        let top = area.y + (area.height - total) / 2;
        let mut y = top;
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                y += self.step(env, i);
            }
            if y >= area.y + area.height {
                break;
            }
            let row = Rect::new(area.x, y, area.width, area.y + area.height - y);
            draw_line(
                canvas, env, &font, line.size, row, align, &line.text, line.color,
            );
        }
        Rect::new(area.x, top, area.width, total)
    }
}

/// The most lines a title is broken into.
const MOST_TITLE_LINES: usize = 4;

/// The letters `text` shows before a "…" that ends it.
fn shown_letters(text: &str) -> usize {
    text.trim_end_matches('…').trim_end().chars().count()
}

/// `text` in at most `most` lines `width` wide at `size`, broken between
/// words: in two where it needs two, where the longer line is shortest;
/// else as many words in each as fit. Where it does not fit so (a word
/// wider than a line, more lines than `most`), on one line, to be
/// shortened where it is drawn: never broken inside a word.
pub(crate) fn wrap_words(
    font: &FontHandle<'_>,
    size: f32,
    text: &str,
    width: u16,
    most: usize,
) -> Vec<String> {
    let width = f32::from(width);
    let fits = |text: &str| font.measure(size, text) <= width;
    let one = || alloc::vec![String::from(text)];
    if most < 2 || fits(text) {
        return one();
    }
    if let Some((head, tail)) = two_lines(font, size, text, width) {
        return alloc::vec![head, tail];
    }
    let lines = title_lines(font, size, text, width, most);
    // Whole only: every word there, none shortened.
    let words = |text: &str| text.split_whitespace().count();
    let shown: usize = lines.iter().map(|l| words(l)).sum();
    if shown == words(text) && lines.iter().all(|l| fits(l) && !l.ends_with('…')) {
        lines
    } else {
        one()
    }
}

/// `text` in at most `most` lines `width` wide at `size` where it does
/// not fit them whole: broken between words up to the word it has to
/// shorten ("Alice's / Adventu…"), where that shows more of it than one
/// line shortened; else that one line.
pub(crate) fn cut_lines(
    font: &FontHandle<'_>,
    size: f32,
    text: &str,
    width: u16,
    most: usize,
) -> Vec<String> {
    let room = f32::from(width);
    let one = ui::fit(font, size, text, room);
    if most < 2 {
        return alloc::vec![one];
    }
    let lines = title_lines(font, size, text, room, most);
    let letters = |lines: &[String]| lines.iter().map(|l| shown_letters(l)).sum::<usize>();
    if lines.len() > 1 && letters(&lines) > shown_letters(&one) {
        lines
    } else {
        alloc::vec![one]
    }
}

/// `text` in two lines `width` pixels wide at `size`, broken between
/// words, the longer line as short as can be; `None` where no break
/// makes both fit.
pub(crate) fn two_lines(
    font: &FontHandle<'_>,
    size: f32,
    text: &str,
    width: f32,
) -> Option<(String, String)> {
    let head = |at: usize| text[..at].trim_end();
    let tail = |at: usize| text[at..].trim_start();
    text.match_indices(' ')
        .map(|(at, _)| at)
        .filter(|&at| !head(at).is_empty() && !tail(at).is_empty())
        .map(|at| {
            let widest = font
                .measure(size, head(at))
                .max(font.measure(size, tail(at)));
            (at, widest)
        })
        .filter(|(_, widest)| *widest <= width)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(at, _)| (String::from(head(at)), String::from(tail(at))))
}

/// The least width of a cover, so that one is still told from another:
/// smaller ones are left out.
pub(crate) fn least_cover_width(env: &Env<'_>) -> u16 {
    env.metrics().mm(5).max(24) as u16
}

/// The size of a cover fitted into `width` × `height`: as its picture
/// is, or 2:3 for a book without one.
pub(crate) fn cover_size(book: &Book, width: u16, height: u16) -> (u16, u16) {
    match &book.cover {
        Some(cover) => ui::fit_image(cover, width, height),
        None => {
            let w = width.min(height * 2 / 3);
            (w, (w * 3 / 2).min(height))
        }
    }
}

/// Covers made small for the places a widget draws them at, when it is
/// placed ([`Widget::place`](paperpaw_sdk::widgets::Widget::place)) or
/// its books change: each picture read once, then drawn from memory in
/// every band of every frame.
#[derive(Debug, Default)]
pub(crate) struct Covers(Vec<(Rect, Thumbnail)>);

impl Covers {
    /// The covers of the books in `places`, each made small for the
    /// place it goes ([`cover_size`] large), as the panel shows it.
    pub(crate) fn make<'b>(
        env: &Env<'_>,
        places: impl IntoIterator<Item = (&'b Book, Rect)>,
    ) -> Self {
        let format = env.device().display.format;
        let made = places
            .into_iter()
            .filter_map(|(book, at)| {
                let cover = book.cover.as_ref()?;
                let small = Thumbnail::new(env.storage(), cover, at.width, at.height, format)?;
                Some((at, small))
            })
            .collect();
        Self(made)
    }

    /// The cover made for `at`, if one was.
    fn at(&self, at: Rect) -> Option<&Thumbnail> {
        self.0
            .iter()
            .find(|(place, small)| {
                *place == at && (small.width(), small.height()) == (at.width, at.height)
            })
            .map(|(_, small)| small)
    }
}

/// Draws `book`'s cover into `at` (as large as [`cover_size`] says): as
/// made small in `covers`, else from its picture (where there was no
/// memory to make it small); a book without one as a frame with its
/// title.
pub(crate) fn draw_cover(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    look: &WidgetLook,
    at: Rect,
    book: &Book,
    covers: &Covers,
) {
    if at.width < 2 || at.height < 2 {
        return;
    }
    match (&book.cover, covers.at(at)) {
        (Some(_), Some(small)) => small.draw(canvas, at.x, at.y),
        (Some(cover), None) => ui::draw_small_image(canvas, env.storage(), cover, at),
        (None, _) => draw_placeholder(canvas, env, look, at, &book.title),
    }
}

/// A book without a cover: a frame with its title inside.
fn draw_placeholder(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    look: &WidgetLook,
    at: Rect,
    title: &str,
) {
    let style = UiStyle::new(env);
    let mut ui = ui::Ui::new(canvas, env, &style);
    ui.canvas()
        .stroke(at, u32::from(style.line()).max(2), look.ink);
    let pad = (style.gap() / 2).min(at.width / 6);
    let inner = Rect::new(
        at.x + pad,
        at.y + pad,
        at.width.saturating_sub(2 * pad),
        at.height.saturating_sub(2 * pad),
    );
    if inner.width < 8 || inner.height < 8 {
        return;
    }
    let size = style.small().min(f32::from(inner.width) / 3.0);
    let Some(font) = style.font(env) else {
        return;
    };
    let line = line_height(env, size).max(1);
    let fits = usize::from(inner.height / line);
    let lines = cover_lines(&font, size, title, f32::from(inner.width), fits);
    for (n, text) in lines.iter().enumerate() {
        let row = Rect::new(inner.x, inner.y + n as u16 * line, inner.width, line);
        ui::Label::new(text)
            .px(size)
            .color(look.ink)
            .draw_from(&mut ui, f32::from(inner.x), row);
    }
}

/// `title` in at most `most` lines `width` pixels wide at `size`, broken
/// between words: a word too long for a line alone is shortened with
/// "…" and ends it (one cut to a letter or two is left out), as does the
/// last line where there is more.
pub(crate) fn title_lines(
    font: &FontHandle<'_>,
    size: f32,
    title: &str,
    width: f32,
    most: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let fits = |text: &str| font.measure(size, text) <= width;
    for word in title.split_whitespace() {
        if let Some(last) = lines.last_mut() {
            let joined = format!("{last} {word}");
            if fits(&joined) {
                *last = joined;
                continue;
            }
        }
        if lines.len() == most {
            // More than there is room for: the last line ends in "…"
            // where that leaves a few letters of the word.
            if let Some(last) = lines.last_mut() {
                let cut = ui::fit(font, size, &format!("{last} {word}"), width);
                *last = if shown_letters(&cut) > last.chars().count() + 3 {
                    cut
                } else {
                    ui::fit(font, size, &format!("{last} …"), width)
                };
            }
            break;
        }
        if !fits(word) {
            let short = ui::fit(font, size, word, width);
            if short.chars().count() > 3 {
                lines.push(short);
            }
            break;
        }
        lines.push(String::from(word));
    }
    lines
}

/// `title` in at most `most` lines `width` pixels wide at `size` on a
/// cover without a picture: broken between words, whole words only (the
/// title is below the cover too), as many as fit.
fn cover_lines(
    font: &FontHandle<'_>,
    size: f32,
    title: &str,
    width: f32,
    most: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let fits = |text: &str| font.measure(size, text) <= width;
    for word in title.split_whitespace() {
        if let Some(last) = lines.last_mut() {
            let joined = format!("{last} {word}");
            if fits(&joined) {
                *last = joined;
                continue;
            }
        }
        if lines.len() == most || !fits(word) {
            // No room left, or a word wider than the cover: whole words
            // only on a cover (the title is below it).
            break;
        }
        lines.push(String::from(word));
    }
    lines
}

/// "16 %".
pub(crate) fn percent_text(env: &Env<'_>, percent: u8) -> String {
    env.i18n().format(
        "widgets.books.percent",
        &[("percent", Arg::Int(i64::from(percent)))],
    )
}

/// Draws a progress ring of `radius` around `center`, `thickness` wide:
/// a fine line all round and over it the part done, `percent`, from
/// twelve o'clock with round ends.
pub(crate) fn draw_ring(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    look: &WidgetLook,
    center: Point,
    radius: u32,
    thickness: u32,
    percent: u8,
) {
    let style = UiStyle::new(env);
    let thickness = thickness.min(radius);
    if thickness == 0 {
        // No room for a ring.
        return;
    }
    let fine = u32::from(style.line()).clamp(1, thickness);
    // The fine line in the middle of the ring's width.
    let track = radius - (thickness - fine) / 2;
    canvas.stroke_circle(center, track, fine, look.ink);
    let percent = u32::from(percent.min(100));
    if percent == 0 {
        return;
    }
    let sweep = percent * 360 / 100;
    canvas.fill_ring(center, radius, thickness, 0, sweep.max(1), look.ink);
    if sweep < 360 && thickness >= 4 {
        // Round ends.
        let middle = (radius - thickness / 2) as i32;
        let end = (thickness / 2).max(1);
        for degrees in [0, sweep as i32] {
            let at = paperpaw_sdk::round::point_at(center, middle, degrees);
            canvas.fill_circle(at, end, look.ink);
        }
    }
}

/// Draws the percentage inside a ring of `radius` around `center`: the
/// number large, the sign small beside it.
pub(crate) fn draw_ring_percent(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    look: &WidgetLook,
    center: Point,
    inner: u32,
    percent: u8,
) {
    let style = UiStyle::new(env);
    let Some(font) = style.font(env) else {
        return;
    };
    let number = env.i18n().number(i64::from(percent));
    let room = inner as f32 * 1.5;
    let least = style.small() * 0.6;
    let sign = |size: f32| (size * 0.45).max(least * 0.8);
    let width = |size: f32| font.measure(size, &number) + font.measure(sign(size), "%");
    // As large as fits inside, measured as drawn (a font of a few sizes
    // draws the nearest of its own).
    let size = widgets::largest_size((inner as f32 * 0.8).max(least), least, |s| {
        width(s) <= room
            && font
                .ink(s, &number)
                .is_none_or(|(top, bottom)| (bottom - top) as u32 <= inner)
    })
    .unwrap_or(least);
    let total = width(size);
    let x = center.x as f32 - total / 2.0;
    // The digits' ink in the middle of the ring.
    let band = Rect::new(
        0,
        (center.y - inner as i32).max(0) as u16,
        1,
        (inner * 2) as u16,
    );
    let baseline = widgets::ink_baseline(&font, size, &number, band);
    font.draw(canvas, size, x, baseline, &number, look.ink);
    let x = x + font.measure(size, &number);
    font.draw(canvas, sign(size), x, baseline, "%", look.faint);
}

/// Draws the percentage alone, as large as fits `area` and placed as
/// `align` says: where a ring would be too small to read.
pub(crate) fn draw_percent_alone(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    look: &WidgetLook,
    area: Rect,
    align: Align,
    percent: u8,
) {
    let style = UiStyle::new(env);
    let text = percent_text(env, percent);
    let max = widgets::largest_size(style.title() * 2.0, style.small(), |s| {
        line_height(env, s) <= area.height
    })
    .unwrap_or(style.small());
    widgets::draw_line(
        canvas,
        env,
        area,
        align,
        max,
        style.small(),
        &text,
        look.ink,
    );
}

/// Draws a bar `percent` full in `area`: a fine line along it, the part
/// done thick, with round ends.
pub(crate) fn draw_bar(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    look: &WidgetLook,
    area: Rect,
    percent: u8,
) {
    let style = UiStyle::new(env);
    let h = area.height.max(2);
    let fine = (style.line()).clamp(1, h);
    let middle = area.y + (h - fine) / 2;
    canvas.fill(Rect::new(area.x, middle, area.width, fine), look.ink);
    let done = (u32::from(area.width) * u32::from(percent.min(100)) / 100) as u16;
    if done > 0 {
        let done = done.max(h);
        canvas.fill_round(
            Rect::new(area.x, area.y, done.min(area.width), h),
            u32::from(h / 2),
            look.ink,
        );
    }
}

/// Draws the message `key` in the middle of `look`'s area (e.g. "Nothing
/// read yet").
pub(crate) fn draw_message(canvas: &mut Canvas<'_>, env: &Env<'_>, look: &WidgetLook, key: &str) {
    widgets::draw_message(canvas, env, look, env.i18n().tr(key));
}

/// The caption of widget `id` (key `caption`), drawn at the top of its
/// area if it shows one; returns the area left below it.
pub(crate) fn caption(
    canvas: &mut Canvas<'_>,
    env: &Env<'_>,
    id: &str,
    look: &WidgetLook,
    text: &str,
) -> Rect {
    let rest = below_caption(env, id, look);
    if rest != look.area {
        widgets::draw_caption(canvas, env, id, look, text);
    }
    rest
}

/// The area [`caption`] leaves below it, without drawing it.
pub(crate) fn below_caption(env: &Env<'_>, id: &str, look: &WidgetLook) -> Rect {
    if !widgets::shows(env, id, "caption") || look.area.height < 3 * caption_room(env) {
        return look.area;
    }
    below(look.area, caption_room(env))
}

/// The height of a caption, if widget `id` shows one.
pub(crate) fn caption_room(env: &Env<'_>) -> u16 {
    widgets::caption_height(env)
}

/// Where a box `width` wide sits in `area` as `align` says.
pub(crate) fn place_x(area: Rect, width: u16, align: Align) -> u16 {
    align.x(area, f32::from(width)) as u16
}
