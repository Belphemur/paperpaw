//! The note PaperPaw leaves on the card when it is installed from another
//! firmware's menu (CrossPoint's *SD firmware update*) and the first start
//! does not get through its trial (`updates`): the device goes back to the
//! firmware before it, which says nothing of why, and a USB-locked reader
//! has no log. The note is on the card's top level, where a person who
//! comes back to the firmware before it finds it, in English and German,
//! with what a bug report needs.
//!
//! Two kinds: [`Kind::Started`], written when an update on trial has
//! mounted the card (and taken away when the trial is over), so that its
//! presence tells "PaperPaw started and got as far as the card"; and
//! [`Kind::Failed`], written by the next start that finds the trial did
//! not end cleanly, just before the device goes back.

use core::fmt;

/// Where the note is: the card's top level.
pub const FILE: &str = "paperpaw-install-note.txt";

/// The first line of a note of the first start: a later start tells it
/// from a note of an earlier install (see [`is_started`]).
const STARTED_LINE: &str = "paperpaw-install-note: started";
/// The first line of the note of a failed start.
const FAILED_LINE: &str = "paperpaw-install-note: failed";

/// Whether the text read from the start of the note file is a note of a
/// first start (and not an older note of a failed install, or any other
/// file of that name).
pub fn is_started(head: &[u8]) -> bool {
    head.starts_with(STARTED_LINE.as_bytes())
}

/// What the note says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind<'a> {
    /// The first start of an update on trial.
    Started,
    /// A later start found the trial of the last one did not end cleanly.
    Failed {
        /// The panic message of the last run, if it ended in one.
        panic: Option<&'a str>,
        /// Whether the note of the first start was on the card: then an
        /// earlier run got as far as the card; if not, the update was
        /// found failed before any run noted itself (a crash before the
        /// card, or a bootloader that marked the start itself).
        started: bool,
    },
}

/// The note, with the facts of the run.
#[derive(Debug, Clone, Copy)]
pub struct InstallNote<'a> {
    pub kind: Kind<'a>,
    /// "Xteink X4", "Xteink X3", ...
    pub device: &'a str,
    pub version: &'a str,
    /// Milliseconds the trial ran in earlier starts (0: none noted).
    pub ran_ms: u32,
    /// Why the last start came about ("PowerOn", "CoreSw", "CoreWdt", ...).
    pub reset: &'a str,
    /// Where the last run was when it ended, as the code names it.
    pub place: &'a str,
}

impl fmt::Display for InstallNote<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            Kind::Started => {
                writeln!(f, "{STARTED_LINE}")?;
                writeln!(
                    f,
                    "PaperPaw {} started on this {} for the first time. It is on trial: it must run for 30 seconds,",
                    self.version, self.device
                )?;
                writeln!(
                    f,
                    "else the reader goes back to the firmware it had before (CrossPoint). PaperPaw deletes this file when the trial is over."
                )?;
                writeln!(
                    f,
                    "If you read this on a computer and PaperPaw is not running, it started but did not get through the first 30 seconds"
                )?;
                writeln!(
                    f,
                    "(or the reader was switched off, or the card was taken out). Please send this file to the PaperPaw project."
                )?;
                writeln!(
                    f,
                    "PaperPaw {} ist auf diesem {} zum ersten Mal gestartet und ist auf Probe: Es muss 30 Sekunden laufen,",
                    self.version, self.device
                )?;
                writeln!(
                    f,
                    "sonst geht das Gerät zur vorherigen Firmware (CrossPoint) zurück. PaperPaw löscht diese Datei, wenn die Probe vorbei ist."
                )?;
                writeln!(
                    f,
                    "Wenn Sie das am Computer lesen und PaperPaw nicht läuft, ist es gestartet, hat aber die ersten 30 Sekunden nicht geschafft"
                )?;
                writeln!(
                    f,
                    "(oder das Gerät wurde ausgeschaltet oder die Karte herausgenommen). Bitte schicken Sie diese Datei an das PaperPaw-Projekt."
                )?;
            }
            Kind::Failed { .. } => {
                writeln!(f, "{FAILED_LINE}")?;
                writeln!(
                    f,
                    "PaperPaw {} did not stay on this {}: its first start did not end cleanly (a crash, a reset or a power loss",
                    self.version, self.device
                )?;
                writeln!(
                    f,
                    "in the first 30 seconds), so the reader went back to the firmware it had before (CrossPoint). Please send this file to the PaperPaw project."
                )?;
                writeln!(
                    f,
                    "PaperPaw {} ist auf diesem {} nicht geblieben: Der erste Start ist nicht sauber zu Ende gegangen (Absturz, Neustart oder Stromausfall",
                    self.version, self.device
                )?;
                writeln!(
                    f,
                    "in den ersten 30 Sekunden), darum ist das Gerät zur vorherigen Firmware (CrossPoint) zurückgegangen. Bitte schicken Sie diese Datei an das PaperPaw-Projekt."
                )?;
            }
        }
        writeln!(f)?;
        writeln!(
            f,
            "Run before / gelaufen vorher: {} ms; reset / Neustart: {}; last place / letzter Ort: {}",
            self.ran_ms, self.reset, self.place
        )?;
        if let Kind::Failed { panic, started } = self.kind {
            let (yes, ja) = if started {
                ("yes", "ja")
            } else {
                ("no", "nein")
            };
            writeln!(
                f,
                "Note of the first start found / Notiz des ersten Starts gefunden: {yes} / {ja}"
            )?;
            if let Some(message) = panic {
                writeln!(f, "Panic: {}", message.trim_end())?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::format;
    use alloc::string::ToString;

    use super::*;

    fn note(kind: Kind<'_>) -> InstallNote<'_> {
        InstallNote {
            kind,
            device: "Xteink X4",
            version: "0.5.0-beta.12",
            ran_ms: 1200,
            reset: "CoreSw",
            place: "card",
        }
    }

    #[test]
    fn a_started_note_says_it_is_on_trial_in_both_languages() {
        let text = note(Kind::Started).to_string();
        assert!(text.contains("on trial"), "{text}");
        assert!(text.contains("auf Probe"), "{text}");
        assert!(text.contains("PaperPaw 0.5.0-beta.12"), "{text}");
        assert!(text.contains("Xteink X4"), "{text}");
        assert!(!text.contains("did not stay"), "{text}");
    }

    #[test]
    fn a_started_note_is_told_from_a_failed_one_by_its_first_line() {
        let started = note(Kind::Started).to_string();
        let failed = note(Kind::Failed {
            panic: None,
            started: false,
        })
        .to_string();
        assert!(is_started(started.as_bytes()));
        assert!(!is_started(failed.as_bytes()));
        assert!(!is_started(b""));
        assert!(!is_started(b"some other file"));
    }

    #[test]
    fn a_failed_note_says_it_went_back_and_carries_the_facts() {
        let text = note(Kind::Failed {
            panic: None,
            started: true,
        })
        .to_string();
        assert!(text.contains("did not stay on this Xteink X4"), "{text}");
        assert!(text.contains("nicht geblieben"), "{text}");
        assert!(text.contains("CrossPoint"), "{text}");
        assert!(text.contains("1200 ms"), "{text}");
        assert!(text.contains("CoreSw"), "{text}");
        assert!(text.contains("last place / letzter Ort: card"), "{text}");
        assert!(!text.contains("Panic:"), "{text}");
        assert!(text.contains("gefunden: yes / ja"), "{text}");
    }

    #[test]
    fn a_failed_note_says_when_no_first_start_was_noted() {
        let text = note(Kind::Failed {
            panic: None,
            started: false,
        })
        .to_string();
        assert!(text.contains("gefunden: no / nein"), "{text}");
    }

    #[test]
    fn a_panic_message_is_in_the_failed_note_only() {
        let text = note(Kind::Failed {
            panic: Some("src/x.rs:1:2 oops\n"),
            started: true,
        })
        .to_string();
        assert!(text.contains("Panic: src/x.rs:1:2 oops"), "{text}");
        let started = format!("{}", note(Kind::Started));
        assert!(!started.contains("Panic:"));
    }

    #[test]
    fn a_note_stays_small() {
        // It is written to a card at a start: a few hundred bytes of text
        // and the panic's 400 at most.
        let long = "x".repeat(400);
        let text = note(Kind::Failed {
            panic: Some(&long),
            started: true,
        })
        .to_string();
        assert!(text.len() < 2048, "{}", text.len());
    }
}
