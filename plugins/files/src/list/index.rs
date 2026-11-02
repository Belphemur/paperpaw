//! What the calm list shows (its filters) of the library's books sorted
//! ([`paperpaw_sdk::library::sorted`]): every book of the catalog with
//! what it is sorted and filtered by, how far each is read included,
//! read from the card a page at a time.

use paperpaw_sdk::library::catalog::Show;
pub(crate) use paperpaw_sdk::library::index::{NEVER, hash, letter, sort_key, surname};
use paperpaw_sdk::library::sorted::View;

/// What the library shows: all books, those by how far they are read,
/// the next book of each series being read, or the shelves (favourites,
/// series, folders).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Filter {
    All,
    Reading,
    Unread,
    Finished,
    /// For each series begun, its unread book of the lowest number.
    Next,
    Shelves,
}

impl Filter {
    /// The filters of the pills, in their order ([`Filter::Next`] is
    /// on the shelves there).
    pub(crate) const ALL: [Filter; 5] = [
        Filter::All,
        Filter::Reading,
        Filter::Unread,
        Filter::Finished,
        Filter::Shelves,
    ];

    /// The translation key of its name.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Filter::All => "files.list.filter.all",
            Filter::Reading => "files.list.filter.reading",
            Filter::Unread => "files.list.filter.unread",
            Filter::Finished => "files.list.filter.finished",
            Filter::Next => "files.list.filter.next",
            Filter::Shelves => "files.list.filter.shelves",
        }
    }

    /// The filter `steps` pills on, round at the ends (from Next in your
    /// series as from the shelves, where it is).
    pub(crate) fn step(self, steps: isize) -> Filter {
        let pill = if self == Filter::Next {
            Filter::Shelves
        } else {
            self
        };
        let at = Self::ALL.iter().position(|f| *f == pill).unwrap_or(0) as isize;
        let len = Self::ALL.len() as isize;
        Self::ALL[(at + steps).rem_euclid(len) as usize]
    }

    /// How it lists the sorted books: the books read last first
    /// (`by_recent`), the others by title; as covers (`covers`) all books
    /// with each series as one stack. None for the next books of the
    /// series and the shelves, which are not a view of them.
    pub(crate) fn view(self, by_recent: bool, covers: bool) -> Option<View> {
        let show = match self {
            Filter::Reading => Show::Reading,
            Filter::Unread => Show::Unread,
            Filter::Finished => Show::Finished,
            Filter::Next | Filter::Shelves => return None,
            Filter::All => Show::All,
        };
        Some(View::new(show, by_recent, covers && self == Filter::All))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_go_round() {
        assert_eq!(Filter::All.step(-1), Filter::Shelves);
        assert_eq!(Filter::Shelves.step(1), Filter::All);
        assert_eq!(Filter::Next.step(1), Filter::All);
    }
}
