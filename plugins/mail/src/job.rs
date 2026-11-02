//! One connection to a server, held by the screen that asked for it (a
//! socket closes with the screen that opened it): which session runs, how
//! far it came and how it ended. The screens lend it the socket's notices
//! and push the commands it gives back.
//!
//! Background fetching would start a [`Job`] from a declared work item
//! instead of a screen; nothing here depends on a screen being open
//! except where the socket lives (see the note on `start`).

use alloc::vec::Vec;

use paperpaw_sdk::{CertificateHash, Command, Distrust, Id, KeyHash, SocketNotice, Storage};

use crate::account::{Account, field, key};
use crate::driver::{self, Socket};
use crate::session::{Event, Failure, Imap, Outgoing, Smtp, Stage, Task};

/// Where a job is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Run {
    #[default]
    Idle,
    Working(Stage),
    Done,
    Failed(Failure),
    /// The server's certificate is not trusted: its hashes for the reader
    /// to compare, and which of the account's servers it is.
    Untrusted { smtp: bool, certificate: CertificateHash, key: KeyHash, why: Distrust },
}

#[derive(Debug)]
enum Session {
    None,
    Imap(Imap),
    Smtp(Smtp),
}

/// A connection, in the making or over.
#[derive(Debug)]
pub struct Job {
    socket: Socket,
    session: Session,
    pub run: Run,
    /// The account it is for.
    pub account: u8,
}

impl Job {
    /// A job on socket `socket`, whose notices come as `events`.
    pub fn new(socket: Id, events: Id) -> Self {
        Self { socket: Socket::new(socket, events), session: Session::None, run: Run::Idle, account: 0 }
    }

    pub fn working(&self) -> bool {
        matches!(self.run, Run::Working(_))
    }

    /// Starts `tasks` on the account's IMAP server. Nothing is done while
    /// another connection of the job is open.
    pub fn start_imap(&mut self, account: &Account, tasks: Vec<Task>) -> Vec<Command> {
        if self.working() {
            return Vec::new();
        }
        self.account = account.number;
        let mut imap = Imap::new(account.imap.endpoint(&account.user, &account.password), tasks);
        let events = imap.start(&mut self.socket);
        self.session = Session::Imap(imap);
        self.apply(events);
        self.socket.take()
    }

    /// Sends `message` through the account's SMTP server.
    pub fn start_smtp(&mut self, account: &Account, message: Outgoing) -> Vec<Command> {
        if self.working() {
            return Vec::new();
        }
        self.account = account.number;
        let mut smtp = Smtp::new(account.smtp.endpoint(&account.user, &account.password), message);
        let events = smtp.start(&mut self.socket);
        self.session = Session::Smtp(smtp);
        self.apply(events);
        self.socket.take()
    }

    /// Gives up: the socket closes.
    pub fn cancel(&mut self) -> Vec<Command> {
        let was = self.working();
        self.session = Session::None;
        if was {
            self.run = Run::Idle;
            return alloc::vec![self.socket.disconnect()];
        }
        Vec::new()
    }

    /// Whether `notice` is for this job's socket.
    pub fn is_mine(&self, notice: &SocketNotice) -> bool {
        self.socket.is_mine(notice)
    }

    /// Feeds a notice to the session.
    pub fn notice(&mut self, notice: SocketNotice, storage: &dyn Storage) -> Vec<Command> {
        if !self.socket.is_mine(&notice) || !self.working() {
            return Vec::new();
        }
        let Some(input) = driver::input_of(notice) else {
            return Vec::new();
        };
        let events = match &mut self.session {
            Session::Imap(s) => s.step(input, &mut self.socket, storage),
            Session::Smtp(s) => s.step(input, &mut self.socket),
            Session::None => Vec::new(),
        };
        self.apply(events);
        self.socket.take()
    }

    fn apply(&mut self, events: Vec<Event>) {
        let smtp = matches!(self.session, Session::Smtp(_));
        for event in events {
            self.run = match event {
                Event::Stage(stage) => Run::Working(stage),
                Event::Done => Run::Done,
                Event::Failed(failure) => Run::Failed(failure),
                Event::Untrusted { certificate, key, why } => Run::Untrusted { smtp, certificate, key, why },
            };
        }
        if !self.working() {
            self.session = Session::None;
        }
    }
}

/// The command that keeps `key` as the trusted key of the account's server.
pub fn pin(account: u8, smtp: bool, trusted: &KeyHash) -> Command {
    let which = if smtp { field::SMTP_PIN } else { field::IMAP_PIN };
    let id = Id::parse(&key(account, which)).unwrap_or_else(|_| Id::new_static("mail.1.imap-pin"));
    Command::SetSetting(id, paperpaw_sdk::SettingValue::Text(alloc::borrow::Cow::Owned(alloc::format!("{trusted}"))))
}
