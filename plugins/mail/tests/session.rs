//! The mail sessions against a fake server: the IMAP read (login, folders,
//! the latest headers, a message into a file, flags) and the SMTP send,
//! over TLS from the start and with STARTTLS, in pieces of any size.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests unwrap what they just made")]

mod fake;

use fake::{Fake, Proto, Script};
use paperpaw_plugin_mail::session::{
    Endpoint, Event, Failure, Imap, Input, Outgoing, Security, Smtp, Stage, Task,
};
use paperpaw_plugin_mail::store;
use paperpaw_sdk::{MemoryStorage, SocketError, Storage};

fn endpoint(security: Security) -> Endpoint {
    Endpoint {
        host: "mail.example.org".into(),
        port: if security == Security::Implicit { 993 } else { 143 },
        security,
        user: "ada".into(),
        password: "secret".into(),
        pin: None,
    }
}

fn message(n: u32, flags: &str) -> (u32, String, String) {
    let body = format!("Hello number {n}.\r\n.A line with a dot.\r\nÜbung = fertig\r\n");
    (
        n * 10,
        flags.into(),
        format!(
            "From: =?utf-8?Q?J=C3=BCrgen?= <j{n}@example.org>\r\nTo: ada@example.org\r\nSubject: Subject {n}\r\nDate: Tue, 3 Nov 2026 09:15:00 +0100\r\nMessage-ID: <{n}@example.org>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}"
        ),
    )
}

/// Runs an IMAP session to its end.
fn imap(script: Script, security: Security, tasks: Vec<Task>) -> (Vec<Event>, Fake) {
    let mut fake = Fake::new(Proto::Imap, script);
    let mut session = Imap::new(endpoint(security), tasks);
    let mut events = session.start(&mut fake);
    for _ in 0..200_000 {
        if events.iter().any(|e| matches!(e, Event::Done | Event::Failed(_) | Event::Untrusted { .. })) {
            break;
        }
        let Some(input) = fake.next_input() else {
            break;
        };
        if matches!(input, Input::Secured { .. }) {
            fake.switched();
        }
        let storage = fake.storage.clone();
        events.extend(session.step(input, &mut fake, &*storage));
    }
    (events, fake)
}

fn done(events: &[Event]) -> bool {
    events.last() == Some(&Event::Done)
}

fn everything(account: u8, messages: usize) -> Vec<Task> {
    vec![
        Task::Folders { out: store::folders_file(account) },
        Task::Sync {
            folder: "INBOX".into(),
            count: 50,
            index: store::index_file(account, "INBOX"),
            status: store::status_file(account, "INBOX"),
            now: 1_793_693_700,
        },
        Task::Body { folder: "INBOX".into(), uid: (messages as u32) * 10, out: store::raw_file(account, "INBOX", (messages as u32) * 10) },
        Task::Flag { folder: "INBOX".into(), uid: 10, flag: "\\Seen", on: true },
    ]
}

#[test]
fn the_whole_read_in_pieces_of_any_size() {
    for chunk in [1usize, 3, 17, 100, 1000, 100_000] {
        let mut script = Script { chunk, ..Script::default() };
        script.messages = vec![message(1, "\\Seen"), message(2, ""), message(3, "\\Flagged")];
        let (events, fake) = imap(script, Security::Implicit, everything(1, 3));
        assert!(done(&events), "chunk {chunk}: {events:?}");
        assert!(!fake.sent_plain_password);
        let st = fake.storage.clone();
        let folders = store::read_folders(&*st, &store::folders_file(1));
        assert_eq!(folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["INBOX", "Sent"], "chunk {chunk}");
        let rows = store::read_index(&*st, &store::index_file(1, "INBOX"));
        assert_eq!(rows.iter().map(|r| r.uid).collect::<Vec<_>>(), [30, 20, 10], "chunk {chunk}");
        assert_eq!(rows[0].subject, "Subject 3");
        assert_eq!(rows[0].from, "Jürgen");
        assert!(rows[0].flagged() && !rows[0].seen());
        assert!(rows[2].seen());
        assert!(rows[0].date > 1_793_000_000);
        let counts = store::Counts::read(&*st, &store::status_file(1, "INBOX")).unwrap();
        assert_eq!((counts.messages, counts.unseen, counts.uid_validity, counts.checked), (3, 2, 77, 1_793_693_700));
        // The message arrived whole, as the server has it.
        let raw = paperpaw_sdk::read_file(&*st, &store::raw_file(1, "INBOX", 30), 1 << 20).unwrap();
        assert_eq!(String::from_utf8(raw).unwrap(), message(3, "").2, "chunk {chunk}");
        assert!(fake.sent.iter().any(|l| l.contains("UID STORE 10 +FLAGS.SILENT (\\Seen)")));
        assert!(fake.closed);
        // The scratch file of a literal is gone.
        let mut names = Vec::new();
        st.list(&format!("{}/{}", store::account_dir(1), store::folder_key("INBOX")), &mut |e| names.push(e.name.to_string())).unwrap();
        assert!(names.iter().all(|n| !n.ends_with(".1")), "{names:?}");
    }
}

#[test]
fn a_message_is_read_into_text() {
    let mut script = Script { chunk: 29, ..Script::default() };
    script.messages = vec![message(1, "")];
    let tasks = vec![Task::Body { folder: "INBOX".into(), uid: 10, out: store::raw_file(1, "INBOX", 10) }];
    let (events, fake) = imap(script, Security::Implicit, tasks);
    assert!(done(&events), "{events:?}");
    let st = fake.storage.clone();
    let letter = store::make_letter(&*st, &store::raw_file(1, "INBOX", 10), &store::text_file(1, "INBOX", 10)).unwrap();
    assert_eq!(letter.subject, "Subject 1");
    assert_eq!(letter.text, "Hello number 1.\n.A line with a dot.\nÜbung = fertig\n");
    assert!(!st.exists(&store::raw_file(1, "INBOX", 10)));
    assert_eq!(store::read_letter(&*st, &store::text_file(1, "INBOX", 10)).unwrap(), letter);
}

#[test]
fn starttls_comes_before_anything_secret() {
    let script = Script { messages: vec![message(1, "")], ..Script::default() };
    let (events, fake) = imap(script, Security::StartTls, everything(1, 1));
    assert!(done(&events), "{events:?}");
    assert_eq!(fake.tls_switches, 1);
    assert!(!fake.sent_plain_password);
    let first = fake.sent.first().unwrap();
    assert!(first.ends_with("STARTTLS"), "{first}");
}

#[test]
fn logins_by_what_the_server_offers() {
    // SASL-IR: one line.
    let script = Script { capability: "IMAP4rev1 AUTH=PLAIN SASL-IR".into(), ..Script::default() };
    let (events, fake) = imap(script, Security::Implicit, vec![]);
    assert!(done(&events), "{events:?}");
    assert!(fake.sent.iter().any(|l| l.contains("AUTHENTICATE PLAIN AGFkYQBzZWNyZXQ=")), "{:?}", fake.sent);
    // LOGIN where there is nothing better.
    let script = Script { capability: "IMAP4rev1".into(), ..Script::default() };
    let (events, fake) = imap(script, Security::Implicit, vec![]);
    assert!(done(&events), "{events:?}");
    assert!(fake.sent.iter().any(|l| l.contains("LOGIN \"ada\" \"secret\"")));
    // LOGINDISABLED and no PLAIN: nothing is sent.
    let script = Script { capability: "IMAP4rev1 LOGINDISABLED".into(), ..Script::default() };
    let (events, fake) = imap(script, Security::Implicit, vec![]);
    assert!(events.contains(&Event::Failed(Failure::Unsafe)), "{events:?}");
    assert!(!fake.sent.iter().any(|l| l.contains("secret")));
}

#[test]
fn a_wrong_password_ends_it() {
    let script = Script { password: "other".into(), ..Script::default() };
    let (events, fake) = imap(script, Security::Implicit, everything(1, 1));
    assert!(matches!(events.last(), Some(Event::Failed(Failure::Login(t))) if t.contains("bad credentials")), "{events:?}");
    assert!(fake.closed);
}

#[test]
fn an_untrusted_server_sees_no_password() {
    let script = Script { untrusted: true, ..Script::default() };
    let (events, fake) = imap(script, Security::Implicit, everything(1, 1));
    assert!(matches!(events.last(), Some(Event::Untrusted { .. })), "{events:?}");
    assert!(!fake.sent.iter().any(|l| l.contains("secret") || l.contains("AUTH")));
}

#[test]
fn without_a_clock_the_password_stays_home() {
    let script = Script { time_checked: false, ..Script::default() };
    let (events, fake) = imap(script, Security::Implicit, everything(1, 1));
    assert_eq!(events.last(), Some(&Event::Failed(Failure::ClockNotSet)));
    assert!(fake.sent.is_empty(), "{:?}", fake.sent);
}

#[test]
fn a_pinned_key_is_what_is_asked_for() {
    let mut e = endpoint(Security::Implicit);
    e.pin = Some(paperpaw_sdk::KeyHash::new([9; 32]));
    let mut fake = Fake::new(Proto::Imap, Script::default());
    let mut session = Imap::new(e, vec![]);
    session.start(&mut fake);
    assert_eq!(fake.tls_used.len(), 1);
    assert!(format!("{:?}", fake.tls_used[0]).contains("PinnedKeys"));
}

#[test]
fn an_empty_folder_gives_an_empty_list() {
    let (events, fake) = imap(Script::default(), Security::Implicit, everything(1, 0)[..2].to_vec());
    assert!(done(&events), "{events:?}");
    let st = fake.storage.clone();
    assert!(store::read_index(&*st, &store::index_file(1, "INBOX")).is_empty());
    assert_eq!(store::Counts::read(&*st, &store::status_file(1, "INBOX")).unwrap().messages, 0);
}

#[test]
fn a_large_message_is_read_into_a_file_a_piece_at_a_time() {
    let big = format!("From: a@b.c\r\nSubject: big\r\n\r\n{}", "line of text for a big message\r\n".repeat(9000));
    assert!(big.len() > store::MAX_FETCH);
    let mut script = Script { chunk: 1000, ..Script::default() };
    script.messages = vec![(10, "".into(), big.clone())];
    script.password = "secret".into();
    let tasks = vec![Task::Body { folder: "INBOX".into(), uid: 10, out: store::raw_file(1, "INBOX", 10) }];
    let (events, fake) = imap(script, Security::Implicit, tasks);
    assert!(done(&events), "{events:?}");
    let st = fake.storage.clone();
    let raw = paperpaw_sdk::read_file(&*st, &store::raw_file(1, "INBOX", 10), 1 << 20).unwrap();
    // Cut at what was asked for.
    assert_eq!(raw.len(), store::MAX_FETCH);
    assert_eq!(&raw[..], &big.as_bytes()[..store::MAX_FETCH]);
    // The file read is one read, not 200 small ones.
    assert!(fake.reads < 100, "{}", fake.reads);
}

#[test]
fn a_server_that_hangs_up_is_a_lost_connection() {
    let mut fake = Fake::new(Proto::Imap, Script::default());
    let mut session = Imap::new(endpoint(Security::Implicit), everything(1, 1));
    let st = MemoryStorage::new();
    let mut events = session.start(&mut fake);
    let mut inputs = 0;
    while let Some(input) = fake.next_input() {
        inputs += 1;
        if inputs == 5 {
            events.extend(session.step(Input::Closed, &mut fake, &st));
            break;
        }
        events.extend(session.step(input, &mut fake, &st));
    }
    assert_eq!(events.last(), Some(&Event::Failed(Failure::Network(SocketError::Lost))));
}

#[test]
fn stages_are_told_in_order() {
    let script = Script { messages: vec![message(1, "")], ..Script::default() };
    let (events, _) = imap(script, Security::Implicit, everything(1, 1));
    let stages: Vec<_> = events.iter().filter_map(|e| if let Event::Stage(s) = e { Some(*s) } else { None }).collect();
    assert_eq!(
        stages,
        [Stage::Connecting, Stage::LoggingIn, Stage::Folders, Stage::Headers, Stage::Message, Stage::Updating]
    );
}

// ---------------------------------------------------------------- SMTP

fn outgoing() -> Outgoing {
    let draft = paperpaw_plugin_mail::compose::Draft {
        from: "ada@example.org".into(),
        to: "bob@example.org, carol@example.org".into(),
        subject: "Hi".into(),
        body: "Line\n.dot first\n".into(),
        ..Default::default()
    };
    let to = draft.check().unwrap();
    Outgoing { from: draft.from.clone(), to, data: paperpaw_plugin_mail::smtp::data(&draft.message("Tue, 3 Nov 2026 09:15:00 +0000", "1")) }
}

fn smtp(script: Script, security: Security) -> (Vec<Event>, Fake) {
    let mut fake = Fake::new(Proto::Smtp, script);
    let mut e = endpoint(security);
    e.port = if security == Security::Implicit { 465 } else { 587 };
    let mut session = Smtp::new(e, outgoing());
    let mut events = session.start(&mut fake);
    for _ in 0..100_000 {
        if events.iter().any(|e| matches!(e, Event::Done | Event::Failed(_) | Event::Untrusted { .. })) {
            break;
        }
        let Some(input) = fake.next_input() else {
            break;
        };
        if matches!(input, Input::Secured { .. }) {
            fake.switched();
        }
        events.extend(session.step(input, &mut fake));
    }
    (events, fake)
}

#[test]
fn a_message_is_sent_over_tls_from_the_start() {
    for chunk in [1usize, 5, 1000] {
        let script = Script { chunk, ..Script::default() };
        let (events, fake) = smtp(script, Security::Implicit);
        assert!(done(&events), "chunk {chunk}: {events:?}");
        let (from, to, body) = fake.smtp_got.clone().unwrap();
        assert_eq!(from, "ada@example.org");
        assert_eq!(to, ["bob@example.org", "carol@example.org"]);
        assert!(body.contains("\r\n..dot first\r\n"), "{body}");
        assert!(body.contains("Subject: Hi\r\n"));
        assert!(!fake.sent_plain_password);
    }
}

#[test]
fn smtp_starttls_asks_again_after_the_switch() {
    let (events, fake) = smtp(Script::default(), Security::StartTls);
    assert!(done(&events), "{events:?}");
    assert_eq!(fake.tls_switches, 1);
    assert!(!fake.sent_plain_password);
    let ehlos = fake.sent.iter().filter(|l| l.starts_with("EHLO")).count();
    assert_eq!(ehlos, 2);
    let at = |text: &str| fake.sent.iter().position(|l| l.starts_with(text)).unwrap();
    assert!(at("STARTTLS") < at("AUTH"));
}

#[test]
fn without_starttls_no_password_is_sent() {
    let script = Script { smtp_extensions: vec!["AUTH PLAIN".into()], ..Script::default() };
    let (events, fake) = smtp(script, Security::StartTls);
    assert_eq!(events.last(), Some(&Event::Failed(Failure::Unsafe)), "{events:?}");
    assert!(!fake.sent.iter().any(|l| l.starts_with("AUTH")));
}

#[test]
fn smtp_logs_in_with_auth_login_too_and_says_a_wrong_password() {
    let script = Script { smtp_extensions: vec!["AUTH LOGIN".into()], ..Script::default() };
    let (events, _) = smtp(script, Security::Implicit);
    assert!(done(&events), "{events:?}");
    let script = Script { password: "nope".into(), ..Script::default() };
    let (events, _) = smtp(script, Security::Implicit);
    assert!(matches!(events.last(), Some(Event::Failed(Failure::Login(_)))), "{events:?}");
}

#[test]
fn a_refused_recipient_is_named() {
    let script = Script { refuse_rcpt: Some("carol@example.org".into()), ..Script::default() };
    let (events, fake) = smtp(script, Security::Implicit);
    assert!(
        matches!(events.last(), Some(Event::Failed(Failure::Recipient(who, _))) if who == "carol@example.org"),
        "{events:?}"
    );
    assert!(fake.smtp_got.is_none());
}

#[test]
fn smtp_untrusted_and_clock() {
    let (events, fake) = smtp(Script { untrusted: true, ..Script::default() }, Security::Implicit);
    assert!(matches!(events.last(), Some(Event::Untrusted { .. })));
    assert!(fake.sent.is_empty());
    let (events, fake) = smtp(Script { time_checked: false, ..Script::default() }, Security::Implicit);
    assert_eq!(events.last(), Some(&Event::Failed(Failure::ClockNotSet)));
    assert!(fake.sent.is_empty());
}
