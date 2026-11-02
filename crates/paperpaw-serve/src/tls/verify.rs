//! Which certificates a TLS socket trusts: the authorities PaperPaw ships
//! (Mozilla's list, chain, name and dates checked) or those a plugin
//! pinned (the server's own certificate or its key; its dates checked,
//! the chain and the name not). There is no way to trust anything else:
//! the handshake's signature is checked against the certificate's key in
//! all modes.
//!
//! The dates are checked against the computer's clock where it can be
//! believed ([`system_clock`]); where it cannot, everything else is
//! checked and the verdict says `time_checked` false (for authorities
//! the chain is then checked as of the time the server's certificate
//! began, when its issuers must have been valid).

use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use paperpaw_core::sdk::Trust;
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, DigitallySignedStruct, Error, OtherError, RootCertStore, SignatureScheme,
};

use super::der;

/// The computer's time, in seconds since 1970, or `None` where it cannot
/// be believed.
pub(crate) type Clock = fn() -> Option<i64>;

/// A clock before this (2025-01-01) was never set: a computer without a
/// battery clock starts in 1970.
const CLOCK_FLOOR: i64 = 1_735_689_600;

/// The system clock, unless it says a time before [`CLOCK_FLOOR`].
pub(crate) fn system_clock() -> Option<i64> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    i64::try_from(secs).ok().filter(|&secs| secs >= CLOCK_FLOOR)
}

/// The server's own certificate, as the handshake showed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Leaf {
    /// SHA-256 of its DER.
    pub certificate: [u8; 32],
    /// SHA-256 of its SubjectPublicKeyInfo.
    pub key: [u8; 32],
    /// Its dates (and its issuers') were checked.
    pub time_checked: bool,
}

/// What is trusted.
#[derive(Debug)]
enum Mode {
    Authorities(Arc<WebPkiServerVerifier>),
    Certificates(Vec<[u8; 32]>),
    Keys(Vec<[u8; 32]>),
}

/// The certificate is none of those pinned (its error, for the shell to
/// tell from a failure).
#[derive(Debug)]
pub(crate) struct NotPinned;

impl fmt::Display for NotPinned {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the server's certificate is not one pinned")
    }
}

impl std::error::Error for NotPinned {}

/// The crypto the handshake uses: ring's.
pub(crate) fn provider() -> Arc<CryptoProvider> {
    static PROVIDER: OnceLock<Arc<CryptoProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| Arc::new(rustls::crypto::ring::default_provider()))
        .clone()
}

/// The authorities PaperPaw ships, Mozilla's list as `webpki-roots` has
/// it (the same as the X4's).
fn shipped() -> Option<Arc<WebPkiServerVerifier>> {
    static SHIPPED: OnceLock<Option<Arc<WebPkiServerVerifier>>> = OnceLock::new();
    SHIPPED
        .get_or_init(|| {
            let roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            authorities(roots)
        })
        .clone()
}

fn authorities(roots: RootCertStore) -> Option<Arc<WebPkiServerVerifier>> {
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
        .build()
        .ok()
}

/// Checks a server's certificate for one socket, and keeps what it saw.
#[derive(Debug)]
pub(crate) struct Verifier {
    mode: Mode,
    clock: Clock,
    seen: Arc<Mutex<Option<Leaf>>>,
}

impl Verifier {
    /// The checks `trust` asks for, with the shipped authorities. `None`
    /// only if the list of authorities cannot be made.
    pub(crate) fn new(trust: &Trust, clock: Clock) -> Option<Self> {
        Self::with_authorities(trust, clock, shipped)
    }

    /// As [`Verifier::new`], the authorities being those `roots` gives
    /// (for tests, with a certificate authority of their own).
    pub(crate) fn with_authorities(
        trust: &Trust,
        clock: Clock,
        roots: impl FnOnce() -> Option<Arc<WebPkiServerVerifier>>,
    ) -> Option<Self> {
        let mode = match trust {
            Trust::Pinned(pins) => Mode::Certificates(pins.iter().map(|p| *p.sha256()).collect()),
            Trust::PinnedKeys(keys) => Mode::Keys(keys.iter().map(|k| *k.sha256()).collect()),
            // Authorities, and any kind this build does not know: the
            // strictest.
            _ => Mode::Authorities(roots()?),
        };
        Some(Self {
            mode,
            clock,
            seen: Arc::default(),
        })
    }

    /// What the handshake showed of the server's certificate, once it did.
    pub(crate) fn seen(&self) -> Option<Leaf> {
        self.seen.lock().ok().and_then(|seen| *seen)
    }

    pub(crate) fn remember(&self, leaf: Leaf) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(leaf);
        }
    }

    /// The dates of a pinned certificate against the clock.
    fn dates(&self, parts: &der::Parts<'_>) -> Result<bool, Error> {
        let Some(now) = (self.clock)() else {
            return Ok(false);
        };
        if now < parts.not_before {
            Err(Error::InvalidCertificate(CertificateError::NotValidYet))
        } else if now > parts.not_after {
            Err(Error::InvalidCertificate(CertificateError::Expired))
        } else {
            Ok(true)
        }
    }
}

fn bad_encoding() -> Error {
    Error::InvalidCertificate(CertificateError::BadEncoding)
}

fn seconds(secs: i64) -> UnixTime {
    UnixTime::since_unix_epoch(Duration::from_secs(u64::try_from(secs).unwrap_or(0)))
}

impl ServerCertVerifier for Verifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let parts = der::parts(end_entity).ok_or_else(bad_encoding)?;
        let mut leaf = Leaf {
            certificate: paperpaw_digest::sha256::digest(end_entity),
            key: paperpaw_digest::sha256::digest(parts.spki),
            time_checked: false,
        };
        // Told even where the certificate is then refused: the reader
        // compares it, and a plugin may pin it.
        self.remember(leaf);
        match &self.mode {
            Mode::Authorities(inner) => {
                let clock = (self.clock)();
                // Where the time is unknown, as the certificate began:
                // its issuers had to be valid then.
                let at = clock
                    .unwrap_or_else(|| parts.not_before.saturating_add(1).min(parts.not_after));
                inner.verify_server_cert(
                    end_entity,
                    intermediates,
                    server_name,
                    ocsp_response,
                    seconds(at),
                )?;
                leaf.time_checked = clock.is_some();
            }
            // The key pinned is found by `der::parts`, which is lenient (a
            // length that is not minimal, bytes after the certificate). That
            // is safe because the handshake's signature is checked next
            // (`verify_tls12/13_signature`) against the key of this same
            // certificate parsed strictly by webpki: a certificate that
            // `parts` reads differently from webpki fails the handshake, and
            // on strict DER both find the same SubjectPublicKeyInfo. The
            // tests `a_certificate_not_in_strict_der_is_refused_even_with_the_pinned_key`
            // keep it so.
            Mode::Certificates(pins) | Mode::Keys(pins) => {
                let shown = if matches!(self.mode, Mode::Keys(_)) {
                    leaf.key
                } else {
                    leaf.certificate
                };
                if !pins.contains(&shown) {
                    return Err(Error::InvalidCertificate(CertificateError::Other(
                        OtherError(Arc::new(NotPinned)),
                    )));
                }
                leaf.time_checked = self.dates(&parts)?;
            }
        }
        self.remember(leaf);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
