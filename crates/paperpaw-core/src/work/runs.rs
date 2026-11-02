//! The runs of background work, and the sockets they open. A socket the
//! commands of a work's event handlers or of a job's step connect is of
//! that run, not of the screens open (a run is no screen's): it stays
//! open while screens open and close, and closes when the run ends.
//!
//! A job's run ends when it is done or failed; an event's when the same
//! work (its id and key) runs again. Either ends when the work is taken
//! back or refused, or its plugin goes.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::{Command, Id, Params};

use crate::Kernel;
use crate::io::sockets::Owner;

/// A run of a work: its event emitted once, or a job from its first step
/// to its last.
#[derive(Debug)]
struct Run {
    number: u32,
    id: Id,
    key: String,
    owner: String,
}

/// The runs that may have sockets open.
#[derive(Debug, Default)]
pub(crate) struct Runs {
    runs: Vec<Run>,
    next: u32,
    /// The sockets a run's commands connect, until the commands are
    /// carried out.
    claims: Vec<(Id, u32)>,
    /// Runs ended since their sockets were last closed.
    ended: Vec<u32>,
}

impl Runs {
    /// A run of the work `id`, `key` of the plugin `owner` begins: the
    /// one before it ends.
    pub(crate) fn start(&mut self, id: &Id, key: &str, owner: &str) -> u32 {
        self.end(|r| r.id == *id && r.key == key);
        self.next = self.next.wrapping_add(1);
        self.runs.push(Run {
            number: self.next,
            id: id.clone(),
            key: String::from(key),
            owner: String::from(owner),
        });
        self.next
    }

    /// The run of the work `id`, `key` going on.
    pub(crate) fn of(&self, id: &Id, key: &str) -> Option<u32> {
        self.runs
            .iter()
            .find(|r| r.id == *id && r.key == key)
            .map(|r| r.number)
    }

    /// The runs of the work `id` (with `key`, any where none) end.
    pub(crate) fn end_work(&mut self, id: &Id, key: Option<&str>) {
        self.end(|r| r.id == *id && key.is_none_or(|k| r.key == k));
    }

    /// The runs of the plugin `owner` end.
    pub(crate) fn end_plugin(&mut self, owner: &str) {
        self.end(|r| r.owner == owner);
    }

    fn end(&mut self, ends: impl Fn(&Run) -> bool) {
        let ended = &mut self.ended;
        self.runs.retain(|r| {
            if ends(r) {
                ended.push(r.number);
            }
            !ends(r)
        });
    }

    /// The commands of the run `run`: the sockets they connect are its.
    pub(crate) fn claim(&mut self, run: u32, commands: &[Command]) {
        for command in commands {
            if let Command::Connect { socket, .. } = command {
                self.claims.push((socket.clone(), run));
            }
        }
    }

    /// The run the socket `socket` being connected is of, where a run's
    /// commands connect it.
    pub(crate) fn owner_of(&mut self, socket: &Id) -> Option<u32> {
        let at = self.claims.iter().position(|(s, _)| s == socket)?;
        Some(self.claims.swap_remove(at).1)
    }

    /// The run's commands were carried out: claims left (a connection
    /// refused before it was made) go; and runs go that have no socket
    /// open (`open`) and no job going on (`going`).
    pub(crate) fn settle(&mut self, open: impl Fn(u32) -> bool, going: impl Fn(&Id, &str) -> bool) {
        self.claims.clear();
        self.runs.retain(|r| open(r.number) || going(&r.id, &r.key));
    }

    /// How many runs are kept.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        self.runs.len()
    }
}

impl Kernel {
    /// Emits the event `id` of the work `id`, `key` of the plugin `owner`
    /// due, as a run of it: the sockets its handlers connect are the
    /// run's.
    pub(crate) fn emit_run(&mut self, id: &Id, params: &Params, key: &str, owner: &str) {
        let run = self.works.runs.start(id, key, owner);
        let from = self.output.commands.len();
        self.emit(id, params);
        let sent = self.output.commands.get(from..).unwrap_or_default();
        self.works.runs.claim(run, sent);
    }

    /// The commands of the runs were carried out: the sockets of runs that
    /// ended close, runs with nothing open are forgotten.
    pub(crate) fn settle_runs(&mut self) {
        self.close_ended_runs();
        let (sockets, works) = (&self.io.sockets, &mut self.works);
        let entries = &works.entries;
        works.runs.settle(
            |run| sockets.of_run(run),
            |id, key| entries.iter().any(|e| e.going(id, key)),
        );
    }

    /// Carries out a plugin's socket command: a socket that a run of
    /// background work's commands connect is the run's, any other the
    /// screens'.
    pub(crate) fn socket_command(&mut self, command: Command) {
        let run = match &command {
            Command::Connect { socket, .. } => self.works.runs.owner_of(socket),
            _ => None,
        };
        let owner = run.map_or(Owner::Screens(self.stack.len()), Owner::Run);
        let network = self.shared.caps.network;
        self.io.sockets.command(
            command,
            network,
            owner,
            &*self.shared.storage,
            &mut self.output.commands,
        );
    }

    /// Closes the sockets of the runs of background work that ended.
    pub(crate) fn close_ended_runs(&mut self) {
        for run in core::mem::take(&mut self.works.runs.ended) {
            self.io
                .sockets
                .close_for_run(run, &mut self.output.commands);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connect(socket: &'static str) -> Command {
        Command::connect(
            Id::new_static(socket),
            "example.org",
            993,
            Id::new_static("a.heard"),
        )
    }

    #[test]
    fn a_claim_not_carried_out_goes_with_the_runs_commands() {
        let mut runs = Runs::default();
        let run = runs.start(&Id::new_static("a.fetch"), "", "a");
        runs.claim(run, &[connect("a.one"), connect("a.two")]);
        assert_eq!(runs.owner_of(&Id::new_static("a.one")), Some(run));
        // The other was refused before it connected: no run claims it later.
        runs.settle(|_| false, |_, _| false);
        assert_eq!(runs.owner_of(&Id::new_static("a.two")), None);
        assert_eq!(runs.count(), 0, "no socket open, no job going on");
        assert_eq!(runs.ended, [] as [u32; 0], "settled, not ended");
    }

    #[test]
    fn runs_take_a_few_words() {
        // What the X4 holds beside the work queue, and for each run.
        let (runs, run) = (size_of::<Runs>(), size_of::<Run>());
        assert!(runs <= 4 * size_of::<Vec<u8>>(), "{runs}");
        assert!(run <= 4 * size_of::<Vec<u8>>(), "{run}");
    }
}
