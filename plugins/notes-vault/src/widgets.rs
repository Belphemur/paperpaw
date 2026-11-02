//! The home screen's widgets (and any screen's that shows widgets):
//!
//! - *Recent notes* (`notes-vault.recent`): the notes changed last; how
//!   many, and from which folder, is set.
//! - *Pinned note* (`notes-vault.pinned`): one note, by its title, with the
//!   beginning of its text.
//! - *Note of the day* (`notes-vault.day`): one note of the vault (or a
//!   folder), the same all day.
//! - *Random note* (`notes-vault.random`): one note, another each time the
//!   page is shown.
//! - *Note tags* (`notes-vault.tags`): the tags with most notes; a tap opens
//!   the notes of a tag.
//! - *Search notes* (`notes-vault.search`): a tap asks for words and lists
//!   the notes whose titles have them.
//!
//! They show what the index holds (made on the vault's page); before it
//! is, they say so, and a tap opens the page. Each line is a tap on its
//! note.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::ui::{self, UiStyle};
use paperpaw_sdk::widgets::{
    self, Align, CellSpan, Frame, Padding, Widget, WidgetContext, WidgetSpec, WidgetTarget,
};
use paperpaw_sdk::{
    Canvas, Command, Env, Id, Params, Rect, RegisterError, Registrar, SettingSpec, time,
};

use crate::home::SEARCH_ENTERED;
use crate::index;
use crate::list::{MODE, WHAT};
use crate::note::{self, folder_in};
use crate::page::{self, Order, Row};
use crate::source::Source;

const RECENT: Id = Id::new_static("notes-vault.recent");
const PINNED: Id = Id::new_static("notes-vault.pinned");
const DAY: Id = Id::new_static("notes-vault.day");
const RANDOM: Id = Id::new_static("notes-vault.random");
const TAGS: Id = Id::new_static("notes-vault.tags");
const SEARCH: Id = Id::new_static("notes-vault.search");

/// What a widget shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Recent,
    Pinned,
    Day,
    Random,
    Tags,
    Search,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Recent => "notes-vault.recent",
            Kind::Pinned => "notes-vault.pinned",
            Kind::Day => "notes-vault.day",
            Kind::Random => "notes-vault.random",
            Kind::Tags => "notes-vault.tags",
            Kind::Search => "notes-vault.search",
        }
    }

    fn id(self) -> Id {
        Id::new_static(self.name())
    }
}

// Each widget's settings, kept in the program: its look, then its own.

static RECENT_SETTINGS: [SettingSpec; 7] = [
    widgets::frame_spec("notes-vault.recent.frame", RECENT, Frame::None),
    widgets::padding_spec("notes-vault.recent.padding", RECENT, Padding::Auto),
    widgets::align_spec("notes-vault.recent.align", RECENT, Align::Start),
    widgets::part_spec(
        "notes-vault.recent.heading",
        RECENT,
        "notes-vault.part.heading",
        true,
    ),
    widgets::part_spec(
        "notes-vault.recent.where",
        RECENT,
        "notes-vault.part.where",
        true,
    ),
    SettingSpec::number(
        "notes-vault.recent.count",
        "notes-vault.widget.count",
        1,
        10,
        5,
        RECENT,
    ),
    SettingSpec::text(
        "notes-vault.recent.folder",
        "notes-vault.widget.folder",
        "",
        RECENT,
    )
    .description("notes-vault.widget.folder.help"),
];

static PINNED_SETTINGS: [SettingSpec; 6] = [
    widgets::frame_spec("notes-vault.pinned.frame", PINNED, Frame::None),
    widgets::padding_spec("notes-vault.pinned.padding", PINNED, Padding::Auto),
    widgets::align_spec("notes-vault.pinned.align", PINNED, Align::Start),
    widgets::part_spec(
        "notes-vault.pinned.heading",
        PINNED,
        "notes-vault.part.heading",
        true,
    ),
    widgets::part_spec(
        "notes-vault.pinned.excerpt",
        PINNED,
        "notes-vault.part.excerpt",
        true,
    ),
    SettingSpec::text(
        "notes-vault.pinned.title",
        "notes-vault.widget.title",
        "",
        PINNED,
    )
    .description("notes-vault.widget.title.help"),
];

static DAY_SETTINGS: [SettingSpec; 6] = [
    widgets::frame_spec("notes-vault.day.frame", DAY, Frame::None),
    widgets::padding_spec("notes-vault.day.padding", DAY, Padding::Auto),
    widgets::align_spec("notes-vault.day.align", DAY, Align::Start),
    widgets::part_spec(
        "notes-vault.day.heading",
        DAY,
        "notes-vault.part.heading",
        true,
    ),
    widgets::part_spec(
        "notes-vault.day.excerpt",
        DAY,
        "notes-vault.part.excerpt",
        true,
    ),
    SettingSpec::text(
        "notes-vault.day.folder",
        "notes-vault.widget.folder",
        "",
        DAY,
    )
    .description("notes-vault.widget.folder.help"),
];

static RANDOM_SETTINGS: [SettingSpec; 6] = [
    widgets::frame_spec("notes-vault.random.frame", RANDOM, Frame::None),
    widgets::padding_spec("notes-vault.random.padding", RANDOM, Padding::Auto),
    widgets::align_spec("notes-vault.random.align", RANDOM, Align::Start),
    widgets::part_spec(
        "notes-vault.random.heading",
        RANDOM,
        "notes-vault.part.heading",
        true,
    ),
    widgets::part_spec(
        "notes-vault.random.excerpt",
        RANDOM,
        "notes-vault.part.excerpt",
        true,
    ),
    SettingSpec::text(
        "notes-vault.random.folder",
        "notes-vault.widget.folder",
        "",
        RANDOM,
    )
    .description("notes-vault.widget.folder.help"),
];

static TAGS_SETTINGS: [SettingSpec; 5] = [
    widgets::frame_spec("notes-vault.tags.frame", TAGS, Frame::None),
    widgets::padding_spec("notes-vault.tags.padding", TAGS, Padding::Auto),
    widgets::align_spec("notes-vault.tags.align", TAGS, Align::Start),
    widgets::part_spec(
        "notes-vault.tags.heading",
        TAGS,
        "notes-vault.part.heading",
        true,
    ),
    SettingSpec::number(
        "notes-vault.tags.count",
        "notes-vault.widget.count",
        1,
        12,
        5,
        TAGS,
    ),
];

static SEARCH_SETTINGS: [SettingSpec; 3] = [
    widgets::frame_spec("notes-vault.search.frame", SEARCH, Frame::Line),
    widgets::padding_spec("notes-vault.search.padding", SEARCH, Padding::Auto),
    widgets::align_spec("notes-vault.search.align", SEARCH, Align::Start),
];

pub(crate) fn register(r: &mut Registrar) -> Result<(), RegisterError> {
    for (kind, settings, cells) in [
        (Kind::Recent, &RECENT_SETTINGS[..], CellSpan::new(2, 2)),
        (Kind::Pinned, &PINNED_SETTINGS[..], CellSpan::new(2, 1)),
        (Kind::Day, &DAY_SETTINGS[..], CellSpan::new(2, 1)),
        (Kind::Random, &RANDOM_SETTINGS[..], CellSpan::new(2, 1)),
        (Kind::Tags, &TAGS_SETTINGS[..], CellSpan::new(2, 1)),
        (Kind::Search, &SEARCH_SETTINGS[..], CellSpan::new(2, 1)),
    ] {
        r.add_setting_group(group(kind))?;
        r.add_settings(settings)?;
        r.add_widget(
            WidgetSpec::new(kind.id(), kind.name(), move |_| {
                Box::new(NoteWidget::new(kind))
            })
            .category(widgets::categories::PERSONAL)
            .cells(cells),
        )?;
    }
    // The words asked for by the search widget (the vault's page has the
    // same): the list of the notes found.
    r.on(SEARCH_ENTERED, |_, typed, out| {
        let words = typed.text.trim();
        if !words.is_empty() {
            out.push(Command::OpenScreen(
                crate::LIST_SCREEN,
                Params::new()
                    .with(MODE, "search")
                    .with(WHAT, words.to_string()),
            ));
        }
    })
}

fn group(kind: Kind) -> &'static paperpaw_sdk::SettingGroupSpec {
    static RECENT_GROUP: paperpaw_sdk::SettingGroupSpec =
        paperpaw_sdk::SettingGroupSpec::new(RECENT, "notes-vault.recent");
    static PINNED_GROUP: paperpaw_sdk::SettingGroupSpec =
        paperpaw_sdk::SettingGroupSpec::new(PINNED, "notes-vault.pinned");
    static DAY_GROUP: paperpaw_sdk::SettingGroupSpec =
        paperpaw_sdk::SettingGroupSpec::new(DAY, "notes-vault.day");
    static RANDOM_GROUP: paperpaw_sdk::SettingGroupSpec =
        paperpaw_sdk::SettingGroupSpec::new(RANDOM, "notes-vault.random");
    static TAGS_GROUP: paperpaw_sdk::SettingGroupSpec =
        paperpaw_sdk::SettingGroupSpec::new(TAGS, "notes-vault.tags");
    static SEARCH_GROUP: paperpaw_sdk::SettingGroupSpec =
        paperpaw_sdk::SettingGroupSpec::new(SEARCH, "notes-vault.search");
    match kind {
        Kind::Recent => &RECENT_GROUP,
        Kind::Pinned => &PINNED_GROUP,
        Kind::Day => &DAY_GROUP,
        Kind::Random => &RANDOM_GROUP,
        Kind::Tags => &TAGS_GROUP,
        Kind::Search => &SEARCH_GROUP,
    }
}

/// A line of a widget: a note (or tag), and what a tap on it does.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Line {
    title: String,
    /// Under the title, small.
    note: String,
    command: Command,
}

struct NoteWidget {
    kind: Kind,
    /// Where it is placed (`notes-vault.recent@2`), for its settings.
    placement: Option<String>,
    lines: Vec<Line>,
    /// Read once: what the widget has chosen.
    read: bool,
    /// The random note: chosen once, kept while the widget lasts.
    random: Option<usize>,
}

impl NoteWidget {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            placement: None,
            lines: Vec::new(),
            read: false,
            random: None,
        }
    }

    /// The setting `name` of this widget (of its placement, if it is one).
    fn key(&self, name: &str) -> String {
        let base = self
            .placement
            .as_deref()
            .unwrap_or_else(|| self.kind.name());
        alloc::format!("{base}.{name}")
    }

    fn text(&self, env: &Env<'_>, name: &str) -> String {
        env.settings().text(&self.key(name)).trim().to_string()
    }

    fn count(&self, env: &Env<'_>, fallback: i64, max: i64) -> usize {
        match env.settings().int(&self.key("count")) {
            c if c >= 1 => c.min(max) as usize,
            _ => fallback as usize,
        }
    }

    fn shows(&self, env: &Env<'_>, part: &str) -> bool {
        env.settings().bool(&self.key(part))
    }

    /// What the widget shows now.
    fn lines(&mut self, env: &Env<'_>) -> Vec<Line> {
        let root = crate::vault(env);
        if self.kind == Kind::Search {
            let ask = paperpaw_sdk::ask::text(env.i18n().tr("notes-vault.search"), &SEARCH_ENTERED)
                .command();
            return alloc::vec![Line {
                title: env.i18n().tr("notes-vault.widget.search").to_string(),
                note: String::new(),
                command: ask,
            }];
        }
        if root.is_empty() {
            return waiting(env, "notes-vault.widget.no-vault");
        }
        if index::kept(env.storage(), &root).is_none() {
            return waiting(env, "notes-vault.widget.no-index");
        }
        let lines = match self.kind {
            Kind::Recent => self.recent(env, &root),
            Kind::Pinned => self.pinned(env),
            Kind::Day | Kind::Random => self.one_of_the_vault(env),
            Kind::Tags => self.tags(env),
            Kind::Search => Vec::new(),
        };
        if lines.is_empty() {
            let key = match self.kind {
                Kind::Pinned => "notes-vault.widget.no-pin",
                Kind::Tags => "notes-vault.empty.tags",
                _ => "notes-vault.empty.recent",
            };
            return waiting(env, key);
        }
        lines
    }

    /// The notes changed last.
    fn recent(&self, env: &Env<'_>, root: &str) -> Vec<Line> {
        let source = Source::Notes {
            folder: self.text(env, "folder").trim_matches('/').to_string(),
            query: String::new(),
        };
        let count = self.count(env, 5, 10);
        let show_where = self.shows(env, "where");
        rows(env, &source, Order::Recent, count)
            .into_iter()
            .map(|r| Line {
                note: if show_where {
                    folder_in(&r.path, root).to_string()
                } else {
                    String::new()
                },
                command: paperpaw_sdk::readers::open(r.path.clone()),
                title: r.title,
            })
            .collect()
    }

    /// The note whose title is set.
    fn pinned(&self, env: &Env<'_>) -> Vec<Line> {
        let wanted = self.text(env, "title");
        if wanted.is_empty() {
            return Vec::new();
        }
        find_by_title(env, &wanted)
            .map(|path| self.note_line(env, &path))
            .into_iter()
            .collect()
    }

    /// One note of the vault (or of a folder): the day's, or a random one.
    fn one_of_the_vault(&mut self, env: &Env<'_>) -> Vec<Line> {
        let source = Source::Notes {
            folder: self.text(env, "folder").trim_matches('/').to_string(),
            query: String::new(),
        };
        let total = count_of(env, &source);
        if total == 0 {
            return Vec::new();
        }
        let pick = if self.kind == Kind::Day {
            let day = time::unix_seconds(env).unwrap_or(0) / 86_400;
            (day.unsigned_abs() as usize) % total
        } else {
            *self.random.get_or_insert_with(|| {
                let seed = env.now().0 ^ time::unix_seconds(env).unwrap_or(0) as u64;
                (seed.wrapping_mul(6_364_136_223_846_793_005) >> 33) as usize
            }) % total
        };
        nth(env, &source, pick)
            .map(|path| self.note_line(env, &path))
            .into_iter()
            .collect()
    }

    /// The tags with most notes.
    fn tags(&self, env: &Env<'_>) -> Vec<Line> {
        let (mut tags, _) = index::tag_counts(env.storage());
        tags.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        tags.into_iter()
            .take(self.count(env, 5, 12))
            .map(|(tag, n)| Line {
                note: env.i18n().plural("notes-vault.notes", i64::from(n), &[]),
                command: Command::OpenScreen(
                    crate::LIST_SCREEN,
                    Params::new().with(MODE, "tag").with(WHAT, tag.clone()),
                ),
                title: alloc::format!("#{tag}"),
            })
            .collect()
    }

    /// The line of the note at `path`: its title, and the beginning of its
    /// text where that is shown.
    fn note_line(&self, env: &Env<'_>, path: &str) -> Line {
        let excerpt = if self.shows(env, "excerpt") {
            note::excerpt_of_file(env.storage(), path)
        } else {
            String::new()
        };
        Line {
            title: note::title(path).to_string(),
            note: excerpt,
            command: paperpaw_sdk::readers::open(path.to_string()),
        }
    }

    fn heading(&self, env: &Env<'_>) -> Option<&'static str> {
        self.shows(env, "heading").then_some(self.kind.name())
    }
}

/// A single line that says why there is nothing to show, and opens the
/// vault's page.
fn waiting(env: &Env<'_>, key: &str) -> Vec<Line> {
    alloc::vec![Line {
        title: env.i18n().tr(key).to_string(),
        note: String::new(),
        command: Command::OpenScreen(crate::HOME_SCREEN, Params::new()),
    }]
}

/// The first `count` entries of `source`, in `order`.
fn rows(env: &Env<'_>, source: &Source, order: Order, count: usize) -> Vec<Row> {
    let mut window = page::window(order, None, &mut |emit| source.feed(env.storage(), emit));
    window.rows.truncate(count);
    window.rows
}

/// How many notes `source` has.
fn count_of(env: &Env<'_>, source: &Source) -> usize {
    let mut count = 0;
    source.feed(env.storage(), &mut |_| count += 1);
    count
}

/// The path of the `n`th note of `source`, in the index's own order.
fn nth(env: &Env<'_>, source: &Source, n: usize) -> Option<String> {
    let (mut at, mut found) = (0, None);
    source.feed(env.storage(), &mut |row| {
        if at == n {
            found = Some(row.path);
        }
        at += 1;
    });
    found
}

/// The path of the note whose title is `title`, whatever the case.
fn find_by_title(env: &Env<'_>, title: &str) -> Option<String> {
    let mut found = None;
    index::notes(env.storage(), &mut |_, path| {
        if note::title(path).eq_ignore_ascii_case(title) {
            found = Some(path.to_string());
            return false;
        }
        true
    });
    found
}

impl Widget for NoteWidget {
    fn update(&mut self, env: &Env<'_>) -> bool {
        let lines = self.lines(env);
        let changed = !self.read || lines != self.lines;
        self.read = true;
        self.lines = lines;
        changed
    }

    fn live(&self) -> bool {
        // What the index holds changes with the vault, not with the time.
        false
    }

    fn context(&mut self, _: &Env<'_>, context: &WidgetContext<'_>) {
        self.placement = context.placement.map(str::to_string);
    }

    fn height(&self, env: &Env<'_>, _width: u16) -> u16 {
        let style = UiStyle::new(env);
        let id = self.kind.name();
        let line = widgets::line_height(env, style.text());
        let small = widgets::line_height(env, style.small());
        let rows: u16 = self
            .lines
            .iter()
            .map(|l| line + if l.note.is_empty() { 0 } else { small })
            .sum();
        let heading = if self.heading(env).is_some() {
            widgets::line_height(env, style.small())
        } else {
            0
        };
        widgets::frame_height(env, id) + heading + rows.max(line)
    }

    fn draw(&self, canvas: &mut Canvas<'_>, env: &Env<'_>, area: Rect) {
        let id = self.kind.name();
        let look = widgets::draw_frame(canvas, env, id, area);
        let style = UiStyle::new(env);
        let mut ui = ui::Ui::new(canvas, env, &style);
        let Some(font) = ui.font() else {
            return;
        };
        let inner = look.area;
        let mut y = inner.y;
        let width = f32::from(inner.width);
        // Where text sits: at the start of its lines, as set.
        let align = widgets::align(env, id).resolved(style.is_rtl());
        let at = |text: &str, size: f32| align.x(inner, font.measure(size, text));
        if let Some(key) = self.heading(env) {
            let h = widgets::line_height(env, style.small());
            let text = ui::fit(&font, style.small(), env.i18n().tr(key), width);
            ui::Label::new(&text)
                .px(style.small())
                .color(look.faint)
                .draw_from(
                    &mut ui,
                    f32::from(inner.x),
                    Rect::new(inner.x, y, inner.width, h),
                );
            y += h;
        }
        for line in &self.lines {
            let h = widgets::line_height(env, style.text());
            if y + h > inner.y + inner.height {
                break;
            }
            let text = ui::fit(&font, style.text(), &line.title, width);
            ui::Label::new(&text)
                .px(style.text())
                .color(look.ink)
                .draw_from(
                    &mut ui,
                    f32::from(inner.x),
                    Rect::new(inner.x, y, inner.width, h),
                );
            y += h;
            if !line.note.is_empty() {
                let h = widgets::line_height(env, style.small());
                let text = ui::fit(&font, style.small(), &line.note, width);
                ui::Label::new(&text)
                    .px(style.small())
                    .color(look.faint)
                    .draw_from(
                        &mut ui,
                        at(&text, style.small()),
                        Rect::new(inner.x, y, inner.width, h),
                    );
                y += h;
            }
        }
    }

    fn targets(&self, env: &Env<'_>, area: Rect) -> Vec<WidgetTarget> {
        let id = self.kind.name();
        let inner = widgets::widget_look(env, id, area).area;
        let style = UiStyle::new(env);
        let mut y = inner.y;
        if self.heading(env).is_some() {
            y += widgets::line_height(env, style.small());
        }
        let line = widgets::line_height(env, style.text());
        let small = widgets::line_height(env, style.small());
        let mut targets = Vec::new();
        for l in &self.lines {
            let h = line + if l.note.is_empty() { 0 } else { small };
            if y + h > inner.y + inner.height {
                break;
            }
            targets.push(WidgetTarget::new(
                Rect::new(inner.x, y, inner.width, h),
                l.command.clone(),
            ));
            y += h;
        }
        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each widget's settings are named after it, as it reads them.
    #[test]
    fn settings_are_the_widget_s() {
        for (settings, widget) in [
            (&RECENT_SETTINGS[..], "notes-vault.recent"),
            (&PINNED_SETTINGS[..], "notes-vault.pinned"),
            (&DAY_SETTINGS[..], "notes-vault.day"),
            (&RANDOM_SETTINGS[..], "notes-vault.random"),
            (&TAGS_SETTINGS[..], "notes-vault.tags"),
            (&SEARCH_SETTINGS[..], "notes-vault.search"),
        ] {
            for spec in settings {
                assert!(spec.id.as_str().starts_with(widget), "{}", spec.id);
                assert_eq!(spec.group.as_str(), widget);
            }
        }
    }
}
