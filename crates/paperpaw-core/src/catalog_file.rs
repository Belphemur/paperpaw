//! Compiled catalogs on the storage, read as needed (ADR 0033).

use alloc::rc::Rc;
use alloc::string::String;

use paperpaw_i18n::CatalogFile;
use paperpaw_platform::Storage;

/// A file of the storage that a catalog reads its messages from.
pub(crate) struct StoredCatalog {
    storage: Rc<dyn Storage>,
    path: String,
    size: u32,
}

impl StoredCatalog {
    /// `path` of `storage`; `None` where it is not there or larger than
    /// a catalog's places reach.
    pub(crate) fn new(storage: Rc<dyn Storage>, path: String) -> Option<StoredCatalog> {
        let size = u32::try_from(storage.size(&path).ok()?).ok()?;
        Some(StoredCatalog {
            storage,
            path,
            size,
        })
    }
}

impl CatalogFile for StoredCatalog {
    fn read_exact_at(&self, offset: u32, buf: &mut [u8]) -> bool {
        let mut at = 0;
        while at < buf.len() {
            let Some(rest) = buf.get_mut(at..) else {
                return false;
            };
            match self
                .storage
                .read_at(&self.path, u64::from(offset) + at as u64, rest)
            {
                Ok(0) | Err(_) => return false,
                Ok(n) => at += n,
            }
        }
        true
    }

    fn size(&self) -> u32 {
        self.size
    }
}
