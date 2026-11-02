//! Dual boot, the card's side (see `paperpaw_x4::dual_boot`): once the card
//! is there, `paperpaw.ini` says what the reader chose, the bootloader says
//! whether it carries the marker, and what is declared is told to updates
//! (nothing installs in the other firmware's partition from then on) and to
//! plugins, by a note on the card they read.

use log::{info, warn};
use paperpaw_platform::Storage;
use paperpaw_x4::dual_boot::{self, Choice, Evidence, IniScan, Source};

use crate::ota::{self, Ota};

/// The file the kernel reads (`paperpaw.ini`).
const CONFIG: &str = "paperpaw.ini";
/// The kernel reads no more than this much of it; so do we.
const CONFIG_MAX: u64 = 64 * 1024;
/// Read a piece at a time.
const PIECE: usize = 1024;

/// What `paperpaw.ini` says, `Auto` where it says nothing or cannot be read.
fn choice(storage: &dyn Storage) -> Choice {
    let mut scan = IniScan::default();
    let mut piece = [0u8; PIECE];
    let mut at = 0u64;
    while at < CONFIG_MAX {
        match storage.read_at(CONFIG, at, &mut piece) {
            Ok(0) => break,
            Ok(n) => {
                scan.feed(&piece[..n]);
                at += n as u64;
            }
            // No file, a card that cannot be read: nothing chosen.
            Err(_) => break,
        }
    }
    scan.finish()
}

/// Settles whether this is a declared dual-boot setup, tells updates and
/// writes or removes the note for plugins. Whether it is declared, for what
/// else must not touch the other partition.
pub fn settle(ota: &mut Option<Ota>, flash: &ota::Flash, storage: &dyn Storage) -> bool {
    let Some(ota) = ota else {
        return false;
    };
    let choice = choice(storage);
    // Looked for at the start where it mattered (and found); else only now,
    // and only where the reader did not decide.
    let marker = if ota.dual() == Some(Source::Marker) {
        Some(true)
    } else if dual_boot::needs_marker(choice) {
        Some(ota::marker(flash))
    } else {
        None
    };
    let source = dual_boot::decide(Evidence { choice, marker });
    ota.declare_dual(source);
    match source {
        Some(source) => info!(
            "dual boot: declared by the {}: nothing is installed in the other slot",
            source.name()
        ),
        None if choice == Choice::Off => info!("dual boot: switched off in {CONFIG}"),
        None => {}
    }
    note(
        storage,
        source.map(|source| dual_boot::note(source, ota.slot())),
    );
    source.is_some()
}

/// The note on the card, only where it changes.
fn note(storage: &dyn Storage, text: Option<alloc::string::String>) {
    let mut head = [0u8; 64];
    let now = storage
        .read_at(dual_boot::NOTE, 0, &mut head)
        .ok()
        .map(|n| &head[..n]);
    match text {
        Some(text) if now != Some(text.as_bytes()) => {
            let _ = storage.create_dir(".paperpaw");
            if let Err(e) = storage.write(dual_boot::NOTE, text.as_bytes()) {
                warn!("dual boot: the note was not written: {e:?}");
            }
        }
        None if now.is_some() => {
            let _ = storage.remove(dual_boot::NOTE);
        }
        _ => {}
    }
}
