//! Answering for the device's name on the local network (multicast DNS,
//! RFC 6762): a computer or a phone asking for `<name>.local` is told the
//! device's address, so that the transfer page opens at
//! `http://paperpaw.local` (the name the reader gave the device) and not
//! at an address read off the screen.
//!
//! Only what that needs: a query for the A record of one name, answered
//! with the address. No probing for a name taken, no announcements, no
//! other records (a query for the IPv6 address gets no answer, which
//! resolvers take as "none" after a moment), no service discovery. A name
//! is the device's label (letters, digits, hyphens), compared without
//! regard to case.

use alloc::string::String;
use alloc::vec::Vec;

/// The port multicast DNS uses.
pub const PORT: u16 = 5353;
/// The group queries are sent to, and answers.
pub const GROUP: [u8; 4] = [224, 0, 0, 251];

/// How long a multicast answer may be kept, in seconds.
const TTL: u32 = 120;
/// How long a legacy answer (to a resolver that is not multicast DNS) may
/// be kept.
const LEGACY_TTL: u32 = 10;
/// The record types and class asked.
const TYPE_A: u16 = 1;
const TYPE_ANY: u16 = 255;
const CLASS_IN: u16 = 1;
const CLASS_ANY: u16 = 255;
/// The top bit of the class: in a question, "answer me alone"; in an
/// answer, "this replaces what you kept".
const BIT: u16 = 0x8000;
/// The suffix of every name.
const LOCAL: &[u8] = b"local";

/// How an answer is to be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// To the group ([`GROUP`], [`PORT`]).
    Group(Vec<u8>),
    /// To the asker alone, at the address and port the query came from.
    Asker(Vec<u8>),
}

/// Whether `label` is one: 1 to 63 letters, digits or hyphens, not
/// beginning or ending with a hyphen.
fn is_label(label: &[u8]) -> bool {
    let ends = |b: &u8| *b == b'-';
    (1..=63).contains(&label.len())
        && label
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && !label.first().is_some_and(ends)
        && !label.last().is_some_and(ends)
}

/// The least time between two answers to the group, in milliseconds
/// (RFC 6762 §6: a record is multicast at most once a second).
const GROUP_GAP_MS: u64 = 1000;

/// The responder for one name at one address.
#[derive(Debug, Clone)]
pub struct Responder {
    label: Vec<u8>,
    address: [u8; 4],
    /// The plain name, once claimed (see [`Claim`]).
    alias: Option<Vec<u8>>,
    /// When the group was last answered for each name (ms), for the rate
    /// limit.
    last_group: [Option<u64>; 2],
}

/// The longest base a name keeps when the suffix is added (63 less the
/// hyphen and four digits).
const BASE_MOST: usize = 58;

/// A name that is the device's own on a network of devices alike: the
/// `base` (the name the user gave, as a label) and the last two bytes of
/// the hardware address in hexadecimal, e.g. `paperpaw-x4-1a2b`. Two
/// readers of one model then do not answer for one name, and a name
/// that is not Latin (it falls back to `paperpaw`) is still told apart.
pub fn label_with_suffix(base: &[u8], mac: [u8; 6]) -> Vec<u8> {
    let mut base = &base[..base.len().min(BASE_MOST)];
    while let [rest @ .., b'-'] = base {
        base = rest;
    }
    let mut label = base.to_vec();
    if !label.is_empty() {
        label.push(b'-');
    }
    for byte in &mac[4..] {
        for nibble in [byte >> 4, byte & 0x0F] {
            label.push(b"0123456789abcdef"[usize::from(nibble)]);
        }
    }
    label
}

/// The name to tell a person who opens a page of the device: the plain
/// name once claimed (`plain`, a label: `paperpaw`), else the one with the
/// suffix (`own`, as [`Responder::name`] gives it, with `.local`). `None`
/// for a name that is not text.
pub fn shown_name(own: &[u8], plain: Option<&[u8]>) -> Option<String> {
    match plain {
        Some(label) => {
            let label = core::str::from_utf8(label).ok()?;
            Some(alloc::format!("{label}.local"))
        }
        None => core::str::from_utf8(own).ok().map(String::from),
    }
}

/// Whether `source` is on the network `address` is on (`prefix` bits of
/// the mask): multicast DNS answers only those (RFC 6762 §11), so that a
/// query with a forged source from far away gets no reply sent there.
pub fn on_link(source: [u8; 4], address: [u8; 4], prefix: u8) -> bool {
    let prefix = u32::from(prefix.min(32));
    if prefix == 0 {
        return false;
    }
    let mask = u32::MAX << (32 - prefix);
    u32::from_be_bytes(source) & mask == u32::from_be_bytes(address) & mask
}

impl Responder {
    /// For `label` (the device's name: 1 to 63 letters, digits or
    /// hyphens, not beginning or ending with a hyphen); `None` for any
    /// other.
    pub fn new(label: &[u8], address: [u8; 4]) -> Option<Self> {
        is_label(label).then(|| Self {
            label: label.to_vec(),
            address,
            alias: None,
            last_group: [None; 2],
        })
    }

    /// The name this answers for, with its suffix: `paperpaw.local`.
    pub fn name(&self) -> Vec<u8> {
        let mut name = self.label.clone();
        name.push(b'.');
        name.extend_from_slice(LOCAL);
        name
    }

    /// Also answers for the plain name `alias` (claimed, see [`Claim`]), or
    /// no longer (`None`, or a label that is none).
    pub fn set_alias(&mut self, alias: Option<&[u8]>) {
        self.alias = alias.filter(|a| is_label(a)).map(<[u8]>::to_vec);
        self.last_group[1] = None;
    }

    /// Another address (the network was joined anew).
    pub fn set_address(&mut self, address: [u8; 4]) {
        self.address = address;
    }

    /// The answer to `query`, a datagram that came from port `from` at
    /// `now_ms`, if it asks for the address of one of the names this
    /// answers for; `None` for anything else (answers, other names and
    /// kinds, what is not DNS), and for a second answer to the group for
    /// the same name within a second of the last.
    pub fn answer(&mut self, query: &[u8], from: u16, now_ms: u64) -> Option<Reply> {
        let header = query.get(..12)?;
        let flags = u16::from_be_bytes([header[2], header[3]]);
        // A query (not an answer), the standard kind.
        if flags & 0xF800 != 0 {
            return None;
        }
        let id = [header[0], header[1]];
        let count = usize::from(u16::from_be_bytes([header[4], header[5]]));
        let mut at = 12;
        let mut asked = None;
        for _ in 0..count.min(16) {
            let (which, next) = read_name(query, at, &self.accepted())?;
            let kind = u16::from_be_bytes([*query.get(next)?, *query.get(next + 1)?]);
            let class = u16::from_be_bytes([*query.get(next + 2)?, *query.get(next + 3)?]);
            at = next + 4;
            let kind_fits = kind == TYPE_A || kind == TYPE_ANY;
            let class_fits = class & !BIT == CLASS_IN || class & !BIT == CLASS_ANY;
            if let Some(which) = which
                && kind_fits
                && class_fits
                && asked.is_none()
            {
                asked = Some((kind, class & BIT != 0, which));
            }
        }
        let (kind, alone, which) = asked?;
        let label = self.label_of(which).to_vec();
        if from == PORT {
            // Multicast DNS: no question repeated, the identifier 0.
            let mut out = header_of([0, 0], 0);
            record(&mut out, &label, self.address, CLASS_IN | BIT, TTL);
            if alone {
                return Some(Reply::Asker(out));
            }
            if self.last_group[which].is_some_and(|last| now_ms.saturating_sub(last) < GROUP_GAP_MS)
            {
                return None;
            }
            self.last_group[which] = Some(now_ms);
            Some(Reply::Group(out))
        } else {
            // A resolver that is not multicast DNS: its identifier and
            // question back, to it alone, kept briefly.
            let mut out = header_of(id, 1);
            name_into(&mut out, &label);
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&CLASS_IN.to_be_bytes());
            record(&mut out, &label, self.address, CLASS_IN, LEGACY_TTL);
            Some(Reply::Asker(out))
        }
    }

    /// The labels asked for are taken as: the device's own, then the
    /// plain one if it is claimed.
    fn accepted(&self) -> Vec<&[u8]> {
        let mut labels: Vec<&[u8]> = alloc::vec![&self.label];
        if let Some(alias) = &self.alias {
            labels.push(alias);
        }
        labels
    }

    fn label_of(&self, which: usize) -> &[u8] {
        match (which, &self.alias) {
            (1, Some(alias)) => alias,
            _ => &self.label,
        }
    }
}

/// Reads the name at `at`: which of the `accepted` first labels it begins
/// with (`None`: none, or not a name of two labels ending in `local`), and
/// where the type that follows it begins. `None` where the message is cut
/// or odd.
fn read_name(msg: &[u8], at: usize, accepted: &[&[u8]]) -> Option<(Option<usize>, usize)> {
    let mut pos = at;
    // Where the first pointer was, to return after it.
    let mut after = None;
    let mut hops = 0;
    let mut labels = 0usize;
    let mut found = None;
    let mut matched = true;
    loop {
        let len = *msg.get(pos)?;
        match len & 0xC0 {
            0x00 if len == 0 => {
                let next = after.unwrap_or(pos + 1);
                let which = if matched && labels == 2 { found } else { None };
                return Some((which, next));
            }
            0x00 => {
                let label = msg.get(pos + 1..pos + 1 + usize::from(len))?;
                match labels {
                    0 => {
                        found = accepted.iter().position(|a| label.eq_ignore_ascii_case(a));
                        matched = found.is_some();
                    }
                    1 => {
                        if !label.eq_ignore_ascii_case(LOCAL) {
                            matched = false;
                        }
                    }
                    _ => matched = false,
                }
                labels += 1;
                pos += 1 + usize::from(len);
            }
            0xC0 => {
                let low = *msg.get(pos + 1)?;
                if after.is_none() {
                    after = Some(pos + 2);
                }
                hops += 1;
                if hops > 4 {
                    return None;
                }
                pos = usize::from(len & 0x3F) << 8 | usize::from(low);
            }
            _ => return None,
        }
    }
}

fn header_of(id: [u8; 2], questions: u16) -> Vec<u8> {
    header_with(id, 0x8400, questions, 1, 0)
}

/// A header with these flags and counts (questions, answers, authority).
fn header_with(id: [u8; 2], flags: u16, questions: u16, answers: u16, authority: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(&id);
    out.extend_from_slice(&flags.to_be_bytes());
    out.extend_from_slice(&questions.to_be_bytes());
    out.extend_from_slice(&answers.to_be_bytes());
    out.extend_from_slice(&authority.to_be_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

fn name_into(out: &mut Vec<u8>, label: &[u8]) {
    out.push(label.len() as u8);
    out.extend_from_slice(label);
    out.push(LOCAL.len() as u8);
    out.extend_from_slice(LOCAL);
    out.push(0);
}

fn record(out: &mut Vec<u8>, label: &[u8], address: [u8; 4], class: u16, ttl: u32) {
    name_into(out, label);
    out.extend_from_slice(&TYPE_A.to_be_bytes());
    out.extend_from_slice(&class.to_be_bytes());
    out.extend_from_slice(&ttl.to_be_bytes());
    out.extend_from_slice(&4u16.to_be_bytes());
    out.extend_from_slice(&address);
}

/// Where the claim to the plain name stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Asking the network whether anyone has the name.
    Probing,
    /// Nobody answered: the name is this device's.
    Owned,
    /// Another host has the name, or is asking for it too: not claimed.
    Conflict,
}

/// Probes sent before the name is taken as free (RFC 6762 §8.1).
const PROBES: u8 = 3;
/// Milliseconds between probes, and after the last.
const PROBE_GAP_MS: u64 = 250;

#[derive(Debug, Clone, Copy)]
enum ClaimState {
    /// `sent` probes sent; the next goes (or, after the third, the name is
    /// taken as free) at `next_at`.
    Probing {
        sent: u8,
        next_at: u64,
    },
    Owned,
    Conflict,
}

/// A claim to the plain name (`<device name>.local`) beside the device's
/// own, suffixed one: the name is probed for as RFC 6762 §8 says (three
/// questions 250 ms apart, with the address proposed), and is the device's
/// if nobody answers or asks for it too. A host that answers with another
/// address later puts it to the test again; one that answers a probe has
/// the name: no claim, and the suffixed name alone is answered for. It
/// takes the time it is given and sends nothing itself: the shell asks
/// [`Claim::poll`] what to send and feeds it what other hosts send
/// ([`Claim::heard`]).
#[derive(Debug, Clone)]
pub struct Claim {
    label: Vec<u8>,
    address: [u8; 4],
    state: ClaimState,
}

impl Claim {
    /// A claim to `label` (see [`Responder::new`] for what it may be) for
    /// `address`, first probing `first_delay_ms` after `now_ms`
    /// (0 to 250: the hosts that start together do not probe together).
    pub fn new(label: &[u8], address: [u8; 4], now_ms: u64, first_delay_ms: u64) -> Option<Self> {
        is_label(label).then(|| Self {
            label: label.to_vec(),
            address,
            state: ClaimState::Probing {
                sent: 0,
                next_at: now_ms + first_delay_ms.min(PROBE_GAP_MS),
            },
        })
    }

    pub fn label(&self) -> &[u8] {
        &self.label
    }

    pub fn standing(&self) -> Standing {
        match self.state {
            ClaimState::Probing { .. } => Standing::Probing,
            ClaimState::Owned => Standing::Owned,
            ClaimState::Conflict => Standing::Conflict,
        }
    }

    /// The probe to send to the group ([`GROUP`], [`PORT`]) at `now_ms`,
    /// if one is due; the claim is the device's once the last has been
    /// out for 250 ms.
    pub fn poll(&mut self, now_ms: u64) -> Option<Vec<u8>> {
        let ClaimState::Probing { sent, next_at } = self.state else {
            return None;
        };
        if now_ms < next_at {
            return None;
        }
        if sent >= PROBES {
            self.state = ClaimState::Owned;
            return None;
        }
        self.state = ClaimState::Probing {
            sent: sent + 1,
            next_at: now_ms + PROBE_GAP_MS,
        };
        Some(self.probe())
    }

    /// A datagram another host sent to the group at `now_ms` (not one
    /// this device sent). An answer for the name with another address, or
    /// another host's probe for it while this one is probing, is a
    /// conflict: while probing, the name is given up; once the device's,
    /// it is probed for again.
    pub fn heard(&mut self, msg: &[u8], now_ms: u64) {
        if !self.conflicts_with(msg) {
            return;
        }
        self.state = match self.state {
            ClaimState::Owned => ClaimState::Probing {
                sent: 0,
                next_at: now_ms + PROBE_GAP_MS,
            },
            ClaimState::Probing { .. } | ClaimState::Conflict => ClaimState::Conflict,
        };
    }

    /// The probe: a question for any record of the name, asked to be
    /// answered to the asker alone, with the address proposed as the
    /// record in the authority section.
    fn probe(&self) -> Vec<u8> {
        let mut out = header_with([0, 0], 0, 1, 0, 1);
        name_into(&mut out, &self.label);
        out.extend_from_slice(&TYPE_ANY.to_be_bytes());
        out.extend_from_slice(&(CLASS_IN | BIT).to_be_bytes());
        record(&mut out, &self.label, self.address, CLASS_IN, TTL);
        out
    }

    fn conflicts_with(&self, msg: &[u8]) -> bool {
        self.read_conflict(msg).unwrap_or(false)
    }

    /// `None` where the message is cut or odd.
    fn read_conflict(&self, msg: &[u8]) -> Option<bool> {
        let header = msg.get(..12)?;
        let flags = u16::from_be_bytes([header[2], header[3]]);
        let count = |at: usize| usize::from(u16::from_be_bytes([header[at], header[at + 1]]));
        let (questions, answers, authority) = (count(4), count(6), count(8));
        let ours: [&[u8]; 1] = [&self.label];
        let mut at = 12;
        let mut asked = false;
        for _ in 0..questions.min(16) {
            let (which, next) = read_name(msg, at, &ours)?;
            asked |= which.is_some();
            at = next + 4;
        }
        if flags & 0x8000 == 0 {
            // A question for the name that proposes a record (a probe)
            // while this one probes too: the one that is later in line
            // would win, here both give way.
            let probing = matches!(self.state, ClaimState::Probing { .. });
            return Some(probing && asked && authority > 0);
        }
        for _ in 0..answers.min(16) + authority.min(16) {
            let (which, next) = read_name(msg, at, &ours)?;
            let kind = u16::from_be_bytes([*msg.get(next)?, *msg.get(next + 1)?]);
            let len = usize::from(u16::from_be_bytes([
                *msg.get(next + 8)?,
                *msg.get(next + 9)?,
            ]));
            let data = msg.get(next + 10..next + 10 + len)?;
            if which.is_some() && (kind == TYPE_A) && data != self.address {
                return Some(true);
            }
            at = next + 10 + len;
        }
        Some(false)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_name_shown_is_the_plain_one_once_claimed() {
        let own = b"paperpaw-x4-1a2b.local";
        assert_eq!(
            shown_name(own, None).as_deref(),
            Some("paperpaw-x4-1a2b.local")
        );
        assert_eq!(
            shown_name(own, Some(b"paperpaw")).as_deref(),
            Some("paperpaw.local")
        );
        assert_eq!(shown_name(&[0xff, 0xfe], None), None);
        assert_eq!(shown_name(own, Some(&[0xff])), None);
    }

    use super::*;

    fn name(label: &str) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
        out.extend_from_slice(b"\x05local\x00");
        out
    }

    fn query(id: u16, questions: &[(&str, u16, u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&(questions.len() as u16).to_be_bytes());
        out.extend_from_slice(&[0; 6]);
        for (label, kind, class) in questions {
            out.extend_from_slice(&name(label));
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&class.to_be_bytes());
        }
        out
    }

    fn responder() -> Responder {
        Responder::new(b"PaperPaw", [192, 168, 1, 20]).unwrap()
    }

    #[test]
    fn a_multicast_query_for_the_name_is_answered_to_the_group() {
        let reply = responder().answer(&query(0, &[("paperpaw", 1, 1)]), PORT, 0);
        let Some(Reply::Group(out)) = reply else {
            panic!("{reply:?}");
        };
        // The identifier 0, a response, no question, one answer.
        assert_eq!(&out[..12], &[0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        let mut record = name("PaperPaw");
        record.extend_from_slice(&[0, 1, 0x80, 1]);
        record.extend_from_slice(&TTL.to_be_bytes());
        record.extend_from_slice(&[0, 4, 192, 168, 1, 20]);
        assert_eq!(&out[12..], record.as_slice());
    }

    #[test]
    fn the_case_of_the_name_does_not_matter() {
        for asked in ["PAPERPAW", "PaperPaw", "paperpaw"] {
            assert!(
                responder()
                    .answer(&query(0, &[(asked, 1, 1)]), PORT, 0)
                    .is_some(),
                "{asked}"
            );
        }
    }

    #[test]
    fn a_question_for_an_answer_alone_is_answered_to_the_asker() {
        let reply = responder().answer(&query(0, &[("paperpaw", 1, 0x8001)]), PORT, 0);
        assert!(matches!(reply, Some(Reply::Asker(_))));
    }

    #[test]
    fn a_resolver_that_is_not_multicast_dns_gets_its_identifier_and_question_back() {
        let q = query(0x1234, &[("PaperPaw", 255, 0x8001)]);
        let Some(Reply::Asker(out)) = responder().answer(&q, 40000, 0) else {
            panic!("no answer");
        };
        assert_eq!(&out[..2], &[0x12, 0x34]);
        // The question is repeated (its kind, class IN), the answer is
        // kept ten seconds.
        assert_eq!(&out[4..6], &[0, 1]);
        let question = &q[12..];
        assert_eq!(
            &out[12..12 + question.len() - 2],
            &question[..question.len() - 2]
        );
        assert_eq!(&out[12 + question.len() - 2..12 + question.len()], &[0, 1]);
        assert_eq!(
            &out[out.len() - 10..out.len() - 6],
            &LEGACY_TTL.to_be_bytes()
        );
        assert_eq!(&out[out.len() - 4..], &[192, 168, 1, 20]);
    }

    #[test]
    fn other_names_kinds_and_messages_get_no_answer() {
        let mut r = responder();
        assert!(r.answer(&query(0, &[("other", 1, 1)]), PORT, 0).is_none());
        // AAAA, TXT, PTR.
        for kind in [28, 16, 12] {
            assert!(
                r.answer(&query(0, &[("paperpaw", kind, 1)]), PORT, 0)
                    .is_none()
            );
        }
        // A name with a third label.
        let mut longer = query(0, &[("paperpaw", 1, 1)]);
        longer.splice(12 + 9 + 6..12 + 9 + 6, [1, b'x']);
        assert!(r.answer(&longer, PORT, 0).is_none());
        // An answer is no query.
        let mut response = query(0, &[("paperpaw", 1, 1)]);
        response[2] = 0x84;
        assert!(r.answer(&response, PORT, 0).is_none());
        assert!(r.answer(&[], PORT, 0).is_none());
    }

    #[test]
    fn the_name_among_several_questions_and_after_a_pointer_is_found() {
        let mut r = responder();
        // Questions for another name, then ours; and ours as a pointer to
        // the name of the question before it.
        assert!(
            r.answer(&query(0, &[("other", 1, 1), ("paperpaw", 1, 1)]), PORT, 0)
                .is_some()
        );
        let mut q = query(0, &[("paperpaw", 28, 1)]);
        q[5] = 2;
        q.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
        assert!(r.answer(&q, PORT, 5_000).is_some());
    }

    #[test]
    fn a_message_cut_anywhere_or_with_a_loop_does_not_panic() {
        let mut r = responder();
        let q = query(7, &[("paperpaw", 255, 0x8001), ("x", 1, 1)]);
        for len in 0..q.len() {
            let _ = r.answer(&q[..len], PORT, 0);
            let _ = r.answer(&q[..len], 5000, 0);
        }
        // A pointer to itself.
        let mut looped = query(0, &[("paperpaw", 1, 1)]);
        looped[12] = 0xC0;
        looped[13] = 12;
        assert!(r.answer(&looped, PORT, 0).is_none());
        // A reserved label type.
        looped[12] = 0x80;
        assert!(r.answer(&looped, PORT, 0).is_none());
    }

    #[test]
    fn only_names_that_are_labels_are_answered_for() {
        assert!(Responder::new(b"", [0; 4]).is_none());
        assert!(Responder::new(b"-a", [0; 4]).is_none());
        assert!(Responder::new(b"a-", [0; 4]).is_none());
        assert!(Responder::new(b"a b", [0; 4]).is_none());
        assert!(Responder::new(&[b'a'; 64], [0; 4]).is_none());
        assert!(Responder::new(&[b'a'; 63], [0; 4]).is_some());
        assert_eq!(responder().name(), b"PaperPaw.local");
    }

    #[test]
    fn a_new_address_is_the_one_answered() {
        let mut r = responder();
        r.set_address([10, 0, 0, 7]);
        let Some(Reply::Group(out)) = r.answer(&query(0, &[("paperpaw", 1, 1)]), PORT, 0) else {
            panic!("no answer");
        };
        assert_eq!(&out[out.len() - 4..], &[10, 0, 0, 7]);
    }

    #[test]
    fn the_group_is_answered_once_a_second_but_an_asker_alone_every_time() {
        let mut r = responder();
        let q = query(0, &[("paperpaw", 1, 1)]);
        assert!(matches!(r.answer(&q, PORT, 5_000), Some(Reply::Group(_))));
        assert!(r.answer(&q, PORT, 5_999).is_none());
        assert!(matches!(r.answer(&q, PORT, 6_000), Some(Reply::Group(_))));
        // "Answer me alone" and a legacy resolver are not limited.
        let alone = query(0, &[("paperpaw", 1, 0x8001)]);
        assert!(matches!(
            r.answer(&alone, PORT, 6_001),
            Some(Reply::Asker(_))
        ));
        assert!(matches!(r.answer(&q, 40000, 6_002), Some(Reply::Asker(_))));
    }

    #[test]
    fn the_label_carries_the_end_of_the_hardware_address() {
        let mac = [0x34, 0x85, 0x18, 0x00, 0x1A, 0x2B];
        assert_eq!(label_with_suffix(b"paperpaw-x4", mac), b"paperpaw-x4-1a2b");
        assert_eq!(label_with_suffix(b"paperpaw", mac), b"paperpaw-1a2b");
        // Nothing left of a name: the digits alone are still a label.
        assert_eq!(label_with_suffix(b"", mac), b"1a2b");
        // A long name is cut so that the label stays within 63; no hyphen
        // is left before the digits' hyphen.
        let long = [b'a'; 80];
        let label = label_with_suffix(&long, mac);
        assert_eq!(label.len(), 63);
        assert!(Responder::new(&label, [0; 4]).is_some());
        let hyphen = label_with_suffix(&[&[b'a'; 57][..], b"-b"].concat(), mac);
        assert!(!hyphen.windows(2).any(|w| w == b"--"));
        assert!(Responder::new(&hyphen, [0; 4]).is_some());
    }

    #[test]
    fn only_a_source_on_the_same_network_is_answered() {
        let me = [192, 168, 1, 20];
        assert!(on_link([192, 168, 1, 99], me, 24));
        assert!(!on_link([192, 168, 2, 99], me, 24));
        assert!(!on_link([8, 8, 8, 8], me, 24));
        assert!(on_link([192, 168, 2, 99], me, 16));
        // The own network of the hotspot, and the host itself.
        assert!(on_link([10, 0, 0, 5], [10, 0, 0, 1], 8));
        // No mask: nothing is on the link; a mask of 32 only the host.
        assert!(!on_link(me, me, 0));
        assert!(on_link(me, me, 32));
        assert!(!on_link([192, 168, 1, 21], me, 32));
    }

    /// An answer for `label` with `address`, as another host sends it.
    fn answer_of(label: &str, address: [u8; 4]) -> Vec<u8> {
        let mut out = header_of([0, 0], 0);
        record(&mut out, label.as_bytes(), address, CLASS_IN | BIT, TTL);
        out
    }

    const ME: [u8; 4] = [192, 168, 1, 20];
    const OTHER: [u8; 4] = [192, 168, 1, 77];

    fn claim_at(now: u64, delay: u64) -> Claim {
        Claim::new(b"paperpaw", ME, now, delay).unwrap()
    }

    /// Runs the clock from `from` to `to` in steps of 10 ms, collecting
    /// the times of the probes.
    fn run(claim: &mut Claim, from: u64, to: u64) -> Vec<u64> {
        (from..=to)
            .step_by(10)
            .filter(|&t| claim.poll(t).is_some())
            .collect()
    }

    #[test]
    fn a_free_name_is_probed_three_times_and_then_the_devices() {
        let mut claim = claim_at(1_000, 100);
        assert_eq!(claim.standing(), Standing::Probing);
        // The first probe after the delay, then 250 ms apart.
        assert_eq!(run(&mut claim, 1_000, 1_740), [1_100, 1_350, 1_600]);
        // Still probing until 250 ms after the third.
        assert_eq!(claim.standing(), Standing::Probing);
        assert!(claim.poll(1_850).is_none());
        assert_eq!(claim.standing(), Standing::Owned);
        // Nothing more is sent.
        assert!(run(&mut claim, 1_860, 5_000).is_empty());
    }

    #[test]
    fn a_probe_is_a_question_for_the_name_with_the_address_proposed() {
        let mut claim = claim_at(0, 0);
        let probe = claim.poll(0).unwrap();
        // One question, no answer, one record in the authority section.
        assert_eq!(&probe[4..12], &[0, 1, 0, 0, 0, 1, 0, 0]);
        let at = 12 + 1 + 8 + 1 + 5 + 1;
        // Any record, wanting an answer to the asker alone.
        assert_eq!(&probe[at..at + 4], &[0, 255, 0x80, 1]);
        assert_eq!(&probe[probe.len() - 4..], &ME);
        // The probe is no answer to anyone: the responder says nothing.
        let mut responder = Responder::new(b"paperpaw-1a2b", ME).unwrap();
        responder.set_alias(Some(b"other"));
        assert!(responder.answer(&probe, PORT, 0).is_none());
    }

    #[test]
    fn an_answer_for_the_name_while_probing_is_a_conflict_for_good() {
        let mut claim = claim_at(0, 0);
        assert!(claim.poll(0).is_some());
        claim.heard(&answer_of("PaperPaw", OTHER), 100);
        assert_eq!(claim.standing(), Standing::Conflict);
        // No more probes, never the device's; a later answer changes nothing.
        assert!(run(&mut claim, 100, 5_000).is_empty());
        claim.heard(&answer_of("paperpaw", OTHER), 5_100);
        assert_eq!(claim.standing(), Standing::Conflict);
    }

    #[test]
    fn another_host_probing_for_the_name_too_is_a_conflict() {
        let mut claim = claim_at(0, 0);
        let mut other = Claim::new(b"paperpaw", OTHER, 0, 0).unwrap();
        let probe = other.poll(0).unwrap();
        claim.heard(&probe, 10);
        assert_eq!(claim.standing(), Standing::Conflict);
        // A device that owns the name answers that probe instead (the
        // responder does), and does not give it up for it.
        let mut owner = claim_at(0, 0);
        run(&mut owner, 0, 1_000);
        assert_eq!(owner.standing(), Standing::Owned);
        owner.heard(&probe, 1_010);
        assert_eq!(owner.standing(), Standing::Owned);
    }

    #[test]
    fn a_late_conflict_probes_again_and_the_name_is_lost_only_if_answered() {
        // The name is the device's; a host answers for it with another
        // address: probed for again.
        let mut claim = claim_at(0, 0);
        run(&mut claim, 0, 1_000);
        assert_eq!(claim.standing(), Standing::Owned);
        claim.heard(&answer_of("paperpaw", OTHER), 2_000);
        assert_eq!(claim.standing(), Standing::Probing);
        // Nobody answers the probes (it was a stray answer): the device's.
        assert_eq!(run(&mut claim, 2_000, 3_000), [2_250, 2_500, 2_750]);
        assert_eq!(claim.standing(), Standing::Owned);
        // Again, and now the other host answers the probe: given up.
        claim.heard(&answer_of("paperpaw", OTHER), 4_000);
        assert_eq!(claim.standing(), Standing::Probing);
        assert!(claim.poll(4_250).is_some());
        claim.heard(&answer_of("paperpaw", OTHER), 4_300);
        assert_eq!(claim.standing(), Standing::Conflict);
    }

    #[test]
    fn answers_that_are_no_conflict_change_nothing() {
        let mut claim = claim_at(0, 0);
        claim.poll(0);
        // The same address (an answer of this device, looped back), another
        // name, another kind of record, and not DNS at all.
        claim.heard(&answer_of("paperpaw", ME), 10);
        claim.heard(&answer_of("someone-else", OTHER), 10);
        let mut aaaa = answer_of("paperpaw", OTHER);
        let kind_at = aaaa.len() - 4 - 2 - 4 - 2 - 2;
        aaaa[kind_at..kind_at + 2].copy_from_slice(&28u16.to_be_bytes());
        claim.heard(&aaaa, 10);
        claim.heard(&[], 10);
        claim.heard(&[0xFF; 40], 10);
        // An ordinary question for the name is no probe.
        claim.heard(&query(0, &[("paperpaw", 1, 1)]), 10);
        assert_eq!(claim.standing(), Standing::Probing);
        // Cut short at every length: no panic, no conflict.
        let full = answer_of("paperpaw", OTHER);
        for len in 0..full.len() {
            claim.heard(&full[..len], 20);
            assert_eq!(claim.standing(), Standing::Probing, "cut at {len}");
        }
    }

    #[test]
    fn the_plain_name_is_answered_beside_the_own_one_once_claimed() {
        let mut r = Responder::new(b"paperpaw-x4-1a2b", ME).unwrap();
        let plain = query(0, &[("paperpaw-x4", 1, 1)]);
        let own = query(0, &[("paperpaw-x4-1a2b", 1, 1)]);
        assert!(r.answer(&plain, PORT, 0).is_none());
        assert!(matches!(r.answer(&own, PORT, 0), Some(Reply::Group(_))));
        r.set_alias(Some(b"paperpaw-x4"));
        let Some(Reply::Group(out)) = r.answer(&plain, PORT, 0) else {
            panic!("the plain name is answered");
        };
        // The record carries the name asked.
        assert_eq!(&out[12 + 1..12 + 1 + 11], b"paperpaw-x4");
        // Each name has its own rate limit.
        assert!(r.answer(&plain, PORT, 500).is_none());
        assert!(r.answer(&own, PORT, 5_000).is_some());
        // Given up again.
        r.set_alias(None);
        assert!(r.answer(&plain, PORT, 9_000).is_none());
        // A label that is none is no alias.
        r.set_alias(Some(b"-bad-"));
        assert!(
            r.answer(&query(0, &[("-bad-", 1, 1)]), PORT, 9_000)
                .is_none()
        );
    }
}
