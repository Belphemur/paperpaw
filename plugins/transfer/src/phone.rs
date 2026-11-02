//! Typing on a phone (slot `core.remote-keyboard`): the device serves a
//! small page with a text field to the local network and shows its
//! address as a QR code; what is sent from there comes back to the
//! keyboard. Handy for passwords and keys, which are tedious to type on
//! an e-reader.
//!
//! The address carries a key drawn anew each time, guarded as the
//! transfer's (see `access`); the page is served only while the screen
//! is open, and the screen closes once the text has come.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_qr::QrCode;
use paperpaw_sdk::builtin::{actions, params};
use paperpaw_sdk::event::{Event, EventData};
use paperpaw_sdk::replies::{ServerNotice, Typed};
use paperpaw_sdk::served::{self, Access, Guard};
use paperpaw_sdk::ui::{self, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Instant, Params, Rect,
    RefreshMode, Screen, ServerResponse, Storage,
};

use crate::{PORT, no_random};

const SERVER: Id = Id::new_static("transfer.phone-server");
const SERVER_EVENT: Event<ServerNotice> = Event::new("transfer.phone-server-event");
/// Longest text taken, in bytes.
pub(crate) const MAX_TEXT: usize = 4096;

/// The web page; its texts come from the translations.
const PAGE: &str = include_str!("../data/phone.html");

#[derive(Debug)]
enum State {
    Starting,
    Serving { url: String, qr: Option<QrCode> },
    Failed(String),
}

pub(crate) struct PhoneScreen {
    style: UiStyle,
    header: ui::Header,
    /// The keyboard's parameters, sent back with the text.
    params: Params,
    state: State,
    /// The key in the address, drawn once the server runs.
    guard: Option<Guard>,
    page: String,
    started: bool,
}

impl PhoneScreen {
    pub(crate) fn new(env: &Env<'_>, params: &Params) -> Self {
        let style = UiStyle::new(env);
        Self {
            header: ui::Header::new(env, &style, true),
            style,
            params: params.clone(),
            state: State::Starting,
            guard: None,
            page: page(env, params),
            started: false,
        }
    }

    fn content(&self, env: &Env<'_>) -> Rect {
        let display = env.device().display;
        let top = self.header.area().height + self.style.gap();
        Rect::new(0, top, display.width, display.height.saturating_sub(top))
    }

    fn on_server_event(&mut self, notice: ServerNotice, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match notice {
            ServerNotice::Started { address, .. } => {
                self.state = match served::key(&env.random()) {
                    Some(key) => {
                        let (url, scanned) =
                            crate::link::urls(env.wifi().map(|wifi| &wifi.state), &address, &key);
                        self.guard = Some(Guard::new(key));
                        let qr = QrCode::new(scanned.as_bytes());
                        State::Serving { url, qr }
                    }
                    None => {
                        cx.command(Command::StopServing(SERVER));
                        State::Failed(no_random(&env))
                    }
                };
            }
            ServerNotice::Failed { reason, .. } => {
                self.state = State::Failed(if reason == "no network" {
                    env.i18n().tr("transfer.no-network").to_string()
                } else {
                    env.i18n()
                        .format("transfer.failed", &[("reason", Arg::Str(&reason))])
                });
            }
            ServerNotice::Request {
                request: number,
                method,
                target,
                body_file,
                ..
            } => {
                let Some(guard) = &mut self.guard else {
                    return;
                };
                let request = Request {
                    method: &method,
                    target: &target,
                    body_file: body_file.as_deref(),
                    at: env.now(),
                };
                let (response, text) = answer(number, guard, &self.page, env.storage(), &request);
                cx.command(Command::respond(response));
                // The text to the keyboard, and done.
                if let Some(text) = text
                    && let Some(reply) = self
                        .params
                        .get(params::REPLY)
                        .and_then(|r| Id::parse(r).ok())
                {
                    cx.command(Command::Back);
                    let typed = Typed::new(text, self.params.clone());
                    cx.command(Command::Emit(reply, typed.to_params()));
                }
                return;
            }
            _ => return,
        }
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Partial);
    }
}

/// A request to the page.
pub(crate) struct Request<'a> {
    pub method: &'a str,
    pub target: &'a str,
    /// Where its body was saved, if it had one.
    pub body_file: Option<&'a str>,
    /// When it came.
    pub at: Instant,
}

/// The answer to `request`, if `guard` lets it in, and the text sent, if
/// that is what it did. A text longer than [`MAX_TEXT`] is refused
/// (*413*), so the page keeps it and says so, and the field it was typed
/// for is left as it is.
pub(crate) fn answer(
    number: u32,
    guard: &mut Guard,
    page: &str,
    storage: &dyn Storage,
    request: &Request<'_>,
) -> (ServerResponse, Option<String>) {
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((request.target, ""));
    let response =
        |status, kind: &str, body: &[u8]| ServerResponse::new(number, status, kind, body.to_vec());
    match guard.check(served::param(query, "k").as_deref(), request.at) {
        Access::Granted => {}
        Access::Locked => {
            let text = b"Too many wrong keys: try again in a minute.";
            return (response(429, "text/plain; charset=utf-8", text), None);
        }
        // Denied, and a state to come: refused as a wrong key is.
        _ => {
            let text = b"The key is missing or wrong: scan the code shown on the device.";
            return (response(403, "text/plain; charset=utf-8", text), None);
        }
    }
    if request.method == "POST" && path == "/send" {
        let plain = "text/plain; charset=utf-8";
        let data = match request.body_file {
            None => Ok(Vec::new()),
            Some(file) if storage.size(file).is_ok_and(|size| size > MAX_TEXT as u64) => {
                let text = b"The text is too long for the device: send a shorter one.";
                return (response(413, plain, text), None);
            }
            Some(file) => paperpaw_sdk::read_file(storage, file, MAX_TEXT),
        };
        return match data {
            Ok(data) => (
                response(200, "application/json", b"{\"ok\":true}"),
                Some(String::from_utf8_lossy(&data).into_owned()),
            ),
            Err(_) => (response(500, plain, b"The text could not be read."), None),
        };
    }
    if path == "/" {
        (
            response(200, "text/html; charset=utf-8", page.as_bytes()),
            None,
        )
    } else {
        (
            response(404, "text/plain; charset=utf-8", b"not found"),
            None,
        )
    }
}

/// The page with its texts and what the keyboard asked for.
fn page(env: &Env<'_>, params: &Params) -> String {
    let texts = [
        ("send", "transfer.phone.send"),
        ("sent", "transfer.phone.sent"),
        ("failed", "transfer.web.failed"),
        ("show", "transfer.phone.show"),
    ];
    let mut json = String::from("{");
    for (name, key) in texts {
        json.push_str(&format!(
            "{name}:{},",
            served::json_string(env.i18n().tr(key))
        ));
    }
    let title = params.get(params::TITLE).unwrap_or_default();
    let title = if title.is_empty() {
        env.i18n().tr("transfer.phone.title")
    } else {
        title
    };
    let password = params.get(params::MODE) == Some("password");
    json.push_str(&format!(
        "title:{},text:{},password:{password}}}",
        served::json_string(title),
        served::json_string(if password {
            ""
        } else {
            params.get(params::TEXT).unwrap_or_default()
        }),
    ));
    PAGE.replace("/*STRINGS*/{}", &json)
}

impl Screen for PhoneScreen {
    fn hint(&self, action: &Id, _: &Env<'_>) -> paperpaw_sdk::Hint {
        // Only back does anything here.
        paperpaw_sdk::Hint::only_back(action)
    }

    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        let title = env.i18n().tr("transfer.phone.title");
        self.header.draw(&mut ui, title, None, None);
        let area = self.content(env);
        match &self.state {
            State::Starting => ui.message(area, env.i18n().tr("transfer.starting")),
            State::Failed(message) => ui.message(area, message),
            State::Serving { url, qr } => {
                let hint = env.i18n().tr("transfer.phone.hint");
                ui::draw_address(ui.canvas(), env, &style, area, url, qr.as_ref(), hint, None);
            }
        }
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        if !self.started {
            self.started = true;
            if !cx.env().random().available() {
                self.state = State::Failed(no_random(cx.env()));
                cx.request_redraw();
                cx.request_refresh(RefreshMode::Partial);
                return;
            }
            cx.command(Command::serve(SERVER, PORT, SERVER_EVENT.into()));
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        if *action == actions::BACK {
            cx.command(Command::Back);
            Handled::Yes
        } else {
            Handled::No
        }
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind == GestureKind::Tap
            && let Some(action) = self.header.tap(cx.env(), &self.style, gesture.start, None)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        Handled::No
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(notice) = SERVER_EVENT.read(event, params)
            && *notice.server() == SERVER
        {
            self.on_server_event(notice, cx);
        }
    }
}
