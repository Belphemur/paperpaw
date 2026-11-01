//! Author keys beyond the first install (ticket #20): a recovery key the
//! author chose in advance, with which a lost or stolen key is replaced by
//! a statement the reader checks itself, and keys the store says were
//! revoked, whose code the kernel does not load.
//!
//! A **recovery key** is pinned with the author's key at a plugin's first
//! install ([`remember_recovery`]); the author keeps it offline. To move
//! to a new key the author signs [`rotation_message`] with it; a store
//! that gets the statement with an update hands it to [`accept_rotation`],
//! which checks it against the pinned keys, so neither a moderator's
//! mistake nor a taken-over server can forge one. Plugins installed
//! without a recovery key keep only the successor way (a new id).
//!
//! **Revoked keys** come from the store (its library lists them); it
//! keeps them with [`keep_revoked`]. The kernel does not load the code of
//! a plugin whose known author's key is among them
//! ([`PluginProblem::Revoked`](crate::PluginProblem::Revoked)).

use alloc::string::String;
use alloc::vec::Vec;

use crate::Storage;

/// Where the recovery keys are kept, beside the authors' keys (a file of
/// its own, as older builds read every line of that one as one key).
const RECOVERY: &str = crate::data_path!("plugin-recovery.txt");

/// Where the revoked keys are kept: one in hex on each line.
const REVOKED: &str = crate::data_path!("revoked-keys.txt");

/// Revoked keys kept at most.
pub const MAX_REVOKED: usize = 256;

/// The recovery key of `plugin`'s author, as pinned at its first install.
pub fn known_recovery(storage: &dyn Storage, plugin: &str) -> Option<[u8; 32]> {
    super::entry(storage, RECOVERY, plugin)
}

/// Pins `key` as the recovery key of `plugin`'s author: only with its
/// first install (no author known yet, or the author's key just
/// remembered and no recovery key yet), never in place of one; `false` if
/// one is pinned already or it could not be kept.
#[cfg(any(test, feature = "kernel"))]
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
pub fn remember_recovery(storage: &dyn Storage, plugin: &str, key: &[u8; 32]) -> bool {
    if known_recovery(storage, plugin).is_some() {
        return false;
    }
    super::keep_entry(storage, RECOVERY, plugin, key)
}

/// The statement the recovery key signs to move `plugin` from the
/// author's key `old` to `new`: these bytes, exactly.
pub fn rotation_message(plugin: &str, old: &[u8; 32], new: &[u8; 32]) -> String {
    paperpaw_native::signature::rotation_message(plugin, old, new)
}

/// Why a rotation was not taken ([`accept_rotation`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RotationError {
    /// No author is known for the plugin: a first install needs none.
    NoAuthor,
    /// No recovery key was pinned for it: only a successor (a new id).
    NoRecoveryKey,
    /// The signature is not the recovery key's over this rotation.
    BadSignature,
    /// The new key is revoked, or the old one again.
    Refused,
    /// The new key could not be kept.
    NotKept,
}

/// Takes the author of `plugin` to `new`, where `signature` is the pinned
/// recovery key's over [`rotation_message`] from the known key: from then
/// on the plugin's code is `new`'s (updates signed by it are taken, and
/// the old key's are not). The recovery key stays.
#[cfg(any(test, feature = "kernel"))]
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
pub fn accept_rotation(
    storage: &dyn Storage,
    plugin: &str,
    new: &[u8; 32],
    signature: &[u8; 64],
) -> Result<(), RotationError> {
    let old = super::known_author(storage, plugin).ok_or(RotationError::NoAuthor)?;
    let recovery = known_recovery(storage, plugin).ok_or(RotationError::NoRecoveryKey)?;
    if *new == old || is_revoked(storage, new) {
        return Err(RotationError::Refused);
    }
    let message = rotation_message(plugin, &old, new);
    if !paperpaw_native::signature::verify_message(&recovery, message.as_bytes(), signature) {
        return Err(RotationError::BadSignature);
    }
    if super::keep_entry(storage, super::AUTHORS, plugin, new) {
        Ok(())
    } else {
        Err(RotationError::NotKept)
    }
}

/// The keys the store said were revoked.
pub fn revoked(storage: &dyn Storage) -> Vec<[u8; 32]> {
    let Ok(text) = crate::read_file(storage, REVOKED, 64 * 1024) else {
        return Vec::new();
    };
    core::str::from_utf8(&text)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| paperpaw_digest::hex::decode(line.trim())?.try_into().ok())
        .take(MAX_REVOKED)
        .collect()
}

/// Whether `key` is revoked.
pub fn is_revoked(storage: &dyn Storage, key: &[u8; 32]) -> bool {
    revoked(storage).contains(key)
}

/// Keeps `keys` as the revoked ones, in place of those before (the
/// store's library lists them all); at most [`MAX_REVOKED`]. `false` if
/// they could not be kept.
#[cfg(any(test, feature = "kernel"))]
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
pub fn keep_revoked(storage: &dyn Storage, keys: &[[u8; 32]]) -> bool {
    let text: String = keys
        .iter()
        .take(MAX_REVOKED)
        .map(|key| alloc::format!("{}\n", paperpaw_digest::hex::encode(key)))
        .collect();
    let _ = storage.create_dir(crate::data::DIR);
    storage.write(REVOKED, text.as_bytes()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use paperpaw_native::signature::{public_key, sign_message};

    const AUTHOR: [u8; 32] = [1; 32];
    const RECOVERY_SEED: [u8; 32] = [7; 32];
    const NEW: [u8; 32] = [9; 32];

    fn installed() -> crate::MemoryStorage {
        let storage = crate::MemoryStorage::new();
        assert!(super::super::remember_author(&storage, "clock", &AUTHOR));
        assert!(remember_recovery(
            &storage,
            "clock",
            &public_key(&RECOVERY_SEED)
        ));
        storage
    }

    #[test]
    fn a_recovery_key_moves_the_author_to_a_new_key() {
        let storage = installed();
        let message = rotation_message("clock", &AUTHOR, &NEW);
        let signature = sign_message(message.as_bytes(), &RECOVERY_SEED);
        assert_eq!(accept_rotation(&storage, "clock", &NEW, &signature), Ok(()));
        assert_eq!(super::super::known_author(&storage, "clock"), Some(NEW));
        assert_eq!(
            super::super::author(&storage, "clock", &AUTHOR),
            super::super::Author::Other
        );
        // The recovery key stays, and is not replaced.
        assert_eq!(
            known_recovery(&storage, "clock"),
            Some(public_key(&RECOVERY_SEED))
        );
        assert!(!remember_recovery(&storage, "clock", &[5; 32]));
        // The same statement again: from a key no longer known.
        assert_eq!(
            accept_rotation(&storage, "clock", &NEW, &signature),
            Err(RotationError::Refused)
        );
    }

    #[test]
    fn a_rotation_is_refused_unless_the_recovery_key_signed_it() {
        let storage = installed();
        let message = rotation_message("clock", &AUTHOR, &NEW);
        // Signed by the author's (stolen) key, or another's.
        for seed in [[2; 32], [8; 32]] {
            let signature = sign_message(message.as_bytes(), &seed);
            assert_eq!(
                accept_rotation(&storage, "clock", &NEW, &signature),
                Err(RotationError::BadSignature)
            );
        }
        // For another plugin, or to another key than signed.
        let signature = sign_message(message.as_bytes(), &RECOVERY_SEED);
        assert_eq!(
            accept_rotation(&storage, "clock", &[4; 32], &signature),
            Err(RotationError::BadSignature)
        );
        assert_eq!(
            accept_rotation(&storage, "other", &NEW, &signature),
            Err(RotationError::NoAuthor)
        );
        // No recovery key pinned: none.
        let bare = crate::MemoryStorage::new();
        assert!(super::super::remember_author(&bare, "clock", &AUTHOR));
        assert_eq!(
            accept_rotation(&bare, "clock", &NEW, &signature),
            Err(RotationError::NoRecoveryKey)
        );
        // To a revoked key: refused.
        assert!(keep_revoked(&storage, &[NEW]));
        assert_eq!(
            accept_rotation(&storage, "clock", &NEW, &signature),
            Err(RotationError::Refused)
        );
        assert_eq!(super::super::known_author(&storage, "clock"), Some(AUTHOR));
    }

    #[test]
    fn revoked_keys_are_kept_whole_and_read_leniently() {
        let storage = crate::MemoryStorage::new();
        assert!(revoked(&storage).is_empty());
        assert!(keep_revoked(&storage, &[AUTHOR, NEW]));
        assert!(is_revoked(&storage, &NEW) && !is_revoked(&storage, &[3; 32]));
        assert!(keep_revoked(&storage, &[NEW]));
        assert!(!is_revoked(&storage, &AUTHOR));
        storage
            .write(REVOKED, b"not hex\n0909\n0101010101010101010101010101010101010101010101010101010101010101\n")
            .unwrap();
        assert_eq!(revoked(&storage), [AUTHOR]);
        let many: Vec<[u8; 32]> = (0..300u16).map(|n| [n as u8; 32]).collect();
        assert!(keep_revoked(&storage, &many));
        assert_eq!(revoked(&storage).len(), MAX_REVOKED);
    }
}
