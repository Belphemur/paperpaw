//! Keys the library says were withdrawn (#20): its list carries
//! `revoked_keys: [{"plugin": …, "key": <64 hex>, "signature": <128 hex>}]`.
//! The store only carries them: the kernel keeps an entry only where its
//! key is the plugin's pinned author here and the signature is that key's
//! or the plugin's recovery key's ([`keys::keep_revoked`]), so a library
//! that lies stops no plugin and fills no list.
//!
//! The entries are handed over a few at a time, so a long list takes no
//! room on the small heap.

use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_sdk::Storage;
use paperpaw_sdk::native::keys::{self, MAX_REVOKED, Revocation};

/// Entries held before they are handed over.
const BATCH: usize = 16;

/// The entries of one list being read.
#[derive(Default)]
pub(crate) struct Revocations {
    plugin: String,
    key: String,
    signature: String,
    batch: Vec<Revocation>,
    /// Entries read, kept or not: no more than the kernel could keep.
    read: usize,
}

impl Revocations {
    /// Takes one field of the entry being read (`plugin`, `key`,
    /// `signature`); others are left.
    pub(crate) fn field(&mut self, field: &str, value: &str) {
        let (to, most) = match field {
            "plugin" => (&mut self.plugin, 64),
            "key" => (&mut self.key, 64),
            "signature" => (&mut self.signature, 128),
            _ => return,
        };
        *to = value.chars().take(most + 1).collect();
    }

    /// The entry is complete: kept for handing over if it is well formed
    /// (a bad one is left out on its own).
    pub(crate) fn entry_done(&mut self, storage: &dyn Storage) {
        let entry = (
            bytes::<32>(&self.key),
            bytes::<64>(&self.signature),
            core::mem::take(&mut self.plugin),
        );
        self.key.clear();
        self.signature.clear();
        let (Some(key), Some(signature), plugin) = entry else {
            return;
        };
        if plugin.is_empty() || self.read >= MAX_REVOKED {
            return;
        }
        self.read += 1;
        self.batch.push(Revocation::new(plugin, key, signature));
        if self.batch.len() >= BATCH {
            self.hand_over(storage);
        }
    }

    /// Hands over what is left. A card that could not be written is tried
    /// again at the next list; more than the kernel keeps cannot happen,
    /// as each plugin has one entry that holds.
    pub(crate) fn hand_over(&mut self, storage: &dyn Storage) {
        if !self.batch.is_empty() {
            let _ = keys::keep_revoked(storage, &self.batch);
            self.batch.clear();
        }
    }
}

fn bytes<const N: usize>(hex: &str) -> Option<[u8; N]> {
    paperpaw_sdk::digest::hex::decode(hex)?.try_into().ok()
}
