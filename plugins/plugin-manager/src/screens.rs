//! *Screens*: which plugin shows each place where several offer a screen
//! (a slot: the home screen, the library, the reader, the quick panel,
//! the lock, the keyboard, the sleep screen, …), and the reading app
//! books open with.
//!
//! - Each slot with more than one candidate is a row, its choice on the
//!   right: *Automatic* (the plugins' own order) or the plugin chosen.
//!   Choosing opens the list of candidates; the choice is kept in
//!   `paperpaw.ini` (`[slots]`). A chosen plugin that is gone counts as
//!   *Automatic* again.
//! - When a plugin that offers a screen for such a slot comes, once, a
//!   question: use it now?

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, events, screens, settings};
use paperpaw_sdk::event::Event;
use paperpaw_sdk::ui::choice::Picked;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Commands, Cx, Env, FixedAction, FixedScreen, Gesture, GestureKind,
    Handled, Id, MenuEntry, Params, RefreshMode, RegisterError, Registrar, Screen,
};

use crate::{MORE, PLUGIN};

/// The list of slots, or (with [`SLOT`]) the candidates of one.
pub(crate) const SCREEN: Id = Id::new_static("plugin-manager.screens");
/// Opens [`SCREEN`].
pub(crate) const OPEN: Id = Id::new_static("plugin-manager.screens.open");
/// Parameter: the slot whose candidates are listed.
const SLOT: &str = "slot";
/// The answer to "use it now?": the slot and the screen, apart by a
/// space, and yes or no.
const USE: Event<Picked> = Event::new("plugin-manager.screens.use");
const YES: &str = "yes";

static ACTIONS: [FixedAction; 1] = [FixedAction::new(OPEN, "plugin-manager.screens")
    .handler(|_, _, out| out.push(Command::OpenScreen(SCREEN, Params::new())))];
static ENTRIES: [MenuEntry; 1] = [MenuEntry::new(MORE, OPEN).order(-10)];
static ICONS: [(Id, &str); 1] = [(OPEN, "layout")];

static SCREENS: [FixedScreen; 1] = [FixedScreen::new(
    SCREEN,
    "plugin-manager.screens",
    |env, params| Box::new(Screens::new(env, params)),
)];

pub(crate) fn register(r: &mut Registrar) -> Result<(), RegisterError> {
    r.add_screens(&SCREENS)?;
    r.add_actions(&ACTIONS)?;
    r.add_menu_entries(&ENTRIES)?;
    r.add_icons(&ICONS)?;
    r.on(events::STARTED, |env, _, out| offer(env, out))?;
    r.on(events::PLUGINS_CHANGED, |env, _, out| offer(env, out))?;
    r.on(USE, |_, picked, out| {
        if picked.option == YES
            && let Some((slot, screen)) = picked.path.split_once(' ')
            && let (Ok(slot), Ok(screen)) = (Id::parse(slot), Id::parse(screen))
        {
            out.push(Command::BindSlot(slot, Some(screen)));
        }
    })
}

/// The screens offered for `slot` to choose from: not the kernel's
/// placeholder, which shows only where nothing else does.
fn candidates<'e>(env: &Env<'e>, slot: &str) -> Vec<&'e Id> {
    env.slot_screens(slot)
        .into_iter()
        .filter(|screen| **screen != screens::PLACEHOLDER)
        .collect()
}

/// Whether `screen` binds `slot` as high as the screen the plugins chose
/// for it, and is not that one: offered before it (see
/// [`Env::slot_screens`]).
fn bound_as_high(env: &Env<'_>, slot: &str, screen: &str) -> bool {
    let offered = env.slot_screens(slot);
    let at = |id: Option<&Id>| id.and_then(|id| offered.iter().position(|s| *s == id));
    let used = at(env.slot_screen(slot));
    let this = offered.iter().position(|s| s.as_str() == screen);
    matches!((this, used), (Some(this), Some(used)) if this < used)
}

/// The slots several screens are offered for.
fn choosable<'e>(env: &Env<'e>) -> Vec<&'e Id> {
    env.slots()
        .iter()
        .filter(|slot| candidates(env, slot.as_str()).len() > 1)
        .collect()
}

/// What a candidate screen is called: its plugin's name, else its own.
fn name(env: &Env<'_>, screen: &Id) -> String {
    match env.plugins().owner(screen) {
        Some(plugin) => String::from(plugin.name(env.i18n())),
        None => String::from(
            env.i18n()
                .tr(env.screen_title(screen).unwrap_or(screen.as_str())),
        ),
    }
}

/// Where the candidates already seen are kept, one `slot screen` a line.
fn known_path() -> String {
    alloc::format!("{}/screens.txt", Env::plugin_dir(PLUGIN.as_str()))
}

/// Asks once, for a screen newly offered for a slot that has several,
/// whether to use it now (not where the user chose, nor where it is
/// used already, nor for a screen bound lower than the one used, e.g. a
/// fallback). The first start only notes what there is.
fn offer(env: &Env<'_>, out: &mut Commands) {
    let now: Vec<String> = choosable(env)
        .into_iter()
        .flat_map(|slot| {
            candidates(env, slot.as_str())
                .into_iter()
                .map(move |screen| alloc::format!("{slot} {screen}"))
        })
        .collect();
    let known = paperpaw_sdk::read_file(env.storage(), &known_path(), 1 << 14).ok();
    let known = known
        .as_deref()
        .and_then(|data| core::str::from_utf8(data).ok())
        .map(|text| text.lines().map(String::from).collect::<Vec<String>>());
    if known.as_ref() != Some(&now) {
        let _ = env
            .storage()
            .write(&known_path(), now.join("\n").as_bytes());
    }
    let Some(known) = known else {
        return;
    };
    let new = now
        .iter()
        .filter(|line| !known.contains(line))
        .find(|line| {
            line.split_once(' ').is_some_and(|(slot, screen)| {
                env.slot_choice(slot).is_none() && bound_as_high(env, slot, screen)
            })
        });
    let Some((slot, screen)) = new.and_then(|line| line.split_once(' ')) else {
        return;
    };
    let Ok(id) = Id::parse(screen) else {
        return;
    };
    let place = env.i18n().tr(env.slot_title(slot).unwrap_or(slot));
    let title = env.i18n().format(
        "plugin-manager.screens.use",
        &[
            ("name", Arg::Str(&name(env, &id))),
            ("slot", Arg::Str(place)),
        ],
    );
    let question = paperpaw_sdk::ask::choice(title, &USE)
        .about(alloc::format!("{slot} {screen}"))
        .options("plugin-manager.screens.use.", &[YES, "no"]);
    out.push(question.command());
}

/// A row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A slot, its choice on the right.
    Slot(Id),
    /// The reading app books open with (a setting).
    ReadingApp,
    /// The plugins' own order (in the list of a slot's candidates).
    Automatic,
    /// A candidate screen.
    Candidate(Id),
}

struct Screens {
    /// The slot whose candidates are listed; none for the slots.
    slot: Option<Id>,
    rows: Vec<Row>,
    style: UiStyle,
    header: ui::Header,
    list: ListView,
}

impl Screens {
    fn new(env: &Env<'_>, params: &Params) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let display = env.device().display;
        let area =
            paperpaw_sdk::Rect::new(0, top, display.width, display.height.saturating_sub(top));
        let list = ListView::new(area, style.noted_row()).focus_on_keys(env);
        let slot = params.get(SLOT).and_then(|s| Id::parse(s).ok());
        let rows = match &slot {
            Some(slot) => core::iter::once(Row::Automatic)
                .chain(
                    candidates(env, slot.as_str())
                        .into_iter()
                        .map(|screen| Row::Candidate(screen.clone())),
                )
                .collect(),
            None => choosable(env)
                .into_iter()
                .map(|slot| Row::Slot(slot.clone()))
                .chain(
                    env.settings()
                        .spec(settings::READING_APP.as_str())
                        .map(|_| Row::ReadingApp),
                )
                .collect(),
        };
        Self {
            slot,
            rows,
            style,
            header,
            list,
        }
    }

    fn title(&self, env: &Env<'_>) -> String {
        match &self.slot {
            Some(slot) => String::from(env.i18n().tr(env.slot_title(slot.as_str()).unwrap_or(""))),
            None => String::from(env.i18n().tr("plugin-manager.screens")),
        }
    }

    /// What row `index` does: a slot's candidates, the reading app's
    /// choice, or a choice made (then back to the slots).
    fn activate(&mut self, env: &Env<'_>, index: usize, cx: &mut Cx<'_, '_>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        match (row, &self.slot) {
            (Row::Slot(slot), _) => cx.command(Command::OpenScreen(
                SCREEN,
                Params::new().with(SLOT, slot.as_str()),
            )),
            (Row::ReadingApp, _) => {
                if let Some(open) = env
                    .settings()
                    .spec(settings::READING_APP.as_str())
                    .and_then(|spec| ui::setting_opens(env, spec))
                {
                    cx.command(open);
                }
            }
            (Row::Automatic, Some(slot)) => {
                cx.command(Command::BindSlot(slot.clone(), None));
                cx.command(Command::Back);
            }
            (Row::Candidate(screen), Some(slot)) => {
                cx.command(Command::BindSlot(slot.clone(), Some(screen.clone())));
                cx.command(Command::Back);
            }
            _ => {}
        }
    }
}

impl Screen for Screens {
    /// Turned or resized: laid out anew, the focus where it was.
    fn on_resize(&mut self, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let focus = self.list.focus();
        let style = UiStyle::new(&env);
        let header = ui::Header::new(&env, &style, true);
        let top = header.area().height + style.gap();
        let display = env.device().display;
        let area =
            paperpaw_sdk::Rect::new(0, top, display.width, display.height.saturating_sub(top));
        self.list = ListView::new(area, style.noted_row()).focus_on_keys(&env);
        self.list.set_focus(focus);
        (self.style, self.header) = (style, header);
        cx.request_redraw();
        Handled::Yes
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.rows.len();
        self.list.set_len(len);
        let title = self.title(env);
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, &title, None, self.list.paging(len));
        let (rows, slot) = (&self.rows, &self.slot);
        let chosen = slot.as_ref().and_then(|s| env.slot_choice(s.as_str()));
        let mark = env.i18n().tr("plugin-manager.screens.chosen");
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let (title, value, note) = match &rows[index] {
                Row::Slot(slot) => {
                    let place = env.i18n().tr(env.slot_title(slot.as_str()).unwrap_or(""));
                    let value = match env.slot_choice(slot.as_str()) {
                        Some(screen) => name(env, screen),
                        None => env
                            .i18n()
                            .tr("plugin-manager.screens.automatic")
                            .to_string(),
                    };
                    (
                        place.to_string(),
                        alloc::format!("{value} ›"),
                        String::new(),
                    )
                }
                Row::ReadingApp => {
                    let Some(spec) = env.settings().spec(settings::READING_APP.as_str()) else {
                        return;
                    };
                    let value = ui::setting_value(env, spec);
                    let title = env.i18n().tr(&spec.title).to_string();
                    (title, alloc::format!("{value} ›"), String::new())
                }
                Row::Automatic => {
                    let value = if chosen.is_none() { mark } else { "" };
                    let note = env.i18n().tr("plugin-manager.screens.automatic.help");
                    let title = env.i18n().tr("plugin-manager.screens.automatic");
                    (title.to_string(), value.to_string(), note.to_string())
                }
                Row::Candidate(screen) => {
                    let value = if chosen == Some(screen) { mark } else { "" };
                    let note = env.i18n().tr(env.screen_title(screen).unwrap_or(""));
                    (name(env, screen), value.to_string(), note.to_string())
                }
            };
            ui::Row::value(&title, &value)
                .note(&note)
                .focused(focused)
                .draw(ui, rect);
        });
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        // A choice made in the list of a slot: shown on its row.
        cx.request_redraw();
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let len = self.rows.len();
        let page = self.list.rows() as isize;
        let a = action;
        let delta = if *a == actions::UP || *a == actions::LEFT || *a == actions::PREVIOUS {
            -1
        } else if *a == actions::DOWN || *a == actions::RIGHT || *a == actions::NEXT {
            1
        } else if *a == actions::NEXT_PAGE {
            page
        } else if *a == actions::PREV_PAGE {
            -page
        } else if *a == actions::CONFIRM {
            self.activate(&env, self.list.focus(), cx);
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
        let len = self.rows.len();
        let paging = self.list.paging(len);
        if let Some(action) = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, len) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        let env = *cx.env();
        self.activate(&env, index, cx);
        cx.request_refresh(RefreshMode::Fast);
        Handled::Yes
    }
}
