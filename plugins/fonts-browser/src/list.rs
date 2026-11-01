//! The fonts of the catalog, each name set in its own font, with what it
//! is and whether it is here; choosing one shows it larger, to download
//! it or read with it. The catalog kept from the last visit shows at
//! once, while a fresh one is fetched: once each time the list is
//! opened, not when coming back to it from a font's page.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::{actions, events, settings};
use paperpaw_sdk::event::Event;
use paperpaw_sdk::install::Progress;
use paperpaw_sdk::replies::HttpReply;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Canvas, Color, Command, Cx, Env, Gesture, GestureKind, Handled, HttpRequest, Id, Params, Rect,
    RefreshMode, Screen, files,
};

use crate::catalog::{self, Font};
use crate::font::FONT;
use crate::install::{DOWNLOADED, INSTALLER};
use crate::samples::{self, SAMPLE, Samples};
use crate::{CATALOG, FONT_SCREEN};

/// The answer with the catalog.
const LOADED: Event<HttpReply> = Event::new("fonts-browser.loaded");
/// The largest list of fonts taken from a library.
const MAX_LIST_FILE: u64 = 512 * 1024;

fn fetched_path() -> String {
    format!("{}.part", catalog::path())
}

/// Why there is no list to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Trouble {
    None,
    Loading,
    Offline,
    Failed,
}

pub(crate) struct ListScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    /// The line at the bottom.
    status: Rect,
    fonts: Vec<Font>,
    samples: Samples,
    trouble: Trouble,
    /// Whether the catalog was asked for since the list was opened.
    asked: bool,
    /// The download going on, kept here: drawing reads nothing from the
    /// storage.
    job: Option<Progress>,
    /// Offline mode: the offer to go online for the catalog.
    offer: ui::offline::OfflineOffer,
}

impl ListScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let line = (style.row()) * 3 / 2;
        let status = Rect::new(0, display.height - line, display.width, line);
        let area = Rect::new(
            0,
            top,
            display.width,
            display.height.saturating_sub(top + line),
        );
        Self {
            list: ListView::new(area, style.noted_row()).focus_on_keys(env),
            status,
            fonts: catalog::kept(env),
            samples: Samples::new(&style),
            trouble: Trouble::None,
            asked: false,
            job: INSTALLER.progress(env.storage()),
            offer: ui::offline::OfflineOffer::new(),
            style,
            header,
        }
    }

    fn redraw(&self, cx: &mut Cx<'_, '_>) {
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }

    /// Whether the offer to go online shows: no list, for Offline mode.
    fn offering(&self, env: &Env<'_>) -> bool {
        self.fonts.is_empty() && self.trouble == Trouble::Offline && self.offer.shows(env)
    }

    /// Fetches the catalog anew.
    fn fetch(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if !env.caps().network {
            self.trouble = Trouble::Offline;
            return self.redraw(cx);
        }
        let url = env.settings().read(&CATALOG);
        let mut request = HttpRequest::new("GET", url.trim(), LOADED);
        request = request.save_to(fetched_path()).max_file(MAX_LIST_FILE);
        cx.command(Command::http(request));
        self.asked = true;
        if self.fonts.is_empty() {
            self.trouble = Trouble::Loading;
            self.redraw(cx);
        }
    }

    fn loaded(&mut self, reply: &HttpReply, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let fetched = fetched_path();
        let base = env.settings().read(&CATALOG);
        let base = reply.final_url.as_deref().unwrap_or(base.trim());
        let fonts = (200..300)
            .contains(&reply.status)
            .then(|| catalog::fetched(&env, &fetched, base))
            .flatten();
        match fonts {
            Some(fonts) => {
                let _ = files::move_to(env.storage(), &fetched, &catalog::path());
                samples::prune(&env, &self.samples, &fonts);
                self.fonts = fonts;
                self.list.set_len(self.fonts.len());
                self.trouble = Trouble::None;
            }
            None => {
                let _ = env.storage().remove(&fetched);
                // The catalog kept still serves; no answer at all is no
                // connection.
                if self.fonts.is_empty() {
                    self.trouble = if reply.status == 0 {
                        Trouble::Offline
                    } else {
                        Trouble::Failed
                    };
                }
            }
        }
        self.want_samples(cx);
        self.redraw(cx);
    }

    /// The fonts of `fonts` on the page `list` shows.
    fn shown<'a>(list: &ListView, fonts: &'a [Font]) -> &'a [Font] {
        let rows = list.rows();
        let shown = fonts.get(list.page() * rows..).unwrap_or_default();
        shown.get(..rows).unwrap_or(shown)
    }

    fn want_samples(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let mut out = paperpaw_sdk::Commands::new();
        let shown = Self::shown(&self.list, &self.fonts);
        if self.samples.want(&env, shown, &mut out) {
            self.redraw(cx);
        }
        for command in out {
            cx.command(command);
        }
    }

    fn activate(&mut self, index: usize, cx: &mut Cx<'_, '_>) {
        match (self.trouble, self.fonts.get(index)) {
            (_, Some(font)) => cx.command(Command::OpenScreen(
                FONT_SCREEN,
                Params::new().with(FONT, font.to_param()),
            )),
            (Trouble::Failed | Trouble::Offline, None) => self.fetch(cx),
            _ => {}
        }
    }

    fn move_focus(&mut self, change: ui::ListChange, cx: &mut Cx<'_, '_>) {
        if change == ui::ListChange::Page {
            self.want_samples(cx);
        }
        change.request(cx);
    }

    /// What the row of `font` says at its end.
    fn state(env: &Env<'_>, job: Option<&Progress>, font: &Font) -> String {
        if let Some(job) = job.filter(|j| j.plugin == font.plugin()) {
            return env
                .i18n()
                .localize_digits(&format!("{} / {}", job.done, job.total))
                .into_owned();
        }
        let key = if font.chosen_for(env, settings::READING_FONT.as_str()) {
            "fonts-browser.state.reading"
        } else if font.newer(env) {
            "fonts-browser.state.update"
        } else if font.installed(env).is_some() {
            "fonts-browser.state.installed"
        } else {
            return files::shown_size(env, font.size());
        };
        env.i18n().tr(key).to_string()
    }
}

/// What a font is: its kind and how many styles, by whom.
pub(crate) fn describe(env: &Env<'_>, font: &Font) -> String {
    let kind = env
        .i18n()
        .get(&format!("fonts-browser.category.{}", font.category))
        .unwrap_or_else(|| env.i18n().tr("fonts-browser.category.other"));
    let styles = env
        .i18n()
        .plural("fonts-browser.styles", i64::from(font.styles.max(1)), &[]);
    let mut text = format!("{kind} · {styles}");
    if !font.author.is_empty() {
        text.push_str(" · ");
        text.push_str(&font.author);
    }
    text
}

impl Screen for ListScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let len = self.fonts.len();
        let title = env.i18n().tr("fonts-browser.title");
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, title, None, self.list.paging(len));
        if self.offering(env) {
            return self.offer.draw(&mut ui, self.list.area());
        }
        if len == 0 {
            let (icon, title, text) = match self.trouble {
                Trouble::Offline => (
                    "wifi-off",
                    "fonts-browser.offline",
                    "fonts-browser.offline.help",
                ),
                Trouble::Failed => (
                    "cloud",
                    "fonts-browser.unreachable",
                    "fonts-browser.unreachable.help",
                ),
                _ => ("cloud-download", "fonts-browser.loading", ""),
            };
            let text = if text.is_empty() {
                ""
            } else {
                ui::tr_input(env, text)
            };
            ui.empty(self.list.area(), icon, env.i18n().tr(title), text);
            return;
        }
        let (fonts, samples, job) = (&self.fonts, &self.samples, self.job.as_ref());
        let size = samples.size as f32;
        self.list.draw(&mut ui, len, |ui, index, rect, focused| {
            let font = &fonts[index];
            let state = ListScreen::state(env, job, font);
            // The name in its own font, once its sample is here, in the
            // room the state leaves.
            let band = ui::title_area(&style, rect);
            ui::Row::value("", &state).focused(focused).draw(ui, band);
            let Some(ui_font) = style.font(env) else {
                return;
            };
            let ink = style.focus_ink(focused);
            let margin = style.margin() as f32;
            let x = f32::from(rect.x) + margin;
            let room = f32::from(rect.width) - 3.0 * margin - ui_font.measure(style.text(), &state);
            let (face, size) = match samples.get(&font.id) {
                Some(sample) => (sample, size),
                None => (ui_font, style.text()),
            };
            let name = ui::fit(&face, size, &font.name, room);
            ui::Label::new(&name)
                .font(face)
                .px(size)
                .color(ink)
                .draw_from(ui, x, band);
            ui::draw_note(
                ui.canvas(),
                env,
                &style,
                rect,
                focused,
                &describe(env, font),
            );
        });
        ui.canvas().fill(self.status, Color::WHITE);
        let note = ui::tr_input(env, "fonts-browser.hint");
        ui.note_line(self.status, note);
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        self.job = INSTALLER.progress(cx.env().storage());
        if !self.asked {
            self.fetch(cx);
        }
        self.want_samples(cx);
    }

    fn on_hide(&mut self) {
        // The font's page above has the memory; the samples are read
        // again on coming back.
        self.samples.clear();
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        if *action != actions::BACK && self.offering(cx.env()) {
            if self.offer.on_action(action, cx).done() == Some(ui::offline::Offer::Online) {
                self.fetch(cx);
            }
            return Handled::Yes;
        }
        let len = self.fonts.len();
        if let Some(change) = self.list.navigate(action, len) {
            self.move_focus(change, cx);
        } else if *action == actions::CONFIRM {
            self.activate(self.list.focus(), cx);
        } else {
            return Handled::No;
        }
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        let len = self.fonts.len();
        let paging = self.list.paging(len);
        if let Some(action) = self
            .header
            .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        if self.offering(cx.env()) {
            if self.offer.on_gesture(&gesture, cx).done() == Some(ui::offline::Offer::Online) {
                self.fetch(cx);
            }
            return Handled::Yes;
        }
        if len == 0 && self.list.area().contains(gesture.start) {
            self.activate(0, cx);
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, len) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        self.activate(index, cx);
        Handled::Yes
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(reply) = LOADED.read(event, params) {
            self.loaded(&reply, cx);
        } else if let Some(reply) = SAMPLE.read(event, params) {
            let env = *cx.env();
            let shown = Self::shown(&self.list, &self.fonts);
            self.samples.received(&env, &reply, shown);
            self.want_samples(cx);
        } else if DOWNLOADED.read(event, params).is_some()
            || *event == events::PLUGINS_CHANGED
            || *event == events::SETTING_CHANGED
        {
            // Progress, or a font installed, removed or chosen.
            self.job = INSTALLER.progress(cx.env().storage());
            self.redraw(cx);
        }
    }
}
