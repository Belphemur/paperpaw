//! Searching a book for text.
//!
//! The search screen reads the book block by block in the background, so
//! the list fills while one waits and input stays responsive; at most
//! [`MAX_HITS`] hits are kept, each with a short snippet. Choosing a hit
//! emits `core.goto` with the start and end of the match, so the
//! reader shows its page and marks it.

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::document::{Block, Document, Locator};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, FontHandle, Gesture, GestureKind, Handled, Id, Params,
    RefreshMode, Screen,
};

use paperpaw_sdk::builtin::events;

/// Most hits kept; more are counted as "100+".
const MAX_HITS: usize = 100;
/// Blocks read per background step.
const BLOCKS_PER_STEP: usize = 24;
/// Characters of context before and after a match in its snippet.
const BEFORE: usize = 24;
const AFTER: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    start: Locator,
    end: Locator,
    percent: u8,
    /// The text around the match as its row shows it (see [`with_hit`]).
    snippet: String,
}

pub(crate) struct SearchScreen {
    path: String,
    query: String,
    /// The query in lower case, by character.
    needle: Vec<char>,
    document: Option<Box<dyn Document>>,
    /// Where reading continues; `None` when the search is done.
    next: Option<Locator>,
    hits: Vec<Hit>,
    /// Hits were left out after [`MAX_HITS`].
    more: bool,
    /// Progress shown in the header, in steps of ten percent.
    shown_percent: u8,
    style: UiStyle,
    header: ui::Header,
    list: ListView,
}

impl SearchScreen {
    /// Parameters: `path` (the book) and `text` (what to find).
    pub(crate) fn open(env: &Env<'_>, params: &Params) -> Self {
        let path = params.get(params::PATH).unwrap_or_default().to_string();
        let query = params
            .get(params::TEXT)
            .unwrap_or_default()
            .trim()
            .to_string();
        let needle: Vec<char> = query.chars().map(lower).collect();
        let options = crate::screen::open_options(env);
        let document = env.formats().open_with(env.storage(), &path, &options).ok();
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let area = paperpaw_sdk::Rect::new(0, top, display.width, display.height - top);
        let next = (document.is_some() && !needle.is_empty()).then_some(Locator::START);
        Self {
            path,
            query,
            needle,
            document,
            next,
            hits: Vec::new(),
            more: false,
            shown_percent: 0,
            list: ListView::new(area, style.noted_row()).focus_on_keys(env),
            style,
            header,
        }
    }

    /// Reads the next blocks; returns whether hits on the visible page
    /// were added.
    fn step(&mut self, env: &Env<'_>) -> bool {
        let (Some(document), Some(mut at)) = (&self.document, self.next) else {
            return false;
        };
        let first_new = self.hits.len();
        let font = self.style.font(env);
        // The row's title: the list's width less the margins and the gap
        // kept before a value.
        let width = f32::from(self.list.area().width) - 4.0 * self.style.margin() as f32;
        // The text as the page shows it (see `paperpaw_sdk::filters`).
        let language = document.metadata().language.as_deref().unwrap_or_default();
        let filtered = paperpaw_sdk::reading::FilteredDocument::new(document.as_ref(), env)
            .in_book(&self.path, language);
        for _ in 0..BLOCKS_PER_STEP {
            match filtered.block_at(env.storage(), at) {
                Ok(Some(block)) => {
                    for (start, end) in find(&block.text, &self.needle) {
                        if self.hits.len() == MAX_HITS {
                            self.more = true;
                            break;
                        }
                        let start_at = Locator::new(at.section, block.source_offset(start));
                        self.hits.push(Hit {
                            start: start_at,
                            end: Locator::new(at.section, block.source_offset(end)),
                            percent: (document.progress(start_at) * 100.0) as u8,
                            snippet: {
                                let shown = snippet(&block, start, end);
                                let at = snippet_at(&block.text, start, end);
                                match &font {
                                    Some(font) => {
                                        with_hit(font, self.style.text(), &shown, at, width)
                                            .into_owned()
                                    }
                                    None => shown,
                                }
                            },
                        });
                    }
                    if self.more {
                        self.next = None;
                        break;
                    }
                    at = Locator::new(at.section, block.end.max(at.offset + 1));
                }
                // The end of a section, or one that cannot be read.
                Ok(None) | Err(_) => {
                    if at.section + 1 >= document.sections() {
                        self.next = None;
                        break;
                    }
                    at = Locator::new(at.section + 1, 0);
                }
            }
        }
        if self.next.is_some() {
            self.next = Some(at);
        }
        let rows = self.list.rows();
        let page_end = (self.list.page() + 1) * rows;
        self.hits.len() > first_new && first_new < page_end
    }

    fn percent(&self) -> u8 {
        match (&self.document, self.next) {
            (Some(document), Some(at)) => (document.progress(at) * 100.0) as u8,
            _ => 100,
        }
    }

    fn header_note(&self, env: &Env<'_>) -> String {
        if self.next.is_some() {
            return env.i18n().format(
                "reader.search.searching",
                &[("percent", Arg::Int(i64::from(self.shown_percent)))],
            );
        }
        let count = self.hits.len() as i64;
        let args = [("count", Arg::Int(count))];
        if self.more {
            env.i18n().format("reader.search.more-hits", &args)
        } else {
            env.i18n().plural("reader.search.hits", count, &args)
        }
    }

    fn open_hit(&self, index: usize, cx: &mut Cx<'_, '_>) {
        if let Some(hit) = self.hits.get(index) {
            cx.command(Command::Back);
            cx.command(
                events::GOTO.emit(events::Goto::to(self.path.clone(), hit.start).marking(hit.end)),
            );
        }
    }
}

/// The lower case of `c` (its first character, which suffices to compare).
fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// Byte ranges of `needle` in `text`, ignoring case.
fn find(text: &str, needle: &[char]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    // Walked in place: a block's characters are not collected.
    let mut at = 0;
    while let Some(c) = text[at..].chars().next() {
        if let Some(length) = match_at(&text[at..], needle) {
            found.push((at, at + length));
            at += length;
        } else {
            at += c.len_utf8();
        }
    }
    found
}

/// The length in bytes of `needle` at the start of `text`, ignoring case;
/// `None` where it is not there.
fn match_at(text: &str, needle: &[char]) -> Option<usize> {
    let mut chars = text.chars();
    for &n in needle {
        if lower(chars.next()?) != n {
            return None;
        }
    }
    Some(text.len() - chars.as_str().len())
}

/// The match with some context, on one line.
fn snippet(block: &Block, start: usize, end: usize) -> String {
    let (from, to) = snippet_range(&block.text, start, end);
    let text = &block.text;
    let mut out = String::new();
    if from > 0 {
        out.push('…');
    }
    out.extend(
        text[from..to]
            .chars()
            .map(|c| if c.is_whitespace() { ' ' } else { c }),
    );
    if to < text.len() {
        out.push('…');
    }
    out
}

/// Where the match `start..end` is in its snippet (see [`snippet`]), in
/// characters.
fn snippet_at(text: &str, start: usize, end: usize) -> (usize, usize) {
    let (from, _) = snippet_range(text, start, end);
    let before = usize::from(from > 0) + text[from..start].chars().count();
    (before, before + text[start..end].chars().count())
}

/// The part of `text` a snippet of the match `start..end` shows.
fn snippet_range(text: &str, start: usize, end: usize) -> (usize, usize) {
    let from = text[..start]
        .char_indices()
        .rev()
        .nth(BEFORE - 1)
        .map_or(0, |(i, _)| i);
    let to = text[end..]
        .char_indices()
        .nth(AFTER)
        .map_or(text.len(), |(i, _)| end + i);
    (from, to)
}

/// `snippet` shown in `width`: where it does not fit up to the end of the
/// match at characters `at`, it begins later (at a word, after `…`), so
/// that the word found is in the row and not cut off with the context
/// after it.
fn with_hit<'s>(
    font: &FontHandle<'_>,
    size: f32,
    snippet: &'s str,
    at: (usize, usize),
    width: f32,
) -> Cow<'s, str> {
    let byte = |chars: usize| {
        snippet
            .char_indices()
            .nth(chars)
            .map_or(snippet.len(), |(i, _)| i)
    };
    let (start, end) = (byte(at.0), byte(at.1));
    // Room for an ellipsis at either end.
    let room = width - 2.0 * font.measure(size, "…");
    let mut used = 0.0;
    let mut from = end;
    for (i, c) in snippet[..end].char_indices().rev() {
        used += font.advance(size, c);
        if used > room {
            break;
        }
        from = i;
    }
    if from == 0 || snippet[..from].chars().all(|c| c == '…') {
        return Cow::Borrowed(snippet);
    }
    // A match wider than the row: from its start, cut at the end.
    from = from.min(start);
    // From the start of a word, before the match.
    if let Some(space) = snippet.get(from..start).and_then(|s| s.find(' ')) {
        from += space;
    }
    Cow::Owned(format!("…{}", snippet[from..].trim_start()))
}

impl Screen for SearchScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.hits.len();
        let title = format!("„{}“", self.query);
        let note = self.header_note(env);
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, &title, Some(&note), self.list.paging(len));
        if len == 0 {
            let message = if self.next.is_some() {
                env.i18n().tr("reader.search.wait").to_string()
            } else {
                env.i18n()
                    .format("reader.search.none", &[("query", Arg::Str(&self.query))])
            };
            ui.message(self.list.area(), &message);
            return;
        }
        let hits = &self.hits;
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let hit = &hits[index];
            let note = env.i18n().format(
                "reader.percent",
                &[("percent", Arg::Int(i64::from(hit.percent)))],
            );
            ui::Row::plain(&hit.snippet)
                .note(&note)
                .focused(focused)
                .draw(ui, rect);
        });
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        if self.next.is_some() {
            let now = cx.env().now();
            cx.wake_at(now);
        }
    }

    fn on_timer(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let visible = self.step(&env);
        if visible {
            cx.request_redraw_area(self.list.area());
        }
        let percent = self.percent() / 10 * 10;
        if self.next.is_none() || percent != self.shown_percent {
            self.shown_percent = percent;
            cx.request_redraw_area(self.header.area());
            if self.hits.is_empty() {
                cx.request_redraw_area(self.list.area());
            }
        }
        cx.request_refresh(RefreshMode::Fast);
        if self.next.is_some() {
            cx.wake_at(env.now().add_millis(2));
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.hits.len();
        let rows = self.list.rows() as isize;
        let a = action;
        let delta = if let Some(delta) = ui::list_step(a, rows) {
            delta
        } else if *a == actions::CONFIRM {
            self.open_hit(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        let change = self.list.move_focus_in(cx.env(), delta, len);
        change.request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let paging = self.list.paging(self.hits.len());
        let tapped = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging);
        if let Some(action) = tapped {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        match self.list.hit(gesture.start, self.hits.len()) {
            Some(index) => {
                self.list.set_focus(index);
                self.open_hit(index, cx);
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ignoring_case() {
        let needle: Vec<char> = "äpfel".chars().map(lower).collect();
        let text = "Äpfel und Birnen; zwei ÄPFEL.";
        let found = find(text, &needle);
        assert_eq!(found.len(), 2);
        assert_eq!(&text[found[0].0..found[0].1], "Äpfel");
        assert_eq!(&text[found[1].0..found[1].1], "ÄPFEL");
        assert!(find(text, &[]).is_empty());
        // Hits do not overlap, and one at the very end is found.
        let needle: Vec<char> = "aa".chars().collect();
        assert_eq!(find("aaa", &needle), [(0, 2)]);
        assert_eq!(find("xaA", &needle), [(1, 3)]);
        assert!(find("a", &needle).is_empty());
    }
}

#[cfg(test)]
mod matching_tests;
