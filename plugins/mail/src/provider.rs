//! Choosing a provider sets its servers; so does typing an address a
//! provider is known by, where nothing is set yet.

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use paperpaw_sdk::{Command, Id, SettingValue};

use crate::account::{self, OWN, Preset, Values, field, key};

/// Which account and field a setting id (`mail.2.address`) is.
pub fn parse(id: &str) -> Option<(u8, &str)> {
    let rest = id.strip_prefix("mail.")?;
    let (number, field) = rest.split_once('.')?;
    let number: u8 = number
        .parse()
        .ok()
        .filter(|n| (1..=account::MAX).contains(n))?;
    Some((number, field))
}

fn servers(number: u8, preset: &Preset) -> Vec<(String, SettingValue)> {
    let text = |v: &str| SettingValue::Text(Cow::Owned(v.to_string()));
    vec![
        (key(number, field::IMAP_HOST), text(preset.imap.0)),
        (
            key(number, field::IMAP_PORT),
            SettingValue::Int(i64::from(preset.imap.1)),
        ),
        (key(number, field::IMAP_SECURITY), text(preset.imap.2.key())),
        (key(number, field::SMTP_HOST), text(preset.smtp.0)),
        (
            key(number, field::SMTP_PORT),
            SettingValue::Int(i64::from(preset.smtp.1)),
        ),
        (key(number, field::SMTP_SECURITY), text(preset.smtp.2.key())),
        // A server of another provider is not the one trusted before.
        (key(number, field::IMAP_PIN), text("")),
        (key(number, field::SMTP_PIN), text("")),
    ]
}

/// What to set after setting `id` changed.
pub fn after_change(values: &dyn Values, id: &str) -> Vec<(String, SettingValue)> {
    let Some((number, changed)) = parse(id) else {
        return Vec::new();
    };
    match changed {
        field::PROVIDER => account::preset_of(&values.text(id))
            .map(|p| servers(number, p))
            .unwrap_or_default(),
        field::ADDRESS
            if values.text(&key(number, field::PROVIDER)) == OWN
                && values.text(&key(number, field::IMAP_HOST)).is_empty() =>
        {
            let Some(preset) = account::preset_for_address(&values.text(id)) else {
                return Vec::new();
            };
            let mut set = servers(number, preset);
            set.push((
                key(number, field::PROVIDER),
                SettingValue::Text(Cow::Borrowed(preset.key)),
            ));
            set
        }
        _ => Vec::new(),
    }
}

/// The commands for [`after_change`].
pub fn commands(values: &dyn Values, id: &str) -> Vec<Command> {
    after_change(values, id)
        .into_iter()
        .filter_map(|(id, v)| Id::parse(&id).ok().map(|id| Command::SetSetting(id, v)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::collections::BTreeMap;

    #[derive(Default)]
    struct Fake(BTreeMap<String, String>);
    impl Values for Fake {
        fn text(&self, id: &str) -> String {
            self.0.get(id).cloned().unwrap_or_default()
        }
        fn int(&self, _: &str) -> i64 {
            0
        }
        fn secret(&self, _: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn ids_are_parsed() {
        assert_eq!(parse("mail.2.address"), Some((2, "address")));
        assert_eq!(parse("mail.4.address"), None);
        assert_eq!(parse("mail.0.address"), None);
        assert_eq!(parse("mail.fetch"), None);
        assert_eq!(parse("core.1.x"), None);
    }

    #[test]
    fn a_chosen_provider_sets_its_servers() {
        let mut f = Fake::default();
        f.0.insert("mail.1.provider".into(), "mail.provider.gmx".into());
        let set = after_change(&f, "mail.1.provider");
        assert!(set.contains(&(
            "mail.1.imap-server".into(),
            SettingValue::Text("imap.gmx.net".into())
        )));
        assert!(set.contains(&("mail.1.smtp-port".into(), SettingValue::Int(465))));
        // The reader's own servers are left alone.
        f.0.insert("mail.1.provider".into(), OWN.into());
        assert!(after_change(&f, "mail.1.provider").is_empty());
    }

    #[test]
    fn a_known_address_sets_the_provider_once() {
        let mut f = Fake::default();
        f.0.insert("mail.2.address".into(), "ada@web.de".into());
        f.0.insert("mail.2.provider".into(), OWN.into());
        let set = after_change(&f, "mail.2.address");
        assert!(set.contains(&(
            "mail.2.provider".into(),
            SettingValue::Text("mail.provider.webde".into())
        )));
        assert!(set.contains(&(
            "mail.2.smtp-security".into(),
            SettingValue::Text("mail.security.starttls".into())
        )));
        // With servers set already an address changes nothing.
        f.0.insert("mail.2.imap-server".into(), "imap.example.org".into());
        assert!(after_change(&f, "mail.2.address").is_empty());
        // Nor does one nobody knows.
        f.0.insert("mail.2.imap-server".into(), String::new());
        f.0.insert("mail.2.address".into(), "ada@example.org".into());
        assert!(after_change(&f, "mail.2.address").is_empty());
    }
}
