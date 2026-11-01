//! Numbers the SDK writes for showing (the clock, a length of time, a
//! file's size, a setting's number) come in the digits of the language
//! shown; English keeps Latin ones.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use std::borrow::Cow;

use common::Harness;
use paperpaw_i18n::Catalog;
use paperpaw_sdk::builtin::settings;
use paperpaw_sdk::{SettingSpec, SettingValue, files, time, ui};

const HOUR: i64 = 3600;

/// A number setting without a unit.
const COUNT: SettingSpec = SettingSpec::number(
    "t.count",
    "t.count",
    0,
    500,
    120,
    paperpaw_sdk::builtin::groups::OTHER,
);

/// English and Persian (which writes Eastern Arabic digits).
fn harness(language: &str) -> Harness {
    let mut h = Harness::new().with_clock((13 * HOUR + 5 * 60) * 1000, Some(0));
    for setting in [
        &settings::CLOCK_FORMAT,
        &settings::TIME_ZONE,
        &settings::TIME_ZONE_RULE,
    ] {
        h.settings
            .define(SettingSpec::new(
                paperpaw_sdk::Id::from(setting),
                "x",
                paperpaw_sdk::SettingKind::Text,
                SettingValue::Text("".into()),
                paperpaw_sdk::builtin::groups::OTHER,
            ))
            .unwrap();
    }
    h.settings.define(COUNT).unwrap();
    let english = "[meta]\nlanguage = en\nname = English\nplural = one-other\n\n[messages]\n\
        core.clock.am = {time} AM\ncore.clock.pm = {time} PM\ncore.decimal = .\n\
        core.duration.minutes = {m} min\ncore.duration.hours = {h} h {m} min\n\
        core.page-of = {page} / {pages}\n";
    let persian = "[meta]\nlanguage = fa\nname = Farsi\nplural = one-other\ndigits = arabext\n\n\
        [messages]\ncore.clock.am = {time} ق.ظ\ncore.clock.pm = {time} ب.ظ\ncore.decimal = ٫\n\
        core.duration.minutes = {m} دقیقه\ncore.duration.hours = {h} ساعت {m} دقیقه\n";
    h.i18n.add(Catalog::parse(english.to_string()).unwrap());
    h.i18n.add(Catalog::parse(persian.to_string()).unwrap());
    h.i18n.set_language(language);
    h
}

fn twelve_hours(h: &mut Harness) {
    h.settings
        .set(
            settings::CLOCK_FORMAT.as_str(),
            SettingValue::Text(Cow::Borrowed(time::CLOCK_12H)),
        )
        .unwrap();
}

#[test]
fn english_keeps_latin_digits() {
    let mut h = harness("en");
    {
        let env = h.env();
        assert_eq!(time::clock_text(&env).as_deref(), Some("13:05"));
        assert_eq!(time::duration_text(&env, 65 * 60), "1 h 05 min");
        assert_eq!(files::shown_size(&env, 2_400_000), "2.3 MB");
        assert_eq!(ui::setting_value(&env, &COUNT), "120");
    }
    twelve_hours(&mut h);
    assert_eq!(time::clock_text(&h.env()).as_deref(), Some("1:05 PM"));
}

#[test]
fn persian_writes_its_own_digits() {
    let mut h = harness("fa");
    {
        let env = h.env();
        assert_eq!(time::clock_text(&env).as_deref(), Some("۱۳:۰۵"));
        assert_eq!(time::duration_text(&env, 65 * 60), "۱ ساعت ۰۵ دقیقه");
        assert_eq!(time::duration_text(&env, 42 * 60), "۴۲ دقیقه");
        assert_eq!(files::shown_size(&env, 2_400_000), "۲٫۳ MB");
        assert_eq!(files::shown_size(&env, 820), "۸۲۰ B");
        assert_eq!(ui::setting_value(&env, &COUNT), "۱۲۰");
    }
    twelve_hours(&mut h);
    assert_eq!(time::clock_text(&h.env()).as_deref(), Some("۱:۰۵ ب.ظ"));
}

/// The pager's label (`ui::Header`, `core.page-of` as the kernel's
/// en.ini has it): numbers given as numbers come in the language's digits,
/// even where its catalog has no words of its own for the label.
#[test]
fn the_pagers_label_in_both_digits() {
    let label = |h: &Harness| {
        h.env().i18n().format(
            "core.page-of",
            &[
                ("page", paperpaw_i18n::Arg::Int(3)),
                ("pages", paperpaw_i18n::Arg::Int(12)),
            ],
        )
    };
    assert_eq!(label(&harness("en")), "3 / 12");
    assert_eq!(label(&harness("fa")), "۳ / ۱۲");
}
