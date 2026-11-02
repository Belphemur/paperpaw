//! The home screen's pages: each a grid of items in a setting of its own,
//! `home.page-1` to `home.page-8`, their layout as
//! `paperpaw_sdk::widgets::grid` writes it. The first page always
//! shows; the others where they have items.
//!
//! Each page has a second setting, `home.page-N.wide`, for a screen on its
//! side (wider than high). While none of those holds anything, the screen
//! on its side shows the pages derived from the others, with more columns
//! ([`pages_wide`]), the items of every page flowing on in turn, each
//! page filled before the next; nothing of that is stored, so turning the
//! screen back and forth changes no page. The first change made on the screen on its side keeps the whole
//! derived layout in the `.wide` settings, which from then on are that
//! orientation's own ([`store`]). A page added and not filled
//! yet, or emptied, keeps a space ([`widgets::SPACE`], which the grid
//! leaves out) so that it shows until it is removed. Above the grid sits
//! the status line, below it the page dots and the pencil.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::settings;
use paperpaw_sdk::ui;
use paperpaw_sdk::widgets;
use paperpaw_sdk::{
    Color, Command, Env, Id, RegisterError, Registrar, SettingExtra, SettingKind, SettingSpec,
    SettingValue,
};

use crate::{ACTIONS, SCREEN};
use paperpaw_sdk::builtin::{actions, menus};
use paperpaw_sdk::setting::{Setting, Text};
use paperpaw_sdk::widgets::grid::{self, Geometry, Item, What};

/// Pages at most.
pub(crate) const MAX_PAGES: usize = 8;

/// Columns of the grid.
pub(crate) const COLUMNS: Setting<i64> = Setting::new("home.columns");
/// Rows of the grid; 0: as many as make cells about as high as wide.
pub(crate) const ROWS: Setting<i64> = Setting::new("home.rows");
/// Whether the status line shows above the grid.
pub(crate) const STATUS_BAR: Setting<bool> = Setting::new("home.status-bar");
/// Below the page: the pencil that edits it, the dots of the pages, and
/// status items of any plugin.
pub(crate) const FOOTER_PENCIL: Setting<bool> = Setting::new("home.footer.pencil");
pub(crate) const FOOTER_DOTS: Setting<bool> = Setting::new("home.footer.dots");
pub(crate) const FOOTER_ITEMS: Setting<Text> = Setting::new("home.footer.items");

/// Draws the dot of a page (or a layout) `size` pixels wide at `x`, `y`,
/// filled for the current one: round where small shapes are pills or
/// corners round, else square.
pub(crate) fn draw_dot(ui: &mut ui::Ui<'_>, (x, y): (i32, i32), size: u32, filled: bool) {
    let (env, style) = (ui.env(), ui.style());
    let corners = env.settings().text(settings::UI_CORNERS.as_str());
    let round = style.pill() || settings::CORNERS[2..].contains(&corners);
    let radius = if round { size / 2 } else { 0 };
    if filled {
        ui.canvas()
            .fill_round_rect(x, y, size, size, radius, Color::BLACK);
    } else {
        let line = u32::from(style.line()).max(1);
        ui.canvas()
            .stroke_round_rect(x, y, size, size, line, radius, Color::BLACK);
    }
}

/// The built-in widgets (see `blocks`).
pub(crate) const STATUS: &str = "home.status";
pub(crate) const DATE: &str = "home.date";
pub(crate) const CONTINUE: &str = "home.continue";
pub(crate) const RECENT: &str = "home.recent";
/// On a page: every tool not on it otherwise, each in a cell.
pub(crate) const TOOLS: &str = "home.tools";

/// What the pages show at first: the clock, the book being read and the
/// recent ones, then the tools.
pub(crate) const DEFAULTS: [&str; MAX_PAGES] = [
    "home.clock, home.continue, home.recent",
    "home.tools",
    "",
    "",
    "",
    "",
    "",
    "",
];

/// The setting of page `index` (0 the first).
pub(crate) fn setting(index: usize) -> Id {
    Id::parse(&format!("home.page-{}", index + 1)).unwrap_or(Id::new_static("home.page-1"))
}

/// The setting of page `index` for a screen on its side.
pub(crate) fn setting_wide(index: usize) -> Id {
    Id::parse(&format!("home.page-{}.wide", index + 1))
        .unwrap_or(Id::new_static("home.page-1.wide"))
}

/// The setting that keeps page `slot` in the layout for a screen on its
/// side (`wide`) or one upright.
fn setting_of(slot: usize, wide: bool) -> Id {
    if wide {
        setting_wide(slot)
    } else {
        setting(slot)
    }
}

/// The home screen's settings, on its page (Settings, Customise, Home screen).
static IN_HOME: SettingExtra = SettingExtra::new();
/// A page's items, edited on the home screen itself.
static PAGE: SettingExtra = SettingExtra::new().editor(SCREEN);
/// The status items below the page, chosen on their picker.
static ITEMS: SettingExtra =
    SettingExtra::new().editor(paperpaw_sdk::builtin::screens::STATUS_ITEMS);

/// The setting of page `n` (1 the first).
const fn page(id: &'static str, title: &'static str, default: &'static str) -> SettingSpec {
    SettingSpec::text(id, title, default, GROUP)
        .with_extra(&PAGE)
        .description("home.setting.page.help")
        .advanced()
}

/// The setting of page `n` (1 the first) for a screen on its side.
const fn page_wide(id: &'static str, title: &'static str) -> SettingSpec {
    SettingSpec::text(id, title, "", GROUP)
        .with_extra(&PAGE)
        .description("home.setting.page-wide.help")
        .advanced()
}

/// The pages, the grid, and what shows above and below it.
static SETTINGS: [SettingSpec; 24] = [
    page("home.page-1", "home.setting.page-1", DEFAULTS[0]),
    page("home.page-2", "home.setting.page-2", DEFAULTS[1]),
    page("home.page-3", "home.setting.page-3", DEFAULTS[2]),
    page("home.page-4", "home.setting.page-4", DEFAULTS[3]),
    page("home.page-5", "home.setting.page-5", DEFAULTS[4]),
    page("home.page-6", "home.setting.page-6", DEFAULTS[5]),
    page("home.page-7", "home.setting.page-7", DEFAULTS[6]),
    page("home.page-8", "home.setting.page-8", DEFAULTS[7]),
    page_wide("home.page-1.wide", "home.setting.page-1-wide"),
    page_wide("home.page-2.wide", "home.setting.page-2-wide"),
    page_wide("home.page-3.wide", "home.setting.page-3-wide"),
    page_wide("home.page-4.wide", "home.setting.page-4-wide"),
    page_wide("home.page-5.wide", "home.setting.page-5-wide"),
    page_wide("home.page-6.wide", "home.setting.page-6-wide"),
    page_wide("home.page-7.wide", "home.setting.page-7-wide"),
    page_wide("home.page-8.wide", "home.setting.page-8-wide"),
    SettingSpec::number(
        COLUMNS.static_str(),
        "home.setting.columns",
        2,
        grid::MAX_COLUMNS as i64,
        3,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.columns.help"),
    SettingSpec::number(
        ROWS.static_str(),
        "home.setting.rows",
        0,
        grid::MAX_ROWS as i64,
        0,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.rows.help"),
    SettingSpec::switch(
        crate::blocks::TOOL_LABELS.static_str(),
        "home.setting.tool-labels",
        true,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.tool-labels.help"),
    SettingSpec::switch(
        crate::blocks::FOCUS_AT_ONCE.static_str(),
        "home.setting.focus-at-once",
        false,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.focus-at-once.help"),
    SettingSpec::switch(
        STATUS_BAR.static_str(),
        "home.setting.status-bar",
        true,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.status-bar.help"),
    SettingSpec::switch(
        FOOTER_PENCIL.static_str(),
        "home.setting.footer-pencil",
        true,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.footer-pencil.help"),
    SettingSpec::switch(
        FOOTER_DOTS.static_str(),
        "home.setting.footer-dots",
        true,
        GROUP,
    )
    .with_extra(&IN_HOME)
    .description("home.setting.footer-dots.help"),
    SettingSpec::new(
        paperpaw_sdk::Id::new_static(FOOTER_ITEMS.static_str()),
        "home.setting.footer-items",
        SettingKind::List,
        SettingValue::Text(Cow::Borrowed("")),
        GROUP,
    )
    .with_extra(&ITEMS)
    .description("home.setting.footer-items.help"),
];

/// The pages as a place of widgets in a grid.
const PLACE: widgets::WidgetPlace = widgets::WidgetPlace::new(
    Id::new_static("home.pages"),
    "home.title",
    PAGE_SETTINGS,
    widgets::PlaceKind::Grid,
);
const PAGE_SETTINGS: &[Id] = &[
    Id::new_static("home.page-1"),
    Id::new_static("home.page-2"),
    Id::new_static("home.page-3"),
    Id::new_static("home.page-4"),
    Id::new_static("home.page-5"),
    Id::new_static("home.page-6"),
    Id::new_static("home.page-7"),
    Id::new_static("home.page-8"),
];

/// The home screen's settings' group (Settings, Customise, Home screen).
pub const GROUP: paperpaw_sdk::Id = paperpaw_sdk::Id::new_static("home.group");

pub(crate) fn register(r: &mut Registrar) -> Result<(), RegisterError> {
    // Its settings on a page of their own (Settings, Customise).
    r.add_setting_group({
        static SPEC: paperpaw_sdk::SettingGroupSpec =
            paperpaw_sdk::SettingGroupSpec::new(GROUP, "home.section").order(220);
        &SPEC
    })?;
    r.add_icon(GROUP, Cow::Borrowed("home"))?;
    r.add_settings(&SETTINGS)?;
    r.add_widget_place(PLACE)?;
    r.set_menu_need(
        crate::blocks::FOCUS_AT_ONCE,
        paperpaw_sdk::MenuNeed::NoTouch,
    )?;
    Ok(())
}

/// A page shown: which setting keeps it, and its items.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Page {
    /// The setting it is kept in: 0 for `home.page-1`.
    pub(crate) slot: usize,
    pub(crate) items: Vec<Item>,
}

/// Every page shown in the layout for a screen upright, in the order of
/// their settings: the first always, the others where they hold
/// something.
pub(crate) fn pages(env: &Env<'_>) -> Vec<Page> {
    read(env, false)
}

/// The pages kept in the settings of one layout (`wide`: the screen on its
/// side).
fn read(env: &Env<'_>, wide: bool) -> Vec<Page> {
    (0..MAX_PAGES)
        .map(|slot| Page {
            slot,
            items: grid::parse(env.settings().text(setting_of(slot, wide).as_str())),
        })
        .filter(|page| page.slot == 0 || !page.items.is_empty())
        .collect()
}

/// Whether any page has a layout of its own for a screen on its side.
pub(crate) fn wide_kept(env: &Env<'_>) -> bool {
    (0..MAX_PAGES).any(|slot| !env.settings().text(setting_wide(slot).as_str()).is_empty())
}

/// The columns the grid has upright.
pub(crate) fn columns_upright(env: &Env<'_>) -> u8 {
    env.settings()
        .read(&COLUMNS)
        .clamp(2, i64::from(grid::MAX_COLUMNS)) as u8
}

/// Every page shown for a screen on its side, whose grid is `geometry`:
/// the pages kept for it where there are any ([`wide_kept`]), else the
/// items of the upright layout laid out anew in it ([`grid::refit`]), page
/// after page in reading order, each page filled before the next is begun
/// (so the room a page has is used, not the split upright; eight pages at
/// most). A widget is no wider than about 110 mm. Nothing is stored.
pub(crate) fn pages_wide(env: &Env<'_>, geometry: &Geometry) -> Vec<Page> {
    if wide_kept(env) {
        return read(env, true);
    }
    let from = columns_upright(env);
    let widest = widest_widget(env, geometry);
    let mut carried: Vec<Item> = Vec::new();
    for page in pages(env) {
        let mut items = expand(env, &page.items);
        // As a page reads: row by row, what has no place last.
        items.sort_by_key(|item| item.at.map_or((true, 0, 0), |c| (false, c.row, c.col)));
        carried.append(&mut items);
    }
    let mut out: Vec<Page> = Vec::new();
    while out.len() < MAX_PAGES && (out.is_empty() || !carried.is_empty()) {
        let (placed, left) = grid::refit(env, geometry, from, widest, &carried);
        // Nothing fits even an empty page (a widget the grid cannot hold):
        // what remains is not shown here.
        let stuck = placed.is_empty() && !out.is_empty();
        if stuck {
            break;
        }
        out.push(Page {
            slot: out.len(),
            items: placed,
        });
        carried = left;
    }
    out
}

/// The most columns of `geometry` a widget takes on a screen on its side:
/// those that make 110 millimetres (the width of the readable column), rounded
/// up, all of them where the screen is not wider.
fn widest_widget(env: &Env<'_>, geometry: &Geometry) -> u8 {
    let column = u32::from(geometry.cell_w) + u32::from(geometry.gap);
    env.metrics()
        .mm(110)
        .div_ceil(column.max(1))
        .clamp(1, u32::from(geometry.columns)) as u8
}

/// A page's items as the grid shows them: the tools for `home.tools`,
/// without what the grid has no use for (the status line above it,
/// spaces: empty cells are).
pub(crate) fn expand(env: &Env<'_>, items: &[Item]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for item in items {
        match &item.what {
            What::Widget(id) if id == TOOLS => {
                for (action, _) in tools(env) {
                    let shown = items.iter().any(|i| i.what == What::Tool(action.clone()));
                    if !shown {
                        out.push(Item::new(What::Tool(action)));
                    }
                }
            }
            What::Widget(id)
                if id == widgets::SPACE
                    || id == widgets::FILL
                    || (widgets::base(id) == STATUS && env.settings().read(&STATUS_BAR)) => {}
            What::Widget(id) if !env.widgets().has(id) => {}
            _ => out.push(item.clone()),
        }
    }
    out
}

/// Whether the page keeps `item` without showing it ([`expand`] leaves it
/// out): a widget of a plugin not loaded now (switched off, say), the
/// status line while it shows above the grid, a fill.
fn hidden(env: &Env<'_>, item: &Item) -> bool {
    match &item.what {
        What::Widget(id) if id == TOOLS || id == widgets::SPACE => false,
        What::Widget(id) => {
            id == widgets::FILL
                || (widgets::base(id) == STATUS && env.settings().read(&STATUS_BAR))
                || !env.widgets().has(id)
        }
        _ => false,
    }
}

/// What a page keeps without showing it (`hidden`) among the items of the
/// settings `from`, added to `items`.
fn with_hidden(env: &Env<'_>, mut items: Vec<Item>, from: &[Id]) -> Vec<Item> {
    for setting in from {
        for item in grid::parse(env.settings().text(setting.as_str())) {
            if hidden(env, &item) && !items.iter().any(|i| i.what == item.what) {
                items.push(item);
            }
        }
    }
    items
}

/// The commands that keep `items` (as the page shows them) as the page in
/// `slot`, with what the page keeps there without showing it: in the
/// layout for a screen upright (`wide` is `None`), or in that for a screen
/// on its side, whose grid is `wide`. The first change in the latter keeps
/// every page of the layout derived from the upright one (see
/// [`pages_wide`]) in the `.wide` settings, which are the layout from then
/// on.
pub(crate) fn store(
    env: &Env<'_>,
    slot: usize,
    items: &[Item],
    wide: Option<&Geometry>,
) -> Vec<Command> {
    let Some(geometry) = wide else {
        let items = with_hidden(env, items.to_vec(), &[setting(slot)]);
        return alloc::vec![write(slot, false, &items)];
    };
    let mut pages = pages_wide(env, geometry);
    if let Some(page) = pages.iter_mut().find(|p| p.slot == slot) {
        page.items = items.to_vec();
    }
    store_all(env, &pages, true)
}

/// The command that keeps `items` as the page in `slot` of a layout as they
/// are: a page other than the first keeps a space when emptied, so that it
/// still shows.
fn write(slot: usize, wide: bool, items: &[Item]) -> Command {
    let text = if items.is_empty() && slot > 0 {
        String::from(widgets::SPACE)
    } else {
        grid::write(items)
    };
    Command::SetSetting(setting_of(slot, wide), SettingValue::Text(Cow::Owned(text)))
}

/// The commands that keep `pages` of a layout (as [`pages`] reads them,
/// what they do not show included; for the screen on its side what the
/// upright pages keep without showing is added) in the first settings, in
/// their order, and empty the others: once a page was added or removed.
pub(crate) fn store_all(env: &Env<'_>, pages: &[Page], wide: bool) -> Vec<Command> {
    if !wide {
        return write_all(pages, false);
    }
    let with: Vec<Page> = pages
        .iter()
        .map(|page| Page {
            slot: page.slot,
            items: with_hidden(
                env,
                page.items.clone(),
                &[setting_wide(page.slot), setting(page.slot)],
            ),
        })
        .collect();
    write_all(&with, true)
}

/// [`store_all`] for the pages as they are.
fn write_all(pages: &[Page], wide: bool) -> Vec<Command> {
    (0..MAX_PAGES)
        .map(|slot| match pages.get(slot) {
            Some(page) => write(slot, wide, &page.items),
            None => Command::SetSetting(
                setting_of(slot, wide),
                SettingValue::Text(Cow::Borrowed("")),
            ),
        })
        .collect()
}

/// Whether any page of either layout holds the widget `id`.
pub(crate) fn holds(env: &Env<'_>, id: &str) -> bool {
    let has = |pages: &[Page]| {
        pages
            .iter()
            .flat_map(|p| &p.items)
            .any(|i| matches!(&i.what, What::Widget(w) if w == id))
    };
    has(&pages(env)) || (wide_kept(env) && has(&read(env, true)))
}

/// The pages of the layout other than the one shown (`wide`: the screen is
/// on its side): the pages kept for that, or none where the layout for the
/// screen on its side is derived and so has none of its own.
pub(crate) fn other_layer(env: &Env<'_>, wide: bool) -> Vec<Page> {
    if wide {
        pages(env)
    } else if wide_kept(env) {
        read(env, true)
    } else {
        Vec::new()
    }
}

/// The commands that add `what` to the page the layout other than the one
/// shown has in `slot` (the last page where it has fewer) without a place,
/// where the grid puts it in the first free cells: a widget or tool added
/// on a screen upright is on its side too, and the other way. Nothing
/// where that layout is the derived one, which has it already.
pub(crate) fn add_to_other(env: &Env<'_>, wide: bool, slot: usize, what: What) -> Vec<Command> {
    let other = other_layer(env, wide);
    let Some(page) = other.get(slot).or(other.last()) else {
        return Vec::new();
    };
    if page.items.iter().any(|i| i.what == what) {
        return Vec::new();
    }
    let mut items = page.items.clone();
    items.push(Item::new(what));
    // The other layout's own grid: only the stored layer is written, which
    // needs none (`wide` is the shown one: the other is upright when it).
    if wide {
        store(env, page.slot, &items, None)
    } else {
        let geometry = crate::frame(env, true).1;
        store(env, page.slot, &items, Some(&geometry))
    }
}

/// The commands that empty the layout for a screen on its side: the
/// derived one shows again.
pub(crate) fn reset_wide() -> Vec<Command> {
    (0..MAX_PAGES)
        .map(|slot| Command::SetSetting(setting_wide(slot), SettingValue::Text(Cow::Borrowed(""))))
        .collect()
}

/// The pages fitted to `geometry` once its columns or rows changed: every
/// item where the grid now shows it (moved inside it as little as can
/// be), and what a page has no room for on the next page (a page added
/// where there is none), so that nothing is lost or hidden; `None`
/// where nothing moved. The tools and the spaces stay as they are.
pub(crate) fn reflow(env: &Env<'_>, geometry: &Geometry, wide: bool) -> Option<Vec<Page>> {
    let kept = |item: &Item| match &item.what {
        What::Widget(id) => id == TOOLS || id == widgets::SPACE || id == widgets::FILL,
        _ => false,
    };
    // The pages kept for a screen on its side where it has its own.
    if wide && !wide_kept(env) {
        return None;
    }
    let mut pages = read(env, wide);
    let mut carried: Vec<Item> = Vec::new();
    let mut changed = false;
    let mut index = 0;
    while index < pages.len() || (!carried.is_empty() && pages.len() < MAX_PAGES) {
        if index == pages.len() {
            pages.push(Page {
                slot: index,
                items: Vec::new(),
            });
        }
        let page = &mut pages[index];
        let (special, mut movable): (Vec<Item>, Vec<Item>) =
            page.items.iter().cloned().partition(|item| kept(item));
        // What did not fit before comes first, so it finds room here.
        movable.splice(0..0, carried.drain(..).map(|item| Item::new(item.what)));
        let (mut placed, left) = grid::fit(env, geometry, &movable);
        placed.extend(special);
        if placed != page.items {
            page.items = placed;
            changed = true;
        }
        carried = left;
        index += 1;
    }
    changed.then_some(pages)
}

/// The setting `id` is, if it keeps a page: 0 for `home.page-1`.
pub(crate) fn slot_of(id: &str) -> Option<usize> {
    (0..MAX_PAGES).find(|&i| setting(i).as_str() == id || setting_wide(i).as_str() == id)
}

/// The tools a page may show: the shortcuts, the tools, what there is to
/// open ([`menus::APPS`], also in the tools' menu), then the settings;
/// their action and the translation key of their name.
pub(crate) fn tools(env: &Env<'_>) -> Vec<(Id, String)> {
    let mut items: Vec<(Id, String)> = Vec::new();
    for menu in [ACTIONS, menus::TOOLS, menus::APPS] {
        for item in env.menus().items(menu.as_str()) {
            // A screenshot from here shows only the home screen: it is
            // for the tools opened over a book (unless listed above).
            let here_useless = menu == menus::TOOLS && *item.id == actions::SCREENSHOT;
            // Hidden by its plugin now (e.g. *Apps* with nothing in it).
            let hidden = !env.menus().state(env, item.id.as_str()).shown;
            if !item.section
                && !here_useless
                && !hidden
                && !items.iter().any(|(id, _)| id == item.id)
            {
                items.push((item.id.clone(), item.title.to_string()));
            }
        }
    }
    if !items.iter().any(|(id, _)| *id == actions::SETTINGS) {
        items.push((actions::SETTINGS, String::from("core.action.settings")));
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_is_stored() {
        let pages = alloc::vec![
            Page {
                slot: 0,
                items: grid::parse("a 0/0 1x1"),
            },
            Page {
                slot: 3,
                items: Vec::new(),
            },
        ];
        let stored = write_all(&pages, false);
        assert_eq!(stored.len(), MAX_PAGES);
        // In order from the first setting, the empty page kept as a space.
        let text = |n: usize| match &stored[n] {
            Command::SetSetting(id, SettingValue::Text(text)) => {
                (String::from(id.as_str()), String::from(text.as_ref()))
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(text(0), ("home.page-1".into(), "a 0/0 1x1".into()));
        assert_eq!(text(1), ("home.page-2".into(), widgets::SPACE.into()));
        assert_eq!(text(2), ("home.page-3".into(), String::new()));
        assert_eq!(slot_of("home.page-2"), Some(1));
        assert_eq!(slot_of("home.columns"), None);
    }
}
