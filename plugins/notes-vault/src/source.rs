//! Where the lists' entries come from: a folder of the vault, as it is on
//! the storage, or the index and the file of tags, a line at a time.

use alloc::string::String;

use paperpaw_sdk::Storage;

use crate::index;
use crate::note::{self, join};
use crate::page::Row;

/// What a list lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// A folder: its folders and notes.
    Folder(String),
    /// The notes of the index: those in `folder` and below it (all, if
    /// empty), whose title has every word of `query` (all, if empty).
    Notes { folder: String, query: String },
    /// The notes that have a tag.
    Tag(String),
}

impl Source {
    /// Calls `emit` once for each entry the source has.
    pub(crate) fn feed(&self, storage: &dyn Storage, emit: &mut dyn FnMut(Row)) {
        match self {
            Source::Folder(dir) => {
                // A folder that cannot be read has no entries.
                storage
                    .list(dir, &mut |entry| {
                        if note::is_hidden(entry.name) {
                            return;
                        }
                        let path = join(dir, entry.name);
                        if entry.is_dir {
                            emit(Row {
                                title: entry.name.into(),
                                path,
                                folder: true,
                                modified: 0,
                            });
                        } else if note::is_note(entry.name) {
                            emit(Row {
                                title: note::title(&path).into(),
                                path,
                                folder: false,
                                modified: entry.modified.unwrap_or(0),
                            });
                        }
                    })
                    .ok();
            }
            Source::Notes { folder, query } => {
                let words: alloc::vec::Vec<String> =
                    query.split_whitespace().map(str::to_lowercase).collect();
                let below = alloc::format!("{}/", folder.trim_matches('/'));
                index::notes(storage, &mut |modified, path| {
                    if !folder.is_empty() && !path.starts_with(&below) {
                        return true;
                    }
                    let title = note::title(path);
                    if !words.is_empty() {
                        let lower = title.to_lowercase();
                        if !words.iter().all(|w| lower.contains(w.as_str())) {
                            return true;
                        }
                    }
                    emit(Row {
                        title: title.into(),
                        path: path.into(),
                        folder: false,
                        modified,
                    });
                    true
                });
            }
            Source::Tag(tag) => index::notes_with_tag(storage, tag, &mut |path| {
                emit(Row {
                    title: note::title(path).into(),
                    path: path.into(),
                    folder: false,
                    modified: 0,
                });
                true
            }),
        }
    }
}
