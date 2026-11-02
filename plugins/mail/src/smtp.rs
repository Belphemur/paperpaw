//! SMTP (RFC 5321) as text: a server's replies, the extensions it offers,
//! the commands to send, and the message ready for the DATA command.

use alloc::string::String;
use alloc::vec::Vec;

use crate::codec;

/// A reply, all its lines together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub code: u16,
    /// The text of each line, without the code.
    pub lines: Vec<String>,
}

impl Reply {
    /// 2xx: done.
    pub fn is_ok(&self) -> bool {
        (200..300).contains(&self.code)
    }

    /// 4xx: try again later.
    pub fn is_temporary(&self) -> bool {
        (400..500).contains(&self.code)
    }

    /// The first line's text.
    pub fn text(&self) -> &str {
        self.lines.first().map_or("", String::as_str)
    }
}

/// Lines of replies, put together: `250-a`, `250 b` is one reply.
#[derive(Debug, Default, Clone)]
pub struct ReplyParser {
    code: u16,
    lines: Vec<String>,
    /// Bytes of the reply so far.
    total: usize,
}

/// The longest line of a reply, and the longest reply, in bytes (RFC 5321
/// has lines of 512): a longer one is no server's.
const MAX_REPLY_LINE: usize = 1000;
const MAX_REPLY: usize = 4096;

impl ReplyParser {
    /// One line of the server (without its line break): the reply, once
    /// its last line came. A line that is no reply line ends the reply
    /// with code 0 (a server that does not speak SMTP).
    pub fn line(&mut self, line: &[u8]) -> Option<Reply> {
        self.total += line.len();
        if line.len() > MAX_REPLY_LINE || self.total > MAX_REPLY {
            self.lines.clear();
            self.total = 0;
            return Some(Reply {
                code: 0,
                lines: alloc::vec![String::from("reply too long")],
            });
        }
        let text = String::from_utf8_lossy(line);
        let bytes = text.as_bytes();
        let code = bytes
            .get(..3)
            .filter(|d| d.iter().all(u8::is_ascii_digit))
            .and_then(|d| core::str::from_utf8(d).ok())
            .and_then(|d| d.parse::<u16>().ok());
        let (Some(code), Some(sep)) = (code, bytes.get(3).copied().or(Some(b' '))) else {
            self.lines.clear();
            self.total = 0;
            return Some(Reply {
                code: 0,
                lines: alloc::vec![String::from(text.trim())],
            });
        };
        if !self.lines.is_empty() && code != self.code {
            // A reply that changes its code half way is no reply.
            self.lines.clear();
        }
        self.code = code;
        if self.lines.len() < 64 {
            self.lines
                .push(String::from(text.get(4..).unwrap_or("").trim()));
        }
        if sep == b'-' {
            return None;
        }
        self.total = 0;
        Some(Reply {
            code,
            lines: core::mem::take(&mut self.lines),
        })
    }
}

/// What a server offers in answer to EHLO.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extensions {
    pub starttls: bool,
    pub auth_plain: bool,
    pub auth_login: bool,
    /// The largest message, in bytes, 0 where it says none.
    pub size: u64,
}

impl Extensions {
    /// From the lines of the reply to EHLO (the first is the greeting).
    pub fn of(reply: &Reply) -> Self {
        let mut out = Self::default();
        for line in reply.lines.iter().skip(1) {
            let upper: String = line.chars().map(|c| c.to_ascii_uppercase()).collect();
            let mut words = upper.split_whitespace();
            match words.next() {
                Some("STARTTLS") => out.starttls = true,
                Some("SIZE") => out.size = words.next().and_then(|n| n.parse().ok()).unwrap_or(0),
                Some("AUTH") => {
                    for mechanism in words {
                        match mechanism {
                            "PLAIN" => out.auth_plain = true,
                            "LOGIN" => out.auth_login = true,
                            _ => {}
                        }
                    }
                }
                // `AUTH=PLAIN` of old servers.
                Some(other) if other.starts_with("AUTH=") => {
                    out.auth_plain |= other.ends_with("PLAIN");
                    out.auth_login |= other.ends_with("LOGIN");
                }
                _ => {}
            }
        }
        out
    }
}

/// `AUTH PLAIN` with the credentials in it.
pub fn auth_plain(user: &str, password: &str) -> String {
    let mut data = Vec::with_capacity(user.len() + password.len() + 2);
    data.push(0);
    data.extend_from_slice(user.as_bytes());
    data.push(0);
    data.extend_from_slice(password.as_bytes());
    let mut line = String::from("AUTH PLAIN ");
    line.push_str(&codec::base64_line(&data));
    line.push_str("\r\n");
    line
}

/// `MAIL FROM:<address>`.
pub fn mail_from(address: &str) -> String {
    let mut line = String::from("MAIL FROM:<");
    line.push_str(&clean(address));
    line.push_str(">\r\n");
    line
}

/// `RCPT TO:<address>`.
pub fn rcpt_to(address: &str) -> String {
    let mut line = String::from("RCPT TO:<");
    line.push_str(&clean(address));
    line.push_str(">\r\n");
    line
}

/// An address with nothing in it that would end the command.
fn clean(address: &str) -> String {
    address
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ' '))
        .collect()
}

/// Whether `address` can be mailed to: one `@`, a local part and a domain
/// with a dot or a bracket, no white space, nothing that ends a command.
pub fn valid_address(address: &str) -> bool {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && address.len() <= 254
        && !address.contains(|c: char| {
            c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';')
        })
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '[' | ']' | ':'))
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
}

/// A message's bytes as the DATA command sends them: lines end in CRLF, a
/// line that starts with a dot gets another, and the end is `.` on a line.
pub fn data(message: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(message.len() + message.len() / 40 + 8);
    let mut at_line_start = true;
    for &byte in message.as_bytes() {
        // Bare LF and bare CR are line breaks too.
        match byte {
            b'\r' => {}
            b'\n' => {
                out.extend_from_slice(b"\r\n");
                at_line_start = true;
            }
            b => {
                if at_line_start && b == b'.' {
                    out.push(b'.');
                }
                out.push(b);
                at_line_start = false;
            }
        }
    }
    if !(out.ends_with(b"\r\n") || out.is_empty()) {
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b".\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_come_in_lines() {
        let mut p = ReplyParser::default();
        assert_eq!(p.line(b"250-mail.example.org"), None);
        assert_eq!(p.line(b"250-STARTTLS"), None);
        assert_eq!(p.line(b"250-AUTH PLAIN LOGIN XOAUTH2"), None);
        assert_eq!(p.line(b"250-SIZE 35882577"), None);
        let reply = p.line(b"250 8BITMIME").unwrap();
        assert!(reply.is_ok());
        let ext = Extensions::of(&reply);
        assert_eq!(
            ext,
            Extensions {
                starttls: true,
                auth_plain: true,
                auth_login: true,
                size: 35_882_577
            }
        );
        let r = p.line(b"354").unwrap();
        assert_eq!((r.code, r.lines.len()), (354, 1));
        assert_eq!(p.line(b"garbage").unwrap().code, 0);
        assert!(p.line(b"451 try later").unwrap().is_temporary());
    }

    #[test]
    fn commands() {
        assert_eq!(auth_plain("u", "p"), "AUTH PLAIN AHUAcA==\r\n");
        assert_eq!(mail_from("a@b.c"), "MAIL FROM:<a@b.c>\r\n");
        assert_eq!(rcpt_to("a@b.c>\r\nDATA"), "RCPT TO:<a@b.cDATA>\r\n");
        for ok in [
            "ada@example.org",
            "a.b+tag@mail.example.co.uk",
            "x@[127.0.0.1]",
        ] {
            assert!(valid_address(ok), "{ok}");
        }
        for bad in [
            "",
            "ada",
            "@x.org",
            "ada@",
            "a b@x.org",
            "a@x..org",
            "a@.x.org",
            "a@b.c\r\nRCPT TO:<x@y.z",
            "a,b@x.org",
            "<a@b.c>",
        ] {
            assert!(!valid_address(bad), "{bad:?}");
        }
    }

    #[test]
    fn data_is_dot_stuffed_and_ended() {
        assert_eq!(data("a\n.b\n..c\n"), b"a\r\n..b\r\n...c\r\n.\r\n");
        assert_eq!(data("no end"), b"no end\r\n.\r\n");
        assert_eq!(data("x\r\n.\r\n"), b"x\r\n..\r\n.\r\n");
        assert_eq!(data(""), b".\r\n");
        assert_eq!(data("a\rb"), b"ab\r\n.\r\n");
    }
}
