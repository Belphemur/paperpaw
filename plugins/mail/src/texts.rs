//! Words for what the screens say: where a connection is, why one failed,
//! when a mail came.

use alloc::string::{String, ToString};

use paperpaw_sdk::{Arg, Env, time};

use crate::session::{Failure, Stage};

/// Where a connection is, in words.
pub fn stage<'e>(env: &Env<'e>, stage: Stage) -> &'e str {
    let key = match stage {
        Stage::Connecting => "mail.stage.connecting",
        Stage::LoggingIn => "mail.stage.logging-in",
        Stage::Folders => "mail.stage.folders",
        Stage::Headers => "mail.stage.headers",
        Stage::Message => "mail.stage.message",
        Stage::Updating => "mail.stage.updating",
        Stage::Sending => "mail.stage.sending",
    };
    env.i18n().tr(key)
}

/// Why a connection failed, kindly, and what to do.
pub fn failure(env: &Env<'_>, failure: &Failure) -> String {
    use paperpaw_sdk::SocketError as E;
    let i18n = env.i18n();
    let key = match failure {
        Failure::Network(E::NoNetwork | E::NotAllowed) => "mail.fail.no-network",
        Failure::Network(E::Dns) => "mail.fail.dns",
        Failure::Network(E::Refused) => "mail.fail.refused",
        Failure::Network(E::Timeout) => "mail.fail.timeout",
        Failure::Network(_) => "mail.fail.lost",
        Failure::Secure(E::NameMismatch) => "mail.fail.name",
        Failure::Secure(E::Expired) => "mail.fail.expired",
        Failure::Secure(_) => "mail.fail.secure",
        Failure::NoTls => "mail.fail.no-tls",
        Failure::ClockNotSet => "mail.fail.clock",
        Failure::Busy => "mail.fail.busy",
        Failure::NoMemory => "mail.fail.memory",
        Failure::Login(words) => {
            return i18n.format("mail.fail.login", &[("words", Arg::Str(words))]);
        }
        Failure::Refused(words) => {
            return i18n.format("mail.fail.server", &[("words", Arg::Str(words))]);
        }
        Failure::Recipient(who, words) => {
            return i18n.format(
                "mail.fail.recipient",
                &[("who", Arg::Str(who)), ("words", Arg::Str(words))],
            );
        }
        Failure::Temporary(words) => {
            return i18n.format("mail.fail.later", &[("words", Arg::Str(words))]);
        }
        Failure::Protocol => "mail.fail.protocol",
        Failure::Unsafe => "mail.fail.unsafe",
        Failure::Storage => "mail.fail.storage",
    };
    i18n.tr(key).to_string()
}

/// When a mail came: the time of day if today, else the date. Nothing where
/// the mail has no date.
pub fn when(env: &Env<'_>, date: i64) -> String {
    if date <= 0 {
        return String::new();
    }
    let local = time::local_seconds(env, date);
    let (day, second) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let today = time::unix_seconds(env).map(|now| time::local_seconds(env, now).div_euclid(86_400));
    if today == Some(day) {
        return time::clock_of(env, (second / 3600) as u8, (second % 3600 / 60) as u8);
    }
    let (_, month, d) = time::civil_from_days(day);
    env.i18n().format(
        "mail.date",
        &[
            ("day", Arg::Int(i64::from(d))),
            ("month", Arg::Int(i64::from(month))),
        ],
    )
}
