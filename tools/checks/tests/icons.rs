//! A plugin names only icons the icon font holds: a name Tabler has but
//! the font (`tools/icons/subset.py`, `ICONS`) lacks draws nothing, which
//! shows as a blank where the icon should be. Found as a string literal
//! that is a Tabler name (hyphenated, or on a line that speaks of an
//! icon) in a plugin or the app. A literal that is not an icon (a CSS
//! property, say) is listed in `icons-not-icons.txt`.
//!
//! To use another icon, add its name to `ICONS` and run
//! `tools/icons/subset.py` (the font grows by a few hundred bytes).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use std::collections::BTreeSet;

use paperpaw_checks::{root, rust_files};

/// The icons the font holds: the names quoted in `ICONS` of the script.
fn in_font() -> BTreeSet<String> {
    let script = std::fs::read_to_string(root().join("tools/icons/subset.py")).unwrap();
    let start = script.find("ICONS = sorted({").expect("ICONS");
    let body = &script[start..];
    let body = &body[..body.find("})").expect("end of ICONS")];
    body.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The quoted words of `line` that are plain names (letters, digits and
/// hyphens).
fn literals(line: &str) -> Vec<&str> {
    line.split('"')
        .skip(1)
        .step_by(2)
        .filter(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        .collect()
}

#[test]
fn plugins_name_only_icons_the_font_holds() {
    let font = in_font();
    assert!(font.len() > 50, "ICONS not read: {}", font.len());
    let all: BTreeSet<&str> = paperpaw_sdk::icons::names().collect();
    let allowed: BTreeSet<&str> = include_str!("../icons-not-icons.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    let mut found = Vec::new();
    for folder in ["plugins", "crates/paperpaw-app"] {
        for (name, path) in rust_files(&root().join(folder)) {
            // Tests say what they like.
            if name.contains("/tests/") || name.ends_with("tests.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for (number, line) in text.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let speaks_of_icons = line.to_lowercase().contains("icon");
                for word in literals(line) {
                    let looks_like_one = word.contains('-') || speaks_of_icons;
                    if looks_like_one
                        && all.contains(word)
                        && !font.contains(word)
                        && !allowed.contains(word)
                    {
                        found.push(format!("{folder}/{name}:{}: {word}", number + 1));
                    }
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "icons the font lacks (add them to ICONS in tools/icons/subset.py): {found:#?}"
    );
}
