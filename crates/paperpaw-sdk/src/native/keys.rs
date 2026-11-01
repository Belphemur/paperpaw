//! Author keys beyond the first install (ticket #20): a recovery key the
//! author chose in advance, with which a lost or stolen key is replaced by
//! a statement the reader checks itself, and keys the store says were
//! revoked, whose code the kernel does not load.
//!
//! A **recovery key** is pinned with the author's key at a plugin's first
//! install, and only then (`remember_first_install` (kernel)): never later, not
//! even signed by the author's key, which may be the stolen one. The
//! author keeps it offline. To move
//! to a new key the author signs [`rotation_message`] with it; a store
//! that gets the statement with an update hands it to `accept_rotation` (kernel),
//! which checks it against the pinned keys, so neither a moderator's
//! mistake nor a taken-over server can forge one. The code installed
//! before the rotation is not loaded from then on: it may be the stolen
//! key's, so only an update signed by the new key runs again. Plugins
//! installed without a recovery key keep only the successor way (a new
//! id).
//!
//! **Revoked keys** come from the store (its library lists them); it
//! adds them with `keep_revoked` (kernel), and none is ever taken off. The kernel does not load the code of
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

/// Remembers `author` as the key of `plugin`'s author and pins `recovery`
/// as its recovery key, at the plugin's first install: only where no
/// author is known for it yet (so an update can never bring a recovery
/// key); `false` otherwise or where they could not be kept. The author's
/// key is kept first: cut short after it, the plugin has no recovery key
/// (only the successor way), never a recovery key without its author.
#[cfg(any(test, feature = "kernel"))]
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
pub fn remember_first_install(
    storage: &dyn Storage,
    plugin: &str,
    author: &[u8; 32],
    recovery: &[u8; 32],
) -> bool {
    if super::known_author(storage, plugin).is_some() {
        return false;
    }
    super::remember_author(storage, plugin, author)
        && super::keep_entry(storage, RECOVERY, plugin, recovery)
}

/// The statement the recovery key signs to move `plugin` from the
/// author's key `old` to `new`: these bytes, exactly.
pub fn rotation_message(plugin: &str, old: &[u8; 32], new: &[u8; 32]) -> String {
    paperpaw_native::signature::rotation_message(plugin, old, new)
}

/// Why a rotation was not taken (`accept_rotation` (kernel)).
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
/// the old key's are not). The code installed or fetched before is no
/// longer named as installed ([`installed_code`](super::installed_code),
/// [`fetched_code`](super::fetched_code)), so it is not loaded at the next
/// start: it may be what a stolen key signed. The recovery key stays.
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
    // The old key's code first: cut short after it, nothing is loaded,
    // and the rotation can be taken again.
    let forgotten = super::write_entry(storage, super::INSTALLED, plugin, None)
        && super::write_entry(storage, super::FETCHED, plugin, None);
    if forgotten && super::keep_entry(storage, super::AUTHORS, plugin, new) {
        Ok(())
    } else {
        Err(RotationError::NotKept)
    }
}

/// The keys the store said were revoked.
pub fn revoked(storage: &dyn Storage) -> Vec<[u8; 32]> {
    match crate::files::load_for_update(storage, REVOKED, MAX_REVOKED_BYTES) {
        Ok(Some(data)) => revoked_in(&data),
        _ => Vec::new(),
    }
}

/// How large the list of revoked keys may be: [`MAX_REVOKED`] lines of
/// 65 bytes, with room for line breaks of two.
const MAX_REVOKED_BYTES: usize = 64 * 1024;

/// The revoked keys in `data`, a line each. A line that is none (a
/// damaged byte) is left out on its own: the others stay revoked.
fn revoked_in(data: &[u8]) -> Vec<[u8; 32]> {
    data.split(|&b| b == b'\n')
        .filter_map(|line| core::str::from_utf8(line).ok())
        .filter_map(|line| paperpaw_digest::hex::decode(line.trim())?.try_into().ok())
        .take(MAX_REVOKED)
        .collect()
}

/// Whether `key` is revoked.
pub fn is_revoked(storage: &dyn Storage, key: &[u8; 32]) -> bool {
    revoked(storage).contains(key)
}

/// Why revoked keys were not all kept (`keep_revoked` (kernel)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RevokedError {
    /// More than [`MAX_REVOKED`] in all: those kept before and the first
    /// of the new ones up to it are kept, the rest not (tell the reader).
    Full,
    /// They could not be written, or the list kept could not be read
    /// (too large, a failing card): it stays as it is.
    NotKept,
}

/// Adds `keys` to the revoked ones (the store's library lists them): a
/// key once revoked stays so, whatever a later list says, as the list
/// may come from a taken-over server. At most [`MAX_REVOKED`] in all;
/// past that, [`RevokedError::Full`].
#[cfg(any(test, feature = "kernel"))]
#[cfg_attr(docsrs, doc(cfg(feature = "kernel")))]
pub fn keep_revoked(storage: &dyn Storage, keys: &[[u8; 32]]) -> Result<(), RevokedError> {
    // A list there that cannot be read is not written over by a shorter
    // one; one with damaged lines is written anew from the others.
    let mut all = match crate::files::load_for_update(storage, REVOKED, MAX_REVOKED_BYTES) {
        Ok(data) => data.as_deref().map(revoked_in).unwrap_or_default(),
        Err(_) => return Err(RevokedError::NotKept),
    };
    let before = all.len();
    let mut full = false;
    for key in keys {
        if all.contains(key) {
            continue;
        }
        if all.len() >= MAX_REVOKED {
            full = true;
            break;
        }
        all.push(*key);
    }
    if all.len() > before {
        let text: String = all
            .iter()
            .map(|key| alloc::format!("{}\n", paperpaw_digest::hex::encode(key)))
            .collect();
        let _ = storage.create_dir(crate::data::DIR);
        if storage.write(REVOKED, text.as_bytes()).is_err() {
            return Err(RevokedError::NotKept);
        }
    }
    match full {
        true => Err(RevokedError::Full),
        false => Ok(()),
    }
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
        let recovery = public_key(&RECOVERY_SEED);
        assert!(remember_first_install(
            &storage, "clock", &AUTHOR, &recovery
        ));
        assert!(super::super::remember_code(&storage, "clock", &[6; 32]));
        assert!(super::super::remember_fetched(&storage, "clock", &[5; 32]));
        assert!(super::super::remember_code(&storage, "other", &[4; 32]));
        storage
    }

    /// Pinned only with the first author's key: an update of a plugin
    /// whose author is known brings none, though none was pinned (#20,
    /// review M1).
    #[test]
    fn a_recovery_key_is_pinned_only_at_the_first_install() {
        let storage = crate::MemoryStorage::new();
        assert!(super::super::remember_author(&storage, "clock", &AUTHOR));
        assert!(!remember_first_install(
            &storage, "clock", &AUTHOR, &[5; 32]
        ));
        assert!(!remember_first_install(
            &storage, "clock", &[3; 32], &[5; 32]
        ));
        assert_eq!(known_recovery(&storage, "clock"), None);
        assert_eq!(super::super::known_author(&storage, "clock"), Some(AUTHOR));
        // A first install pins both, and a second one neither.
        assert!(remember_first_install(&storage, "new", &AUTHOR, &[5; 32]));
        assert_eq!(known_recovery(&storage, "new"), Some([5; 32]));
        assert!(!remember_first_install(&storage, "new", &[3; 32], &[8; 32]));
        assert_eq!(known_recovery(&storage, "new"), Some([5; 32]));
        assert_eq!(super::super::known_author(&storage, "new"), Some(AUTHOR));
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
        assert!(!remember_first_install(&storage, "clock", &NEW, &[5; 32]));
        // The old key's code is no longer named as installed or fetched:
        // not loaded until an update by the new key (review M2); other
        // plugins' as they were.
        assert_eq!(super::super::installed_code(&storage, "clock"), None);
        assert_eq!(super::super::fetched_code(&storage, "clock"), None);
        assert_eq!(
            super::super::installed_code(&storage, "other"),
            Some([4; 32])
        );
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
        // To a revoked key: refused, and the code stays named.
        assert_eq!(keep_revoked(&storage, &[NEW]), Ok(()));
        assert_eq!(
            accept_rotation(&storage, "clock", &NEW, &signature),
            Err(RotationError::Refused)
        );
        assert_eq!(super::super::known_author(&storage, "clock"), Some(AUTHOR));
        assert_eq!(
            super::super::installed_code(&storage, "clock"),
            Some([6; 32])
        );
    }

    #[test]
    fn revoked_keys_are_only_added_and_read_leniently() {
        let storage = crate::MemoryStorage::new();
        assert!(revoked(&storage).is_empty());
        assert_eq!(keep_revoked(&storage, &[AUTHOR, NEW]), Ok(()));
        assert!(is_revoked(&storage, &NEW) && !is_revoked(&storage, &[3; 32]));
        // A later list without a key does not take it off (review S1).
        assert_eq!(keep_revoked(&storage, &[NEW, [3; 32]]), Ok(()));
        assert_eq!(revoked(&storage), [AUTHOR, NEW, [3; 32]]);
        assert_eq!(keep_revoked(&storage, &[]), Ok(()));
        assert_eq!(revoked(&storage).len(), 3);
        storage
            .write(REVOKED, b"not hex\n0909\n0101010101010101010101010101010101010101010101010101010101010101\n")
            .unwrap();
        assert_eq!(revoked(&storage), [AUTHOR]);
        // Past the most: those before and the first new ones kept, and
        // said so (review S2).
        let many: Vec<[u8; 32]> = (2..300u16)
            .map(|n| {
                [
                    n as u8,
                    (n >> 8) as u8,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    1,
                ]
            })
            .collect();
        assert_eq!(keep_revoked(&storage, &many), Err(RevokedError::Full));
        let kept = revoked(&storage);
        assert_eq!(kept.len(), MAX_REVOKED);
        assert_eq!((kept[0], kept[1]), (AUTHOR, many[0]));
        assert!(is_revoked(&storage, &many[MAX_REVOKED - 2]));
        assert!(!is_revoked(&storage, &many[MAX_REVOKED - 1]));
    }

    /// A damaged line leaves the others revoked, and does not keep new
    /// keys out: the list is written anew from what can be read. One that
    /// cannot be read at all (too large) is not written over.
    #[test]
    fn a_damaged_line_costs_only_itself() {
        let storage = crate::MemoryStorage::new();
        let mut text = alloc::format!("{}\n", paperpaw_digest::hex::encode(&AUTHOR)).into_bytes();
        text.extend_from_slice(&[0xff, 0xfe, b'\n']);
        text.extend_from_slice(
            alloc::format!("{}\r\n", paperpaw_digest::hex::encode(&[3; 32])).as_bytes(),
        );
        storage.write(REVOKED, &text).unwrap();
        assert_eq!(revoked(&storage), [AUTHOR, [3; 32]]);
        assert_eq!(keep_revoked(&storage, &[NEW]), Ok(()));
        assert_eq!(revoked(&storage), [AUTHOR, [3; 32], NEW]);
        assert!(
            !crate::read_file(&storage, REVOKED, MAX_REVOKED_BYTES)
                .unwrap()
                .contains(&0xff)
        );

        storage
            .write(REVOKED, &alloc::vec![b'0'; MAX_REVOKED_BYTES + 1])
            .unwrap();
        assert_eq!(
            keep_revoked(&storage, &[[4; 32]]),
            Err(RevokedError::NotKept)
        );
        assert_eq!(storage.size(REVOKED), Ok(MAX_REVOKED_BYTES as u64 + 1));
    }
}
