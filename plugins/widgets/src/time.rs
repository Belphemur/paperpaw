//! Clocks, calendars and the year:
//!
//! - analogue clock: a round face with hands (`analogue`);
//! - word clock: the time in words, as text or as a grid of letters
//!   (`word_clock`);
//! - clock in tiles: each digit on a tile of its own (`tiles`);
//! - world clock: the time in up to three other places (`world`);
//! - calendar: this month as a grid, and the next (`calendar`);
//! - the year so far: a bar or a ring (`year`);
//! - countdown: the days until a date the reader sets (`countdown`);
//! - moon phase: today's moon, drawn (`moon`);
//! - day and date: the weekday and the date large (`day`).
//!
//! Each has a settings group of its own, with its frame, where its text
//! sits, and switches for what it shows. They read nothing from the
//! storage: they draw from the clock and their settings.

mod analogue;
mod calendar;
mod countdown;
mod dates;
mod day;
mod moon;
mod tiles;
mod word_clock;
mod world;
mod year;

use alloc::borrow::Cow;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{params, slots};
use paperpaw_sdk::time::{self, LocalTime};
use paperpaw_sdk::ui::UiStyle;
use paperpaw_sdk::widgets::{self, Align, Drawn, Widget, WidgetSpec, WidgetTarget};
use paperpaw_sdk::{
    Arg, Canvas, Color, Command, Env, FontHandle, Id, Params, Rect, RegisterError, Registrar,
    SettingSpec,
};

use crate::frame_height;

pub(crate) fn register(r: &mut Registrar) -> Result<(), RegisterError> {
    r.add_translations(Cow::Borrowed(include_str!("../i18n/time.en.ini")))?;
    r.add_translations(Cow::Borrowed(include_str!("../i18n/time.de.ini")))?;
    analogue::register(r)?;
    word_clock::register(r)?;
    tiles::register(r)?;
    world::register(r)?;
    calendar::register(r)?;
    year::register(r)?;
    countdown::register(r)?;
    moon::register(r)?;
    day::register(r)?;
    Ok(())
}

/// Adds widget `id`, drawn by `drawn`, to the kind "Time and date", with
/// its settings group (its settings come from a list kept in the
/// program, each module's `SETTINGS`).
fn add(
    r: &mut Registrar,
    group: &'static paperpaw_sdk::SettingGroupSpec,
    drawn: Drawn,
) -> Result<(), RegisterError> {
    let title: &'static str = &group.title;
    r.add_widget(
        WidgetSpec::drawn(group.id.clone(), title, drawn).category(widgets::categories::TIME),
    )?;
    r.add_setting_group(group)
}

/// Adds widget `id` like [`add`], with a tap on it opening its settings:
/// for widgets that show what the reader chose (a date, places).
fn add_settable(
    r: &mut Registrar,
    group: &'static paperpaw_sdk::SettingGroupSpec,
    drawn: Drawn,
) -> Result<(), RegisterError> {
    let (id, title): (&Id, &'static str) = (&group.id, &group.title);
    let widget = id.clone();
    r.add_widget(
        WidgetSpec::new(id.clone(), title, move |_| {
            Box::new(Settable {
                id: widget.clone(),
                drawn,
                shown: None,
            })
        })
        .category(widgets::categories::TIME),
    )?;
    r.add_setting_group(group)
}

/// A widget that draws from the clock and its settings, which a tap
/// opens.
struct Settable {
    id: Id,
    drawn: Drawn,
    shown: Option<String>,
}

impl Widget for Settable {
    fn update(&mut self, env: &Env<'_>) -> bool {
        let Some(shows) = self.drawn.shows else {
            return false;
        };
        let now = Some(shows(env));
        let changed = self.shown.is_some() && now != self.shown;
        self.shown = now;
        changed
    }

    fn height(&self, env: &Env<'_>, width: u16) -> u16 {
        (self.drawn.height)(env, width)
    }

    fn draw(&self, canvas: &mut Canvas<'_>, env: &Env<'_>, area: Rect) {
        (self.drawn.draw)(canvas, env, area);
    }

    fn targets(&self, _: &Env<'_>, area: Rect) -> Vec<WidgetTarget> {
        let params = Params::new().with(params::ID, alloc::string::ToString::to_string(&self.id));
        alloc::vec![WidgetTarget::new(
            area,
            Command::OpenSlot(slots::SETTINGS, params)
        )]
    }
}

/// The name of the value of `widget`'s choice `key`: the part after the
/// last dot (e.g. `circle` of `widgets.mark.circle`).
fn chosen_name<'a>(env: &'a Env<'_>, widget: &str, key: &str) -> &'a str {
    let value = env.settings().text(&format!("{widget}.{key}"));
    value.rsplit('.').next().unwrap_or(value)
}

/// The colour lighter of the look's ink and paper, and the darker one:
/// e.g. the lit and the dark part of the moon.
fn light_and_dark(look: &widgets::WidgetLook) -> (Color, Color) {
    if look.ink.luma() > look.paper.luma() {
        (look.ink, look.paper)
    } else {
        (look.paper, look.ink)
    }
}

/// The name of weekday `n` (0: Sunday).
fn weekday_name<'a>(env: &'a Env<'_>, n: u8) -> &'a str {
    const KEYS: [&str; 7] = [
        "core.weekday.0",
        "core.weekday.1",
        "core.weekday.2",
        "core.weekday.3",
        "core.weekday.4",
        "core.weekday.5",
        "core.weekday.6",
    ];
    env.i18n().tr(KEYS[usize::from(n % 7)])
}

/// The name of month `n` (1 to 12).
fn month_name<'a>(env: &'a Env<'_>, n: u8) -> &'a str {
    const KEYS: [&str; 12] = [
        "core.month.1",
        "core.month.2",
        "core.month.3",
        "core.month.4",
        "core.month.5",
        "core.month.6",
        "core.month.7",
        "core.month.8",
        "core.month.9",
        "core.month.10",
        "core.month.11",
        "core.month.12",
    ];
    env.i18n().tr(KEYS[usize::from(n.clamp(1, 12) - 1)])
}

/// The short name of month `n` (1 to 12): "Oct".
fn month_short<'a>(env: &'a Env<'_>, n: u8) -> &'a str {
    const KEYS: [&str; 12] = [
        "core.month.short.1",
        "core.month.short.2",
        "core.month.short.3",
        "core.month.short.4",
        "core.month.short.5",
        "core.month.short.6",
        "core.month.short.7",
        "core.month.short.8",
        "core.month.short.9",
        "core.month.short.10",
        "core.month.short.11",
        "core.month.short.12",
    ];
    env.i18n().tr(KEYS[usize::from(n.clamp(1, 12) - 1)])
}

/// The short name of weekday `n` (0: Sunday): "Wed".
fn weekday_short<'a>(env: &'a Env<'_>, n: u8) -> &'a str {
    const KEYS: [&str; 7] = [
        "core.weekday.short.0",
        "core.weekday.short.1",
        "core.weekday.short.2",
        "core.weekday.short.3",
        "core.weekday.short.4",
        "core.weekday.short.5",
        "core.weekday.short.6",
    ];
    env.i18n().tr(KEYS[usize::from(n % 7)])
}

/// The values of a date format setting ([`date_format_spec`]).
const DATE_FORMATS: [&str; 4] = [
    "widgets.date-format.long",
    "widgets.date-format.weekday",
    "widgets.date-format.numeric",
    "widgets.date-format.iso",
];

/// A setting of how `widget` writes dates: "14 October 2026",
/// "Wednesday, 14 October", "14/10/2026" or "2026-10-14"; `id` is
/// `<widget>.date-format`.
const fn date_format_spec(id: &'static str, widget: Id, default: &'static str) -> SettingSpec {
    SettingSpec::choice(
        id,
        "widgets.setting.date-format",
        &DATE_FORMATS,
        default,
        widget,
    )
    .description("widgets.setting.date-format.help")
}

/// `year`-`month`-`day` written as `widget`'s date format says
/// ([`date_format_spec`]); `year` left out where `with_year` is not
/// set (but in numbers).
fn date_text(
    env: &Env<'_>,
    widget: &str,
    year: i32,
    month: u8,
    day: u8,
    with_year: bool,
) -> String {
    let month_name = month_name(env, month);
    let weekday = weekday_name(env, dates::weekday(year, month, day));
    let args = [
        ("day", Arg::Int(i64::from(day))),
        ("month", Arg::Str(month_name)),
        ("year", Arg::Int(i64::from(year))),
        ("weekday", Arg::Str(weekday)),
    ];
    match chosen_name(env, widget, "date-format") {
        "iso" => env
            .i18n()
            .localize_digits(&format!("{year:04}-{month:02}-{day:02}"))
            .into_owned(),
        "numeric" => env.i18n().format(
            "widgets.date.numeric",
            &[
                (
                    "day",
                    Arg::Str(&env.i18n().localize_digits(&format!("{day:02}"))),
                ),
                (
                    "month",
                    Arg::Str(&env.i18n().localize_digits(&format!("{month:02}"))),
                ),
                ("year", Arg::Int(i64::from(year))),
            ],
        ),
        "weekday" => env.i18n().format("core.date", &args),
        _ if with_year => env.i18n().format("widgets.date.long", &args),
        _ => env.i18n().format("widgets.date.short", &args),
    }
}

/// The time `t` as the reader wants to see it: `14:05` or `2:05 PM`
/// (setting `core.clock-format`).
fn clock_text(env: &Env<'_>, t: &LocalTime) -> String {
    time::clock_of(env, t.hour, t.minute)
}

/// `hour` (0 to 23) on a twelve-hour clock: 1 to 12.
fn twelve(hour: u8) -> u8 {
    match hour % 12 {
        0 => 12,
        h => h,
    }
}

/// Draws in `look`'s area that the time is not known yet, in place of
/// what a widget would show of it: on a device without a clock of its
/// own, until Wi-Fi sets it.
fn draw_no_time(canvas: &mut Canvas<'_>, env: &Env<'_>, look: &widgets::WidgetLook) {
    widgets::draw_message(canvas, env, look, env.i18n().tr("widgets.time.unknown"));
}

/// What a widget that shows the date shows: drawn again when the day
/// changes.
fn today_shows(env: &Env<'_>) -> String {
    time::local_time(env).map_or_else(String::new, |t| format!("{}-{}-{}", t.year, t.month, t.day))
}

/// What a widget that shows the time shows: drawn again each minute.
fn minute_shows(env: &Env<'_>) -> String {
    time::local_time(env).map_or_else(String::new, |t| {
        format!("{}-{}-{} {}:{}", t.year, t.month, t.day, t.hour, t.minute)
    })
}

/// Draws `text` `size` pixels large in the row `row`, placed as `align`
/// says, shortened with "…" where it is wider.
#[allow(
    clippy::too_many_arguments,
    reason = "its inputs are separate things; a struct of them would serve this call alone"
)]
fn label(
    canvas: &mut Canvas<'_>,
    font: &FontHandle<'_>,
    size: f32,
    align: Align,
    row: Rect,
    text: &str,
    color: Color,
) {
    let text = paperpaw_sdk::ui::fit(font, size, text, f32::from(row.width));
    let x = align.x(row, font.measure(size, &text));
    paperpaw_sdk::ui::draw_label(canvas, font, size, x, row, &text, color);
}

/// The size at which `text` is `width` pixels wide at most, and `size`
/// at most, measured as it is drawn ([`widgets::largest_size`]): a font
/// of a few sizes (a pack) gives one of its own that really fits. Where
/// none does, the smallest tried (drawn at the font's smallest).
fn fit_size(font: &FontHandle<'_>, size: f32, text: &str, width: f32) -> f32 {
    let least = (size * 0.1).max(1.0);
    widgets::largest_size(size, least, |s| font.measure(s, text) <= width).unwrap_or(least)
}

/// `text` broken between words into lines `width` pixels wide at `size`
/// (never inside a word), and whether a word is wider than a line alone.
pub(crate) fn break_words(
    font: &FontHandle<'_>,
    size: f32,
    text: &str,
    width: f32,
) -> (Vec<String>, bool) {
    let mut lines: Vec<String> = Vec::new();
    let mut too_wide = false;
    let mut line = String::new();
    for word in text.split_whitespace() {
        if font.measure(size, word) > width {
            too_wide = true;
        }
        if line.is_empty() {
            line.push_str(word);
            continue;
        }
        let longer = format!("{line} {word}");
        if font.measure(size, &longer) <= width {
            line = longer;
        } else {
            lines.push(core::mem::replace(&mut line, String::from(word)));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    (lines, too_wide)
}

/// `text` broken between words into lines `width` pixels wide at `size`,
/// `most` at most, the last one shortened with "…" where there is more;
/// a word wider than a line ends the text there, shortened.
pub(crate) fn wrap(
    font: &FontHandle<'_>,
    size: f32,
    text: &str,
    width: f32,
    most: usize,
) -> Vec<String> {
    let (mut lines, _) = break_words(font, size, text, width);
    let most = most.max(1);
    // Up to the first line with a word wider than a line: it ends there.
    if let Some(wide) = lines.iter().position(|l| font.measure(size, l) > width) {
        let rest = lines[wide..].join(" ");
        lines.truncate(wide);
        lines.push(rest);
    }
    if lines.len() > most {
        let rest = lines[most - 1..].join(" ");
        lines.truncate(most - 1);
        lines.push(rest);
    }
    if let Some(last) = lines.last_mut()
        && font.measure(size, last) > width
    {
        *last = paperpaw_sdk::ui::fit(font, size, last, width);
    }
    lines
}

/// `text` whole in `most` lines at most, `width` pixels wide and
/// `height` pixels high, broken between words: at the largest size from
/// `max` down to `min` at which it fits so, measured as drawn; `None`
/// where it does not even at `min` (then it is better left out, or
/// shortened, than broken in a word).
fn wrap_whole(
    font: &FontHandle<'_>,
    (max, min): (f32, f32),
    text: &str,
    (width, height): (f32, f32),
    most: usize,
) -> Option<(f32, Vec<String>)> {
    let size = widgets::largest_size(max, min, |size| {
        let (lines, too_wide) = break_words(font, size, text, width);
        !too_wide && lines.len() <= most.max(1) && line(font, size) * lines.len() as f32 <= height
    })?;
    Some((size, break_words(font, size, text, width).0))
}

/// `items` (e.g. "Day 287 of 365", "Week 42") in lines `width` pixels
/// wide at `size`, separated by " · " within a line, `most` lines at
/// most: an item is not broken where it can go whole to the next line,
/// and one wider than a line is broken between its words; what does not
/// fit is shortened with "…" on the last line.
fn pack(
    font: &FontHandle<'_>,
    size: f32,
    items: &[String],
    width: f32,
    most: usize,
) -> Vec<String> {
    let most = most.max(1);
    let mut lines: Vec<String> = Vec::new();
    // Each item's pieces: itself where it fits a line, else its lines.
    let pieces = items.iter().flat_map(|item| {
        if font.measure(size, item) <= width {
            alloc::vec![(item.clone(), true)]
        } else {
            let (parts, _) = break_words(font, size, item, width);
            let first = parts.len();
            parts
                .into_iter()
                .enumerate()
                .map(move |(n, p)| (p, n == 0 && first > 0))
                .collect()
        }
    });
    let mut cut = false;
    for (piece, starts_item) in pieces {
        let n = lines.len();
        if n > 0 {
            let sep = if starts_item { " · " } else { " " };
            let joined = format!("{}{sep}{piece}", lines[n - 1]);
            if font.measure(size, &joined) <= width {
                lines[n - 1] = joined;
                continue;
            }
            if n >= most {
                // No more lines: the rest shortened on the last.
                lines[n - 1] = joined;
                cut = true;
                break;
            }
        }
        lines.push(piece);
    }
    for line in &mut lines {
        if cut || font.measure(size, line) > width {
            *line = paperpaw_sdk::ui::fit(font, size, line, width);
        }
    }
    lines
}

/// The size of each of `n` parts side by side in `room` pixels with
/// `gap` between them: nothing where the gaps take all the room.
fn share(room: u16, gap: u16, n: u16) -> u16 {
    room.saturating_sub(gap.saturating_mul(n.saturating_sub(1))) / n.max(1)
}

/// The height of a line of text `size` pixels large, as a float.
fn line(font: &FontHandle<'_>, size: f32) -> f32 {
    font.line_metrics(size).line_height
}

/// Draws `text` with the middle of its digits (or capitals) at `y` and
/// its middle at `x`: centred by its ink, not by its line, e.g. a
/// numeral on a clock face or a digit on a tile.
fn draw_centred(
    canvas: &mut Canvas<'_>,
    font: &FontHandle<'_>,
    size: f32,
    x: f32,
    y: f32,
    text: &str,
    color: Color,
) {
    let width = font.measure(size, text);
    let baseline = y + cap_height(font, size) / 2.0;
    font.draw(canvas, size, x - width / 2.0, baseline as i32, text, color);
}

/// A part of a stack of text ([`draw_stack`]): its lines, their size and
/// colour.
pub(crate) struct Part<'a> {
    pub lines: Vec<&'a str>,
    pub size: f32,
    pub color: Color,
    /// Its own font, where it is not the stack's.
    pub font: Option<&'a FontHandle<'a>>,
}

/// How far the ink of `part` reaches above its first baseline and below
/// it (its lines a line height apart), in pixels (above negative).
fn part_ink(font: &FontHandle<'_>, part: &Part<'_>) -> (f32, f32) {
    let font = part.font.unwrap_or(font);
    let pitch = line(font, part.size);
    let ink = |text: &str| {
        font.ink(part.size, text).map_or_else(
            || {
                let metrics = font.line_metrics(part.size);
                (-metrics.ascent, -metrics.descent)
            },
            |(top, bottom)| (top as f32, bottom as f32),
        )
    };
    let first = part.lines.first().map_or((0.0, 0.0), |l| ink(l));
    let last = part.lines.last().map_or((0.0, 0.0), |l| ink(l));
    let below = pitch * part.lines.len().saturating_sub(1) as f32 + last.1;
    (first.0, below)
}

/// The height the ink of `parts` takes stacked with `gap` between them
/// ([`draw_stack`]).
pub(crate) fn stack_height(font: &FontHandle<'_>, parts: &[Part<'_>], gap: f32) -> f32 {
    let inked: f32 = parts
        .iter()
        .map(|part| {
            let (top, bottom) = part_ink(font, part);
            bottom - top
        })
        .sum();
    inked + gap * parts.len().saturating_sub(1) as f32
}

/// Draws `parts` one below the other, `gap` pixels between the ink of
/// one and the next, the whole in the middle of `area` by its ink from
/// top to bottom, each line placed as `align` says: e.g. a number large
/// and what it counts below it, centred as one sees them rather than by
/// their lines (which leave more room above letters than below).
pub(crate) fn draw_stack(
    canvas: &mut Canvas<'_>,
    font: &FontHandle<'_>,
    area: Rect,
    align: Align,
    parts: &[Part<'_>],
    gap: f32,
) {
    // The last parts left out where they have no room after all, and
    // all where not even the first has.
    let mut count = parts.len();
    while count > 0 && stack_height(font, &parts[..count], gap) > f32::from(area.height) {
        count -= 1;
    }
    let parts = &parts[..count];
    let total = stack_height(font, parts, gap);
    let mut top = f32::from(area.y) + (f32::from(area.height) - total).max(0.0) / 2.0;
    for part in parts {
        let (above, below) = part_ink(font, part);
        let font = part.font.unwrap_or(font);
        let pitch = line(font, part.size);
        // Told as one text, drawn as lines.
        let told = Rect::new(area.x, top as u16, area.width, (below - above) as u16);
        let text = part.lines.join(" ");
        canvas.describe(paperpaw_sdk::semantics::Role::Text, &text, told);
        let mut baseline = top - above;
        for text in &part.lines {
            // Shortened where it is wider than the place after all.
            let width = f32::from(area.width);
            let shown = if font.measure(part.size, text) > width {
                Cow::Owned(paperpaw_sdk::ui::fit(font, part.size, text, width))
            } else {
                Cow::Borrowed(*text)
            };
            let wide = font.measure(part.size, &shown);
            if wide <= width {
                let x = align.x(area, wide);
                let baseline_y = (baseline + 0.5) as i32;
                font.draw(canvas, part.size, x, baseline_y, &shown, part.color);
            }
            baseline += pitch;
        }
        top += below - above + gap;
    }
}

/// The height of digits and capitals at `size`, about.
fn cap_height(font: &FontHandle<'_>, size: f32) -> f32 {
    // Most fonts' digits are about three quarters of their ascent (at
    // the size the font draws: fonts of fixed sizes take the nearest).
    font.line_metrics(size).ascent * 0.74
}

/// The UI font and its style.
fn ui<'a>(env: &Env<'a>) -> (UiStyle, Option<FontHandle<'a>>) {
    let style = UiStyle::new(env);
    let font = style.font(env);
    (style, font)
}

#[cfg(test)]
mod digits_tests;
#[cfg(test)]
mod logic_tests;
