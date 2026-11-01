//! Plugins another plugin asks for (the reader's packs of letters for a
//! book) with the real kernel on an X4: the reader is asked in a dialog
//! that names them with their sizes, *Download* fetches them in the
//! background and the one who asked is told what came of it, once;
//! *Not now*, no connection and a plugin the library does not have are
//! answered at once.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{Answers, Shopping};
use paperpaw_sdk::{Storage, Version};

const PAGE: &str = "https://paperpaw.org/api/v1/plugins/script-greek.json?device=x4&lang=en";

/// The list for the X4: the Greek letters, and another plugin.
fn list() -> String {
    r#"{"format": 2, "plugins": [
  {"id": "script-greek", "name": "Greek letters", "summary": "Greek books.",
    "group": "look", "author": "PaperPaw", "licence": "OFL-1.1", "kind": "data",
    "latest": {"version": "1.0.0", "sdk": "0.4", "formats": 1, "size": 2900000}},
  {"id": "airy", "name": "Airy", "summary": "Wide margins.",
    "group": "reading", "author": "Someone", "licence": "MIT", "kind": "data",
    "latest": {"version": "1.0.0", "sdk": "0.4", "formats": 1, "size": 300}}
]}"#
    .to_string()
}

const MANIFEST: &str =
    "[plugin]\nid = script-greek\nname = Greek letters\nversion = 1.0.0\nformats = 1\n";

/// The pack's page at the library: one file.
fn page() -> String {
    format!(
        r#"{{"format": 2, "plugin": {{"id": "script-greek", "name": "Greek letters",
  "summary": "Greek books.", "group": "look", "author": "PaperPaw",
  "licence": "OFL-1.1", "kind": "data",
  "latest": {{"version": "1.0.0", "sdk": "0.4", "formats": 1, "size": 2900000,
    "files": [{{"path": "plugin.ini", "url": "script-greek/1.0.0/plugin.ini",
                "size": {}, "sha1": "{}"}}]}}}}}}"#,
        MANIFEST.len(),
        paperpaw_digest::sha1::hex(MANIFEST.as_bytes())
    )
}

/// What the one who asked was told, as `(installed, missing, declined,
/// error)`.
fn told(answers: &Answers) -> Vec<(String, String, bool, String)> {
    answers
        .borrow()
        .iter()
        .map(|a| {
            assert_eq!(a.by.as_str(), "reader");
            (
                a.installed.clone(),
                a.missing.clone(),
                a.declined,
                a.error.clone(),
            )
        })
        .collect()
}

#[test]
fn the_dialog_names_the_plugins_with_their_sizes() {
    let (mut shopping, answers) = Shopping::asked_for(&list(), true);
    shopping.ask("script-greek");
    let shown = shopping.shown();
    // The question wraps over lines.
    assert!(shown.contains("This book needs more"), "{shown}");
    assert!(shown.contains("letters to show its text."), "{shown}");
    assert!(shown.contains("Greek letters (2.8 MB)"), "{shown}");
    assert!(
        shown.contains("Download") && shown.contains("Not now"),
        "{shown}"
    );
    // Nobody is told before the reader has answered.
    assert!(told(&answers).is_empty());
}

#[test]
fn not_now_is_told_as_declined() {
    let (mut shopping, answers) = Shopping::asked_for(&list(), true);
    shopping.ask("script-greek");
    shopping.choose("Not now");
    assert_eq!(
        told(&answers),
        [(
            String::new(),
            "script-greek".to_string(),
            true,
            String::new()
        )]
    );
    // Nothing was fetched.
    assert!(shopping.requests().is_empty());
}

#[test]
fn download_fetches_the_pack_and_tells_the_one_who_asked() {
    let (mut shopping, answers) = Shopping::asked_for(&list(), true);
    shopping.ask("script-greek");
    shopping.choose("Download");
    shopping.serve(PAGE, page().as_bytes());
    shopping.serve(
        "https://paperpaw.org/api/v1/plugins/script-greek/1.0.0/plugin.ini",
        MANIFEST.as_bytes(),
    );
    // Installed at once, and said to the one who asked, once, not as a
    // notice of the store's.
    assert_eq!(
        shopping
            .kernel
            .plugins()
            .get("script-greek")
            .map(|p| p.manifest.version),
        Some(Version::new(1, 0, 0))
    );
    assert_eq!(
        told(&answers),
        [(
            "script-greek".to_string(),
            String::new(),
            false,
            String::new()
        )]
    );
    assert!(
        !shopping
            .storage
            .exists(".paperpaw/plugins/store/request.txt")
    );
    let shown = shopping.shown();
    assert!(!shown.contains("start the next time"), "{shown}");
}

#[test]
fn a_pack_that_cannot_be_fetched_is_told_with_the_reason() {
    let (mut shopping, answers) = Shopping::asked_for(&list(), true);
    shopping.ask("script-greek");
    shopping.choose("Download");
    let call = shopping.request(PAGE);
    shopping.answer(&call, 500, b"");
    assert_eq!(
        told(&answers),
        [(
            String::new(),
            "script-greek".to_string(),
            false,
            "Could not download: Greek letters.".to_string()
        )]
    );
    assert!(shopping.kernel.plugins().get("script-greek").is_none());
}

#[test]
fn without_a_connection_the_one_who_asked_is_told_so_and_no_dialog_opens() {
    let (mut shopping, answers) = Shopping::asked_for(&list(), false);
    shopping.ask("script-greek");
    let told = told(&answers);
    assert_eq!(told.len(), 1, "{told:?}");
    assert_eq!(told[0].1, "script-greek");
    assert!(!told[0].2, "{told:?}");
    assert!(told[0].3.starts_with("No connection."), "{told:?}");
    let shown = shopping.shown();
    assert!(!shown.contains("Download"), "{shown}");
}

/// What the library does not offer is not asked about and not an error:
/// it is told as missing, quietly.
#[test]
fn a_plugin_the_library_does_not_have_is_missing_quietly() {
    let (mut shopping, answers) = Shopping::asked_for(&list(), true);
    shopping.ask("script-korean");
    assert_eq!(
        told(&answers),
        [(
            String::new(),
            "script-korean".to_string(),
            false,
            String::new()
        )]
    );
    let shown = shopping.shown();
    assert!(!shown.contains("Download"), "{shown}");
}
