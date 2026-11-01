//! The English catalogs built into the program as one table
//! ([`BuiltCatalogs`]): a build script indexes them together, so a
//! translator keeps one catalog for all of them, each plugin's source a
//! word or two, rather than a catalog for each (a hundred bytes and more
//! each, for some eighty plugins on the X4).

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use crate::catalog::{Catalog, CatalogError, cmp_plain_key, fingerprint, plain_value};

/// In an entry of [`BuiltCatalogs::entries`]: its message is
/// `decoded[entry & !DECODED]`, not a plain line of a source.
pub const DECODED: u32 = 1 << 31;
/// Bits of an entry's offset in its source; the bits above, below
/// [`DECODED`], are its source's slot.
const OFFSET_BITS: u32 = 24;

/// The English catalogs of the plugins built in, indexed together by a
/// build script (`paperpaw-app`'s, with [`BuiltTable`]): kept in the
/// program, handed to the translator
/// ([`Translator::built_catalogs`](crate::Translator::built_catalogs)).
#[derive(Debug)]
#[non_exhaustive]
pub struct BuiltCatalogs {
    /// Each source's [`fingerprint`], by slot: a catalog added with this
    /// source fills its slot rather than being kept as a catalog.
    pub prints: &'static [u64],
    /// The messages of all, sorted by key: a slot and the offset of the
    /// key's line in that source, or [`DECODED`] and a place in `decoded`.
    pub entries: &'static [u32],
    /// Keys and messages a source does not give plainly (quoted values,
    /// escape sequences), decoded.
    pub decoded: &'static [(&'static str, &'static str)],
}

impl BuiltCatalogs {
    /// None (a program that indexes no catalogs together).
    pub const NONE: BuiltCatalogs = BuiltCatalogs {
        prints: &[],
        entries: &[],
        decoded: &[],
    };

    /// Made of what a build script wrote.
    pub const fn new(
        prints: &'static [u64],
        entries: &'static [u32],
        decoded: &'static [(&'static str, &'static str)],
    ) -> Self {
        Self {
            prints,
            entries,
            decoded,
        }
    }
}

/// [`BuiltCatalogs`] in a translator: the sources added so far by slot.
#[derive(Debug, Clone)]
pub(crate) struct Built {
    table: &'static BuiltCatalogs,
    sources: Vec<Option<&'static str>>,
}

impl Built {
    pub(crate) fn new(table: &'static BuiltCatalogs) -> Self {
        Self {
            table,
            sources: alloc::vec![None; table.prints.len()],
        }
    }

    /// Takes `source` into its slot, where the table holds it.
    pub(crate) fn take(&mut self, source: &'static str) -> bool {
        let print = fingerprint(source);
        let Some(slot) = self.table.prints.iter().position(|p| *p == print) else {
            return false;
        };
        self.sources[slot] = Some(source);
        true
    }

    fn key_cmp(&self, entry: u32, key: &str) -> Ordering {
        if entry & DECODED != 0 {
            let (theirs, _) = self.table.decoded[(entry & !DECODED) as usize];
            return theirs.cmp(key);
        }
        let slot = (entry >> OFFSET_BITS) as usize;
        match self.sources.get(slot).copied().flatten() {
            Some(source) => cmp_plain_key(source, entry & ((1 << OFFSET_BITS) - 1), key),
            // Not added (its plugin is not here): compared by nothing;
            // the build script checks that keys never collide, so such an
            // entry is skipped by the search below.
            None => Ordering::Less,
        }
    }

    /// The message for `key`, where its plugin's source was added.
    pub(crate) fn get(&self, key: &str) -> Option<&'static str> {
        let entries = self.table.entries;
        let (mut low, mut high) = (0, entries.len());
        while low < high {
            let middle = low + (high - low) / 2;
            let entry = entries[middle];
            // An entry of a source not added: look at its neighbours'
            // keys instead (entries are sorted across all sources).
            let found = if entry & DECODED == 0
                && self
                    .sources
                    .get((entry >> OFFSET_BITS) as usize)
                    .copied()
                    .flatten()
                    .is_none()
            {
                return self.get_scanning(key, low, high);
            } else {
                self.key_cmp(entry, key)
            };
            match found {
                Ordering::Less => low = middle + 1,
                Ordering::Greater => high = middle,
                Ordering::Equal => return self.message(entry),
            }
        }
        None
    }

    /// The same, through `entries[low..high]` one by one: only where the
    /// search met a source not added (a plugin built in but left out).
    fn get_scanning(&self, key: &str, low: usize, high: usize) -> Option<&'static str> {
        self.table.entries[low..high]
            .iter()
            .find(|&&entry| {
                let added = entry & DECODED != 0
                    || self
                        .sources
                        .get((entry >> OFFSET_BITS) as usize)
                        .copied()
                        .flatten()
                        .is_some();
                added && self.key_cmp(entry, key).is_eq()
            })
            .and_then(|&entry| self.message(entry))
    }

    fn message(&self, entry: u32) -> Option<&'static str> {
        if entry & DECODED != 0 {
            return Some(self.table.decoded[(entry & !DECODED) as usize].1);
        }
        let source = self
            .sources
            .get((entry >> OFFSET_BITS) as usize)
            .copied()??;
        plain_value(source, entry & ((1 << OFFSET_BITS) - 1))
    }

    pub(crate) fn len(&self) -> usize {
        self.table.entries.len()
    }

    pub(crate) fn index_on_heap(&self) -> usize {
        self.sources.len() * core::mem::size_of::<Option<&str>>()
    }
}

/// [`BuiltCatalogs`] being made by a build script: the English catalogs
/// added together ([`BuiltTable::add`]), written as Rust
/// ([`BuiltTable::to_rust`]).
#[derive(Debug, Default)]
pub struct BuiltTable {
    prints: Vec<u64>,
    /// Key, then the entry.
    entries: Vec<(String, u32)>,
    decoded: Vec<(String, String)>,
}

impl BuiltTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the English catalog `source`; `false`, leaving it out (it then
    /// stays a catalog of its own), where it is not English, holds a key
    /// another source added holds, or its slot or an offset would not fit.
    pub fn add(&mut self, source: &str) -> Result<bool, CatalogError> {
        let catalog = Catalog::parse(String::from(source))?;
        let print = fingerprint(source);
        if catalog.language() != "en" || self.prints.contains(&print) {
            return Ok(false);
        }
        let slot = self.prints.len() as u32;
        if slot >= (DECODED >> OFFSET_BITS) || source.len() >= 1 << OFFSET_BITS {
            return Ok(false);
        }
        let mut added = Vec::new();
        let mut decoded = Vec::new();
        for key in catalog.keys() {
            if self
                .entries
                .binary_search_by(|(k, _)| k.as_str().cmp(key))
                .is_ok()
            {
                return Ok(false);
            }
            match catalog.plain_entry(key) {
                Some(offset) => added.push((String::from(key), slot << OFFSET_BITS | offset)),
                None => {
                    let message = catalog.get(key).unwrap_or_default();
                    decoded.push((String::from(key), String::from(message)));
                }
            }
        }
        self.prints.push(print);
        for (key, message) in decoded {
            let entry = DECODED | self.decoded.len() as u32;
            self.decoded.push((key.clone(), message));
            added.push((key, entry));
        }
        self.entries.extend(added);
        self.entries.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
        Ok(true)
    }

    /// The table as a Rust expression of [`BuiltCatalogs`], for
    /// `include!`.
    pub fn to_rust(&self) -> String {
        use core::fmt::Write;
        let mut out = String::from("paperpaw_i18n::BuiltCatalogs::new(&[");
        for print in &self.prints {
            let _ = write!(out, "{print:#x},");
        }
        out.push_str("], &[");
        for (_, entry) in &self.entries {
            let _ = write!(out, "{entry:#x},");
        }
        out.push_str("], &[");
        for (key, message) in &self.decoded {
            let _ = write!(out, "({key:?}, {message:?}),");
        }
        out.push_str("])");
        out
    }

    /// The table as [`BuiltCatalogs`] at run time, its memory never given
    /// back: for tests and tools (a program writes it with
    /// [`to_rust`](Self::to_rust)).
    pub fn leak(self) -> &'static BuiltCatalogs {
        let prints = alloc::boxed::Box::leak(self.prints.into_boxed_slice());
        let entries: Vec<u32> = self.entries.into_iter().map(|(_, entry)| entry).collect();
        let entries = alloc::boxed::Box::leak(entries.into_boxed_slice());
        let decoded: Vec<(&'static str, &'static str)> = self
            .decoded
            .into_iter()
            .map(|(key, message)| {
                (
                    &*alloc::boxed::Box::leak(key.into_boxed_str()),
                    &*alloc::boxed::Box::leak(message.into_boxed_str()),
                )
            })
            .collect();
        let decoded = alloc::boxed::Box::leak(decoded.into_boxed_slice());
        alloc::boxed::Box::leak(alloc::boxed::Box::new(BuiltCatalogs::new(
            prints, entries, decoded,
        )))
    }

    /// Sources added.
    pub fn len(&self) -> usize {
        self.prints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prints.is_empty()
    }
}
