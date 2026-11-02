//! The catalog's job, one step at a time: what it does to the library's
//! files.

use alloc::string::String;

use paperpaw_sdk::builtin::{settings, slots};
use paperpaw_sdk::{Env, library};

/// What the kernel does to the catalog: walk the library, or catch up;
/// before either, once, move the books' details and covers that earlier
/// versions kept into the folders split by keys ([`library::split`]).
pub(super) enum CatalogJob {
    Walk(library::catalog::Walk),
    Rewrite(library::catalog::Rewrite),
    Split(library::split::Split),
    /// The books sorted for the view of them ([`library::sorted`]), after
    /// the catalog was made anew.
    Sort(library::sorted::Build),
}

impl CatalogJob {
    pub(super) fn step(&mut self, env: &Env<'_>) -> Result<bool, paperpaw_sdk::StorageError> {
        match self {
            CatalogJob::Walk(walk) => walk.step(env),
            CatalogJob::Rewrite(rewrite) => rewrite.step(env),
            CatalogJob::Split(split) => split.step(env.storage(), env.book_data()),
            CatalogJob::Sort(sort) => sort.step(env),
        }
    }

    pub(super) fn abort(self, storage: &dyn paperpaw_sdk::Storage) {
        match self {
            CatalogJob::Walk(walk) => walk.abort(storage),
            CatalogJob::Rewrite(rewrite) => rewrite.abort(storage),
            // Each move is whole: the next goes on from what is left.
            CatalogJob::Split(_) => {}
            CatalogJob::Sort(sort) => sort.abort(storage),
        }
    }

    pub(super) fn pause(&mut self, storage: &dyn paperpaw_sdk::Storage) {
        match self {
            CatalogJob::Walk(walk) => walk.pause(),
            CatalogJob::Rewrite(rewrite) => rewrite.pause(),
            CatalogJob::Split(_) => {}
            CatalogJob::Sort(sort) => sort.pause(storage),
        }
    }
}

/// Whether the catalog's job waits (paused, it goes on where it was):
/// while a book is read (the reader is the screen shown, overlays over it
/// aside), its memory and time are the reader's; on a device of little
/// memory (the X4), a walk waits while the library is open (shown, or
/// under the screens it opened: finding, a shelf, a book's information),
/// whose index takes the memory. Catching up, which takes little, goes on
/// beside the library; a library with no catalog yet, or a catalog without
/// books, holds no index.
pub(super) fn waits(env: &Env<'_>, walk: bool) -> bool {
    let shown = env.stack().iter().rev().find(|entry| !entry.overlay);
    let reading = shown.is_some_and(|entry| entry.slot.as_ref() == Some(&slots::READER));
    let library_open = env
        .stack()
        .iter()
        .any(|entry| entry.slot.as_ref() == Some(&slots::LIBRARY));
    reading
        || (walk
            && env.device().memory == paperpaw_sdk::MemoryClass::Small
            && library_open
            && matches!(
                library::catalog::state(env.storage()),
                library::catalog::State::Ready { .. }
            )
            && !(library::emptiness::is_empty(env.storage())
                && !library::emptiness::log_has_entries(env.storage())))
}

/// The library's folder is there from the first start on: made where it
/// is not (never removed), so that books copied onto the card or sent
/// over Wi-Fi have their place from the start.
pub(super) fn make_library_folder(env: &Env<'_>) {
    let folder = env.settings().read(&settings::LIBRARY_FOLDER);
    let folder = folder.trim_matches('/');
    if folder.is_empty() || paperpaw_sdk::files::is_dir(env.storage(), folder) {
        return;
    }
    let mut made = String::new();
    for part in folder.split('/') {
        made = paperpaw_sdk::files::join(&made, part);
        if !paperpaw_sdk::files::is_dir(env.storage(), &made) {
            let _ = env.storage().create_dir(&made);
        }
    }
}

/// The sorted books, for a view leaving an archive out
/// ([`library::sorted`]): made where they are not of the library there
/// is now.
pub(super) fn register_sorted(
    r: &mut paperpaw_sdk::Registrar,
    catalog: &alloc::rc::Rc<super::Catalog>,
) -> Result<(), paperpaw_sdk::RegisterError> {
    let catalog = alloc::rc::Rc::clone(catalog);
    r.on(
        paperpaw_sdk::builtin::events::SORTED_WANTED,
        move |env, wanted, out| {
            *catalog.archive.borrow_mut() = Some(wanted.archive.clone());
            if catalog.sort_wanted(env).is_some() {
                catalog.declare(true, out);
            }
        },
    )
}
