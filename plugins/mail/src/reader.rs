//! A mail: its headers and text, fetched when it is first opened and kept
//! on the card; *Reply*, *Reply all* and *Star* below.

use alloc::format;
use alloc::string::{String, ToString};

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::ui::{self, Response, TextView, UiStyle};
use paperpaw_sdk::{Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, Screen};

use crate::account::Account;
use crate::browser::param;
use crate::job::{Job, Run};
use crate::session::Task;
use crate::settings::MARK_READ;
use crate::store::{self, Letter, Row};
use crate::{COMPOSE, EVENTS, SOCKET, texts};

/// How a reply is made (parameter of the compose screen).
pub const REPLY: &str = "reply";
pub const REPLY_ALL: &str = "all";
pub const REPLY_ONE: &str = "one";

const BUTTONS: [&str; 3] = ["mail.reply", "mail.reply-all", "mail.star"];

pub(crate) struct Reader {
    style: UiStyle,
    header: ui::Header,
    view: TextView,
    /// Where the text goes.
    body: Rect,
    buttons: [Rect; 3],
    /// Which button has the focus; none while the text is scrolled.
    focus: Option<usize>,
    account: u8,
    folder: String,
    uid: u32,
    letter: Option<Letter>,
    /// The headers and the text as shown.
    shown: String,
    job: Job,
    status: String,
    started: bool,
}

impl Reader {
    pub(crate) fn new(env: &Env<'_>, params: &Params) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let button_h = style.row();
        let top = header.area().height + style.gap();
        let bottom = display.height.saturating_sub(button_h + style.gap());
        let note_h = style.small() as u16 + style.gap();
        let area = Rect::new(style.margin() as u16 / 2, top, display.width.saturating_sub(style.margin() as u16), bottom.saturating_sub(top + note_h));
        let row = Rect::new(style.margin() as u16, bottom, display.width.saturating_sub(2 * style.margin() as u16), button_h);
        let columns = ui::columns(row, 3, style.gap());
        let buttons = [columns[0], columns[1], columns[2]];
        let account = params.get(param::ACCOUNT).and_then(|n| n.parse().ok()).unwrap_or(1);
        let folder = params.get(param::FOLDER).unwrap_or("INBOX").to_string();
        let uid = params.get(param::UID).and_then(|n| n.parse().ok()).unwrap_or(0);
        let mut screen = Self {
            view: TextView::new(env, &style, area, ""),
            body: area,
            style,
            header,
            buttons,
            focus: None,
            account,
            folder,
            uid,
            letter: None,
            shown: String::new(),
            job: Job::new(SOCKET, EVENTS.into()),
            status: String::new(),
            started: false,
        };
        screen.read_kept(env);
        screen
    }

    fn text_file(&self) -> String {
        store::text_file(self.account, &self.folder, self.uid)
    }

    /// Reads the mail from the card if it is there.
    fn read_kept(&mut self, env: &Env<'_>) {
        let Some(letter) = store::read_letter(env.storage(), &self.text_file()) else {
            return;
        };
        let i18n = env.i18n();
        let mut shown = String::new();
        let mut put = |key: &str, value: &str| {
            if !value.is_empty() {
                shown.push_str(i18n.tr(key));
                shown.push_str(": ");
                shown.push_str(value);
                shown.push('\n');
            }
        };
        put("mail.from", &letter.from);
        put("mail.to", &letter.to);
        put("mail.cc", &letter.cc);
        put("mail.subject", &letter.subject);
        put("mail.date-label", &texts::when(env, letter.date));
        if !letter.attachments.is_empty() {
            put("mail.attachments", &letter.attachments.join(", "));
        }
        shown.push('\n');
        shown.push_str(&letter.text);
        if letter.truncated {
            shown.push_str("\n\n");
            shown.push_str(i18n.tr("mail.cut"));
        }
        let area = self.view_area();
        self.view = TextView::new(env, &self.style, area, &shown);
        self.shown = shown;
        self.letter = Some(letter);
    }

    fn view_area(&self) -> Rect {
        self.body
    }

    fn account(&self, env: &Env<'_>) -> Option<Account> {
        Account::all(env).into_iter().find(|a| a.number == self.account)
    }

    /// Fetches the mail (and marks it read on the server, if the settings
    /// say so).
    fn fetch(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let Some(account) = self.account(&env) else {
            self.status = env.i18n().tr("mail.no-account").to_string();
            return;
        };
        let mut tasks = alloc::vec![Task::Body {
            folder: self.folder.clone(),
            uid: self.uid,
            out: store::raw_file(self.account, &self.folder, self.uid),
        }];
        if env.settings().read(&MARK_READ) {
            tasks.push(Task::Flag { folder: self.folder.clone(), uid: self.uid, flag: "\\Seen", on: true });
        }
        for command in self.job.start_imap(&account, tasks) {
            cx.command(command);
        }
    }

    /// The row of this mail in the folder's list, changed by `f`.
    fn change_row(&self, env: &Env<'_>, f: impl FnOnce(&mut Row)) {
        let file = store::index_file(self.account, &self.folder);
        let mut rows = store::read_index(env.storage(), &file);
        if let Some(row) = rows.iter_mut().find(|r| r.uid == self.uid) {
            f(row);
            let _ = store::write_index(env.storage(), &file, &rows);
        }
    }

    fn starred(&self, env: &Env<'_>) -> bool {
        store::read_index(env.storage(), &store::index_file(self.account, &self.folder))
            .iter()
            .any(|r| r.uid == self.uid && r.flagged())
    }

    fn toggle_star(&mut self, cx: &mut Cx<'_, '_>) {
        if self.job.working() {
            return;
        }
        let env = *cx.env();
        let on = !self.starred(&env);
        self.change_row(&env, |row| {
            row.flags.retain(|c| c != 'F');
            if on {
                row.flags.push('F');
            }
        });
        if let Some(account) = self.account(&env) {
            let task = Task::Flag { folder: self.folder.clone(), uid: self.uid, flag: "\\Flagged", on };
            for command in self.job.start_imap(&account, alloc::vec![task]) {
                cx.command(command);
            }
        }
        cx.request_redraw();
    }

    fn reply(&self, all: bool, cx: &mut Cx<'_, '_>) {
        if self.letter.is_none() {
            return;
        }
        cx.command(Command::OpenScreen(
            COMPOSE,
            Params::new()
                .with(param::ACCOUNT, format!("{}", self.account))
                .with(param::FOLDER, self.folder.clone())
                .with(param::UID, format!("{}", self.uid))
                .with(REPLY, if all { REPLY_ALL } else { REPLY_ONE }),
        ));
    }

    fn press(&mut self, button: usize, cx: &mut Cx<'_, '_>) {
        match button {
            0 => self.reply(false, cx),
            1 => self.reply(true, cx),
            _ => self.toggle_star(cx),
        }
    }

    fn progressed(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match self.job.run.clone() {
            Run::Done => {
                if self.letter.is_none() {
                    let raw = store::raw_file(self.account, &self.folder, self.uid);
                    if store::make_letter(env.storage(), &raw, &self.text_file()).is_ok() {
                        self.read_kept(&env);
                    } else {
                        self.status = env.i18n().tr("mail.fail.storage").to_string();
                    }
                    if env.settings().read(&MARK_READ) {
                        self.change_row(&env, |row| {
                            if !row.flags.contains('S') {
                                row.flags.push('S');
                            }
                        });
                    }
                }
                self.job.run = Run::Idle;
            }
            Run::Failed(failure) => self.status = texts::failure(&env, &failure),
            Run::Untrusted { .. } => self.status = env.i18n().tr("mail.pin.open-list").to_string(),
            Run::Idle | Run::Working(_) => {}
        }
        cx.request_redraw();
    }
}

impl Screen for Reader {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let subject = self.letter.as_ref().map_or("", |l| l.subject.as_str());
        let title = if subject.is_empty() { env.i18n().tr("mail.title") } else { subject };
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(&mut ui, title, None, None);
        let note = match &self.job.run {
            Run::Working(stage) => texts::stage(env, *stage).to_string(),
            _ => self.status.clone(),
        };
        if self.letter.is_some() {
            self.view.draw(&mut ui, &self.shown);
        } else if note.is_empty() {
            ui.message(self.view_area(), env.i18n().tr("mail.stage.message"));
        }
        if !note.is_empty() {
            let at = Rect::new(0, self.buttons[0].y.saturating_sub(style.small() as u16 + style.gap()), env.device().display.width, style.small() as u16 + style.gap());
            ui.note_line(at, &note);
        }
        let starred = self.starred(env);
        let shown = ui::focus_shown(env);
        for (i, key) in BUTTONS.iter().enumerate() {
            let key = if i == 2 && starred { "mail.unstar" } else { key };
            ui.button(self.buttons[i], env.i18n().tr(key), false, shown && self.focus == Some(i));
        }
    }

    fn on_show(&mut self, cx: &mut Cx<'_, '_>) {
        if !self.started && self.letter.is_none() {
            self.started = true;
            self.fetch(cx);
        }
        cx.request_redraw();
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        if let Some(notice) = EVENTS.read(event, params) {
            let env = *cx.env();
            for command in self.job.notice(notice, env.storage()) {
                cx.command(command);
            }
            self.progressed(cx);
        }
    }

    fn on_close(&mut self, cx: &mut Cx<'_, '_>) {
        for command in self.job.cancel() {
            cx.command(command);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        if *action == actions::LEFT || *action == actions::RIGHT {
            let step: isize = if *action == actions::RIGHT { 1 } else { -1 };
            let next = (self.focus.map_or(if step > 0 { -1 } else { 3 }, |f| f as isize) + step).rem_euclid(3) as usize;
            self.focus = Some(next);
            cx.request_redraw();
            return Handled::Yes;
        }
        if *action == actions::CONFIRM {
            self.press(self.focus.unwrap_or(0), cx);
            return Handled::Yes;
        }
        match self.view.on_action(action) {
            Response::Ignored => Handled::No,
            Response::Changed => {
                cx.request_redraw();
                Handled::Yes
            }
            _ => Handled::Yes,
        }
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        if gesture.kind == GestureKind::Tap {
            if let Some(action) = self.header.tap(&env, &self.style, gesture.start, None) {
                cx.command(Command::run(action));
                return Handled::Yes;
            }
            if let Some(i) = self.buttons.iter().position(|b| b.contains(gesture.start)) {
                self.press(i, cx);
                return Handled::Yes;
            }
        }
        match self.view.on_gesture(&gesture) {
            Response::Ignored => Handled::No,
            Response::Changed => {
                cx.request_redraw();
                Handled::Yes
            }
            _ => Handled::Yes,
        }
    }
}
