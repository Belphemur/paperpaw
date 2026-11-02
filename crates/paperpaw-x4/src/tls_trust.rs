//! Which certificates a TLS socket trusts: what becomes of the findings of
//! mbedTLS's verify callback (its flags, a certificate at a time, from the
//! top of the chain down to the server's own at depth 0), for the
//! authorities PaperPaw ships and for certificates a plugin pinned
//! ([`Trust`]); and what an unfinished handshake makes of the socket
//! ([`Fault`] to [`Outcome`]).
//!
//! The flags are mbedTLS's `MBEDTLS_X509_BADCERT_*`, here by value (the
//! firmware checks they are the same at build time).

use alloc::vec::Vec;

use paperpaw_sdk::{CertificateHash, Distrust, KeyHash, SocketError, Trust};

/// `MBEDTLS_X509_BADCERT_EXPIRED`.
pub const EXPIRED: u32 = 0x01;
/// `MBEDTLS_X509_BADCERT_CN_MISMATCH`.
pub const CN_MISMATCH: u32 = 0x04;
/// `MBEDTLS_X509_BADCERT_NOT_TRUSTED`.
pub const NOT_TRUSTED: u32 = 0x08;
/// `MBEDTLS_X509_BADCERT_FUTURE`.
pub const FUTURE: u32 = 0x200;

/// What the certificates are trusted by.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    /// The authorities the firmware ships.
    Authorities,
    /// The server's own certificate is one of these (SHA-256 of its DER).
    Certificates(Vec<[u8; 32]>),
    /// The server's own key is one of these (SHA-256 of its
    /// SubjectPublicKeyInfo).
    Keys(Vec<[u8; 32]>),
}

/// The server's own certificate, as the handshake showed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leaf {
    pub certificate: CertificateHash,
    pub key: KeyHash,
    /// Its dates were checked; not, where the device had no time.
    pub time_checked: bool,
}

/// What a session checks a server's certificate by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checks {
    mode: Mode,
    /// The server's own certificate, once the handshake has shown it.
    leaf: Option<Leaf>,
}

impl Checks {
    /// The checks `trust` asks for.
    #[must_use]
    pub fn new(trust: &Trust) -> Self {
        let mode = match trust {
            Trust::Pinned(pins) => {
                Mode::Certificates(pins.iter().map(|pin| *pin.sha256()).collect())
            }
            Trust::PinnedKeys(keys) => Mode::Keys(keys.iter().map(|key| *key.sha256()).collect()),
            _ => Mode::Authorities,
        };
        Self { mode, leaf: None }
    }

    /// Whether certificates or keys are pinned (not the authorities).
    #[must_use]
    pub fn pinned(&self) -> bool {
        self.mode != Mode::Authorities
    }

    /// The server's own certificate, once it was shown.
    #[must_use]
    pub fn leaf(&self) -> Option<Leaf> {
        self.leaf
    }

    /// What the flags of the certificate at `depth` become. `leaf` makes
    /// the SHA-256 of its DER and of its key, only for the server's own
    /// certificate; `dates` is what its dates found (`EXPIRED`, `FUTURE`
    /// or none), `None` where the device has no time: then they go
    /// unchecked, and [`Leaf::time_checked`] says so; `from_root` says,
    /// only when asked, whether one of the authorities signed it or is it.
    ///
    /// - Authorities: mbedTLS's findings stay, and the dates are added; a
    ///   certificate it finds untrusted is trusted if `from_root` says so.
    /// - Pinned (certificates or keys): the chain and the name are not
    ///   asked about; the server's own certificate (depth 0) must be, or
    ///   have the key of, one pinned, and its dates are checked. Whatever
    ///   else mbedTLS found of it (its key's use, a name) goes: the reader
    ///   trusts that certificate, not what it says of itself.
    pub fn flags(
        &mut self,
        depth: u32,
        leaf: impl FnOnce() -> (CertificateHash, KeyHash),
        found: u32,
        dates: Option<u32>,
        from_root: impl FnOnce() -> bool,
    ) -> u32 {
        let shown = (depth == 0).then(leaf);
        if let Some((certificate, key)) = shown {
            self.leaf = Some(Leaf {
                certificate,
                key,
                time_checked: dates.is_some(),
            });
        }
        let dates = dates.unwrap_or(0);
        let pins = match &self.mode {
            Mode::Authorities => {
                let flags = found | dates;
                return if flags & NOT_TRUSTED != 0 && from_root() {
                    flags & !NOT_TRUSTED
                } else {
                    flags
                };
            }
            Mode::Certificates(pins) => (pins, shown.map(|(certificate, _)| *certificate.sha256())),
            Mode::Keys(pins) => (pins, shown.map(|(_, key)| *key.sha256())),
        };
        match pins.1 {
            // The others of the chain do not matter.
            None => 0,
            Some(shown) => {
                dates
                    | if pins.0.contains(&shown) {
                        0
                    } else {
                        NOT_TRUSTED
                    }
            }
        }
    }
}

/// Why a session failed, as the firmware's TLS says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// The certificate is trusted by no authority, or is not pinned.
    Untrusted,
    /// The certificate is for another name.
    OtherHost,
    /// The certificate has expired or is not valid yet.
    Dates,
    /// Not enough memory for a session.
    Memory,
    /// Anything else (mbedTLS's code, for the log).
    Other(i32),
}

/// What a plugin hears of a session that did not come about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The certificate is not trusted: its hash goes with the news, for
    /// the reader to compare and a plugin to pin.
    Untrusted(Distrust),
    Failed(SocketError, &'static str),
}

/// What `fault` comes to for a socket whose certificates are `checks`
/// ([`Checks::pinned`]) and whose server showed its own certificate
/// (`shown`).
#[must_use]
pub fn outcome(fault: Fault, pinned: bool, shown: bool) -> Outcome {
    match fault {
        Fault::Untrusted if shown => Outcome::Untrusted(if pinned {
            Distrust::NotPinned
        } else {
            Distrust::UnknownIssuer
        }),
        Fault::Untrusted => Outcome::Failed(SocketError::Tls, "the server showed no certificate"),
        Fault::OtherHost => Outcome::Failed(
            SocketError::NameMismatch,
            "the server's certificate is for another host",
        ),
        Fault::Dates => Outcome::Failed(
            SocketError::Expired,
            "the server's certificate has expired or is not valid yet",
        ),
        Fault::Memory => Outcome::Failed(
            SocketError::NoMemory,
            "not enough memory for a secure connection",
        ),
        Fault::Other(_) => Outcome::Failed(SocketError::Tls, "secure connection failed"),
    }
}

/// Why a switch to TLS is refused on a socket: bytes of the server wait
/// unread (`waiting`: in the connection's buffer, or gathered for a read),
/// sent past its go-ahead. They might be injected plaintext, and would
/// pass as secure; the socket is closed, never started over them.
#[must_use]
pub fn starttls_refusal(waiting: usize) -> Option<(SocketError, &'static str)> {
    (waiting > 0).then_some((
        SocketError::NotReady,
        "the server sent bytes before the switch to TLS",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(n: u8) -> CertificateHash {
        CertificateHash::new([n; 32])
    }

    fn key(n: u8) -> KeyHash {
        KeyHash::new([n; 32])
    }

    /// A leaf of certificate `n` with its key `n + 100`.
    fn leaf(n: u8) -> impl FnOnce() -> (CertificateHash, KeyHash) {
        move || (hash(n), key(n + 100))
    }

    fn pinned(pins: &[u8]) -> Checks {
        Checks::new(&Trust::Pinned(pins.iter().map(|&n| hash(n)).collect()))
    }

    fn pinned_keys(keys: &[u8]) -> Checks {
        Checks::new(&Trust::PinnedKeys(keys.iter().map(|&n| key(n)).collect()))
    }

    #[test]
    fn authorities_keep_mbedtls_findings_and_add_the_dates() {
        let mut checks = Checks::new(&Trust::Authorities);
        assert!(!checks.pinned());
        // A name that does not fit stays, with the date.
        let flags = checks.flags(0, leaf(1), CN_MISMATCH, Some(EXPIRED), || false);
        assert_eq!(flags, CN_MISMATCH | EXPIRED);
        assert_eq!(checks.leaf().map(|l| l.certificate), Some(hash(1)));
        assert_eq!(checks.leaf().map(|l| l.key), Some(key(101)));
    }

    #[test]
    fn authorities_trust_what_a_root_signed_and_only_then() {
        let mut checks = Checks::new(&Trust::Authorities);
        assert_eq!(checks.flags(1, leaf(2), NOT_TRUSTED, Some(0), || true), 0);
        assert_eq!(
            checks.flags(1, leaf(2), NOT_TRUSTED, Some(0), || false),
            NOT_TRUSTED
        );
        // Nobody asks the roots where mbedTLS found nothing wrong.
        assert_eq!(
            checks.flags(0, leaf(3), 0, Some(0), || panic!("not asked")),
            0
        );
    }

    #[test]
    fn the_leaf_is_the_one_at_depth_0() {
        let mut checks = Checks::new(&Trust::Authorities);
        checks.flags(2, || panic!("not made"), 0, Some(0), || true);
        assert_eq!(checks.leaf(), None);
        checks.flags(0, leaf(4), 0, Some(0), || true);
        assert_eq!(checks.leaf().map(|l| l.certificate), Some(hash(4)));
    }

    #[test]
    fn without_a_time_the_dates_go_unchecked_and_it_is_said() {
        let mut checks = Checks::new(&Trust::Authorities);
        assert_eq!(checks.flags(0, leaf(1), 0, None, || false), 0);
        assert_eq!(checks.leaf().map(|l| l.time_checked), Some(false));
        // With a time they are, and are told so.
        let mut checks = Checks::new(&Trust::Authorities);
        assert_eq!(checks.flags(0, leaf(1), 0, Some(0), || false), 0);
        assert_eq!(checks.leaf().map(|l| l.time_checked), Some(true));
        // A pinned certificate likewise.
        let mut checks = pinned(&[1]);
        assert_eq!(checks.flags(0, leaf(1), 0, None, || false), 0);
        assert_eq!(checks.leaf().map(|l| l.time_checked), Some(false));
    }

    #[test]
    fn a_pinned_certificate_is_trusted_whatever_mbedtls_says_of_it() {
        let mut checks = pinned(&[5, 6]);
        assert!(checks.pinned());
        // Self-signed, another name, no server use: all of it goes.
        let found = NOT_TRUSTED | CN_MISMATCH | 0x1000;
        assert_eq!(checks.flags(0, leaf(6), found, Some(0), || false), 0);
        // Its dates are checked.
        assert_eq!(
            checks.flags(0, leaf(5), found, Some(EXPIRED), || false),
            EXPIRED
        );
        assert_eq!(
            checks.flags(0, leaf(5), found, Some(FUTURE), || false),
            FUTURE
        );
    }

    #[test]
    fn a_certificate_that_is_not_pinned_is_not_trusted() {
        let mut checks = pinned(&[5]);
        assert_eq!(checks.flags(0, leaf(7), 0, Some(0), || true), NOT_TRUSTED);
        // And it is the one the reader is shown.
        assert_eq!(checks.leaf().map(|l| l.certificate), Some(hash(7)));
        // The others of the chain do not matter.
        assert_eq!(
            checks.flags(1, leaf(8), NOT_TRUSTED | EXPIRED, Some(0), || false),
            0
        );
    }

    #[test]
    fn four_pins_are_all_looked_at() {
        let mut checks = pinned(&[1, 2, 3, 4]);
        assert_eq!(checks.flags(0, leaf(4), NOT_TRUSTED, Some(0), || false), 0);
    }

    #[test]
    fn a_pinned_key_outlasts_a_renewed_certificate() {
        // Certificate 9 is another than before but has the key 105 (of
        // certificate 5).
        let mut checks = pinned_keys(&[105, 106]);
        assert!(checks.pinned());
        let found = NOT_TRUSTED | CN_MISMATCH;
        assert_eq!(checks.flags(0, leaf(5), found, Some(0), || false), 0);
        assert_eq!(checks.flags(0, leaf(6), found, Some(0), || false), 0);
        // Dates are checked, a key that is not pinned is not trusted.
        assert_eq!(
            checks.flags(0, leaf(5), found, Some(EXPIRED), || false),
            EXPIRED
        );
        assert_eq!(checks.flags(0, leaf(7), 0, Some(0), || true), NOT_TRUSTED);
        assert_eq!(checks.leaf().map(|l| l.key), Some(key(107)));
    }

    #[test]
    fn an_untrusted_certificate_is_told_with_why() {
        assert_eq!(
            outcome(Fault::Untrusted, false, true),
            Outcome::Untrusted(Distrust::UnknownIssuer)
        );
        assert_eq!(
            outcome(Fault::Untrusted, true, true),
            Outcome::Untrusted(Distrust::NotPinned)
        );
        // Without a certificate to show it is a failure.
        assert!(matches!(
            outcome(Fault::Untrusted, false, false),
            Outcome::Failed(SocketError::Tls, _)
        ));
    }

    #[test]
    fn other_faults_have_their_errors() {
        for (fault, error) in [
            (Fault::OtherHost, SocketError::NameMismatch),
            (Fault::Dates, SocketError::Expired),
            (Fault::Memory, SocketError::NoMemory),
            (Fault::Other(-0x7780), SocketError::Tls),
        ] {
            let Outcome::Failed(got, reason) = outcome(fault, true, true) else {
                panic!("{fault:?} is a failure");
            };
            assert_eq!(got, error);
            assert!(!reason.is_empty());
        }
    }

    #[test]
    fn a_switch_over_bytes_that_came_is_refused() {
        assert_eq!(starttls_refusal(0), None);
        assert_eq!(
            starttls_refusal(1).map(|(error, _)| error),
            Some(SocketError::NotReady)
        );
    }
}
