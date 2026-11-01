//! A plugin file's author's signature (ADR 0024): authors sign their own
//! files with their own key, and a device remembers the key at the first
//! install and refuses an update signed by another. The signature is a
//! trailer at the file's end, so the file carries it wherever it goes
//! (the store, the web flasher's bundle, a copy by hand):
//!
//! | Bytes | What |
//! |-------|------|
//! | 0..4 | `PPNS` |
//! | 4..6 | the format, 1 |
//! | 6..8 | zero |
//! | 8..12 | the bytes signed: the file before the trailer |
//! | 12..44 | the author's Ed25519 public key |
//! | 44..108 | the signature |
//! | 108..112 | zero |
//!
//! The signature (Ed25519) covers [`DOMAIN`], the file before the trailer
//! and then the trailer's first 44 bytes, so that neither the length nor
//! the key can be changed and it cannot be taken for a signature of
//! anything else. Who signed it is for the device to judge.

use core::fmt;

use ed25519_compact::{PublicKey, VerifyingState};

/// The trailer's length.
pub const SIGNATURE_LEN: usize = 112;
/// The trailer's first bytes.
pub const SIGNATURE_MAGIC: [u8; 4] = *b"PPNS";
/// The trailer's format this crate reads and writes.
pub const SIGNATURE_FORMAT: u16 = 1;
/// What the signature covers before the file.
pub const DOMAIN: &[u8] = b"PaperPaw plugin file\0";
/// The trailer's bytes the signature covers after the file.
const SIGNED_LEN: usize = 44;

/// A signature that does not hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadSignature;

impl fmt::Display for BadSignature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the author's signature does not hold")
    }
}

impl core::error::Error for BadSignature {}

/// A plugin file's signature trailer (see the [module documentation](self)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Signature {
    /// The bytes it signs: the file before it.
    pub signed_len: u32,
    /// The author's Ed25519 public key.
    pub key: [u8; 32],
    pub signature: [u8; 64],
}

impl Signature {
    /// The trailer in `bytes` ([`SIGNATURE_LEN`] of them), if they are
    /// one: `None` for the end of a file not signed.
    pub fn read(bytes: &[u8]) -> Option<Signature> {
        if bytes.len() != SIGNATURE_LEN
            || bytes[..4] != SIGNATURE_MAGIC
            || u16::from_le_bytes([bytes[4], bytes[5]]) != SIGNATURE_FORMAT
            || bytes[6..8] != [0, 0]
            || bytes[108..] != [0; 4]
        {
            return None;
        }
        let mut key = [0; 32];
        key.copy_from_slice(&bytes[12..44]);
        let mut signature = [0; 64];
        signature.copy_from_slice(&bytes[44..108]);
        Some(Signature {
            signed_len: u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
            key,
            signature,
        })
    }

    /// The trailer's bytes.
    pub fn to_bytes(&self) -> [u8; SIGNATURE_LEN] {
        let mut bytes = [0; SIGNATURE_LEN];
        bytes[..SIGNED_LEN].copy_from_slice(&signed_part(self.signed_len, &self.key));
        bytes[44..108].copy_from_slice(&self.signature);
        bytes
    }

    /// Checks the signature as the signed bytes are read: give
    /// [`SignatureCheck::absorb`] all of them in order, then
    /// [`SignatureCheck::finish`].
    pub fn check(&self) -> Result<SignatureCheck, BadSignature> {
        let mut state = PublicKey::new(self.key)
            .verify_incremental(&ed25519_compact::Signature::new(self.signature))
            .map_err(|_| BadSignature)?;
        state.absorb(DOMAIN);
        Ok(SignatureCheck {
            state,
            signed: signed_part(self.signed_len, &self.key),
            signed_len: self.signed_len,
            absorbed: 0,
        })
    }

    /// Whether it holds for `signed`, the file before it, read whole.
    pub fn holds(&self, signed: &[u8]) -> Result<(), BadSignature> {
        let mut check = self.check()?;
        check.absorb(signed);
        check.finish()
    }
}

fn signed_part(signed_len: u32, key: &[u8; 32]) -> [u8; SIGNED_LEN] {
    let mut bytes = [0; SIGNED_LEN];
    bytes[..4].copy_from_slice(&SIGNATURE_MAGIC);
    bytes[4..6].copy_from_slice(&SIGNATURE_FORMAT.to_le_bytes());
    bytes[8..12].copy_from_slice(&signed_len.to_le_bytes());
    bytes[12..44].copy_from_slice(key);
    bytes
}

/// A signature checked piece by piece (see [`Signature::check`]).
pub struct SignatureCheck {
    state: VerifyingState,
    signed: [u8; SIGNED_LEN],
    signed_len: u32,
    absorbed: u64,
}

impl fmt::Debug for SignatureCheck {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignatureCheck")
            .field("signed_len", &self.signed_len)
            .field("absorbed", &self.absorbed)
            .finish_non_exhaustive()
    }
}

impl SignatureCheck {
    /// The file's next bytes.
    pub fn absorb(&mut self, bytes: &[u8]) {
        self.state.absorb(bytes);
        self.absorbed += bytes.len() as u64;
    }

    /// Whether the signature holds for all absorbed.
    pub fn finish(mut self) -> Result<(), BadSignature> {
        if self.absorbed != u64::from(self.signed_len) {
            return Err(BadSignature);
        }
        self.state.absorb(self.signed);
        self.state.verify().map_err(|_| BadSignature)
    }
}

/// Signs `file` (a plugin file without a trailer) with the Ed25519 key
/// made from `seed`: the trailer to append.
pub fn sign(file: &[u8], seed: &[u8; 32]) -> Signature {
    let pair = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(*seed));
    let key = *pair.pk;
    let signed_len = u32::try_from(file.len()).unwrap_or(u32::MAX);
    // Deterministic Ed25519 over the whole message: the streaming signer
    // would need noise that never repeats.
    let mut message = alloc::vec::Vec::with_capacity(DOMAIN.len() + file.len() + SIGNED_LEN);
    message.extend_from_slice(DOMAIN);
    message.extend_from_slice(file);
    message.extend_from_slice(&signed_part(signed_len, &key));
    Signature {
        signed_len,
        key,
        signature: *pair.sk.sign(&message, None),
    }
}

/// The statement an author's recovery key signs to move `plugin` from the
/// author's key `old` to `new` (`paperpaw_sdk::native::keys`): these
/// bytes, exactly.
pub fn rotation_message(plugin: &str, old: &[u8; 32], new: &[u8; 32]) -> alloc::string::String {
    alloc::format!(
        "paperpaw key rotation 1\n{plugin}\n{}\n{}\n",
        paperpaw_digest::hex::encode(old),
        paperpaw_digest::hex::encode(new)
    )
}

/// The statement that revokes `key`, the key of `plugin`'s author
/// (`paperpaw_sdk::native::keys`): signed by that key itself or by the
/// author's recovery key; these bytes, exactly.
pub fn revocation_message(plugin: &str, key: &[u8; 32]) -> alloc::string::String {
    alloc::format!(
        "paperpaw key revocation 1\n{plugin}\n{}\n",
        paperpaw_digest::hex::encode(key)
    )
}

/// Signs `message` (a short statement, e.g. a key rotation) with the
/// Ed25519 key made from `seed`.
pub fn sign_message(message: &[u8], seed: &[u8; 32]) -> [u8; 64] {
    let pair = ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(*seed));
    *pair.sk.sign(message, None)
}

/// Whether `signature` is `key`'s over `message` ([`sign_message`]).
pub fn verify_message(key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(key) = PublicKey::from_slice(key) else {
        return false;
    };
    key.verify(message, &ed25519_compact::Signature::new(*signature))
        .is_ok()
}

/// The public key of the Ed25519 key made from `seed`.
pub fn public_key(seed: &[u8; 32]) -> [u8; 32] {
    *ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(*seed)).pk
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [3; 32];

    /// A revocation is signed and checked as a rotation is, and the two
    /// statements never read the same.
    #[test]
    fn a_revocation_is_a_statement_of_its_own() {
        let key = public_key(&SEED);
        let message = revocation_message("clock", &key);
        let signature = sign_message(message.as_bytes(), &SEED);
        assert!(verify_message(&key, message.as_bytes(), &signature));
        assert!(!verify_message(
            &key,
            revocation_message("clocks", &key).as_bytes(),
            &signature
        ));
        assert_ne!(message, rotation_message("clock", &key, &key));
    }

    #[test]
    fn a_signature_holds_in_any_pieces() {
        let file = alloc::vec![7u8; 5000];
        let signature = sign(&file, &SEED);
        assert_eq!(signature.key, public_key(&SEED));
        assert_eq!(Signature::read(&signature.to_bytes()), Some(signature));
        assert_eq!(signature.holds(&file), Ok(()));
        let mut check = signature.check().unwrap();
        for piece in file.chunks(333) {
            check.absorb(piece);
        }
        assert_eq!(check.finish(), Ok(()));
    }

    #[test]
    fn a_change_breaks_it() {
        let file = alloc::vec![7u8; 5000];
        let signature = sign(&file, &SEED);
        let mut changed = file.clone();
        changed[4000] ^= 1;
        assert_eq!(signature.holds(&changed), Err(BadSignature));
        assert_eq!(signature.holds(&file[..4999]), Err(BadSignature));
        let other_key = Signature {
            key: public_key(&[4; 32]),
            ..signature
        };
        assert_eq!(other_key.holds(&file), Err(BadSignature));
        let longer = Signature {
            signed_len: 5001,
            ..signature
        };
        assert_eq!(longer.holds(&file), Err(BadSignature));
    }

    #[test]
    fn what_is_no_trailer_is_none() {
        let bytes = sign(b"plugin", &SEED).to_bytes();
        assert_eq!(Signature::read(&bytes[..111]), None);
        for at in [0, 4, 6, 108] {
            let mut changed = bytes;
            changed[at] ^= 1;
            assert_eq!(Signature::read(&changed), None, "{at}");
        }
    }
}
