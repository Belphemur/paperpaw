//! The settings: a group *Mail* with a page for each account (a provider to
//! start from, the address, the password; the servers and the user name
//! under advanced), and a few for all.

use paperpaw_sdk::setting::{Setting, Text};
use paperpaw_sdk::{Id, SettingExtra, SettingGroupSpec, SettingSpec};

use crate::account::{PROVIDERS, SECURITIES};

pub const MAIL: Id = Id::new_static("mail");
pub const ACCOUNT_1: Id = Id::new_static("mail.account.1");
pub const ACCOUNT_2: Id = Id::new_static("mail.account.2");
pub const ACCOUNT_3: Id = Id::new_static("mail.account.3");

/// The group and the parts of it, one for each account.
pub static GROUPS: [SettingGroupSpec; 4] = [
    SettingGroupSpec::new(MAIL, "mail.group").order(27),
    SettingGroupSpec::new(ACCOUNT_1, "mail.account.1")
        .within(MAIL)
        .order(1),
    SettingGroupSpec::new(ACCOUNT_2, "mail.account.2")
        .within(MAIL)
        .order(2),
    SettingGroupSpec::new(ACCOUNT_3, "mail.account.3")
        .within(MAIL)
        .order(3),
];

/// How many of the latest mails of a folder are fetched.
pub const FETCH: Setting<i64> = Setting::new("mail.fetch");
/// A mail is marked read on the server when it is opened.
pub const MARK_READ: Setting<bool> = Setting::new("mail.mark-read");
/// What follows a mail written here (a signature).
pub const SIGNATURE: Setting<Text> = Setting::new("mail.signature");

static PASSWORD: SettingExtra = SettingExtra::new().secret();
static PORT_STEP: SettingExtra = SettingExtra::new().step(1);

/// The settings of one account, `$n` its number, in group `$group`.
macro_rules! account {
    ($n:literal, $group:expr) => {
        [
            SettingSpec::choice(
                concat!("mail.", $n, ".provider"),
                "mail.provider",
                &PROVIDERS,
                PROVIDERS[0],
                $group,
            )
            .kept_on_reset()
            .description("mail.provider.help"),
            SettingSpec::text(concat!("mail.", $n, ".address"), "mail.address", "", $group)
                .kept_on_reset()
                .description("mail.address.help"),
            SettingSpec::text(
                concat!("mail.", $n, ".password"),
                "mail.password",
                "",
                $group,
            )
            .with_extra(&PASSWORD)
            .description("mail.password.help"),
            SettingSpec::text(concat!("mail.", $n, ".name"), "mail.name", "", $group)
                .kept_on_reset()
                .description("mail.name.help"),
            SettingSpec::text(concat!("mail.", $n, ".user"), "mail.user", "", $group)
                .kept_on_reset()
                .advanced()
                .description("mail.user.help"),
            SettingSpec::text(
                concat!("mail.", $n, ".imap-server"),
                "mail.imap-server",
                "",
                $group,
            )
            .kept_on_reset()
            .advanced(),
            SettingSpec::number(
                concat!("mail.", $n, ".imap-port"),
                "mail.port",
                0,
                65535,
                993,
                $group,
            )
            .with_extra(&PORT_STEP)
            .kept_on_reset()
            .advanced(),
            SettingSpec::choice(
                concat!("mail.", $n, ".imap-security"),
                "mail.imap-security",
                &SECURITIES,
                SECURITIES[0],
                $group,
            )
            .kept_on_reset()
            .advanced(),
            SettingSpec::text(concat!("mail.", $n, ".imap-pin"), "mail.pin", "", $group)
                .kept_on_reset()
                .advanced()
                .description("mail.pin.help"),
            SettingSpec::text(
                concat!("mail.", $n, ".smtp-server"),
                "mail.smtp-server",
                "",
                $group,
            )
            .kept_on_reset()
            .advanced(),
            SettingSpec::number(
                concat!("mail.", $n, ".smtp-port"),
                "mail.port",
                0,
                65535,
                465,
                $group,
            )
            .with_extra(&PORT_STEP)
            .kept_on_reset()
            .advanced(),
            SettingSpec::choice(
                concat!("mail.", $n, ".smtp-security"),
                "mail.smtp-security",
                &SECURITIES,
                SECURITIES[0],
                $group,
            )
            .kept_on_reset()
            .advanced(),
            SettingSpec::text(concat!("mail.", $n, ".smtp-pin"), "mail.pin", "", $group)
                .kept_on_reset()
                .advanced()
                .description("mail.pin.help"),
        ]
    };
}

pub static ACCOUNT_SETTINGS: [[SettingSpec; 13]; 3] = [
    account!(1, ACCOUNT_1),
    account!(2, ACCOUNT_2),
    account!(3, ACCOUNT_3),
];

/// The settings for all accounts.
pub static GENERAL: [SettingSpec; 3] = [
    SettingSpec::number(FETCH.static_str(), "mail.fetch", 5, 50, 20, MAIL)
        .with_extra(&PORT_STEP_5)
        .description("mail.fetch.help"),
    SettingSpec::switch(MARK_READ.static_str(), "mail.mark-read", true, MAIL)
        .description("mail.mark-read.help"),
    SettingSpec::text(SIGNATURE.static_str(), "mail.signature", "", MAIL)
        .description("mail.signature.help"),
];

static PORT_STEP_5: SettingExtra = SettingExtra::new().step(5);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{field, key};

    #[test]
    fn every_field_of_an_account_has_a_setting() {
        for (i, specs) in ACCOUNT_SETTINGS.iter().enumerate() {
            let number = i as u8 + 1;
            let ids: alloc::vec::Vec<&str> = specs.iter().map(|s| s.id.as_str()).collect();
            for f in [
                field::PROVIDER,
                field::ADDRESS,
                field::NAME,
                field::USER,
                field::PASSWORD,
                field::IMAP_HOST,
                field::IMAP_PORT,
                field::IMAP_SECURITY,
                field::IMAP_PIN,
                field::SMTP_HOST,
                field::SMTP_PORT,
                field::SMTP_SECURITY,
                field::SMTP_PIN,
            ] {
                assert!(ids.contains(&key(number, f).as_str()), "{number} {f}");
            }
        }
    }
}
