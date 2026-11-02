//! Mail: read and send e-mail over IMAP and SMTP with a password or an app
//! password (no browser login).

#![no_std]

extern crate alloc;

pub mod account;
pub mod browser;
pub mod codec;
pub mod compose;
pub mod driver;
pub mod header;
pub mod html;
pub mod imap;
pub mod job;
mod plugin;
pub mod provider;
pub(crate) mod reader;
pub mod render;
pub mod session;
pub mod settings;
pub mod smtp;
pub mod store;
pub mod texts;
pub mod widgets;
pub(crate) mod writer;

pub use plugin::Mail;

#[cfg(test)]
mod tests;

use paperpaw_sdk::Id;
use paperpaw_sdk::event::Event;
use paperpaw_sdk::replies::{Confirmed, SocketNotice, Typed};

pub const PLUGIN: Id = Id::new_static("mail");
/// Opens the folders.
pub const OPEN: Id = Id::new_static("mail.open");
/// Writes a mail.
pub const WRITE: Id = Id::new_static("mail.write");
pub const BOXES: Id = Id::new_static("mail.boxes");
pub const LIST: Id = Id::new_static("mail.list");
pub const READ: Id = Id::new_static("mail.read");
pub const COMPOSE: Id = Id::new_static("mail.compose");
/// The socket of a screen's connection, and what it tells.
pub const SOCKET: Id = Id::new_static("mail.socket");
pub const EVENTS: Event<SocketNotice> = Event::new("mail.events");
/// The answer to *Trust this server?*.
pub const PIN_ANSWER: Event<Confirmed> = Event::new("mail.pin-answer");
/// Text typed for a part of a mail being written.
pub const TYPED: Event<Typed> = Event::new("mail.typed");
