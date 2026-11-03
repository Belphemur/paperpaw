//! The rhythm of reading: the last days as a heat map, when in the day
//! the reading fell, and how fast and how long one reads now.
//!
//! All of it is what the log of page stays already keeps, seen as
//! CrossPoint saw it: the minutes of each day as a shade, the reading
//! gathered into morning, afternoon, evening and night, and the words a
//! minute and the length of a session of what was read on this device
//! (see `pace`).

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::ui::{self, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Color, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Rect, Screen,
};

use crate::pace::SharedPace;
use crate::store::{self, Day};
use crate::{chart, fmt, widget};

/// The days the heat map shows, as many as CrossPoint kept minutes of.
const DAYS: i32 = 91;

/// When in the day reading fell: the parts of the day CrossPoint summed
/// its reading into, by the hour the local day had then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bucket {
    Morning,
    Afternoon,
    Evening,
    Night,
}

/// The part of the day an hour of it falls in: morning from five to
/// twelve, afternoon to five in the evening, evening to nine, the night
/// for the rest.
pub(crate) fn bucket(hour: u32) -> Bucket {
    match hour {
        5..=11 => Bucket::Morning,
        12..=16 => Bucket::Afternoon,
        17..=20 => Bucket::Evening,
        _ => Bucket::Night,
    }
}

/// The gray of a day's shade, as dark as the calendar shades its days
/// (see `widget::calendar`): darker the longer one read.
const SHADES: [u8; 5] = [255, 220, 180, 120, 40];

/// The words a minute and the length of a session, as read here, as a
/// line, or nothing where neither is known.
fn pace_line(env: &Env<'_>, pace: &SharedPace) -> Option<String> {
    let pace = pace.borrow();
    let wpm = pace.wpm(None);
    let session = pace.avg_session(env.storage(), None);
    let i18n = env.i18n();
    match (wpm, session) {
        (Some(wpm), Some(session)) => Some(i18n.format(
            "stats.rhythm.pace",
            &[
                (
                    "wpm",
                    Arg::Str(&i18n.plural("stats.wpm", i64::from(wpm), &[])),
                ),
                ("session", Arg::Str(&fmt::duration(env, u64::from(session)))),
            ],
        )),
        (Some(wpm), None) => Some(i18n.plural("stats.wpm", i64::from(wpm), &[])),
        (None, Some(session)) => Some(i18n.format(
            "stats.rhythm.session",
            &[("session", Arg::Str(&fmt::duration(env, u64::from(session))))],
        )),
        (None, None) => None,
    }
}

/// The reading of the last days, read when the screen is opened.
pub(crate) struct Rhythm {
    style: UiStyle,
    header: ui::Header,
    heat: Rect,
    bars: Rect,
    line: Rect,
    days: Vec<Day>,
    /// The seconds read in each part of the day, in the days shown.
    buckets: [u32; 4],
    /// How fast and how long one reads, read when the screen was opened
    /// (working it out again reads the log whole, and is not for every
    /// repaint).
    pace_line: Option<String>,
    /// Today, if the device knows the time.
    today: Option<i32>,
}

/// The local hour a moment in Unix seconds was in.
fn hour_of(env: &Env<'_>, unix: u32) -> u32 {
    (paperpaw_sdk::time::local_seconds(env, i64::from(unix)).rem_euclid(86_400) / 3600) as u32
}

impl Rhythm {
    pub(crate) fn new(env: &Env<'_>, pace: SharedPace) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let row = style.row();
        let margin = style.margin();
        let top = header.area().height + style.gap();
        // The heat map takes the room above the parts of the day; the
        // line of pace goes at the very bottom.
        let line = Rect::new(0, display.height.saturating_sub(row), display.width, row);
        let bars = Rect::new(
            margin,
            display
                .height
                .saturating_sub(row * 4 + line.height + style.gap() * 2),
            display.width - 2 * margin,
            row * 4,
        );
        let heat = Rect::new(
            margin,
            top + style.gap(),
            display.width - 2 * margin,
            bars.y.saturating_sub(top + style.gap() * 2),
        );
        let today = fmt::today(env);
        let days = store::days(env.storage(), |unix| fmt::day_of(env, unix));
        // The reading of the days shown, in the parts of the day. The
        // bounds reach a day to either side, for the days of the
        // statistics are local days and a moment's Unix seconds need
        // not fall in the day shown from any other zone; what is said
        // here is the day each stay was read on.
        let mut buckets = [0u32; 4];
        if let Some(today) = today {
            let first = today - DAYS + 1;
            let near = |day: i32, out: i64| {
                (fmt::unix_near(day) + out).clamp(0, i64::from(u32::MAX)) as u32
            };
            store::scan_range(
                env.storage(),
                near(first, -86_400),
                near(today + 1, 86_400),
                |stat| {
                    let day = fmt::day_of(env, stat.start);
                    if !(first..=today).contains(&day) {
                        return;
                    }
                    buckets[usize::from(bucket(hour_of(env, stat.start)) as u8)] +=
                        u32::from(stat.duration);
                },
            );
        }
        // How fast and how long one reads, once: drawing says it again
        // without reading for it.
        let pace_line = pace_line(env, &pace);
        Self {
            style,
            header,
            heat,
            bars,
            line,
            days,
            buckets,
            pace_line,
            today,
        }
    }

    /// The words a minute and the length of a session, as read here, as
    /// a line, or nothing where neither is known.
    fn pace_line(&self, _env: &Env<'_>) -> Option<String> {
        self.pace_line.clone()
    }
    /// The reading of one day, shaded the way the widgets shade theirs.
    fn level(&self, env: &Env<'_>, day: i32) -> u8 {
        widget::level(fmt::seconds_on(&self.days, day), widget::goal(env))
    }
}

impl Screen for Rhythm {
    fn hint(&self, action: &Id, _: &Env<'_>) -> paperpaw_sdk::Hint {
        // Only back does anything here.
        paperpaw_sdk::Hint::only_back(action)
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, env.i18n().tr("stats.rhythm"), None, None);
        let Some(today) = self.today else {
            // The heat map and the parts of the day need the time of day.
            let area = Rect::new(
                0,
                self.header.area().height,
                ui.env().device().display.width,
                self.line.y.saturating_sub(self.header.area().height),
            );
            ui.message(area, env.i18n().tr("stats.no-clock"));
            return;
        };
        self.draw_heat(&mut ui, today);
        self.draw_bars(&mut ui);
        if let Some(text) = self.pace_line(env) {
            let Some(font) = ui.font() else {
                return;
            };
            let w = font.measure(style.small(), &text);
            let x = (f32::from(self.line.width) - w).max(0.0) / 2.0;
            ui::Label::new(&text)
                .px(style.small())
                .color(style.faint(ui::Ink::Note))
                .draw_from(&mut ui, x, self.line);
        }
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

impl Rhythm {
    /// The last days as a heat map: a column a week, a row a day of the
    /// week, each day shaded by how long one read, the newest at the
    /// right, today framed.
    fn draw_heat(&self, ui: &mut ui::Ui<'_>, today: i32) {
        let style = self.style;
        let env = ui.env();
        let Some(font) = ui.font() else {
            return;
        };
        let area = self.heat;
        let from = fmt::week_start(env);
        // A name of a weekday beside its row, where they fit.
        let label_w = (0..7)
            .map(|w| font.measure(style.small(), widget::initial(env, w)))
            .fold(0.0f32, f32::max) as u16
            + style.gap() / 2;
        let across = area.width.saturating_sub(label_w);
        let cell = (across / 13).min(area.height / 7);
        if cell < 3 {
            return;
        }
        let weeks = i32::from((across / cell).max(1));
        // The first day shown: the start of the week the oldest shown
        // day is in, so that the weeks line up.
        let weekday = i32::from(fmt::date_of(today).weekday);
        let this_week = today - (weekday - i32::from(from)).rem_euclid(7);
        let first = this_week - 7 * (weeks - 1);
        let left =
            area.x + (across - cell * u16::try_from(weeks).unwrap_or(u16::MAX)) / 2 + label_w;
        // The days of the week beside their rows, where the rows are
        // tall enough to read.
        let every = if cell >= font.line_metrics(style.small()).line_height as u16 {
            1
        } else {
            2
        };
        for row in (0..7u8).step_by(every) {
            let name = widget::initial(env, (from + row) % 7);
            let r = Rect::new(area.x, area.y + cell * u16::from(row), label_w, cell);
            ui::Label::new(name)
                .px(style.small())
                .color(style.faint(ui::Ink::Muted))
                .draw_from(ui, f32::from(area.x), r);
        }
        for index in 0..(weeks as usize) * 7 {
            let day = first + index as i32;
            // A week column holds the seven days of one weekday: the
            // row is the day of the week, the column the week.
            if !(first..=today).contains(&day) || day < today - DAYS + 1 {
                continue;
            }
            let (col, row) = (index / 7, index % 7);
            let pad = (cell / 8).max(1);
            let at = Rect::new(
                left + col as u16 * cell + pad,
                area.y + row as u16 * cell + pad,
                cell.saturating_sub(2 * pad),
                cell.saturating_sub(2 * pad),
            );
            let level = self.level(env, day);
            if level > 0 {
                ui.canvas()
                    .fill(at, Color::gray(SHADES[usize::from(level.min(4))]));
            }
            if day == today {
                ui.canvas()
                    .stroke(at, u32::from(style.line()).max(1), Color::BLACK);
            }
        }
    }

    /// When in the day the reading fell: the parts of the day as bars,
    /// the fullest named above them.
    fn draw_bars(&self, ui: &mut ui::Ui<'_>) {
        let env = ui.env();
        let names: [String; 4] = [
            String::from(env.i18n().tr("stats.rhythm.morning")),
            String::from(env.i18n().tr("stats.rhythm.afternoon")),
            String::from(env.i18n().tr("stats.rhythm.evening")),
            String::from(env.i18n().tr("stats.rhythm.night")),
        ];
        let labels: Vec<&str> = names.iter().map(String::as_str).collect();
        let values: Vec<u32> = self.buckets.to_vec();
        let fullest = self
            .buckets
            .iter()
            .enumerate()
            .max_by_key(|(_, s)| **s)
            .filter(|(_, s)| **s > 0)
            .map(|(i, _)| fmt::duration(env, u64::from(self.buckets[i])))
            .unwrap_or_default();
        chart::bars(ui, self.bars, &values, &labels, (None, &fullest));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parts_of_the_day_are_as_crosspoint_summed_them() {
        // Morning from five to twelve, afternoon to five, evening to
        // nine, the night for the rest.
        for (hour, want) in [
            (0, Bucket::Night),
            (4, Bucket::Night),
            (5, Bucket::Morning),
            (11, Bucket::Morning),
            (12, Bucket::Afternoon),
            (16, Bucket::Afternoon),
            (17, Bucket::Evening),
            (20, Bucket::Evening),
            (21, Bucket::Night),
            (23, Bucket::Night),
        ] {
            assert_eq!(bucket(hour), want, "{hour}");
        }
    }
}
