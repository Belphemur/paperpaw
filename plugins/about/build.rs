//! Packs the licences for the build's product: the firmware's
//! (`data/licences-firmware.md`, for the readers, `target_os = "none"`) or the
//! apps' (`data/licences-app.md`), both from `tools/licences/build.py`. They
//! are split into parts of about [`PART`] bytes at paragraphs, each
//! deflated on its own, so that the page writes them out to read one
//! part at a time, with little memory, and they take little of the
//! readers' flash.

use std::path::PathBuf;

/// The largest part, before it is deflated.
const PART: usize = 8 * 1024;

fn main() {
    let product = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("none") => "firmware",
        _ => "app",
    };
    let source = format!("data/licences-{product}.md");
    println!("cargo:rerun-if-changed={source}");
    println!("cargo:rerun-if-changed=build.rs");
    let text = std::fs::read_to_string(&source).unwrap_or_else(|e| panic!("{source}: {e}"));
    let mut out = Vec::new();
    for part in parts(&text) {
        let packed = miniz_oxide::deflate::compress_to_vec(part.as_bytes(), 10);
        out.extend_from_slice(&u32::try_from(packed.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&u32::try_from(part.len()).unwrap_or(0).to_le_bytes());
        out.extend_from_slice(&packed);
    }
    let dir = PathBuf::from(
        std::env::var_os("OUT_DIR").unwrap_or_else(|| panic!("OUT_DIR is set by cargo")),
    );
    std::fs::write(dir.join("licences.bin"), out).unwrap_or_else(|e| panic!("licences.bin: {e}"));
}

/// `text` in parts of at most [`PART`] bytes, cut after an empty line
/// where there is one (else at a line, else at a character).
fn parts(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = text;
    while rest.len() > PART {
        let mut end = PART;
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        let window = &rest[..end];
        let cut = window
            .rfind("\n\n")
            .map(|at| at + 2)
            .or_else(|| window.rfind('\n').map(|at| at + 1))
            .filter(|at| *at > 0)
            .unwrap_or(end);
        parts.push(&rest[..cut]);
        rest = &rest[cut..];
    }
    if !rest.is_empty() {
        parts.push(rest);
    }
    parts
}
