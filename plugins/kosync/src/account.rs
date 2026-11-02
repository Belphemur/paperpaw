//! The account screen: the server, user name and password, then *Sign in*
//! or *Create account*; *Sign out* once signed in. A line below tells what
//! happened, whole: where the screen has no room for it, a row *More about
//! this* shows it to read. Another server signs out: accounts belong to their server
//! (set elsewhere, the plugin signs out as well).
//!
//! The password never leaves this screen: only its MD5 is sent and kept,
//! as KOReader does.

use alloc::borrow::Cow;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::ask;
use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::{Confirmed, HttpReply, Typed};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, RefreshMode,
    Screen,
};

use paperpaw_sdk::digest::md5;
use paperpaw_sdk::json;

use crate::{
    ACCOUNT_CHANGED, SERVER, USER, normalize_server, save_key, server, server_label, server_request,
};

/// Replies of the keyboard and the server.
const SERVER_ENTERED: Event<Typed> = Event::new("kosync.server-entered");
const USER_ENTERED: Event<Typed> = Event::new("kosync.user-entered");
const PASSWORD_ENTERED: Event<Typed> = Event::new("kosync.password-entered");
const AUTHORIZED: Event<HttpReply> = Event::new("kosync.authorized");
const CREATED: Event<HttpReply> = Event::new("kosync.created");
/// The message read whole, closed.
const READ: Event<Confirmed> = Event::new("kosync.message-read");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Row {
    Server,
    User,
    Password,
    SignIn,
    Create,
    SignOut,
    /// The message to read whole, where the screen has no room for it.
    More,
}

#[derive(Debug)]
pub(crate) struct AccountScreen {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    /// Where the rows go, as many as the screen holds.
    list_area: Rect,
    user: String,
    /// The MD5 of the password entered.
    key: Option<String>,
    /// The account signed in with (the setting).
    signed_in: String,
    /// Waiting for the server.
    busy: bool,
    message: String,
    /// Offline mode: the offer to go online, and the row (*Sign in* or
    /// *Create*) that waits for it.
    online: ui::offline::OfflineOffer,
    waiting: Option<Row>,
}

impl AccountScreen {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        // No longer than the screen: the list pages where its rows are tall.
        let rows = ((u32::from(style.row()) * 6) as u16).min(display.height.saturating_sub(top));
        let list = ListView::new(
            Rect::new(0, top, display.width, rows),
            u32::from(style.row()),
        )
        .focus_on_keys(env);
        let signed_in = env.settings().read(&USER).to_string();
        let mut screen = Self {
            style,
            header,
            list,
            list_area: Rect::new(0, top, display.width, rows),
            user: signed_in.clone(),
            key: None,
            signed_in,
            busy: false,
            message: String::new(),
            online: ui::offline::OfflineOffer::new(),
            waiting: None,
        };
        screen.message = screen.state_message(env);
        screen
    }

    fn rows(&self, env: &Env<'_>) -> Vec<Row> {
        let mut rows = Vec::from([
            Row::Server,
            Row::User,
            Row::Password,
            Row::SignIn,
            Row::Create,
        ]);
        if !self.signed_in.is_empty() {
            rows.push(Row::SignOut);
        }
        if !self.fits(env, rows.len()) {
            rows.push(Row::More);
        }
        rows
    }

    /// Whether the message shows whole below `rows` rows.
    fn fits(&self, env: &Env<'_>, rows: usize) -> bool {
        let needs = ui::message_height(env, &self.style, self.list_area.width, &self.message);
        self.note(env, rows).height >= needs
    }

    /// Where the message goes: right below the `rows` shown, down to the
    /// margin. An explanation takes all the room there is, so that it shows
    /// whole wherever the screen holds it.
    fn note(&self, env: &Env<'_>, rows: usize) -> Rect {
        let style = &self.style;
        let shown = (style.row())
            .saturating_mul(rows as u16)
            .min(self.list_area.height);
        let top = self.list_area.y + shown + style.gap();
        let bottom = env.device().display.height.saturating_sub(style.margin());
        let width = self.list_area.width;
        // As high as the message needs, right below the rows.
        let needs = ui::message_height(env, style, width, &self.message);
        Rect::new(0, top, width, needs.min(bottom.saturating_sub(top)))
    }

    /// What the line below says when nothing happened yet.
    fn state_message(&self, env: &Env<'_>) -> String {
        if !env.caps().network {
            env.i18n().tr("kosync.no-network").to_string()
        } else if self.signed_in.is_empty() {
            env.i18n().tr("kosync.signed-out").to_string()
        } else {
            let url = server(env);
            let args = [
                ("user", Arg::Str(&self.signed_in)),
                ("server", Arg::Str(server_label(&url))),
            ];
            env.i18n().format("kosync.signed-in", &args)
        }
    }

    /// Whether the offer to go online shows in place of the rows.
    fn offering(&self, env: &Env<'_>) -> bool {
        self.waiting.is_some() && self.online.shows(env)
    }

    /// What the reader chose on the offer: the server is asked, or the
    /// screen says it needs the network.
    fn offered(&mut self, chosen: Option<ui::offline::Offer>, cx: &mut Cx<'_, '_>) {
        let (Some(chosen), Some(row)) = (chosen, self.waiting.take()) else {
            return;
        };
        match chosen {
            ui::offline::Offer::Online => self.activate(row, cx),
            _ => {
                let message = cx.env().i18n().tr("kosync.no-network").to_string();
                self.show(cx, message);
            }
        }
    }

    fn show(&mut self, cx: &mut Cx<'_, '_>, message: String) {
        self.message = message;
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }

    fn activate(&mut self, row: Row, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match row {
            Row::Server => cx.command(
                ask::text(env.i18n().tr("kosync.server"), &SERVER_ENTERED)
                    .url()
                    .initial(server(&env))
                    .command(),
            ),
            Row::User => cx.command(
                ask::text(env.i18n().tr("kosync.user"), &USER_ENTERED)
                    .initial(self.user.clone())
                    .command(),
            ),
            Row::Password => cx.command(
                ask::text(env.i18n().tr("kosync.password"), &PASSWORD_ENTERED)
                    .password()
                    .command(),
            ),
            Row::SignIn | Row::Create => {
                if !env.caps().network {
                    return self.show(cx, env.i18n().tr("kosync.no-network").to_string());
                }
                let Some(key) = self.key.clone().filter(|_| !self.user.is_empty()) else {
                    return self.show(cx, env.i18n().tr("kosync.missing").to_string());
                };
                // Offline mode: asked first, once.
                if self.online.shows(&env) {
                    self.waiting = Some(row);
                    cx.request_redraw();
                    cx.request_refresh(RefreshMode::Fast);
                    return;
                }
                let request = if row == Row::SignIn {
                    let auth = Some((self.user.as_str(), key.as_str()));
                    server_request(
                        &env,
                        "GET",
                        "/users/auth",
                        None,
                        &AUTHORIZED,
                        Params::new(),
                        auth,
                    )
                } else {
                    let body = format!(
                        "{{\"username\":{},\"password\":{}}}",
                        json::string(&self.user),
                        json::string(&key)
                    );
                    server_request(
                        &env,
                        "POST",
                        "/users/create",
                        Some(body),
                        &CREATED,
                        Params::new(),
                        None,
                    )
                };
                cx.command(Command::http(request));
                self.busy = true;
                self.show(cx, env.i18n().tr("kosync.connecting").to_string());
            }
            Row::More => cx.command(
                ask::confirm(env.i18n().tr("kosync.group"), &READ)
                    .text(self.message.clone())
                    .only_yes()
                    .command(),
            ),
            Row::SignOut => {
                self.sign_out(cx);
                self.list.set_focus(0);
                let message = self.state_message(&env);
                self.show(cx, message);
            }
        }
    }

    fn sign_out(&mut self, cx: &mut Cx<'_, '_>) {
        self.signed_in.clear();
        for command in crate::sign_out() {
            cx.command(command);
        }
    }

    /// The server's answer to signing in or creating the account.
    fn answered(&mut self, created: bool, status: &str, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        self.busy = false;
        let ok = if created { "201" } else { "200" };
        if status == ok {
            let key = self.key.clone().unwrap_or_default();
            cx.command(save_key(Some(&key)));
            self.signed_in = self.user.clone();
            cx.command(Command::set(&USER, Cow::Owned(self.user.clone())));
            cx.command(ACCOUNT_CHANGED.emit(()));
            let message = self.state_message(&env);
            return self.show(cx, message);
        }
        let key = match status {
            "402" if created => "kosync.taken",
            _ => crate::failure(status),
        };
        let message = env.i18n().format(key, &[("status", Arg::Str(status))]);
        self.show(cx, message);
    }
}

impl Screen for AccountScreen {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let mut ui = ui::Ui::new(canvas, env, &style);
        let rows = self.rows(env);
        self.list.set_len(rows.len());
        let title = env.i18n().tr("kosync.group");
        self.header.draw(&mut ui, title, None, None);
        if self.offering(env) {
            let top = self.list_area.y;
            let area = Rect::new(
                0,
                top,
                env.device().display.width,
                env.device().display.height.saturating_sub(top),
            );
            return self.online.draw(&mut ui, area);
        }
        let i18n = env.i18n();
        self.list
            .draw(&mut ui, rows.len(), |ui, index, rect, focused| {
                let url = server(env);
                let (title, value) = match rows[index] {
                    Row::Server => (i18n.tr("kosync.server"), server_label(&url).to_string()),
                    Row::User => (i18n.tr("kosync.user"), self.user.clone()),
                    Row::Password => {
                        let dots = if self.key.is_some() {
                            "••••••"
                        } else {
                            ""
                        };
                        (i18n.tr("kosync.password"), dots.to_string())
                    }
                    Row::SignIn => (i18n.tr("kosync.sign-in"), "›".to_string()),
                    Row::Create => (i18n.tr("kosync.create"), "›".to_string()),
                    Row::SignOut => (i18n.tr("kosync.sign-out"), "›".to_string()),
                    Row::More => (i18n.tr("kosync.more"), "›".to_string()),
                };
                ui::Row::value(title, &value)
                    .focused(focused)
                    .draw(ui, rect);
            });
        // Whole, or not at all: then *More about this* shows it.
        if !rows.contains(&Row::More) {
            ui.message(self.note(env, rows.len()), &self.message);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let rows = self.rows(cx.env());
        let a = action;
        if *a != actions::BACK && self.offering(cx.env()) {
            let chosen = self.online.on_action(a, cx).done();
            self.offered(chosen, cx);
            return Handled::Yes;
        }
        let delta = if *a == actions::UP || *a == actions::LEFT || *a == actions::PREVIOUS {
            -1
        } else if *a == actions::DOWN || *a == actions::RIGHT || *a == actions::NEXT {
            1
        } else if *a == actions::CONFIRM {
            if !self.busy {
                self.activate(rows[self.list.focus().min(rows.len() - 1)], cx);
            }
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list
            .move_focus_in(cx.env(), delta, rows.len())
            .request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        if gesture.kind != GestureKind::Tap {
            return Handled::No;
        }
        if let Some(action) = self.header.tap(cx.env(), &self.style, gesture.start, None) {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        if self.offering(cx.env()) {
            let chosen = self.online.on_gesture(&gesture, cx).done();
            self.offered(chosen, cx);
            return Handled::Yes;
        }
        let rows = self.rows(cx.env());
        let Some(index) = self.list.hit(gesture.start, rows.len()) else {
            return Handled::No;
        };
        if index != self.list.focus() {
            self.list.set_focus(index);
            cx.request_redraw();
            cx.request_refresh(RefreshMode::Fast);
        }
        if !self.busy {
            self.activate(rows[index], cx);
        }
        Handled::Yes
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(typed) = SERVER_ENTERED.read(event, params) {
            let url = normalize_server(&typed.text);
            if url != server(cx.env()) {
                let changed = !self.signed_in.is_empty();
                if changed {
                    self.sign_out(cx);
                }
                cx.command(Command::set(&SERVER, Cow::Owned(url.clone())));
                let key = if changed {
                    "kosync.server-changed"
                } else {
                    "kosync.server-set"
                };
                let args = [("server", Arg::Str(server_label(&url)))];
                self.message = cx.env().i18n().format(key, &args);
            }
            // Next: the user name.
            self.list.set_focus(1);
        } else if let Some(typed) = USER_ENTERED.read(event, params) {
            self.user = typed.text.trim().to_string();
            // Next: the password.
            self.list.set_focus(2);
        } else if let Some(typed) = PASSWORD_ENTERED.read(event, params) {
            self.key = (!typed.text.is_empty()).then(|| md5::of(&typed.text));
            self.list.set_focus(3);
        } else if let Some(reply) = AUTHORIZED.read(event, params) {
            return self.answered(false, &reply.status.to_string(), cx);
        } else if let Some(reply) = CREATED.read(event, params) {
            return self.answered(true, &reply.status.to_string(), cx);
        } else {
            return;
        }
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }
}
