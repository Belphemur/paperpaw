//! Entries of a damaged folder: one whose cluster holds something else
//! (another file's data, or what was there before), read as entries with
//! names no file system gives and sizes past the card. They cannot be
//! named, so a folder removed takes them out where they are found, their
//! data left as it is: it may be another file's, freed it would be given
//! to a third. The clusters stay taken until the card is checked on a
//! computer; the folder goes.

use super::*;

/// Whether `name`, as rust-fatfs reads an entry's name, is none a file
/// system gives: bytes it cannot read (U+FFFD), control characters, or
/// none a long name may hold.
pub(super) fn is_damaged_name(name: &str) -> bool {
    name.contains('\u{FFFD}')
        || name.chars().any(char::is_control)
        || !fatfs::is_valid_long_name(name)
}

/// Whether `entry`, named `name`, is one of a damaged folder: its name
/// (see [`is_damaged_name`]), its first cluster or its size past the card.
pub(super) fn is_damaged<IO: ReadWriteSeek + 'static>(
    fs: &FileSystem<IO>,
    entry: &FatEntry<IO>,
    name: &str,
) -> bool {
    let clusters = fs.total_clusters();
    // Clusters are numbered from 2.
    let past = entry
        .first_cluster()
        .is_some_and(|first| first < 2 || first - 2 >= clusters);
    let card = u64::from(clusters) * u64::from(fs.cluster_size());
    is_damaged_name(name) || past || entry.len() > card
}

impl<IO: ReadWriteSeek + 'static> FatStorage<IO> {
    /// Takes out of folder `dir` what is left in it after its entries
    /// were removed by name: entries of a damaged folder and ones of
    /// names PaperPaw does not take (made elsewhere, `nul.txt`), each
    /// where it is found. A file that is not damaged is removed with its
    /// data; others leave theirs (see the [module documentation](self)).
    /// Whether one was damaged.
    pub(super) fn sweep(&self, dir: &FatDir<IO>) -> Result<bool, StorageError> {
        let mut left = Vec::new();
        for entry in entries(dir) {
            let entry = entry?;
            let name = entry.file_name();
            if name != "." && name != ".." {
                left.push((is_damaged(self.fs, &entry, &name), entry));
            }
        }
        // Marked removed in their places: the others stay where found.
        for (damaged, entry) in &left {
            if *damaged || entry.is_dir() {
                dir.unlink_found(entry).map_err(error)?;
            } else {
                dir.remove_found(entry).map_err(error)?;
            }
        }
        Ok(left.iter().any(|(damaged, _)| *damaged))
    }
}
