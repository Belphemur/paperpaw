//! PaperPaw as one of two firmwares beside another's (#95, phase 1): where
//! the setup is declared a dual-boot one (the marker in the bootloader, or
//! `dual-boot = on` in `paperpaw.ini`), nothing is written that the other
//! firmware's slot or the OTA data holds: the start leaves the OTA data as it
//! finds it, no trial begins, and no update or other firmware installs.
//! Where it is not declared, everything is as before. On a flash in memory,
//! with each partition table PaperPaw ships and CrossPoint's, in either slot.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

mod common;

use common::{FLASH, layouts, table};
use paperpaw_sdk::MemoryStorage;
use paperpaw_sdk::firmware::{CHIP_ESP32_C3, Firmware, InstallFailure};
use paperpaw_x4::dual_boot::{MARKER, Source, WINDOW, scan_marker};
use paperpaw_x4::ota::{Flash, FlashError, OtaData, SLOTS, Start, State};
use paperpaw_x4::partitions::{Slots, TABLE_AT, TABLE_LEN};
use paperpaw_x4::updates::{TRIAL_MS, Updates};

/// The whole flash, in the chip's addresses, as the firmware has it.
struct Chip(Vec<u8>);

impl Flash for Chip {
    fn read(&mut self, at: u32, buf: &mut [u8]) -> Result<(), FlashError> {
        buf.copy_from_slice(&self.0[at as usize..at as usize + buf.len()]);
        Ok(())
    }

    fn program(&mut self, at: u32, data: &[u8]) -> Result<(), FlashError> {
        for (byte, new) in self.0[at as usize..].iter_mut().zip(data) {
            *byte &= new;
        }
        Ok(())
    }

    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        self.0[from as usize..to as usize].fill(0xFF);
        Ok(())
    }
}

/// A flash with the table and something of the other firmware in each slot
/// (CrossPoint's, or another's: a few bytes of its own).
fn flash(csv: &str) -> (Chip, Slots) {
    let mut flash = vec![0xFF; FLASH];
    flash[TABLE_AT as usize..TABLE_AT as usize + TABLE_LEN].copy_from_slice(&table(csv));
    let slots = Slots::find(&flash[TABLE_AT as usize..TABLE_AT as usize + TABLE_LEN]).unwrap();
    for (n, &(at, _)) in slots.apps.iter().enumerate() {
        flash[at as usize..at as usize + 256].fill(0xE9 ^ n as u8);
    }
    (Chip(flash), slots)
}

fn this() -> Firmware {
    Firmware::new("0.6.0-beta.1", "paperpaw-xteink", CHIP_ESP32_C3)
}

fn data(chip: &Chip, slots: &Slots) -> Vec<u8> {
    chip.0[slots.data as usize..slots.data as usize + 0x2000].to_vec()
}

/// The other firmware chose its own slot (`chosen`), and the bootloader of
/// the tool started `running` anyway: the case where the firmware, left to
/// itself, would write the OTA data to choose its own again.
fn disagreeing(chip: &mut Chip, slots: &Slots, chosen: u32, state: State) {
    let mut part = Part(chip, slots.data);
    let mut ota = OtaData::read(&mut part).unwrap();
    ota.choose(&mut part, chosen, state, 0).unwrap();
}

/// The OTA data's partition, as `Updates` hands it to `OtaData`.
struct Part<'a>(&'a mut Chip, u32);

impl Flash for Part<'_> {
    fn read(&mut self, at: u32, buf: &mut [u8]) -> Result<(), FlashError> {
        self.0.read(self.1 + at, buf)
    }

    fn program(&mut self, at: u32, data: &[u8]) -> Result<(), FlashError> {
        self.0.program(self.1 + at, data)
    }

    fn erase(&mut self, from: u32, to: u32) -> Result<(), FlashError> {
        self.0.erase(self.1 + from, self.1 + to)
    }
}

fn start(
    chip: Chip,
    slots: &Slots,
    running: u32,
    declared: Option<Source>,
) -> (Updates<Chip>, Option<Start>, u32) {
    let mut asked = 0;
    let (updates, start) = Updates::start_guarded(chip, slots, running, this(), "x4", &mut |_| {
        asked += 1;
        declared
    })
    .unwrap();
    (updates, start, asked)
}

#[test]
fn a_declared_setup_leaves_the_ota_data_as_it_finds_it() {
    for (name, csv) in layouts() {
        for running in 0..SLOTS {
            let (mut chip, slots) = flash(csv);
            // The other firmware's choice, which this start would undo.
            disagreeing(&mut chip, &slots, (running + 1) % SLOTS, State::Valid);
            let before = chip.0.clone();
            let (updates, started, asked) = start(chip, &slots, running, Some(Source::Marker));
            assert_eq!(started, Some(Start::Kept), "{name} {running}");
            assert_eq!(asked, 1, "{name}");
            assert_eq!(updates.dual(), Some(Source::Marker), "{name}");
            assert!(!updates.on_trial(), "{name}");
            // Not one byte of the flash written.
            assert!(updates.into_flash().0 == before, "{name} {running}");
        }
    }
}

#[test]
fn without_a_declaration_the_start_is_as_before() {
    for (name, csv) in layouts() {
        let (mut chip, slots) = flash(csv);
        disagreeing(&mut chip, &slots, 1, State::Valid);
        let before = data(&chip, &slots);
        let (updates, started, asked) = start(chip, &slots, 0, None);
        assert_eq!(started, Some(Start::Undone), "{name}");
        assert_eq!(asked, 1, "{name}: asked, since it would write");
        assert_eq!(updates.dual(), None, "{name}");
        assert_ne!(data(&updates.into_flash(), &slots), before, "{name}");
    }
}

#[test]
fn a_normal_start_costs_nothing_and_never_asks() {
    let (chip, slots) = flash(layouts()[0].1);
    let (updates, started, asked) = start(chip, &slots, 0, Some(Source::Marker));
    assert_eq!(started, Some(Start::Kept));
    // Nothing would be written, so nothing was looked for: no 32 KB read.
    assert_eq!(asked, 0);
    assert_eq!(updates.dual(), None);
}

#[test]
fn no_trial_begins_in_a_declared_setup() {
    // A tool chose this slot as `New`, as CrossPoint's installer does: the
    // firmware would mark it pending.
    for (name, csv) in layouts() {
        let (mut chip, slots) = flash(csv);
        disagreeing(&mut chip, &slots, 0, State::New);
        let before = chip.0.clone();
        let (mut updates, started, _) = start(chip, &slots, 0, Some(Source::Ini));
        assert_eq!(started, Some(Start::Kept), "{name}");
        assert!(!updates.on_trial(), "{name}");
        // And nothing is marked later either.
        assert!(!updates.check_trial(TRIAL_MS * 2).unwrap(), "{name}");
        updates.pause_trial(1000).unwrap();
        updates.resume_trial(2000).unwrap();
        assert!(updates.into_flash().0 == before, "{name}");
    }
}

#[test]
fn nothing_installs_in_a_declared_setup() {
    for (name, csv) in layouts() {
        let (chip, slots) = flash(csv);
        let other = (
            slots.apps[1].0 as usize,
            chip.0[slots.apps[1].0 as usize..][..4096].to_vec(),
        );
        // A normal start asks nothing; the firmware says what it found after
        // the card (`declare_dual`).
        let (mut updates, _, _) = start(chip, &slots, 0, None);
        updates.declare_dual(Some(Source::Marker));
        let card = MemoryStorage::new();
        // Refused before the file is even read, whatever it is.
        assert_eq!(
            updates
                .begin(&card, "updates/paperpaw.bin", None, true)
                .err(),
            Some(InstallFailure::TooLarge),
            "{name}"
        );
        assert_eq!(
            updates.begin_foreign(&card, "updates/other.bin").err(),
            Some(InstallFailure::TooLarge),
            "{name}"
        );
        assert!(!updates.installing(), "{name}");
        // The page's own check finds no room for any update either.
        assert_eq!(updates.firmware().room, 0, "{name}");
        let chip = updates.into_flash();
        assert!(
            chip.0[other.0..][..4096] == other.1[..],
            "{name}: the other slot"
        );
    }
}

#[test]
fn without_a_declaration_installs_are_as_before() {
    let (chip, slots) = flash(layouts()[0].1);
    let (mut updates, _, _) = start(chip, &slots, 0, None);
    let card = MemoryStorage::new();
    // Not refused for being in a dual-boot setup: the file is not there.
    assert_eq!(
        updates
            .begin(&card, "updates/paperpaw.bin", None, true)
            .err(),
        Some(InstallFailure::Unreadable)
    );
    assert!(updates.firmware().room > 0);
}

#[test]
fn a_declaration_from_the_ini_comes_after_the_start_and_guards_from_then_on() {
    let (chip, slots) = flash(layouts()[0].1);
    let (mut updates, _, _) = start(chip, &slots, 0, None);
    let card = MemoryStorage::new();
    updates.declare_dual(Some(Source::Ini));
    assert_eq!(updates.dual(), Some(Source::Ini));
    assert_eq!(
        updates
            .begin(&card, "updates/paperpaw.bin", None, true)
            .err(),
        Some(InstallFailure::TooLarge)
    );
    assert_eq!(updates.firmware().room, 0);
    // Declared off again (`dual-boot = off` beats the marker): as before.
    updates.declare_dual(None);
    assert!(updates.firmware().room > 0);
}

#[test]
fn a_trial_begun_before_the_ini_was_read_still_ends_cleanly() {
    // The OTA data is marked before the card is there; an ini that declares
    // the setup afterwards must not leave the trial half done (the next
    // start would see it failed and go to the other firmware).
    let (mut chip, slots) = flash(layouts()[0].1);
    disagreeing(&mut chip, &slots, 0, State::New);
    let (mut updates, started, _) = start(chip, &slots, 0, None);
    assert_eq!(started, Some(Start::Trial { ran: 0 }));
    updates.declare_dual(Some(Source::Ini));
    assert!(updates.on_trial());
    assert!(updates.check_trial(TRIAL_MS).unwrap());
    let mut chip = updates.into_flash();
    let mut part = Part(&mut chip, slots.data);
    let ota = OtaData::read(&mut part).unwrap();
    assert_eq!(ota.state_of(0), Some((State::Valid, 0)));
}

#[test]
fn the_marker_is_looked_for_in_the_bootloaders_window_only() {
    let (mut chip, _) = flash(layouts()[0].1);
    assert_eq!(scan_marker(&mut chip), Ok(false));
    // Across two pieces, at the very end of the window, and just beyond it.
    for (at, found) in [
        (0x1000 - 7, true),
        (WINDOW as usize - MARKER.len(), true),
        (WINDOW as usize - MARKER.len() + 1, false),
        (WINDOW as usize + 0x100, false),
    ] {
        let (mut chip, _) = flash(layouts()[0].1);
        chip.0[at..at + MARKER.len()].copy_from_slice(MARKER);
        assert_eq!(scan_marker(&mut chip), Ok(found), "at {at:#x}");
    }
}

#[test]
fn the_note_the_firmware_writes_is_the_one_the_update_plugin_reads() {
    use paperpaw_sdk::Storage;
    use paperpaw_x4::dual_boot::{NOTE, note, noted};
    let card = MemoryStorage::new();
    assert!(!paperpaw_plugin_update::dual_boot_declared(&card));
    for source in [Source::Ini, Source::Marker] {
        for slot in 0..SLOTS {
            let text = note(source, slot);
            assert!(noted(&text));
            card.write(NOTE, text.as_bytes()).unwrap();
            assert!(paperpaw_plugin_update::dual_boot_declared(&card));
            card.write(NOTE, b"").unwrap();
            assert!(!paperpaw_plugin_update::dual_boot_declared(&card));
        }
    }
}
