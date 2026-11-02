//! A folder listed a piece at a time ([`Storage::list_from`]): a piece
//! goes on where the last one stopped, not from the folder's start, so a
//! folder of thousands of books is read about twice in all, not once a
//! piece. A place says where its entry begins in the folder (its first
//! slot: rust-fatfs lists from there, `Dir::iter_from`) and how many
//! entries were shown before it; where the entry that began there is gone
//! and another one was written over its slots, the place is found by
//! that count from the start instead. See the [crate documentation](crate).

use super::*;

/// The `DirEntry` of `entry` in folder `dir`, to `visit`, where it is
/// shown: not `.` and `..`, nor the storage's own files (see `check`).
pub(super) fn shown<IO: ReadWriteSeek + 'static, T>(
    dir: &str,
    entry: &FatEntry<IO>,
    visit: &mut dyn FnMut(DirEntry<'_>) -> T,
) -> Option<T> {
    let name = entry.file_name();
    let own = is_temp_name(&name) || (dir.is_empty() && same_path(&name, JOURNAL));
    if name == "." || name == ".." || own {
        return None;
    }
    let is_dir = entry.is_dir();
    let modified = Some(seconds(entry.modified()));
    Some(visit(
        DirEntry::new(&name, is_dir, if is_dir { 0 } else { entry.len() }).with_modified(modified),
    ))
}

/// Bytes of a slot of a folder.
const SLOT: u64 = 32;

/// The place of `entry`, `before` entries shown before it: the count in
/// the high half, its first slot in the low one (a folder holds at most
/// [`ENTRIES`] of either).
fn place<IO: ReadWriteSeek + 'static>(before: u64, entry: &FatEntry<IO>) -> u64 {
    let half = u64::from(u32::MAX);
    before.min(half) << 32 | (entry.offset_range().0 / SLOT).min(half)
}

/// Visits the entries of `items` that are shown, `before` shown before
/// them, until `visit` refuses one; its place, or `None` at the end.
/// Every entry is met by `read`, where given, also after one was refused.
fn visit_on<IO: ReadWriteSeek + 'static>(
    dir: &str,
    items: impl Iterator<Item = Result<FatEntry<IO>, StorageError>>,
    mut before: u64,
    mut read: Option<&mut Scan>,
    visit: &mut dyn FnMut(DirEntry<'_>) -> bool,
) -> Result<Option<u64>, StorageError> {
    let mut stopped = None;
    for entry in items {
        let entry = entry?;
        if let Some(read) = read.as_deref_mut() {
            read.met(&entry);
        }
        if stopped.is_some() {
            if read.is_none() {
                break;
            }
            continue;
        }
        match shown(dir, &entry, visit) {
            Some(true) => before += 1,
            Some(false) => stopped = Some(place(before, &entry)),
            None => {}
        }
    }
    Ok(stopped)
}

impl<IO: ReadWriteSeek + 'static> FatStorage<IO> {
    /// [`Storage::list_from`]. From the start, the folder is read whole
    /// (the entries after the piece only looked at), as `list` does: a
    /// big one is noted for the lookups of its names that follow (see
    /// [`found::BIG`]).
    pub(super) fn list_piece(
        &self,
        dir: &str,
        from: u64,
        visit: &mut dyn FnMut(DirEntry<'_>) -> bool,
    ) -> Result<Option<u64>, StorageError> {
        if !dir.is_empty() {
            check(dir)?;
        }
        // The sizes in the folders, as appended.
        self.sync()?;
        let folder = self.dir(dir)?;
        if from == 0 {
            let mut read = Scan::of(&folder);
            let stopped = visit_on(dir, entries(&folder), 0, Some(&mut read), visit)?;
            read.note(&folder);
            return Ok(stopped);
        }
        let (before, slot) = (from >> 32, from & u64::from(u32::MAX));
        if folder.begins_entry(slot * SLOT).map_err(error)? {
            let after = folder.iter_from(slot * SLOT).map_err(error)?;
            return visit_on(
                dir,
                bounded(after.map(|e| e.map_err(error))),
                before,
                None,
                visit,
            );
        }
        // Its entry's slots taken by another: as many shown skipped.
        let mut items = entries(&folder);
        let mut skip = before;
        while skip > 0 {
            match items.next() {
                Some(Ok(entry)) => {
                    if shown(dir, &entry, &mut |_| ()).is_some() {
                        skip -= 1;
                    }
                }
                Some(Err(e)) => return Err(e),
                None => return Ok(None),
            }
        }
        visit_on(dir, items, before, None, visit)
    }
}
