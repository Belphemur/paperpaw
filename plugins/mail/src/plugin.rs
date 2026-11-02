//! The plugin: its settings, screens, actions, widgets and translations.

use alloc::borrow::Cow;
use alloc::boxed::Box;

use paperpaw_sdk::builtin::{events, menus};
use paperpaw_sdk::{
    Command, FixedAction, FixedScreen, Instances, Manifest, MenuEntry, Params, Plugin, RegisterError,
    Registrar,
};

use crate::browser::{self, Browser};
use crate::reader::Reader;
use crate::writer::Writer;
use crate::{BOXES, COMPOSE, LIST, OPEN, PIN_ANSWER, PLUGIN, READ, WRITE, provider, settings, widgets};

/// How it works, by language (see `Manifest::how`).
static HOW: [(&str, &str); 2] = [
    ("en", include_str!("../how/en.txt")),
    ("de", include_str!("../how/de.txt")),
];

static ACTIONS: [FixedAction; 2] = [
    FixedAction::new(OPEN, "mail.title").handler(|_, _, out| out.push(Command::OpenScreen(BOXES, Params::new()))),
    FixedAction::new(WRITE, "mail.write").handler(|_, params, out| out.push(Command::OpenScreen(COMPOSE, params.clone()))),
];
static ENTRIES: [MenuEntry; 1] = [MenuEntry::new(menus::TOOLS, OPEN).order(47)];
static ICONS: [(paperpaw_sdk::Id, &str); 2] = [(OPEN, "inbox"), (WRITE, "pencil")];

static SCREENS: [FixedScreen; 4] = [
    FixedScreen::new(BOXES, "mail.title", |env, params| Box::new(Browser::new(env, params)))
        .instances(Instances::SINGLE),
    FixedScreen::new(LIST, "mail.title", |env, params| Box::new(Browser::new(env, params))),
    FixedScreen::new(READ, "mail.title", |env, params| Box::new(Reader::new(env, params))),
    FixedScreen::new(COMPOSE, "mail.write", |env, params| Box::new(Writer::new(env, params))).instances(Instances::SINGLE),
];

/// The Mail plugin.
#[derive(Debug, Default)]
pub struct Mail;

impl Plugin for Mail {
    fn manifest(&self) -> Manifest {
        Manifest::new(PLUGIN, "Mail", paperpaw_sdk::package_version!())
            .description("Reads and sends e-mail with a password or an app password (IMAP and SMTP), no browser login.")
            .how(&HOW)
            .uses(paperpaw_sdk::Uses::NETWORK)
    }

    fn register(&self, r: &mut Registrar) -> Result<(), RegisterError> {
        for group in &settings::GROUPS {
            r.add_setting_group(group)?;
        }
        for account in &settings::ACCOUNT_SETTINGS {
            r.add_settings(account)?;
        }
        r.add_settings(&settings::GENERAL)?;
        r.add_screens(&SCREENS)?;
        r.add_actions(&ACTIONS)?;
        r.add_menu_entries(&ENTRIES)?;
        r.add_icons(&ICONS)?;
        widgets::register(r)?;

        // Choosing a provider, or typing a known address, fills in the servers.
        r.on(events::SETTING_CHANGED, |env, changed, out| {
            out.extend(provider::commands(env, changed.id.as_str()));
        })?;
        r.on(PIN_ANSWER, |_, answer, out| browser::pin_answered(&answer, out))?;

        r.add_translations(Cow::Borrowed(include_str!("../i18n/en.ini")))?;
        r.add_translations(Cow::Borrowed(include_str!("../i18n/de.ini")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use paperpaw_i18n::{Catalog, check};

    #[test]
    fn translations_are_complete() {
        let en = Catalog::parse(include_str!("../i18n/en.ini")).unwrap();
        let de = Catalog::parse(include_str!("../i18n/de.ini")).unwrap();
        let issues = check::compare(&en, &de);
        assert!(issues.is_empty(), "{issues:?}");
    }
}
