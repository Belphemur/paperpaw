//! The vault's page: where to go (folders, latest changes, tags, search),
//! and the index, made in the background while the page is open; and the
//! list of tags.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, events};
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::Typed;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, RefreshMode,
    Screen, time,
};

use crate::index::{self, Indexer, State, Step, Tagger};
use crate::list::{MODE, WHAT};

/// What the keyboard answers to the search.
pub(crate) const SEARCH_ENTERED: Event<Typed> = Event::new("notes-vault.search-entered");

/// An index older than this, in seconds, is made anew when the page opens.
const REFRESH: i64 = 6 * 3600;
/// Milliseconds between steps of the index.
const STEP_MS: u64 = 5;
/// The page is drawn anew after this many steps, to show how far it is.
const SHOW_EVERY: u32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    /// No vault is chosen: choose its folder.
    Choose,
    Folders,
    Recent,
    Tags,
    Search,
    Update,
}

fn frame(env: &Env<'_>) -> (UiStyle, ui::Header, ListView) {
    let style = UiStyle::new(env);
    let header = ui::Header::new(env, &style, true);
    let display = env.device().display;
    let top = header.area().height + style.gap();
    let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
    let list = ListView::new(area, style.noted_row()).focus_on_keys(env);
    (style, header, list)
}

#[allow(missing_debug_implementations, reason = "a screen")]
pub(crate) struct HomeScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    vault: String,
    /// The index kept for the vault.
    state: Option<State>,
    indexer: Option<Indexer>,
    tagger: Option<Tagger>,
    /// Steps since the page was drawn.
    steps: u32,
    failed: bool,
    hidden: bool,
}

impl HomeScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let (style, header, list) = frame(env);
        let vault = crate::vault(env);
        Self {
            style,
            header,
            list,
            state: index::kept(env.storage(), &vault),
            vault,
            indexer: None,
            tagger: None,
            steps: 0,
            failed: false,
            hidden: false,
        }
    }

    fn items(&self) -> Vec<Item> {
        if self.vault.is_empty() {
            return alloc::vec![Item::Choose];
        }
        alloc::vec![
            Item::Folders,
            Item::Recent,
            Item::Tags,
            Item::Search,
            Item::Update
        ]
    }

    /// Starts the index anew.
    fn index_now(&mut self, cx: &mut Cx<'_, '_>) {
        if self.vault.is_empty() {
            return;
        }
        self.indexer = Some(Indexer::new(&self.vault));
        self.tagger = None;
        self.failed = false;
        self.steps = 0;
        let now = cx.env().now();
        cx.wake_at(now);
        cx.request_redraw();
    }

    /// Whether the index is to be made anew now: there is none, or it is
    /// old.
    fn due(&self, env: &Env<'_>) -> bool {
        let Some(state) = &self.state else {
            return true;
        };
        match (time::unix_seconds(env), state.built) {
            (Some(now), built) if built > 0 => now - built > REFRESH,
            _ => false,
        }
    }

    /// What the vault's row says: how far the index is, or what it has.
    fn status(&self, env: &Env<'_>) -> String {
        let tr = |key: &str, args: &[(&str, Arg<'_>)]| env.i18n().format(key, args);
        if let Some(indexer) = &self.indexer {
            return tr(
                "notes-vault.status.indexing",
                &[("notes", Arg::Int(i64::from(indexer.notes)))],
            );
        }
        if let (Some(tagger), Some(state)) = (&self.tagger, &self.state) {
            return tr(
                "notes-vault.status.tags",
                &[
                    ("done", Arg::Int(i64::from(tagger.done))),
                    ("notes", Arg::Int(i64::from(state.notes))),
                ],
            );
        }
        if self.failed {
            return env.i18n().tr("notes-vault.status.failed").to_string();
        }
        let Some(state) = &self.state else {
            return env.i18n().tr("notes-vault.status.none").to_string();
        };
        let notes = ("notes", Arg::Int(i64::from(state.notes)));
        match time::unix_seconds(env).filter(|_| state.built > 0) {
            Some(now) => {
                let hours = ((now - state.built) / 3600).max(0);
                let age = if hours < 1 {
                    env.i18n().tr("notes-vault.age.now").to_string()
                } else if hours < 48 {
                    env.i18n().plural("notes-vault.age.hours", hours, &[])
                } else {
                    env.i18n().plural("notes-vault.age.days", hours / 24, &[])
                };
                tr(
                    "notes-vault.status.ready-age",
                    &[notes, ("age", Arg::Str(&age))],
                )
            }
            None => tr("notes-vault.status.ready", &[notes]),
        }
    }

    fn open(&mut self, item: Item, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match item {
            Item::Choose => cx.command(Command::OpenScreen(
                paperpaw_sdk::builtin::screens::FOLDER_EDITOR,
                Params::new().with("id", crate::FOLDER.static_str()),
            )),
            Item::Folders => cx.command(Command::OpenScreen(
                crate::LIST_SCREEN,
                Params::new().with(MODE, "folder"),
            )),
            Item::Recent | Item::Tags if self.state.is_none() => {
                cx.command(Command::Notify(
                    env.i18n().tr("notes-vault.wait-index").to_string(),
                ));
            }
            Item::Recent => cx.command(Command::OpenScreen(
                crate::LIST_SCREEN,
                Params::new().with(MODE, "recent"),
            )),
            Item::Tags => cx.command(Command::OpenScreen(crate::TAGS_SCREEN, Params::new())),
            Item::Search if self.state.is_none() => {
                cx.command(Command::Notify(
                    env.i18n().tr("notes-vault.wait-index").to_string(),
                ));
            }
            Item::Search => cx.command(
                paperpaw_sdk::ask::text(env.i18n().tr("notes-vault.search"), &SEARCH_ENTERED)
                    .command(),
            ),
            Item::Update => self.index_now(cx),
        }
    }

    fn redraw(cx: &mut Cx<'_, '_>) {
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }

    /// The index is made: kept, and its tags next where they are not.
    fn indexed(&mut self, mut state: State, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        state.built = time::unix_seconds(&env).unwrap_or(0);
        // The notes are as they were: the tags made are still right.
        let same = self
            .state
            .as_ref()
            .is_some_and(|old| old.stamp == state.stamp && old.folder == state.folder)
            && env.storage().exists(&index::tags_path());
        state.tagged = same && self.state.as_ref().is_some_and(|old| old.tagged);
        state.write(env.storage());
        if !state.tagged {
            self.tagger = Some(Tagger::new());
        }
        self.state = Some(state);
        self.indexer = None;
        Self::redraw(cx);
    }
}

impl Screen for HomeScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let items = self.items();
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, env.i18n().tr("notes-vault.title"), None, None);
        let status = self.status(env);
        let this = &*self;
        this.list
            .draw(&mut ui, items.len(), |ui, index, rect, focused| {
                let Some(item) = items.get(index) else {
                    return;
                };
                let (key, note, value): (&str, String, &str) = match item {
                    Item::Choose => (
                        "notes-vault.choose",
                        env.i18n().tr("notes-vault.choose.help").to_string(),
                        "›",
                    ),
                    Item::Folders => (
                        "notes-vault.folders",
                        this.vault
                            .rsplit('/')
                            .next()
                            .unwrap_or_default()
                            .to_string(),
                        "›",
                    ),
                    Item::Recent => ("notes-vault.recent", String::new(), "›"),
                    Item::Tags => ("notes-vault.tags", String::new(), "›"),
                    Item::Search => ("notes-vault.search", String::new(), "›"),
                    Item::Update => ("notes-vault.update", status.clone(), ""),
                };
                let row = ui::Row::value(env.i18n().tr(key), value).focused(focused);
                if note.is_empty() {
                    row.draw(ui, rect);
                } else {
                    row.note(&note).draw(ui, rect);
                }
            });
    }

    fn on_resize(&mut self, cx: &mut Cx<'_, '_>) -> Handled {
        let focus = self.list.focus();
        let (style, header, mut list) = frame(cx.env());
        list.set_focus(focus);
        (self.style, self.header, self.list) = (style, header, list);
        cx.request_redraw();
        Handled::Yes
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        self.hidden = false;
        let vault = crate::vault(&env);
        if vault != self.vault {
            self.vault = vault;
            self.state = index::kept(env.storage(), &self.vault);
            self.indexer = None;
            self.tagger = None;
        }
        if self.indexer.is_none() && self.tagger.is_none() && !self.vault.is_empty() {
            if self.due(&env) {
                self.index_now(cx);
            } else if self.state.as_ref().is_some_and(|s| !s.tagged) {
                self.tagger = Some(Tagger::new());
            }
        }
        if self.indexer.is_some() || self.tagger.is_some() {
            cx.wake_at(env.now());
        }
        cx.request_redraw();
    }

    fn on_hide(&mut self) {
        self.hidden = true;
    }

    fn on_timer(&mut self, cx: &mut Cx<'_, '_>) {
        if self.hidden {
            return;
        }
        let env = *cx.env();
        let next = env.now().add_millis(STEP_MS);
        if let Some(indexer) = &mut self.indexer {
            match indexer.step(env.storage()) {
                Step::More => {
                    self.steps += 1;
                    if self.steps.is_multiple_of(SHOW_EVERY) {
                        Self::redraw(cx);
                    }
                    cx.wake_at(next);
                }
                Step::Done(state) => {
                    self.indexed(state, cx);
                    cx.wake_at(next);
                }
                Step::Failed => {
                    self.indexer = None;
                    self.failed = true;
                    Self::redraw(cx);
                }
            }
            return;
        }
        if let Some(tagger) = &mut self.tagger {
            match tagger.step(env.storage()) {
                Some(false) => {
                    self.steps += 1;
                    if self.steps.is_multiple_of(SHOW_EVERY * 4) {
                        Self::redraw(cx);
                    }
                    cx.wake_at(next);
                }
                Some(true) => {
                    self.tagger = None;
                    if let Some(state) = &mut self.state {
                        state.tagged = true;
                        state.write(env.storage());
                    }
                    Self::redraw(cx);
                }
                None => {
                    self.tagger = None;
                    self.failed = true;
                    Self::redraw(cx);
                }
            }
        }
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(changed) = events::SETTING_CHANGED.read(event, params)
            && changed.id == *crate::FOLDER.id()
        {
            // Another vault: its own index.
            let env = *cx.env();
            self.vault = crate::vault(&env);
            self.state = index::kept(env.storage(), &self.vault);
            self.indexer = None;
            self.tagger = None;
            if !self.vault.is_empty() && self.due(&env) {
                self.index_now(cx);
            }
            Self::redraw(cx);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.items().len();
        let rows = self.list.rows() as isize;
        let delta = if let Some(delta) = ui::list_step(action, rows) {
            delta
        } else if *action == actions::CONFIRM {
            if let Some(item) = self.items().get(self.list.focus()).copied() {
                self.open(item, cx);
            }
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
        let len = self.items().len();
        let paging = self.list.paging(len);
        if let Some(action) = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        match self.list.hit(gesture.start, len) {
            Some(index) => {
                self.list.set_focus(index);
                if let Some(item) = self.items().get(index).copied() {
                    self.open(item, cx);
                }
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}

/// The tags of the vault, each with how many notes have it.
#[allow(missing_debug_implementations, reason = "a screen")]
pub(crate) struct TagsScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    tags: Vec<(String, u32)>,
}

impl TagsScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let (style, header, list) = frame(env);
        let (tags, _) = index::tag_counts(env.storage());
        Self {
            style,
            header,
            list,
            tags,
        }
    }

    fn open(&self, index: usize, cx: &mut Cx<'_, '_>) {
        if let Some((tag, _)) = self.tags.get(index) {
            cx.command(Command::OpenScreen(
                crate::LIST_SCREEN,
                Params::new().with(MODE, "tag").with(WHAT, tag.clone()),
            ));
        }
    }
}

impl Screen for TagsScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.tags.len();
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(
            &mut ui,
            env.i18n().tr("notes-vault.tags"),
            None,
            self.list.paging(len),
        );
        if len == 0 {
            let text = ui::tr_input(env, "notes-vault.empty.tags");
            ui.empty(self.list.area(), "hash", "", text);
            return;
        }
        let tags = &self.tags;
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let (tag, count) = &tags[index];
            let note = env
                .i18n()
                .plural("notes-vault.notes", i64::from(*count), &[]);
            ui::Row::value(&alloc::format!("#{tag}"), "›")
                .note(&note)
                .focused(focused)
                .draw(ui, rect);
        });
    }

    fn on_resize(&mut self, cx: &mut Cx<'_, '_>) -> Handled {
        let focus = self.list.focus();
        let (style, header, mut list) = frame(cx.env());
        list.set_focus(focus);
        (self.style, self.header, self.list) = (style, header, list);
        cx.request_redraw();
        Handled::Yes
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let rows = self.list.rows() as isize;
        let delta = if let Some(delta) = ui::list_step(action, rows) {
            delta
        } else if *action == actions::CONFIRM {
            self.open(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        let len = self.tags.len();
        self.list.move_focus(delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let len = self.tags.len();
        let paging = self.list.paging(len);
        if let Some(action) = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        match self.list.hit(gesture.start, len) {
            Some(index) => {
                self.list.set_focus(index);
                self.open(index, cx);
                Handled::Yes
            }
            None => Handled::No,
        }
    }
}
