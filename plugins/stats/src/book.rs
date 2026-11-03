//! One book's numbers: time, pages, pace, time left, days, and its
//! reading time on the last days it was read.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::params;
use paperpaw_sdk::ui::{self, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, Screen,
};

use crate::pace::SharedPace;
use crate::store::{self, Book};
use crate::totals::{self, BookTotals};
use crate::{books, chart, fmt};

/// Reading days in the chart.
const CHART_DAYS: usize = 10;

pub(crate) struct BookView {
    style: UiStyle,
    header: ui::Header,
    book: Option<Book>,
    totals: BookTotals,
    /// How fast the book was read and how long a stretch of reading
    /// lasted, read when the screen was opened.
    pace: Vec<(String, String)>,
}

impl BookView {
    pub(crate) fn new(env: &Env<'_>, params: &Params, pace: SharedPace) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let id = params.get(params::ID).and_then(|id| id.parse::<u16>().ok());
        let found = id.and_then(|id| store::books(env.storage()).into_iter().find(|b| b.id == id));
        let totals = found.as_ref().map_or_else(BookTotals::default, |book| {
            totals::of_book(env.storage(), book, |unix| fmt::day_of(env, unix))
        });
        let pace = pace_rows(env, found.as_ref(), &pace);
        Self {
            style,
            header,
            book: found,
            totals,
            pace,
        }
    }

    /// The rows: what and its value.
    fn rows(&self, env: &Env<'_>, book: &Book) -> Vec<(String, String)> {
        let t = &self.totals;
        let mut rows = Vec::new();
        let tr = |key: &str| env.i18n().tr(key).to_string();
        let authors = books::authors(&book.authors);
        if !authors.is_empty() {
            rows.push((tr("stats.authors"), authors));
        }
        rows.push((tr("stats.time-read"), fmt::duration(env, t.seconds)));
        let pages = if book.pages > 0 {
            env.i18n().format(
                "stats.pages-of",
                &[
                    ("read", Arg::Int(i64::from(t.pages_read))),
                    ("pages", Arg::Int(i64::from(book.pages))),
                ],
            )
        } else {
            env.i18n()
                .plural("stats.pages", i64::from(t.pages_read), &[])
        };
        rows.push((tr("stats.pages-read"), pages));
        if t.pages_read > 0 {
            let per_page = t.seconds / u64::from(t.pages_read);
            rows.push((
                tr("stats.pace"),
                env.i18n()
                    .format("stats.per-page", &[("seconds", Arg::Int(per_page as i64))]),
            ));
            let left = book.pages.saturating_sub(t.pages_read);
            if book.pages > 0 && left > 0 {
                rows.push((
                    tr("stats.time-left"),
                    fmt::duration(env, per_page * u64::from(left)),
                ));
            }
        }
        rows.push((
            tr("stats.reading-days-title"),
            env.i18n().plural("stats.days", t.days.len() as i64, &[]),
        ));
        if let (Some((first, _)), Some((last, _))) = (t.days.first(), t.days.last()) {
            rows.push((tr("stats.first-read"), fmt::date(env, *first)));
            if last != first {
                rows.push((tr("stats.last-read-title"), fmt::date(env, *last)));
            }
        }
        rows
    }
}

/// How fast the book was read and how long a stretch of reading lasted,
/// as far as there is to say of it, as rows of what and its value.
fn pace_rows(env: &Env<'_>, book: Option<&Book>, pace: &SharedPace) -> Vec<(String, String)> {
    let Some(book) = book else {
        return Vec::new();
    };
    let pace = pace.borrow();
    let mut rows = Vec::new();
    if let Some(wpm) = pace.wpm(Some(book.id)) {
        rows.push((
            env.i18n().tr("stats.reading-speed").to_string(),
            env.i18n().plural("stats.wpm", i64::from(wpm), &[]),
        ));
    }
    if let Some(session) = pace.avg_session(env.storage(), Some(book.id)) {
        rows.push((
            env.i18n().tr("stats.session-avg").to_string(),
            fmt::duration(env, u64::from(session)),
        ));
    }
    rows
}

impl Screen for BookView {
    fn hint(&self, action: &Id, _: &Env<'_>) -> paperpaw_sdk::Hint {
        // Only back does anything here.
        paperpaw_sdk::Hint::only_back(action)
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let display = env.device().display;
        let title = self.book.as_ref().map_or("", |b| b.title.as_str());
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(&mut ui, title, None, None);
        let top = self.header.area().height + style.gap();
        let Some(book) = &self.book else {
            let area = Rect::new(0, top, display.width, display.height - top);
            ui.message(area, env.i18n().tr("stats.no-books"));
            return;
        };
        let row = style.row() * 4 / 5;
        let mut rows = self.rows(env, book);
        rows.extend(
            self.pace
                .iter()
                .map(|(what, value)| (what.clone(), value.clone())),
        );
        for (n, (label, value)) in rows.iter().enumerate() {
            let rect = Rect::new(0, top + row * n as u16, display.width, row);
            ui::Row::value(label, value).draw(&mut ui, rect);
        }
        // The last days it was read.
        let chart_top = top + row * rows.len() as u16 + style.gap() * 2;
        let height = display.height.saturating_sub(chart_top + style.gap());
        if height < style.row() * 2 || self.totals.days.is_empty() {
            return;
        }
        if ui.font().is_none() {
            return;
        }
        let margin = style.margin();
        let title_h = (style.small() * 1.6) as u16;
        let label_area = Rect::new(margin, chart_top, display.width - 2 * margin, title_h);
        let label = env.i18n().tr("stats.by-day");
        ui::Label::new(label)
            .px(style.small())
            .color(style.faint(ui::Ink::Muted))
            .draw_from(&mut ui, f32::from(margin), label_area);
        let days = &self.totals.days[self.totals.days.len().saturating_sub(CHART_DAYS)..];
        let values: Vec<u32> = days.iter().map(|(_, s)| *s).collect();
        let dates: Vec<String> = days
            .iter()
            .map(|(d, _)| {
                let date = fmt::date_of(*d);
                env.i18n().format(
                    "stats.day-month",
                    &[
                        ("day", Arg::Int(i64::from(date.day))),
                        ("month", Arg::Int(i64::from(date.month))),
                    ],
                )
            })
            .collect();
        let labels: Vec<&str> = dates.iter().map(String::as_str).collect();
        let max = values.iter().copied().max().unwrap_or(0);
        let top_label = fmt::duration(env, u64::from(max));
        let area = Rect::new(
            margin,
            chart_top + title_h,
            display.width - 2 * margin,
            height.saturating_sub(title_h).min(style.row() * 5),
        );
        chart::bars(&mut ui, area, &values, &labels, (None, &top_label));
    }

    fn on_action(&mut self, _: &Id, _: &mut Cx<'_, '_>) -> Handled {
        Handled::No
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        if let Some(action) = self.header.tap(cx.env(), &self.style, gesture.start, None) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        Handled::No
    }
}
