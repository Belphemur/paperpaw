//! A book in a script the reading font has no letters of (the X4 family's
//! firmware carries Latin alone): said once, with the pack from the store
//! that brings them, instead of leaving the reader with question marks.

use alloc::format;
use alloc::string::{String, ToString};

use paperpaw_sdk::{Command, Cx, Env, FontHandle};

/// The script (ISO 15924), a letter every font of it has, and the notice
/// that names its pack.
const SCRIPTS: [(&str, char, &str); 4] = [
    ("Grek", 'α', "reader.script.greek"),
    ("Cyrl", 'ж', "reader.script.cyrillic"),
    ("Arab", 'ع', "reader.script.arabic"),
    ("Hebr", 'א', "reader.script.hebrew"),
];

/// Where the notices said are kept, one key a line.
fn kept() -> String {
    format!("{}/script-hints.txt", Env::plugin_dir("reader"))
}

fn said(env: &Env<'_>, key: &str) -> bool {
    paperpaw_sdk::read_file(env.storage(), &kept(), 1024)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .is_some_and(|text| text.lines().any(|line| line == key))
}

/// The notice for a book in the language `tag` that says it declares: when
/// the reading font has not the letters of the script that language is
/// written in, and it was not said before.
pub(super) fn missing(env: &Env<'_>, font: &FontHandle<'_>, tag: &str) -> Option<&'static str> {
    let script = paperpaw_sdk::lang::likely_script_of(tag)?;
    let (_, letter, key) = SCRIPTS.iter().find(|(s, ..)| *s == script)?;
    (!font.has_char(*letter) && !said(env, key)).then_some(*key)
}

/// Says the notice `key` (from [`missing`]) and keeps that it was said.
pub(super) fn say(cx: &mut Cx<'_, '_>, key: &str) {
    let env = *cx.env();
    #[expect(
        clippy::let_underscore_must_use,
        reason = "a notice not kept is said again at the next opening: nothing is lost"
    )]
    {
        let _ = env.storage().create_dir(&Env::plugin_dir("reader"));
        let _ = env.storage().append(&kept(), format!("{key}\n").as_bytes());
    }
    cx.command(Command::Notify(env.i18n().tr(key).to_string()));
}
