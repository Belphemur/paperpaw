//! The catalog's job, one step at a time: what it does to the library's
//! files.

use paperpaw_sdk::{Env, library};

/// What the kernel does to the catalog: walk the library, or catch up;
/// before either, once, move the books' details and covers that earlier
/// versions kept into the folders split by keys ([`library::split`]).
pub(super) enum CatalogJob {
    Walk(library::catalog::Walk),
    Rewrite(library::catalog::Rewrite),
    Split(library::split::Split),
}

impl CatalogJob {
    pub(super) fn step(&mut self, env: &Env<'_>) -> Result<bool, paperpaw_sdk::StorageError> {
        match self {
            CatalogJob::Walk(walk) => walk.step(env),
            CatalogJob::Rewrite(rewrite) => rewrite.step(env),
            CatalogJob::Split(split) => split.step(env.storage(), env.book_data()),
        }
    }

    pub(super) fn abort(self, storage: &dyn paperpaw_sdk::Storage) {
        match self {
            CatalogJob::Walk(walk) => walk.abort(storage),
            CatalogJob::Rewrite(rewrite) => rewrite.abort(storage),
            // Each move is whole: the next goes on from what is left.
            CatalogJob::Split(_) => {}
        }
    }

    pub(super) fn pause(&mut self) {
        match self {
            CatalogJob::Walk(walk) => walk.pause(),
            CatalogJob::Rewrite(rewrite) => rewrite.pause(),
            CatalogJob::Split(_) => {}
        }
    }
}
