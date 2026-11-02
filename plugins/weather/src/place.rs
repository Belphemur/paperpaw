//! The place of the weather: searched by its name (typed on the keyboard)
//! with Open-Meteo's search, the places found listed to pick one; or none,
//! which stops fetching the weather. Only the name typed goes out to
//! search; the place picked is kept as its name and its coordinates.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::ask;
use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::{HttpReply, Typed};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, HttpRequest, Id, Params, Rect,
    RefreshMode, Screen,
};

use crate::data::{self, Place};
use crate::{COORDINATES, PLACE};

/// The name to search for, from the keyboard.
const TYPED: Event<Typed> = Event::new("weather.typed");
/// The search's answer.
const SEARCHED: Event<HttpReply> = Event::new("weather.searched");

/// How the search went.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Search {
    /// None yet.
    Idle,
    Searching(String),
    Found,
    Nothing(String),
    Failed,
    NoNetwork,
}

/// The rows: *Search for a place*, the places found, and *No place* when
/// one is set.
pub(crate) struct PlaceScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    places: Vec<Place>,
    search: Search,
    /// The keyboard was opened at once (no place was set).
    asked: bool,
    /// Offline mode: the offer to go online, and the name to search for
    /// then.
    offer: ui::offline::OfflineOffer,
    waiting: Option<String>,
}

impl PlaceScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        Self {
            list: ListView::new(area, style.noted_row()).focus_on_keys(env),
            style,
            header,
            places: Vec::new(),
            search: Search::Idle,
            asked: false,
            offer: ui::offline::OfflineOffer::new(),
            waiting: None,
        }
    }

    fn has_place(env: &Env<'_>) -> bool {
        !env.settings().read(&COORDINATES).trim().is_empty()
    }

    fn len(&self, env: &Env<'_>) -> usize {
        1 + self.places.len() + usize::from(Self::has_place(env))
    }

    fn redraw(cx: &mut Cx<'_, '_>) {
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }

    /// Whether the offer to go online shows (Offline mode, a name to
    /// search for).
    fn offering(&self, env: &Env<'_>) -> bool {
        self.waiting.is_some() && self.offer.shows(env)
    }

    /// Searches for `name`.
    fn find(&mut self, name: String, env: &Env<'_>, cx: &mut Cx<'_, '_>) {
        let url = data::search_url(&name, env.i18n().language());
        let request = HttpRequest::new("GET", &url, SEARCHED).header(
            String::from("User-Agent"),
            String::from("PaperPaw/0.1 (e-reader)"),
        );
        cx.command(Command::http(request));
        self.search = Search::Searching(name);
    }

    /// What the reader chose on the offer: the search is sent, or said to
    /// need the network.
    fn offered(&mut self, chosen: Option<ui::offline::Offer>, cx: &mut Cx<'_, '_>) {
        let (Some(chosen), Some(name)) = (chosen, self.waiting.take()) else {
            return;
        };
        match chosen {
            ui::offline::Offer::Online => {
                let env = *cx.env();
                self.find(name, &env, cx);
            }
            _ => self.search = Search::NoNetwork,
        }
        Self::redraw(cx);
    }

    fn ask(&self, env: &Env<'_>, cx: &mut Cx<'_, '_>) {
        let initial = match &self.search {
            Search::Searching(name) | Search::Nothing(name) => name.clone(),
            _ => String::new(),
        };
        cx.command(
            ask::text(env.i18n().tr("weather.place.name"), &TYPED)
                .initial(initial)
                .command(),
        );
    }

    fn open(&mut self, row: usize, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if row == 0 {
            self.ask(&env, cx);
            return;
        }
        let set = |cx: &mut Cx<'_, '_>, name: String, coordinates: String| {
            cx.command(Command::SetSetting(
                PLACE.id().clone(),
                PLACE.value(Cow::Owned(name)),
            ));
            cx.command(Command::SetSetting(
                COORDINATES.id().clone(),
                COORDINATES.value(Cow::Owned(coordinates)),
            ));
            cx.command(Command::Back);
        };
        match self.places.get(row - 1) {
            Some(place) => set(cx, place.name.clone(), place.coordinates()),
            // The last row: no place.
            None if Self::has_place(&env) => set(cx, String::new(), String::new()),
            None => {}
        }
    }

    /// What the first row tells: the place set, or how the search went.
    fn note(&self, env: &Env<'_>) -> String {
        match &self.search {
            Search::Idle => {
                let place = env.settings().read(&PLACE).trim();
                if place.is_empty() {
                    env.i18n().tr("weather.place.help").to_string()
                } else {
                    env.i18n()
                        .format("weather.place.now", &[("place", Arg::Str(place))])
                }
            }
            Search::Searching(_) => env.i18n().tr("weather.place.searching").to_string(),
            Search::Found => {
                env.i18n()
                    .plural("weather.place.found", self.places.len() as i64, &[])
            }
            Search::Nothing(name) => env
                .i18n()
                .format("weather.place.nothing", &[("name", Arg::Str(name))]),
            Search::Failed => env.i18n().tr("weather.place.failed").to_string(),
            Search::NoNetwork => env.i18n().tr("weather.place.no-network").to_string(),
        }
    }

    fn searched(&mut self, reply: &HttpReply) {
        let name = match &self.search {
            Search::Searching(name) => name.clone(),
            _ => String::new(),
        };
        (self.places, self.search) = outcome(reply, name);
        self.list.set_focus(usize::from(!self.places.is_empty()));
    }
}

/// The places a search for `name` found, and how it went.
fn outcome(reply: &HttpReply, name: String) -> (Vec<Place>, Search) {
    if !reply.ok() {
        return (Vec::new(), Search::Failed);
    }
    let places = data::parse_places(&reply.body);
    let search = if places.is_empty() {
        Search::Nothing(name)
    } else {
        Search::Found
    };
    (places, search)
}

impl Screen for PlaceScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        let len = self.len(env);
        self.header.draw(
            &mut ui,
            env.i18n().tr("weather.setting.place"),
            None,
            self.list.paging(len),
        );
        if self.offering(env) {
            return self.offer.draw(&mut ui, self.list.area());
        }
        let places = &self.places;
        let note = self.note(env);
        self.list.draw(&mut ui, len, |ui, row, rect, focused| {
            if row == 0 {
                let title = env.i18n().tr("weather.place.search");
                ui::Row::value(title, "›")
                    .note(&note)
                    .focused(focused)
                    .draw(ui, rect);
            } else if let Some(place) = places.get(row - 1) {
                let note = place.whereabouts();
                ui::Row::value(&place.name, "")
                    .note(&note)
                    .focused(focused)
                    .draw(ui, rect);
            } else {
                let title = env.i18n().tr("weather.place.none");
                let note = env.i18n().tr("weather.place.none.help");
                ui::Row::value(title, "")
                    .note(note)
                    .focused(focused)
                    .draw(ui, rect);
            }
        });
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        // Nothing to choose from yet: the keyboard at once.
        if !self.asked && !Self::has_place(&env) {
            self.asked = true;
            self.ask(&env, cx);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        if *action != actions::BACK && self.offering(&env) {
            let chosen = self.offer.on_action(action, cx).done();
            self.offered(chosen, cx);
            return Handled::Yes;
        }
        let len = self.len(&env);
        let rows = self.list.rows() as isize;
        let delta = if let Some(delta) = ui::list_step(action, rows) {
            delta
        } else if *action == actions::CONFIRM {
            self.open(self.list.focus(), cx);
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        let change = self.list.move_focus_in(cx.env(), delta, len);
        change.request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let len = self.len(&env);
        if gesture.kind == GestureKind::Tap
            && let Some(action) =
                self.header
                    .tap(&env, &self.style, gesture.start, self.list.paging(len))
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        if self.offering(&env) {
            let chosen = self.offer.on_gesture(&gesture, cx).done();
            self.offered(chosen, cx);
            return Handled::Yes;
        }
        let Some(row) = self.list.hit(gesture.start, len) else {
            return Handled::No;
        };
        self.list.set_focus(row);
        if gesture.kind == GestureKind::Tap {
            self.open(row, cx);
            return Handled::Yes;
        }
        Handled::No
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if let Some(typed) = TYPED.read(event, params) {
            let name = typed.text.trim().to_string();
            if name.is_empty() {
                return;
            }
            self.places.clear();
            self.list.set_focus(0);
            if !env.caps().network {
                self.search = Search::NoNetwork;
            } else if self.offer.shows(&env) {
                // Offline mode: asked first, once.
                self.waiting = Some(name);
            } else {
                self.find(name, &env, cx);
            }
            Self::redraw(cx);
        } else if let Some(reply) = SEARCHED.read(event, params) {
            self.searched(&reply);
            Self::redraw(cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::Params;

    #[test]
    fn answers_become_places() {
        let reply = |status, body: &str| HttpReply::new(status, body);
        let name = || String::from("Berlin");
        let (places, search) =
            outcome(&reply(200, include_str!("../testdata/search.json")), name());
        assert_eq!((places.len(), search), (3, Search::Found));
        let (places, search) = outcome(&reply(200, r#"{"generationtime_ms":0.3}"#), name());
        assert!(places.is_empty());
        assert_eq!(search, Search::Nothing(name()));
        let failed = HttpReply::failed(
            paperpaw_sdk::HttpError::NoNetwork,
            "no network",
            Params::new(),
        );
        let (_, search) = outcome(&failed, name());
        assert_eq!(search, Search::Failed);
    }
}
