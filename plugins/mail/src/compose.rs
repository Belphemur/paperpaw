//! A message written on the device: the headers and the text as a mail
//! (RFC 5322, MIME: UTF-8 text, quoted-printable where it is not ASCII).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::codec;
use crate::header;
use crate::smtp;

/// The most text of a message written here, in bytes (the keyboard writes
/// letters, not books; the message is built in memory).
pub const MAX_BODY: usize = 16 * 1024;
/// The most recipients of one message.
pub const MAX_RECIPIENTS: usize = 20;

/// What the reader wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    /// The sender's name shown, may be empty.
    pub from_name: String,
    pub from: String,
    /// Addresses, as typed (comma or semicolon apart, names allowed).
    pub to: String,
    pub cc: String,
    pub subject: String,
    pub body: String,
    /// The `Message-ID` of what this answers, and its `References`.
    pub in_reply_to: String,
    pub references: String,
}

/// Why a draft is not sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NoSender,
    NoRecipient,
    /// An address that cannot be mailed to; it is the first one.
    BadAddress,
    TooManyRecipients,
    TooLong,
}

impl Draft {
    /// The draft as kept between two openings: the headers a line each,
    /// a blank line, the text.
    pub fn to_text(&self) -> String {
        let line = |name: &str, value: &str| format!("{name}: {}\n", crate::store::field(value));
        let mut out = String::new();
        out.push_str(&line("To", &self.to));
        out.push_str(&line("Cc", &self.cc));
        out.push_str(&line("Subject", &self.subject));
        out.push_str(&line("In-Reply-To", &self.in_reply_to));
        out.push_str(&line("References", &self.references));
        out.push('\n');
        out.push_str(&self.body);
        out
    }

    /// A draft kept by [`Draft::to_text`], for the account `from`.
    pub fn from_text(text: &str) -> Self {
        let (head, body) = text.split_once("\n\n").unwrap_or((text, ""));
        let mut draft = Self {
            body: body.into(),
            ..Self::default()
        };
        for line in head.lines() {
            let Some((name, value)) = line.split_once(": ") else {
                continue;
            };
            let value = String::from(value);
            match name {
                "To" => draft.to = value,
                "Cc" => draft.cc = value,
                "Subject" => draft.subject = value,
                "In-Reply-To" => draft.in_reply_to = value,
                "References" => draft.references = value,
                _ => {}
            }
        }
        draft
    }

    /// The addresses to send to: to and cc, each valid, no repeats.
    pub fn recipients(&self) -> Result<Vec<String>, Problem> {
        let mut out: Vec<String> = Vec::new();
        for list in [&self.to, &self.cc] {
            for field in split_list(list) {
                let Some(found) = header::addresses(&field).into_iter().next() else {
                    return Err(Problem::BadAddress);
                };
                if !smtp::valid_address(&found.address) {
                    return Err(Problem::BadAddress);
                }
                if !out.iter().any(|a| a.eq_ignore_ascii_case(&found.address)) {
                    out.push(found.address);
                }
            }
        }
        if out.is_empty() {
            return Err(Problem::NoRecipient);
        }
        if out.len() > MAX_RECIPIENTS {
            return Err(Problem::TooManyRecipients);
        }
        Ok(out)
    }

    /// Whether it can be sent.
    pub fn check(&self) -> Result<Vec<String>, Problem> {
        if !smtp::valid_address(self.from.trim()) {
            return Err(Problem::NoSender);
        }
        if self.body.len() > MAX_BODY {
            return Err(Problem::TooLong);
        }
        self.recipients()
    }

    /// The message with its headers, lines ended by CRLF. `date` is the
    /// Date header's value, `id` the part of the Message-ID before the `@`.
    pub fn message(&self, date: &str, id: &str) -> String {
        let domain = self.from.rsplit_once('@').map_or("localhost", |(_, d)| d);
        let mut out = String::with_capacity(self.body.len() + 600);
        let mut header = |name: &str, value: &str| {
            out.push_str(name);
            out.push_str(": ");
            out.push_str(&one_line(value));
            out.push_str("\r\n");
        };
        header("Date", date);
        let from = if self.from_name.trim().is_empty() {
            self.from.trim().into()
        } else {
            format!("{} <{}>", display_name(&self.from_name), self.from.trim())
        };
        header("From", &from);
        header("To", &list_header(&self.to));
        if !self.cc.trim().is_empty() {
            header("Cc", &list_header(&self.cc));
        }
        header("Subject", &codec::encode_words(&one_line(&self.subject)));
        header("Message-ID", &format!("<{id}@{domain}>"));
        if !self.in_reply_to.trim().is_empty() {
            header("In-Reply-To", self.in_reply_to.trim());
            let mut references = String::from(self.references.trim());
            if !references.is_empty() {
                references.push(' ');
            }
            references.push_str(self.in_reply_to.trim());
            header("References", &references);
        }
        header("MIME-Version", "1.0");
        header("Content-Type", "text/plain; charset=utf-8");
        let ascii = self.body.is_ascii();
        header(
            "Content-Transfer-Encoding",
            if ascii { "7bit" } else { "quoted-printable" },
        );
        header("X-Mailer", "PaperPaw");
        out.push_str("\r\n");
        if ascii {
            for line in self.body.split('\n') {
                out.push_str(line.strip_suffix('\r').unwrap_or(line));
                out.push_str("\r\n");
            }
            // `split` of a text ending in a line break gave one empty line more.
            if self.body.ends_with('\n') {
                out.truncate(out.len() - 2);
            }
        } else {
            out.push_str(&codec::encode_qp(&self.body));
        }
        out
    }
}

/// `text` on one line: line breaks and control characters become spaces.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .into()
}

/// A name for the From header: quoted where it needs, encoded where it is
/// not ASCII.
fn display_name(name: &str) -> String {
    let name = one_line(name);
    if name.is_ascii() {
        let plain = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' || c == '\'');
        if plain {
            return name;
        }
        let mut quoted = String::from("\"");
        for c in name.chars() {
            if c == '"' || c == '\\' {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        quoted.push('"');
        return quoted;
    }
    codec::encode_words(&name)
}

/// The addresses of a typed list as one header value.
fn list_header(list: &str) -> String {
    let mut out = String::new();
    for field in split_list(list) {
        if !out.is_empty() {
            out.push_str(", ");
        }
        match header::addresses(&field).into_iter().next() {
            Some(a) if a.name.is_empty() => out.push_str(&a.address),
            Some(a) => {
                out.push_str(&display_name(&a.name));
                out.push_str(" <");
                out.push_str(&a.address);
                out.push('>');
            }
            None => out.push_str(&one_line(&field)),
        }
    }
    out
}

/// The fields of a list typed with commas or semicolons (a comma in
/// quotes is none).
pub fn split_list(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut quoted, mut angle) = (false, false);
    let mut current = String::new();
    for c in list.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.push(c);
            }
            '<' if !quoted => {
                angle = true;
                current.push(c);
            }
            '>' if !quoted => {
                angle = false;
                current.push(c);
            }
            ',' | ';' | '\n' if !quoted && !angle => {
                if !current.trim().is_empty() {
                    out.push(String::from(current.trim()));
                }
                current.clear();
            }
            c => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        out.push(String::from(current.trim()));
    }
    out
}

/// The draft that answers `letter` from the address `own`: to whom it was
/// from (or its `Reply-To`), with everyone else who got it in Cc for
/// `all`; `Re: ` before the subject unless it has one; the original
/// quoted after `intro`.
pub fn reply_to(letter: &crate::store::Letter, own: &str, all: bool, intro: &str) -> Draft {
    let own_address = |field: &str| {
        header::addresses(field)
            .first()
            .is_some_and(|a| a.address.eq_ignore_ascii_case(own))
    };
    let to = if letter.reply_to.trim().is_empty() {
        &letter.from
    } else {
        &letter.reply_to
    };
    let to = String::from(to.trim());
    let mut cc: Vec<String> = Vec::new();
    if all {
        for field in split_list(&letter.to)
            .into_iter()
            .chain(split_list(&letter.cc))
        {
            let known = |list: &str| {
                let found = header::addresses(&field).into_iter().next();
                found.is_some_and(|f| {
                    header::addresses(list)
                        .iter()
                        .any(|a| a.address.eq_ignore_ascii_case(&f.address))
                })
            };
            if !own_address(&field) && !known(&to) && !cc.iter().any(|c| known(c)) {
                cc.push(field);
            }
        }
    }
    let subject = letter.subject.trim();
    let has_re = subject.len() >= 3
        && subject.is_char_boundary(3)
        && subject[..3].eq_ignore_ascii_case("re:");
    let mut references = String::from(letter.references.trim());
    if !references.is_empty() {
        references.push(' ');
    }
    references.push_str(letter.message_id.trim());
    Draft {
        from: String::from(own),
        to,
        cc: cc.join(", "),
        subject: if has_re {
            String::from(subject)
        } else {
            format!("Re: {subject}")
        },
        body: format!("\n\n{}", quote(&letter.text, intro, 40)),
        in_reply_to: String::from(letter.message_id.trim()),
        references: header::last_ids(&references, header::MAX_REFERENCES),
        ..Draft::default()
    }
}

/// Date header value for `seconds` since 1970 (UTC).
pub fn date_header(seconds: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    let (year, month, day) = header::civil_from_days(days);
    format!(
        "{}, {day} {} {year} {:02}:{:02}:{:02} +0000",
        DAYS[days.rem_euclid(7) as usize],
        MONTHS[(month - 1).clamp(0, 11) as usize],
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The text of a reply: the original quoted, each line after `> `, with
/// `intro` (a line saying who wrote it, in the reader's language) before.
pub fn quote(original: &str, intro: &str, max_lines: usize) -> String {
    let mut out = String::new();
    if !intro.is_empty() {
        out.push_str(intro);
        out.push('\n');
    }
    for line in original.lines().take(max_lines) {
        out.push_str("> ");
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Draft {
        Draft {
            from_name: "Ada Lovelace".into(),
            from: "ada@example.org".into(),
            to: "\"Doe, Jane\" <jane@example.org>; bob@example.org".into(),
            cc: "BOB@example.org".into(),
            subject: "Grüße".into(),
            body: "Hallo Jürgen,\n.punkt am Anfang\n".into(),
            in_reply_to: "<1@x>".into(),
            references: "<0@x>".into(),
        }
    }

    #[test]
    fn a_reply_goes_to_the_sender_and_all_to_the_others() {
        let letter = crate::store::Letter {
            from: "Jane Doe <jane@example.org>".into(),
            to: "Ada <ada@example.org>, bob@example.org".into(),
            cc: "Jane Doe <JANE@example.org>, carol@example.org".into(),
            subject: "Lunch".into(),
            message_id: "<m1@x>".into(),
            references: "<m0@x>".into(),
            text: "Noon?\n".into(),
            ..Default::default()
        };
        let one = reply_to(&letter, "ada@example.org", false, "Jane wrote:");
        assert_eq!(
            (one.to.as_str(), one.cc.as_str()),
            ("Jane Doe <jane@example.org>", "")
        );
        assert_eq!(one.subject, "Re: Lunch");
        assert_eq!(
            (one.in_reply_to.as_str(), one.references.as_str()),
            ("<m1@x>", "<m0@x> <m1@x>")
        );
        assert_eq!(one.body, "\n\nJane wrote:\n> Noon?\n");
        let all = reply_to(&letter, "ada@example.org", true, "");
        assert_eq!(all.cc, "bob@example.org, carol@example.org");
        // Reply-To wins, and Re: is not stacked.
        let mut again = letter.clone();
        again.reply_to = "list@example.org".into();
        again.subject = "RE: Lunch".into();
        let d = reply_to(&again, "ada@example.org", false, "");
        assert_eq!(
            (d.to.as_str(), d.subject.as_str()),
            ("list@example.org", "RE: Lunch")
        );
        assert_eq!(
            reply_to(&Default::default(), "a@b.c", true, "").subject,
            "Re: "
        );
    }

    #[test]
    fn a_draft_is_kept_and_read_again() {
        let mut d = draft();
        d.body = "Line one\n\nLine three\n".into();
        let again = Draft::from_text(&d.to_text());
        assert_eq!(
            (again.to.as_str(), again.subject.as_str()),
            (d.to.as_str(), d.subject.as_str())
        );
        assert_eq!(
            (again.in_reply_to.as_str(), again.references.as_str()),
            ("<1@x>", "<0@x>")
        );
        assert_eq!(again.body, d.body);
        // Nothing kept is an empty draft.
        assert_eq!(Draft::from_text(""), Draft::default());
    }

    #[test]
    fn a_draft_becomes_a_message() {
        let d = draft();
        assert_eq!(
            d.recipients().unwrap(),
            ["jane@example.org", "bob@example.org"]
        );
        let m = d.message("Tue, 3 Nov 2026 09:15:00 +0000", "42");
        assert!(m.starts_with(
            "Date: Tue, 3 Nov 2026 09:15:00 +0000\r\nFrom: Ada Lovelace <ada@example.org>\r\n"
        ));
        assert!(m.contains("To: \"Doe, Jane\" <jane@example.org>, bob@example.org\r\n"));
        assert!(m.contains("Subject: =?utf-8?B?"));
        assert!(m.contains("Message-ID: <42@example.org>\r\n"));
        assert!(m.contains("References: <0@x> <1@x>\r\n"));
        assert!(m.contains("Content-Transfer-Encoding: quoted-printable\r\nX-Mailer: PaperPaw\r\n\r\nHallo J=C3=BCrgen,\r\n.punkt"));
        assert!(m.is_ascii());
        // What a reader of it makes of it again.
        let storage = paperpaw_sdk::MemoryStorage::new().with("m.eml", m.as_bytes());
        let info = crate::render::render(&storage, "m.eml", "m.txt", 4096).unwrap();
        assert_eq!(info.subject, "Grüße");
        assert_eq!(info.to.len(), 2);
        let text = paperpaw_sdk::read_file(&storage, "m.txt", 4096).unwrap();
        assert_eq!(
            String::from_utf8(text).unwrap(),
            "Hallo Jürgen,\n.punkt am Anfang\n"
        );
    }

    #[test]
    fn what_cannot_be_sent_is_found_first() {
        let mut d = draft();
        d.to = "".into();
        d.cc = "".into();
        assert_eq!(d.check(), Err(Problem::NoRecipient));
        d.to = "not an address".into();
        assert_eq!(d.check(), Err(Problem::BadAddress));
        d.to = "a@b.org".into();
        d.from = "".into();
        assert_eq!(d.check(), Err(Problem::NoSender));
        d.from = "x@y.org".into();
        d.body = "x".repeat(MAX_BODY + 1);
        assert_eq!(d.check(), Err(Problem::TooLong));
        d.body = "x".into();
        d.to = (0..21)
            .map(|n| alloc::format!("u{n}@x.org"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(d.check(), Err(Problem::TooManyRecipients));
    }

    #[test]
    fn headers_cannot_be_broken_out_of() {
        let mut d = draft();
        d.subject = "Hi\r\nBcc: spy@example.org".into();
        d.from_name = "Eve\r\nBcc: spy@example.org".into();
        d.to = "a@b.org".into();
        let m = d.message("x", "1");
        assert!(!m.contains("\r\nBcc:"));
    }

    #[test]
    fn dates_and_quotes() {
        assert_eq!(date_header(0), "Thu, 1 Jan 1970 00:00:00 +0000");
        assert_eq!(date_header(1_793_693_700), "Tue, 3 Nov 2026 08:15:00 +0000");
        assert_eq!(
            crate::header::date_seconds(&date_header(1_793_693_700)),
            Some(1_793_693_700)
        );
        assert_eq!(quote("a\nb\n", "Ada wrote:", 10), "Ada wrote:\n> a\n> b\n");
        assert_eq!(quote("a\nb\nc", "", 2), "> a\n> b\n");
        assert_eq!(
            split_list("a@b.c, \"x, y\" <z@w.v>;"),
            ["a@b.c", "\"x, y\" <z@w.v>"]
        );
    }
}
