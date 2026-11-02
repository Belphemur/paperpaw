//! The sessions' socket, as the SDK has it: what a session asks of [`Io`]
//! becomes [`Command`]s on one socket, and the socket's notices become the
//! session's [`Input`].

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::{Command, Id, SocketNotice, SocketRead, Tls};

use crate::session::{Input, Io};

/// Commands for one socket, collected while a session steps.
#[derive(Debug)]
pub struct Socket {
    socket: Id,
    events: Id,
    commands: Vec<Command>,
}

impl Socket {
    /// A socket named `socket`, heard as `events`.
    pub fn new(socket: Id, events: Id) -> Self {
        Self {
            socket,
            events,
            commands: Vec::new(),
        }
    }

    /// What was asked since the last call.
    pub fn take(&mut self) -> Vec<Command> {
        core::mem::take(&mut self.commands)
    }

    /// The command that closes the socket.
    pub fn disconnect(&self) -> Command {
        Command::Disconnect(self.socket.clone())
    }

    /// Whether `notice` is about this socket.
    pub fn is_mine(&self, notice: &SocketNotice) -> bool {
        notice.socket() == &self.socket
    }
}

impl Io for Socket {
    fn connect_tls(&mut self, host: &str, port: u16, tls: Tls) {
        self.commands.push(Command::connect_tls(
            self.socket.clone(),
            host,
            port,
            self.events.clone(),
            tls,
        ));
    }

    fn connect_plain(&mut self, host: &str, port: u16) {
        self.commands.push(Command::connect(
            self.socket.clone(),
            host,
            port,
            self.events.clone(),
        ));
    }

    fn start_tls(&mut self, tls: Tls) {
        self.commands
            .push(Command::start_tls(self.socket.clone(), tls));
    }

    fn send(&mut self, data: Vec<u8>) {
        self.commands
            .push(Command::socket_send(self.socket.clone(), data));
    }

    fn read_bytes(&mut self, max: u32) {
        self.commands.push(Command::socket_read(
            self.socket.clone(),
            SocketRead::Bytes { max },
        ));
    }

    fn read_file(&mut self, path: &str, len: u64) {
        self.commands.push(Command::socket_read(
            self.socket.clone(),
            SocketRead::File {
                path: String::from(path),
                len,
            },
        ));
    }

    fn close(&mut self) {
        self.commands.push(Command::Disconnect(self.socket.clone()));
    }
}

/// What a session makes of a notice; none for those it does not use.
pub fn input_of(notice: SocketNotice) -> Option<Input> {
    Some(match notice {
        SocketNotice::Connected { .. } => Input::Connected,
        SocketNotice::Secured {
            certificate,
            key,
            time_checked,
            ..
        } => Input::Secured {
            certificate,
            key,
            time_checked,
        },
        SocketNotice::Untrusted {
            certificate,
            key,
            why,
            ..
        } => Input::Untrusted {
            certificate,
            key,
            why,
        },
        SocketNotice::Bytes { data, .. } => Input::Bytes(data),
        SocketNotice::Saved { path, .. } => Input::Saved(path),
        SocketNotice::Closed { .. } => Input::Closed,
        SocketNotice::Failed { error, reason, .. } => Input::Failed(error, reason),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_sdk::SocketError;

    fn socket() -> Socket {
        Socket::new(Id::new_static("mail.socket"), Id::new_static("mail.events"))
    }

    #[test]
    fn requests_become_commands_in_order() {
        let mut s = socket();
        s.connect_plain("imap.example.org", 143);
        s.send(b"a1 CAPABILITY\r\n".to_vec());
        s.read_bytes(1024);
        s.read_file("mail/1/x.eml", 10);
        s.start_tls(Tls::new());
        s.close();
        let commands = s.take();
        assert_eq!(commands.len(), 6);
        assert!(matches!(
            commands[0],
            Command::Connect {
                port: 143,
                tls: None,
                ..
            }
        ));
        assert!(matches!(commands[4], Command::StartTls { .. }));
        assert!(matches!(commands[5], Command::Disconnect(_)));
        assert!(s.take().is_empty());
    }

    #[test]
    fn notices_become_input() {
        let id = Id::new_static("mail.socket");
        assert_eq!(
            input_of(SocketNotice::Closed {
                socket: id.clone(),
                reason: String::new()
            }),
            Some(Input::Closed)
        );
        assert_eq!(
            input_of(SocketNotice::Failed {
                socket: id.clone(),
                reason: "x".into(),
                error: SocketError::Lost
            }),
            Some(Input::Failed(SocketError::Lost, "x".into()))
        );
        assert_eq!(
            input_of(SocketNotice::Done {
                socket: id,
                reason: String::new()
            }),
            None
        );
    }
}
