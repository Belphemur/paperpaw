//! Mail: read and send e-mail over IMAP and SMTP with a password or an app
//! password (no browser login).

#![no_std]

extern crate alloc;

pub mod account;
pub mod codec;
pub mod compose;
pub mod driver;
pub mod header;
pub mod html;
pub mod imap;
pub mod job;
pub mod provider;
pub mod render;
pub mod session;
pub mod settings;
pub mod smtp;
pub mod store;

#[cfg(test)]
mod tests;
