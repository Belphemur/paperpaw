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

/// The responder for one name at one address.
#[derive(Debug, Clone)]
pub struct Responder {
    label: Vec<u8>,
    address: [u8; 4],
}

impl Responder {
    /// For `label` (the device's name: 1 to 63 letters, digits or
    /// hyphens, not beginning or ending with a hyphen); `None` for any
    /// other.
    pub fn new(label: &[u8], address: [u8; 4]) -> Option<Self> {
        let ends = |b: &u8| *b == b'-';
        let valid = (1..=63).contains(&label.len())
            && label
                .iter()
                .all(|b| b.is_ascii_alphanumeric() || *b == b'-')
            && !label.first().is_some_and(ends)
            && !label.last().is_some_and(ends);
        valid.then(|| Self {
            label: label.to_vec(),
            address,
        })
    }

    /// The name this answers for, with its suffix: `paperpaw.local`.
    pub fn name(&self) -> Vec<u8> {
        let mut name = self.label.clone();
        name.push(b'.');
        name.extend_from_slice(LOCAL);
        name
    }

    /// Another address (the network was joined anew).
    pub fn set_address(&mut self, address: [u8; 4]) {
        self.address = address;
    }

    /// The answer to `query`, a datagram that came from port `from`, if
    /// it asks for this name's address; `None` for anything else
    /// (answers, other names and kinds, what is not DNS).
    pub fn answer(&self, query: &[u8], from: u16) -> Option<Reply> {
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
            let (matches, next) = self.read_question(query, at)?;
            let kind = u16::from_be_bytes([*query.get(next)?, *query.get(next + 1)?]);
            let class = u16::from_be_bytes([*query.get(next + 2)?, *query.get(next + 3)?]);
            at = next + 4;
            let kind_fits = kind == TYPE_A || kind == TYPE_ANY;
            let class_fits = class & !BIT == CLASS_IN || class & !BIT == CLASS_ANY;
            if matches && kind_fits && class_fits && asked.is_none() {
                asked = Some((kind, class & BIT != 0));
            }
        }
        let (kind, alone) = asked?;
        if from == PORT {
            // Multicast DNS: no question repeated, the identifier 0.
            let mut out = self.header([0, 0], 0);
            self.record(&mut out, CLASS_IN | BIT, TTL);
            Some(if alone {
                Reply::Asker(out)
            } else {
                Reply::Group(out)
            })
        } else {
            // A resolver that is not multicast DNS: its identifier and
            // question back, to it alone, kept briefly.
            let mut out = self.header(id, 1);
            self.name_into(&mut out);
            out.extend_from_slice(&kind.to_be_bytes());
            out.extend_from_slice(&CLASS_IN.to_be_bytes());
            self.record(&mut out, CLASS_IN, LEGACY_TTL);
            Some(Reply::Asker(out))
        }
    }

    /// Reads the question's name at `at`: whether it is this name, and
    /// where its type begins. `None` where the message is cut or odd.
    fn read_question(&self, msg: &[u8], at: usize) -> Option<(bool, usize)> {
        let mut pos = at;
        // Where the first pointer was, to return after it.
        let mut after = None;
        let mut hops = 0;
        let mut labels = 0usize;
        let mut matched = true;
        loop {
            let len = *msg.get(pos)?;
            match len & 0xC0 {
                0x00 if len == 0 => {
                    let next = after.unwrap_or(pos + 1);
                    return Some((matched && labels == 2, next));
                }
                0x00 => {
                    let label = msg.get(pos + 1..pos + 1 + usize::from(len))?;
                    let want: &[u8] = match labels {
                        0 => &self.label,
                        1 => LOCAL,
                        _ => &[],
                    };
                    if !label.eq_ignore_ascii_case(want) {
                        matched = false;
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

    fn header(&self, id: [u8; 2], questions: u16) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&id);
        // A response, authoritative.
        out.extend_from_slice(&0x8400u16.to_be_bytes());
        out.extend_from_slice(&questions.to_be_bytes());
        // One answer, no more records.
        out.extend_from_slice(&[0, 1, 0, 0, 0, 0]);
        out
    }

    fn name_into(&self, out: &mut Vec<u8>) {
        out.push(self.label.len() as u8);
        out.extend_from_slice(&self.label);
        out.push(LOCAL.len() as u8);
        out.extend_from_slice(LOCAL);
        out.push(0);
    }

    fn record(&self, out: &mut Vec<u8>, class: u16, ttl: u32) {
        self.name_into(out);
        out.extend_from_slice(&TYPE_A.to_be_bytes());
        out.extend_from_slice(&class.to_be_bytes());
        out.extend_from_slice(&ttl.to_be_bytes());
        out.extend_from_slice(&4u16.to_be_bytes());
        out.extend_from_slice(&self.address);
    }
}

#[cfg(test)]
mod tests {
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
        let reply = responder().answer(&query(0, &[("paperpaw", 1, 1)]), PORT);
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
                    .answer(&query(0, &[(asked, 1, 1)]), PORT)
                    .is_some(),
                "{asked}"
            );
        }
    }

    #[test]
    fn a_question_for_an_answer_alone_is_answered_to_the_asker() {
        let reply = responder().answer(&query(0, &[("paperpaw", 1, 0x8001)]), PORT);
        assert!(matches!(reply, Some(Reply::Asker(_))));
    }

    #[test]
    fn a_resolver_that_is_not_multicast_dns_gets_its_identifier_and_question_back() {
        let q = query(0x1234, &[("PaperPaw", 255, 0x8001)]);
        let Some(Reply::Asker(out)) = responder().answer(&q, 40000) else {
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
        let r = responder();
        assert!(r.answer(&query(0, &[("other", 1, 1)]), PORT).is_none());
        // AAAA, TXT, PTR.
        for kind in [28, 16, 12] {
            assert!(
                r.answer(&query(0, &[("paperpaw", kind, 1)]), PORT)
                    .is_none()
            );
        }
        // A name with a third label.
        let mut longer = query(0, &[("paperpaw", 1, 1)]);
        longer.splice(12 + 9 + 6..12 + 9 + 6, [1, b'x']);
        assert!(r.answer(&longer, PORT).is_none());
        // An answer is no query.
        let mut response = query(0, &[("paperpaw", 1, 1)]);
        response[2] = 0x84;
        assert!(r.answer(&response, PORT).is_none());
        assert!(r.answer(&[], PORT).is_none());
    }

    #[test]
    fn the_name_among_several_questions_and_after_a_pointer_is_found() {
        let r = responder();
        // Questions for another name, then ours; and ours as a pointer to
        // the name of the question before it.
        assert!(
            r.answer(&query(0, &[("other", 1, 1), ("paperpaw", 1, 1)]), PORT)
                .is_some()
        );
        let mut q = query(0, &[("paperpaw", 28, 1)]);
        q[5] = 2;
        q.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1]);
        assert!(r.answer(&q, PORT).is_some());
    }

    #[test]
    fn a_message_cut_anywhere_or_with_a_loop_does_not_panic() {
        let r = responder();
        let q = query(7, &[("paperpaw", 255, 0x8001), ("x", 1, 1)]);
        for len in 0..q.len() {
            let _ = r.answer(&q[..len], PORT);
            let _ = r.answer(&q[..len], 5000);
        }
        // A pointer to itself.
        let mut looped = query(0, &[("paperpaw", 1, 1)]);
        looped[12] = 0xC0;
        looped[13] = 12;
        assert!(r.answer(&looped, PORT).is_none());
        // A reserved label type.
        looped[12] = 0x80;
        assert!(r.answer(&looped, PORT).is_none());
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
        let Some(Reply::Group(out)) = r.answer(&query(0, &[("paperpaw", 1, 1)]), PORT) else {
            panic!("no answer");
        };
        assert_eq!(&out[out.len() - 4..], &[10, 0, 0, 7]);
    }
}
