//! Writing a mail: the sender, the recipients, the subject and the text, a
//! row each, typed on the keyboard; *Send* below. What is written is kept
//! on the card as a draft until it is sent.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::builtin::actions;
use paperpaw_sdk::replies::Typed;
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Id, Params, Rect, Screen, Storage,
};

use crate::account::Account;
use crate::browser::param;
use crate::compose::{self, Draft, Problem};
use crate::job::{Job, Run};
use crate::reader::{REPLY, REPLY_ALL};
use crate::session::Outgoing;
use crate::settings::SIGNATURE;
use crate::{COMPOSE_CHANNEL, TYPED, smtp, store, texts};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    From,
    To,
    Cc,
    Subject,
    Body,
    Send,
}

impl Part {
    fn title(self) -> &'static str {
        match self {
            Part::From => "mail.from",
            Part::To => "mail.to",
            Part::Cc => "mail.cc",
            Part::Subject => "mail.subject",
            Part::Body => "mail.body",
            Part::Send => "mail.send",
        }
    }

    fn field(self) -> &'static str {
        match self {
            Part::To => "to",
            Part::Cc => "cc",
            Part::Subject => "subject",
            Part::Body => "body",
            Part::From | Part::Send => "",
        }
    }

    fn of_field(field: &str) -> Option<Self> {
        [Part::To, Part::Cc, Part::Subject, Part::Body]
            .into_iter()
            .find(|p| p.field() == field)
    }
}

pub(crate) struct Writer {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
    note: Rect,
    accounts: Vec<Account>,
    /// Which of `accounts` sends.
    from: usize,
    draft: Draft,
    job: Job,
    status: String,
    sent: bool,
}

impl Writer {
    pub(crate) fn new(env: &Env<'_>, params: &Params) -> Self {
        let style = UiStyle::new(env);
        let header = ui::Header::new(env, &style, true);
        let display = env.device().display;
        let top = header.area().height + style.gap();
        let note = Rect::new(
            0,
            display.height.saturating_sub(style.row()),
            display.width,
            style.row(),
        );
        let area = Rect::new(0, top, display.width, note.y.saturating_sub(top));
        let accounts = Account::all(env);
        let wanted: u8 = params
            .get(param::ACCOUNT)
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let from = accounts
            .iter()
            .position(|a| a.number == wanted)
            .unwrap_or(0);
        let mut screen = Self {
            list: ListView::new(area, u32::from(style.row())).focus_on_keys(env),
            style,
            header,
            note,
            accounts,
            from,
            draft: Draft::default(),
            job: Job::new(
                COMPOSE_CHANNEL.socket.clone(),
                COMPOSE_CHANNEL.events.id().clone(),
            ),
            status: String::new(),
            sent: false,
        };
        screen.start(env, params);
        screen
    }

    fn account(&self) -> Option<&Account> {
        self.accounts.get(self.from)
    }

    /// A reply, or the draft kept, or a mail with the signature only.
    fn start(&mut self, env: &Env<'_>, params: &Params) {
        let Some(account) = self.account().cloned() else {
            return;
        };
        let letter = params
            .get(param::UID)
            .and_then(|uid| uid.parse().ok())
            .zip(params.get(param::FOLDER))
            .and_then(|(uid, folder)| {
                store::read_letter(
                    env.storage(),
                    &store::text_file(account.number, folder, uid),
                )
            });
        let kept = || {
            paperpaw_sdk::files::read_text(
                env.storage(),
                &store::draft_file(account.number),
                compose::MAX_BODY + 4096,
            )
            .ok()
        };
        self.draft = if let Some(letter) = letter {
            let who = header_name(&letter.from);
            let intro = env
                .i18n()
                .format("mail.reply.intro", &[("who", Arg::Str(&who))]);
            compose::reply_to(
                &letter,
                &account.address,
                params.get(REPLY) == Some(REPLY_ALL),
                &intro,
            )
        } else if let Some(text) = kept() {
            Draft::from_text(&text)
        } else {
            let signature = env
                .settings()
                .text(crate::settings::SIGNATURE.as_str())
                .trim()
                .to_string();
            Draft {
                body: if signature.is_empty() {
                    String::new()
                } else {
                    format!("\n\n-- \n{signature}\n")
                },
                ..Draft::default()
            }
        };
        let _ = SIGNATURE;
        self.draft.from = account.address.clone();
        self.draft.from_name = account.name.clone();
    }

    fn parts(&self) -> Vec<Part> {
        let mut parts = Vec::new();
        if self.accounts.len() > 1 {
            parts.push(Part::From);
        }
        parts.extend([Part::To, Part::Cc, Part::Subject, Part::Body, Part::Send]);
        parts
    }

    fn value(&self, part: Part, env: &Env<'_>) -> String {
        match part {
            Part::From => self
                .account()
                .map(|a| a.address.clone())
                .unwrap_or_default(),
            Part::To => self.draft.to.clone(),
            Part::Cc => self.draft.cc.clone(),
            Part::Subject => self.draft.subject.clone(),
            Part::Body => {
                let first = self
                    .draft
                    .body
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("");
                if first.is_empty() {
                    env.i18n().tr("mail.body.empty").into()
                } else {
                    first.to_string()
                }
            }
            Part::Send => String::new(),
        }
    }

    fn keep(&self, storage: &dyn Storage) {
        if let Some(account) = self.account() {
            let file = store::draft_file(account.number);
            let _ = store::write_atomic(storage, &file, self.draft.to_text().as_bytes());
        }
    }

    fn open(&mut self, part: Part, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match part {
            Part::From => {
                self.from = (self.from + 1) % self.accounts.len().max(1);
                if let Some((address, name)) =
                    self.account().map(|a| (a.address.clone(), a.name.clone()))
                {
                    self.draft.from = address;
                    self.draft.from_name = name;
                }
                cx.request_redraw();
            }
            Part::Send => self.send(cx),
            part => {
                let i18n = env.i18n();
                let current = match part {
                    Part::To => &self.draft.to,
                    Part::Cc => &self.draft.cc,
                    Part::Subject => &self.draft.subject,
                    _ => &self.draft.body,
                };
                let mut question = paperpaw_sdk::ask::text(i18n.tr(part.title()), &TYPED)
                    .initial(current.clone())
                    .with("field", part.field());
                if matches!(part, Part::To | Part::Cc) {
                    question = question.email();
                }
                // A message has lines: the keyboard has a key for the break.
                if part == Part::Body {
                    question = question.multiline();
                }
                cx.command(question.command());
            }
        }
    }

    fn send(&mut self, cx: &mut Cx<'_, '_>) {
        if self.job.working() || self.sent {
            return;
        }
        let env = *cx.env();
        let Some(account) = self.account().cloned() else {
            return;
        };
        let to = match self.draft.check() {
            Ok(to) => to,
            Err(problem) => {
                let key = match problem {
                    Problem::NoSender => "mail.problem.sender",
                    Problem::NoRecipient => "mail.problem.recipient",
                    Problem::BadAddress => "mail.problem.address",
                    Problem::TooManyRecipients => "mail.problem.many",
                    Problem::TooLong => "mail.problem.long",
                };
                self.status = env.i18n().tr(key).to_string();
                cx.request_redraw();
                return;
            }
        };
        let now = paperpaw_sdk::time::unix_seconds(&env).unwrap_or(0);
        let mut random = [0u8; 8];
        let id = if env.random().fill(&mut random) {
            random.iter().map(|b| format!("{b:02x}")).collect()
        } else {
            format!("{now}")
        };
        let message = self.draft.message(&compose::date_header(now), &id);
        let outgoing = Outgoing {
            from: account.address.clone(),
            to,
            data: smtp::data(&message),
        };
        self.status.clear();
        for command in self.job.start_smtp(&account, outgoing) {
            cx.command(command);
        }
        self.job.watch(cx);
        cx.request_redraw();
    }

    fn progressed(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        match self.job.run.clone() {
            Run::Done => {
                self.sent = true;
                self.job.run = Run::Idle;
                if let Some(a) = self.account() {
                    let _ = env.storage().remove(&store::draft_file(a.number));
                }
                self.status = env.i18n().tr("mail.sent").to_string();
            }
            Run::Failed(failure) => self.status = texts::failure(&env, &failure),
            Run::Untrusted { smtp, .. } => {
                self.status = texts::untrusted(&env, self.job.account, smtp);
            }
            Run::Idle | Run::Working(_) => {}
        }
        cx.request_redraw();
    }
}

/// The name of a sender, else its address: `Jane Doe <jane@x>` is `Jane Doe`.
fn header_name(from: &str) -> String {
    crate::header::addresses(from)
        .first()
        .map_or_else(|| from.to_string(), |a| a.shown().to_string())
}

impl Screen for Writer {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let parts = self.parts();
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header.draw(
            &mut ui,
            env.i18n().tr("mail.write"),
            None,
            self.list.paging(parts.len()),
        );
        if self.accounts.is_empty() {
            ui.message(self.list.area(), env.i18n().tr("mail.set-up.help"));
            return;
        }
        let values: Vec<String> = parts.iter().map(|p| self.value(*p, env)).collect();
        self.list
            .draw(&mut ui, parts.len(), |ui, row, rect, focused| {
                let Some(part) = parts.get(row) else {
                    return;
                };
                let value = if *part == Part::Send {
                    "›"
                } else {
                    values[row].as_str()
                };
                ui::Row::value(env.i18n().tr(part.title()), value)
                    .focused(focused)
                    .draw(ui, rect);
            });
        let note = match &self.job.run {
            Run::Working(stage) => texts::stage(env, *stage).to_string(),
            _ => self.status.clone(),
        };
        if !note.is_empty() {
            ui.note_line(self.note, &note);
        }
    }

    fn on_event(&mut self, event: &Id, params: &Params, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        if let Some(notice) = COMPOSE_CHANNEL.events.read(event, params) {
            for command in self.job.notice(notice, env.storage()) {
                cx.command(command);
            }
            self.job.watch(cx);
            self.progressed(cx);
        } else if let Some(typed) = TYPED.read(event, params).filter(|t: &Typed| !t.typing) {
            let text = typed.text;
            match typed.context.get("field").and_then(Part::of_field) {
                Some(Part::To) => self.draft.to = text.trim().into(),
                Some(Part::Cc) => self.draft.cc = text.trim().into(),
                Some(Part::Subject) => self.draft.subject = text.trim().into(),
                Some(Part::Body) => self.draft.body = text,
                _ => return,
            }
            self.keep(env.storage());
            self.status.clear();
            cx.request_redraw();
        }
    }

    fn on_timer(&mut self, cx: &mut Cx<'_, '_>) {
        if self.job.timer(cx) {
            self.progressed(cx);
        }
    }

    fn on_close(&mut self, cx: &mut Cx<'_, '_>) {
        for command in self.job.cancel() {
            cx.command(command);
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = self.parts().len();
        if *action == actions::CONFIRM {
            if self.sent {
                cx.command(Command::Back);
                return Handled::Yes;
            }
            if let Some(part) = self.parts().get(self.list.focus()).copied() {
                self.open(part, cx);
            }
            return Handled::Yes;
        }
        let Some(delta) = ui::list_step(action, self.list.rows() as isize) else {
            return Handled::No;
        };
        self.list.move_focus(delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        let env = *cx.env();
        let parts = self.parts();
        if gesture.kind == GestureKind::Tap
            && let Some(action) = self.header.tap(
                &env,
                &self.style,
                gesture.start,
                self.list.paging(parts.len()),
            )
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        let Some(row) = self.list.hit(gesture.start, parts.len()) else {
            return Handled::No;
        };
        self.list.set_focus(row);
        if gesture.kind == GestureKind::Tap {
            if self.sent {
                cx.command(Command::Back);
            } else if let Some(part) = parts.get(row).copied() {
                self.open(part, cx);
            }
            return Handled::Yes;
        }
        Handled::No
    }
}
