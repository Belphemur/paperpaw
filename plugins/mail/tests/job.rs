//! The job as a screen uses it: commands out, notices in.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use paperpaw_plugin_mail::account::Account;
use paperpaw_plugin_mail::job::{self, Job, Run};
use paperpaw_plugin_mail::session::{Failure, Stage, Task};
use paperpaw_sdk::{CertificateHash, Command, Distrust, Id, KeyHash, MemoryStorage, SocketError, SocketNotice};

fn account() -> Account {
    struct V;
    impl paperpaw_plugin_mail::account::Values for V {
        fn text(&self, id: &str) -> String {
            match id {
                "mail.1.address" => "ada@example.org".into(),
                "mail.1.imap-server" => "imap.example.org".into(),
                "mail.1.smtp-server" => "smtp.example.org".into(),
                "mail.1.imap-security" | "mail.1.smtp-security" => "mail.security.implicit".into(),
                _ => String::new(),
            }
        }
        fn int(&self, id: &str) -> i64 {
            if id.ends_with("imap-port") { 993 } else { 465 }
        }
        fn secret(&self, _: &str) -> Option<String> {
            Some("secret".into())
        }
    }
    Account::read(&V, 1).unwrap()
}

fn job() -> Job {
    Job::new(Id::new_static("mail.socket"), Id::new_static("mail.events"))
}

fn notice_failed(error: SocketError) -> SocketNotice {
    SocketNotice::Failed { socket: Id::new_static("mail.socket"), reason: String::new(), error }
}

#[test]
fn starting_connects_and_a_failure_ends_the_run() {
    let mut job = job();
    let storage = MemoryStorage::new();
    let commands = job.start_imap(&account(), vec![Task::Folders { out: "f".into() }]);
    assert!(matches!(commands.as_slice(), [Command::Connect { port: 993, tls: Some(_), .. }]), "{commands:?}");
    assert_eq!(job.run, Run::Working(Stage::Connecting));
    // A second start while one runs does nothing.
    assert!(job.start_imap(&account(), vec![]).is_empty());
    job.notice(notice_failed(SocketError::NoNetwork), &storage);
    assert_eq!(job.run, Run::Failed(Failure::Network(SocketError::NoNetwork)));
    // Notices after the end change nothing.
    assert!(job.notice(notice_failed(SocketError::Lost), &storage).is_empty());
    assert_eq!(job.run, Run::Failed(Failure::Network(SocketError::NoNetwork)));
}

#[test]
fn an_untrusted_server_says_which_one_and_the_pin_is_a_setting() {
    let mut job = job();
    let storage = MemoryStorage::new();
    let out = paperpaw_plugin_mail::session::Outgoing {
        from: "ada@example.org".into(),
        to: vec!["jane@example.org".into()],
        data: b"x\r\n.\r\n".to_vec(),
    };
    job.start_smtp(&account(), out);
    let (certificate, key) = (CertificateHash::new([1; 32]), KeyHash::new([2; 32]));
    job.notice(
        SocketNotice::Untrusted { socket: Id::new_static("mail.socket"), certificate, key, why: Distrust::UnknownIssuer },
        &storage,
    );
    let Run::Untrusted { smtp, key: shown, .. } = job.run.clone() else { panic!("{:?}", job.run) };
    assert!(smtp);
    let Command::SetSetting(id, _) = job::pin(1, smtp, &shown) else { panic!() };
    assert_eq!(id.as_str(), "mail.1.smtp-pin");
}

#[test]
fn cancel_closes_the_socket_of_a_running_job_only() {
    let mut job = job();
    assert!(job.cancel().is_empty());
    job.start_imap(&account(), vec![]);
    assert!(matches!(job.cancel().as_slice(), [Command::Disconnect(_)]));
    assert_eq!(job.run, Run::Idle);
}
