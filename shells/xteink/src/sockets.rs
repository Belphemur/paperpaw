//! Plugins' sockets (the kernel's `SocketCall`s, see ADR 0014) over the
//! network of `net`, a step at a time from the main loop: TCP connections
//! that read only when asked (so what waits for the kernel is at most one
//! part per socket), and UDP broadcasts that collect answers for a while
//! (e.g. to find Calibre in the local network).
//!
//! No TLS for sockets yet (the kernel asks for none while the shell holds
//! no TLS sessions for them, `KernelBuilder::tls_sessions`): a TLS
//! connection or a switch to TLS fails, `SocketError::Unsupported`. To
//! come: `tls::Session` over `link::Link`, its sessions shared with HTTPS
//! (notes `designs/tls-sockets.md`).

use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_core::sdk::SocketError;
use paperpaw_core::{Effects, Kernel, SocketCall, SocketEvent};
use paperpaw_platform::Instant;
use smoltcp::socket::tcp;
use smoltcp::wire::IpAddress;

use crate::net::{self, Handle, Net, Query, Resolve};

/// Connections at a time.
const MAX_SOCKETS: usize = 2;
/// A connection's buffers: much to receive (books), little to send.
const BUFFERS: net::Buffers = net::Buffers {
    receive: 8 * 1024,
    send: 2048,
};
/// Bytes of a large read passed on at a time.
const PART: usize = 16 * 1024;
/// How long finding the host and connecting may take.
const CONNECT_MS: u64 = 10_000;
/// A quiet connection asks the other side this often whether it is there.
const KEEP_ALIVE_MS: u64 = 15_000;
/// A connection whose other side answers nothing for this long ends.
const GIVE_UP_MS: u64 = 45_000;
/// Answers to a broadcast taken at most, and their bytes in all.
const ANSWERS: usize = 8;
const ANSWER_BYTES: usize = 1024;

enum Phase {
    Resolving {
        query: Query,
        port: u16,
    },
    Connecting(Handle),
    Open(Handle),
    /// Nothing left to let go (a lookup that failed is gone by itself).
    Ended,
}

struct Socket {
    number: u32,
    phase: Phase,
    /// When connecting gives up.
    until: u64,
    /// Reads asked for (their most bytes), answered in turn.
    reads: VecDeque<u32>,
    /// What is still to send, and how much of the first was sent.
    sends: VecDeque<Vec<u8>>,
    sent: usize,
    /// Received for the first read, not passed on yet.
    gathered: Vec<u8>,
}

struct Broadcast {
    number: u32,
    udp: Handle,
    until: u64,
}

/// The sockets and broadcasts of all plugins.
#[derive(Default)]
pub struct Sockets {
    sockets: Vec<Socket>,
    broadcasts: Vec<Broadcast>,
    /// The network they are open on.
    generation: u32,
    /// Something came or went in the last step.
    moved: bool,
}

impl Sockets {
    /// Whether a socket or broadcast is open.
    pub fn any(&self) -> bool {
        !self.sockets.is_empty() || !self.broadcasts.is_empty()
    }

    /// Whether data is moving (or about to): the main loop comes by again
    /// soon.
    pub fn moving(&self) -> bool {
        self.moved
            || self
                .sockets
                .iter()
                .any(|s| !s.sends.is_empty() || !s.gathered.is_empty())
    }

    /// Does what the kernel asks; what it says at once.
    pub fn apply(&mut self, call: SocketCall, net: &mut Net, now: Instant) -> Option<SocketEvent> {
        if !self.any() {
            self.generation = net.generation();
        }
        match call {
            SocketCall::Connect {
                socket,
                tls: Some(_),
                ..
            } => Some(no_tls(socket)),
            SocketCall::StartTls { socket, .. } => {
                if let Some(at) = self.sockets.iter().position(|s| s.number == socket) {
                    self.sockets.remove(at).release(net);
                }
                Some(no_tls(socket))
            }
            SocketCall::Connect {
                socket,
                host,
                port,
                tls: None,
            } => {
                if self.sockets.len() >= MAX_SOCKETS {
                    return Some(failed(socket, SocketError::TooMany, "too many connections"));
                }
                let phase = match net.resolve(&host) {
                    Ok(Resolve::Done(address)) => match connect(net, address, port) {
                        Ok(phase) => phase,
                        Err(reason) => return Some(failed(socket, SocketError::Refused, reason)),
                    },
                    Ok(Resolve::Asking(query)) => Phase::Resolving { query, port },
                    Err(_) => return Some(failed(socket, SocketError::Dns, "no such host")),
                };
                self.sockets.push(Socket {
                    number: socket,
                    phase,
                    until: now.0 + CONNECT_MS,
                    reads: VecDeque::new(),
                    sends: VecDeque::new(),
                    sent: 0,
                    gathered: Vec::new(),
                });
                None
            }
            SocketCall::Send { socket, data } => {
                if let Some(s) = self.sockets.iter_mut().find(|s| s.number == socket) {
                    s.sends.push_back(data);
                }
                None
            }
            SocketCall::Read { socket, max } => {
                if let Some(s) = self.sockets.iter_mut().find(|s| s.number == socket) {
                    s.reads.push_back(max.max(1));
                }
                None
            }
            SocketCall::Close { socket } => {
                if let Some(at) = self.sockets.iter().position(|s| s.number == socket) {
                    self.sockets.remove(at).release(net);
                }
                None
            }
            SocketCall::Broadcast {
                broadcast,
                ports,
                data,
                wait_ms,
            } => {
                self.broadcast(broadcast, &ports, &data, wait_ms, net, now);
                // Nothing could be sent: done at once.
                (!self.broadcasts.iter().any(|b| b.number == broadcast))
                    .then_some(SocketEvent::Done { broadcast })
            }
        }
    }

    fn broadcast(
        &mut self,
        number: u32,
        ports: &[u16],
        data: &[u8],
        wait_ms: u32,
        net: &mut Net,
        now: Instant,
    ) {
        let Some(to) = net.broadcast_address() else {
            return;
        };
        // Room to send to every port at once, and for the answers.
        let packets = ports.len().max(ANSWERS);
        let bytes = (data.len() * ports.len()).max(ANSWER_BYTES);
        let Ok(udp) = net.open_udp(packets, bytes) else {
            return;
        };
        for &port in ports {
            let _ = net.udp(udp).send_slice(data, (to, port));
        }
        self.broadcasts.push(Broadcast {
            number,
            udp,
            until: now.0 + u64::from(wait_ms),
        });
    }

    /// The network went (or is another one now): every socket fails, every
    /// broadcast is done. What they hold is let go on `net` if they were
    /// opened on it (else it went with an earlier one).
    pub fn lost(
        &mut self,
        net: Option<&mut Net>,
        kernel: &mut Kernel,
        now: Instant,
    ) -> Vec<Effects> {
        let mut net = net.filter(|net| net.generation() == self.generation);
        let mut effects = Vec::new();
        for socket in core::mem::take(&mut self.sockets) {
            if let Some(net) = net.as_deref_mut() {
                socket.release(net);
            }
            let event = failed(
                socket.number,
                SocketError::NoNetwork,
                "the network went away",
            );
            effects.push(kernel.socket_event(now, event));
        }
        for broadcast in core::mem::take(&mut self.broadcasts) {
            if let Some(net) = net.as_deref_mut() {
                net.release_udp(broadcast.udp);
            }
            let event = SocketEvent::Done {
                broadcast: broadcast.number,
            };
            effects.push(kernel.socket_event(now, event));
        }
        effects
    }

    /// One step for every socket and broadcast.
    pub fn poll(&mut self, net: &mut Net, kernel: &mut Kernel, now: Instant) -> Vec<Effects> {
        if !self.any() {
            return Vec::new();
        }
        if net.generation() != self.generation {
            return self.lost(None, kernel, now);
        }
        let mut events = Vec::new();
        self.sockets.retain_mut(|socket| {
            let keep = socket.step(net, now, &mut events);
            if !keep {
                socket.release(net);
            }
            keep
        });
        self.broadcasts.retain(|broadcast| {
            let udp = net.udp(broadcast.udp);
            while let Ok((data, meta)) = udp.recv() {
                events.push(SocketEvent::Answer {
                    broadcast: broadcast.number,
                    from: format!("{}", meta.endpoint.addr),
                    data: data.to_vec(),
                });
            }
            if now.0 < broadcast.until {
                return true;
            }
            net.release_udp(broadcast.udp);
            events.push(SocketEvent::Done {
                broadcast: broadcast.number,
            });
            false
        });
        self.moved = !events.is_empty();
        events
            .into_iter()
            .map(|event| kernel.socket_event(now, event))
            .collect()
    }
}

fn failed(socket: u32, error: SocketError, reason: &str) -> SocketEvent {
    SocketEvent::Failed {
        socket,
        error,
        reason: String::from(reason),
    }
}

/// TLS asked for `socket`: not done here yet.
fn no_tls(socket: u32) -> SocketEvent {
    let reason = "secure connections are not supported here";
    failed(socket, SocketError::Unsupported, reason)
}

/// Starts connecting to `address`.
fn connect(net: &mut Net, address: IpAddress, port: u16) -> Result<Phase, &'static str> {
    match net.open((address, port), BUFFERS) {
        Ok(handle) => {
            // A connection the other side no longer answers ends, instead
            // of waiting for ever (e.g. while a book comes, with its end
            // lost on the way): asked every while it is quiet, it is given
            // up once nothing comes back.
            let socket = net.socket(handle);
            socket.set_keep_alive(Some(smoltcp::time::Duration::from_millis(KEEP_ALIVE_MS)));
            socket.set_timeout(Some(smoltcp::time::Duration::from_millis(GIVE_UP_MS)));
            Ok(Phase::Connecting(handle))
        }
        Err(net::Error::Memory) => Err("not enough memory for a connection"),
        Err(_) => Err("could not connect"),
    }
}

impl Socket {
    /// Lets its lookup or connection go.
    fn release(&self, net: &mut Net) {
        match self.phase {
            Phase::Resolving { query, .. } => net.forget(query),
            Phase::Connecting(handle) | Phase::Open(handle) => net.release(handle),
            Phase::Ended => {}
        }
    }

    /// One step; whether it stays open (else it failed, with the event).
    fn step(&mut self, net: &mut Net, now: Instant, events: &mut Vec<SocketEvent>) -> bool {
        let number = self.number;
        match self.phase {
            Phase::Resolving { query, port } => match net.resolved(query) {
                Some(Ok(address)) => match connect(net, address, port) {
                    Ok(phase) => {
                        self.phase = phase;
                        true
                    }
                    Err(reason) => {
                        self.phase = Phase::Ended;
                        events.push(failed(number, SocketError::Refused, reason));
                        false
                    }
                },
                Some(Err(_)) => {
                    self.phase = Phase::Ended;
                    events.push(failed(number, SocketError::Dns, "no such host"));
                    false
                }
                None if now.0 > self.until => {
                    events.push(failed(number, SocketError::Timeout, "no answer"));
                    false
                }
                None => true,
            },
            Phase::Connecting(handle) => {
                let socket = net.socket(handle);
                match socket.state() {
                    tcp::State::Established => {
                        self.phase = Phase::Open(handle);
                        events.push(SocketEvent::Connected { socket: number });
                        self.exchange(net, events)
                    }
                    tcp::State::SynSent | tcp::State::SynReceived if now.0 <= self.until => true,
                    _ => {
                        events.push(failed(number, SocketError::Refused, "could not connect"));
                        false
                    }
                }
            }
            Phase::Open(_) => self.exchange(net, events),
            Phase::Ended => false,
        }
    }

    /// Sends what waits and answers the reads asked for, as far as the
    /// connection allows; whether it stays open.
    fn exchange(&mut self, net: &mut Net, events: &mut Vec<SocketEvent>) -> bool {
        let Phase::Open(handle) = self.phase else {
            return true;
        };
        let number = self.number;
        let socket = net.socket(handle);
        while let Some(data) = self.sends.front() {
            if !socket.may_send() {
                log::info!(
                    "socket {number}: lost (no more sending), {:?}",
                    socket.state()
                );
                events.push(failed(number, SocketError::Lost, "connection lost"));
                return false;
            }
            match socket.send_slice(&data[self.sent..]) {
                Ok(n) => self.sent += n,
                Err(_) => {
                    log::info!(
                        "socket {number}: lost (sending failed), {:?}",
                        socket.state()
                    );
                    events.push(failed(number, SocketError::Lost, "connection lost"));
                    return false;
                }
            }
            if self.sent < data.len() {
                break;
            }
            self.sends.pop_front();
            self.sent = 0;
        }
        while let Some(&max) = self.reads.front() {
            // Large reads (a file) gathered into parts of up to [`PART`]
            // bytes, for fewer, larger writes to the card; what came is
            // passed on once the part is full or nothing more comes.
            let goal = (max as usize).min(PART);
            let mut came = false;
            while self.gathered.len() < goal && socket.can_recv() {
                if self.gathered.capacity() < goal
                    && self
                        .gathered
                        .try_reserve_exact(goal - self.gathered.len())
                        .is_err()
                {
                    break;
                }
                let gathered = &mut self.gathered;
                let got = socket.recv(|bytes| {
                    let n = bytes.len().min(goal - gathered.len());
                    gathered.extend_from_slice(&bytes[..n]);
                    (n, n)
                });
                match got {
                    Ok(n) if n > 0 => came = true,
                    _ => break,
                }
            }
            if self.gathered.len() >= goal || (!came && !self.gathered.is_empty()) {
                self.reads.pop_front();
                events.push(SocketEvent::Data {
                    socket: number,
                    data: core::mem::take(&mut self.gathered),
                });
            } else if !self.gathered.is_empty() || socket.can_recv() {
                break;
            } else if socket.state() == tcp::State::Closed {
                // Reset (or gone after both sides closed).
                log::info!("socket {number}: lost (closed), {:?}", socket.state());
                events.push(failed(number, SocketError::Lost, "connection lost"));
                return false;
            } else if !socket.may_recv() {
                // The other side closed: nothing more comes.
                self.reads.pop_front();
                events.push(SocketEvent::Data {
                    socket: number,
                    data: Vec::new(),
                });
            } else {
                break;
            }
        }
        true
    }
}
