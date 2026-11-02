//! Accounts: what the reader sets in the settings (a provider to start
//! from, the address, the password, the servers) read into one value, and
//! the providers' servers as presets.
//!
//! Providers appear here as presets, a server and a port each, and nowhere
//! as a name of this plugin.

use alloc::format;
use alloc::string::{String, ToString};

use paperpaw_sdk::{Env, KeyHash};

use crate::session::{Endpoint, Security};
use crate::store;

/// The most accounts.
pub const MAX: u8 = store::MAX_ACCOUNTS;

/// The id of an account's setting (or secret): `mail.2.address`.
pub fn key(number: u8, field: &str) -> String {
    format!("mail.{number}.{field}")
}

/// The fields of an account's settings.
pub mod field {
    pub const PROVIDER: &str = "provider";
    pub const ADDRESS: &str = "address";
    pub const NAME: &str = "name";
    pub const USER: &str = "user";
    pub const PASSWORD: &str = "password";
    pub const IMAP_HOST: &str = "imap-server";
    pub const IMAP_PORT: &str = "imap-port";
    pub const IMAP_SECURITY: &str = "imap-security";
    pub const IMAP_PIN: &str = "imap-pin";
    pub const SMTP_HOST: &str = "smtp-server";
    pub const SMTP_PORT: &str = "smtp-port";
    pub const SMTP_SECURITY: &str = "smtp-security";
    pub const SMTP_PIN: &str = "smtp-pin";
}

/// The values of `security` settings.
pub const IMPLICIT: &str = "mail.security.implicit";
pub const STARTTLS: &str = "mail.security.starttls";
pub const SECURITIES: [&str; 2] = [IMPLICIT, STARTTLS];

impl Security {
    /// The setting's value for it.
    pub fn key(self) -> &'static str {
        match self {
            Security::Implicit => IMPLICIT,
            Security::StartTls => STARTTLS,
        }
    }

    /// The security a setting's value stands for; implicit TLS for any
    /// other (never plain).
    pub fn of_key(key: &str) -> Self {
        if key == STARTTLS {
            Security::StartTls
        } else {
            Security::Implicit
        }
    }
}

/// A provider's servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// The translation key of its name, and the value of the provider
    /// setting.
    pub key: &'static str,
    pub imap: (&'static str, u16, Security),
    pub smtp: (&'static str, u16, Security),
}

const fn preset(
    key: &'static str,
    imap: (&'static str, u16, Security),
    smtp: (&'static str, u16, Security),
) -> Preset {
    Preset { key, imap, smtp }
}

use Security::{Implicit, StartTls};

/// The value of the provider setting that sets nothing.
pub const OWN: &str = "mail.provider.own";

/// What to do about a server whose certificate no authority vouches for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offer {
    /// An own server, trusted before by no key: ask once.
    Ask,
    /// A key was trusted and is another now: no connection, and no way to
    /// trust the new one with a tap (clear the old one in the settings).
    Changed,
    /// A provider's server (its certificate is one an authority vouches
    /// for): someone else is answering. No connection, no offer.
    Attack,
}

/// What may be offered to the reader for the account's IMAP (`smtp` false)
/// or SMTP server that was not trusted.
pub fn offer(values: &dyn Values, number: u8, smtp: bool) -> Offer {
    let provider = values.text(&key(number, field::PROVIDER));
    if !provider.is_empty() && provider != OWN {
        return Offer::Attack;
    }
    let pin = values.text(&key(
        number,
        if smtp {
            field::SMTP_PIN
        } else {
            field::IMAP_PIN
        },
    ));
    if pin.is_empty() {
        Offer::Ask
    } else {
        Offer::Changed
    }
}

/// The providers to start from. Their servers are public; the reader still
/// needs the password (often an app password) the provider asks for.
pub const PRESETS: [Preset; 10] = [
    preset(
        "mail.provider.gmail",
        ("imap.gmail.com", 993, Implicit),
        ("smtp.gmail.com", 465, Implicit),
    ),
    preset(
        "mail.provider.outlook",
        ("outlook.office365.com", 993, Implicit),
        ("smtp-mail.outlook.com", 587, StartTls),
    ),
    preset(
        "mail.provider.icloud",
        ("imap.mail.me.com", 993, Implicit),
        ("smtp.mail.me.com", 587, StartTls),
    ),
    preset(
        "mail.provider.yahoo",
        ("imap.mail.yahoo.com", 993, Implicit),
        ("smtp.mail.yahoo.com", 465, Implicit),
    ),
    preset(
        "mail.provider.gmx",
        ("imap.gmx.net", 993, Implicit),
        ("mail.gmx.net", 465, Implicit),
    ),
    preset(
        "mail.provider.webde",
        ("imap.web.de", 993, Implicit),
        ("smtp.web.de", 587, StartTls),
    ),
    preset(
        "mail.provider.tonline",
        ("secureimap.t-online.de", 993, Implicit),
        ("securesmtp.t-online.de", 465, Implicit),
    ),
    preset(
        "mail.provider.posteo",
        ("posteo.de", 993, Implicit),
        ("posteo.de", 465, Implicit),
    ),
    preset(
        "mail.provider.mailbox",
        ("imap.mailbox.org", 993, Implicit),
        ("smtp.mailbox.org", 465, Implicit),
    ),
    preset(
        "mail.provider.fastmail",
        ("imap.fastmail.com", 993, Implicit),
        ("smtp.fastmail.com", 465, Implicit),
    ),
];

/// The provider setting's options: the reader's own servers, then each
/// preset.
pub const PROVIDERS: [&str; 11] = [
    OWN,
    "mail.provider.gmail",
    "mail.provider.outlook",
    "mail.provider.icloud",
    "mail.provider.yahoo",
    "mail.provider.gmx",
    "mail.provider.webde",
    "mail.provider.tonline",
    "mail.provider.posteo",
    "mail.provider.mailbox",
    "mail.provider.fastmail",
];

/// The preset for a provider setting's value.
pub fn preset_of(provider: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.key == provider)
}

/// The preset a guess from an address's domain suggests (`ada@gmx.net`),
/// for a reader who typed the address first.
pub fn preset_for_address(address: &str) -> Option<&'static Preset> {
    let domain = address.rsplit_once('@')?.1.trim().to_ascii_lowercase();
    let key = match domain.as_str() {
        "gmail.com" | "googlemail.com" => "mail.provider.gmail",
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "outlook.de" | "hotmail.de" => {
            "mail.provider.outlook"
        }
        "icloud.com" | "me.com" | "mac.com" => "mail.provider.icloud",
        "yahoo.com" | "yahoo.de" | "ymail.com" => "mail.provider.yahoo",
        "gmx.de" | "gmx.net" | "gmx.at" | "gmx.ch" | "gmx.com" => "mail.provider.gmx",
        "web.de" => "mail.provider.webde",
        "t-online.de" | "magenta.de" => "mail.provider.tonline",
        "posteo.de" | "posteo.net" => "mail.provider.posteo",
        "mailbox.org" => "mail.provider.mailbox",
        "fastmail.com" | "fastmail.fm" => "mail.provider.fastmail",
        _ => return None,
    };
    preset_of(key)
}

/// One of an account's servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// The key the reader trusted (see [`crate::session::Event::Untrusted`]).
    pub pin: Option<KeyHash>,
}

impl Server {
    /// Where to connect, as whom.
    pub fn endpoint(&self, user: &str, password: &str) -> Endpoint {
        Endpoint {
            host: self.host.clone(),
            port: self.port,
            security: self.security,
            user: user.to_string(),
            password: password.to_string(),
            pin: self.pin,
        }
    }
}

/// An account, as set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub number: u8,
    pub address: String,
    /// The name others see in From.
    pub name: String,
    /// What to log in as: the address unless set otherwise.
    pub user: String,
    pub password: String,
    pub imap: Server,
    pub smtp: Server,
}

/// Where an account's values are read from: the settings and the secrets,
/// or a test's.
pub trait Values {
    fn text(&self, id: &str) -> String;
    fn int(&self, id: &str) -> i64;
    fn secret(&self, id: &str) -> Option<String>;
}

impl Values for Env<'_> {
    fn text(&self, id: &str) -> String {
        self.settings().text(id).to_string()
    }

    fn int(&self, id: &str) -> i64 {
        self.settings().int(id)
    }

    fn secret(&self, id: &str) -> Option<String> {
        self.secrets().get(id).map(ToString::to_string)
    }
}

impl Account {
    /// Account `number` if it has what it needs to log in: an address, a
    /// password and the servers.
    pub fn read(values: &dyn Values, number: u8) -> Option<Self> {
        let text = |f: &str| values.text(&key(number, f)).trim().to_string();
        let address = text(field::ADDRESS);
        let password = values
            .secret(&key(number, field::PASSWORD))
            .unwrap_or_default();
        let server = |host: &str, port: &str, security: &str, pin: &str| {
            let port = u16::try_from(values.int(&key(number, port)))
                .ok()
                .filter(|p| *p != 0)?;
            let host = text(host);
            (!host.is_empty()).then(|| Server {
                host,
                port,
                security: Security::of_key(&text(security)),
                pin: KeyHash::parse(&text(pin)),
            })
        };
        let imap = server(
            field::IMAP_HOST,
            field::IMAP_PORT,
            field::IMAP_SECURITY,
            field::IMAP_PIN,
        )?;
        let smtp = server(
            field::SMTP_HOST,
            field::SMTP_PORT,
            field::SMTP_SECURITY,
            field::SMTP_PIN,
        )?;
        if address.is_empty() || password.is_empty() {
            return None;
        }
        let user = Some(text(field::USER))
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| address.clone());
        Some(Self {
            number,
            address,
            name: text(field::NAME),
            user,
            password,
            imap,
            smtp,
        })
    }

    /// The numbers of the accounts with an address, password or not: what a
    /// widget needs to know (it is not handed the passwords, which are the
    /// plugin's own).
    pub fn numbers(values: &dyn Values) -> alloc::vec::Vec<u8> {
        (1..=MAX)
            .filter(|n| !values.text(&key(*n, field::ADDRESS)).trim().is_empty())
            .collect()
    }

    /// Every account that is set up, in order.
    pub fn all(values: &dyn Values) -> alloc::vec::Vec<Self> {
        (1..=MAX).filter_map(|n| Self::read(values, n)).collect()
    }

    /// The part of the address before the `@`, as a short name.
    pub fn short(&self) -> &str {
        self.address.split('@').next().unwrap_or(&self.address)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    #[derive(Default)]
    struct Fake {
        text: BTreeMap<String, String>,
        int: BTreeMap<String, i64>,
        secret: BTreeMap<String, String>,
    }

    impl Values for Fake {
        fn text(&self, id: &str) -> String {
            self.text.get(id).cloned().unwrap_or_default()
        }
        fn int(&self, id: &str) -> i64 {
            self.int.get(id).copied().unwrap_or(0)
        }
        fn secret(&self, id: &str) -> Option<String> {
            self.secret.get(id).cloned()
        }
    }

    fn complete() -> Fake {
        let mut f = Fake::default();
        for (field, value) in [
            ("address", " ada@example.org "),
            ("name", "Ada"),
            ("imap-server", "imap.example.org"),
            ("imap-security", IMPLICIT),
            ("smtp-server", "smtp.example.org"),
            ("smtp-security", STARTTLS),
        ] {
            f.text.insert(key(1, field), value.into());
        }
        f.int.insert(key(1, "imap-port"), 993);
        f.int.insert(key(1, "smtp-port"), 587);
        f.secret.insert(key(1, "password"), "secret".into());
        f
    }

    #[test]
    fn providers_and_presets_agree() {
        assert_eq!(PROVIDERS[0], OWN);
        assert_eq!(PROVIDERS.len(), PRESETS.len() + 1);
        for (provider, preset) in PROVIDERS[1..].iter().zip(PRESETS) {
            assert_eq!(*provider, preset.key);
            // Never plain: all go through TLS.
            assert!(preset.imap.1 == 993 || preset.imap.2 == StartTls);
        }
        assert!(preset_of(OWN).is_none());
    }

    #[test]
    fn an_account_needs_address_password_and_servers() {
        let a = Account::read(&complete(), 1).unwrap();
        assert_eq!(a.address, "ada@example.org");
        assert_eq!(a.user, "ada@example.org");
        assert_eq!((a.imap.port, a.imap.security), (993, Security::Implicit));
        assert_eq!((a.smtp.port, a.smtp.security), (587, Security::StartTls));
        assert_eq!(a.short(), "ada");
        assert!(Account::read(&complete(), 2).is_none());
        for missing in ["password", "address", "imap-server", "smtp-server"] {
            let mut f = complete();
            f.text.remove(&key(1, missing));
            f.secret.remove(&key(1, missing));
            assert!(Account::read(&f, 1).is_none(), "{missing}");
        }
        let mut f = complete();
        f.int.insert(key(1, "smtp-port"), 70_000);
        assert!(Account::read(&f, 1).is_none());
    }

    #[test]
    fn a_user_name_overrides_the_address_and_the_endpoint_carries_it() {
        let mut f = complete();
        f.text.insert(key(1, "user"), "ada".into());
        let a = Account::read(&f, 1).unwrap();
        let e = a.imap.endpoint(&a.user, &a.password);
        assert_eq!(
            (e.host.as_str(), e.user.as_str(), e.password.as_str()),
            ("imap.example.org", "ada", "secret")
        );
        assert_eq!(Account::all(&f).len(), 1);
    }

    #[test]
    fn the_address_suggests_a_provider() {
        assert_eq!(
            preset_for_address("a@GMX.net").unwrap().key,
            "mail.provider.gmx"
        );
        assert_eq!(
            preset_for_address("a@googlemail.com").unwrap().key,
            "mail.provider.gmail"
        );
        assert!(preset_for_address("a@example.org").is_none());
        assert!(preset_for_address("no-at").is_none());
    }
}
