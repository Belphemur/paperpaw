//! Lists of any length, a window at a time: a source of entries is read
//! through once for the window that follows the key of the last entry
//! shown, keeping only the window in memory. Memory is the window's, never
//! the vault's.

use alloc::string::String;
use alloc::vec::Vec;

/// Entries of a window.
pub(crate) const WINDOW: usize = 100;

/// An entry of a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    /// What is shown: a note's title, a folder's name.
    pub title: String,
    /// Where it is on the storage.
    pub path: String,
    pub folder: bool,
    /// When it was last changed, if it is known.
    pub modified: i64,
}

/// Where an entry is in the order of a list, so that a window can begin
/// after one: folders first, then by `rank`, then by name.
pub(crate) type Key = (u8, i64, String, String);

/// An entry with its place in the order.
type Placed = (Key, Row);

/// How a list is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    /// Folders first, then notes, each by name.
    Name,
    /// The latest change first.
    Recent,
}

impl Order {
    pub(crate) fn key(self, row: &Row) -> Key {
        let rank = match self {
            Order::Name => 0,
            Order::Recent => -row.modified,
        };
        (
            u8::from(!row.folder),
            rank,
            row.title.to_lowercase(),
            row.path.clone(),
        )
    }
}

/// A window of a list: its entries, whether more follow.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Window {
    pub rows: Vec<Row>,
    pub more: bool,
}

/// What offers the entries of a list: it calls the function it is given
/// once for each.
type Source<'a> = dyn FnMut(&mut dyn FnMut(Row)) + 'a;

/// Collects the window that follows `after` from the entries `source`
/// offers (it calls the function it is given once for each entry of the
/// list, in any order).
pub(crate) fn window(order: Order, after: Option<&Key>, source: &mut Source<'_>) -> Window {
    // The `WINDOW + 1` first entries after `after`, in order: the last
    // only says that there are more.
    let mut kept: Vec<Placed> = Vec::with_capacity(WINDOW + 1);
    source(&mut |row: Row| {
        let key = order.key(&row);
        if after.is_some_and(|after| key <= *after) {
            return;
        }
        if kept.len() > WINDOW && kept.last().is_some_and(|(last, _)| key >= *last) {
            return;
        }
        let at = kept.partition_point(|(k, _)| *k < key);
        if kept.get(at).is_some_and(|(k, _)| *k == key) {
            return;
        }
        kept.insert(at, (key, row));
        kept.truncate(WINDOW + 1);
    });
    let more = kept.len() > WINDOW;
    kept.truncate(WINDOW);
    Window {
        rows: kept.into_iter().map(|(_, row)| row).collect(),
        more,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(title: &str, modified: i64) -> Row {
        Row {
            title: title.into(),
            path: alloc::format!("v/{title}.md"),
            folder: false,
            modified,
        }
    }

    fn titles(w: &Window) -> Vec<&str> {
        w.rows.iter().map(|r| r.title.as_str()).collect()
    }

    #[test]
    fn a_list_is_read_a_window_at_a_time_in_any_order_of_its_source() {
        let all: Vec<Row> = (0..250)
            .rev()
            .map(|n| note(&alloc::format!("n{n:03}"), n))
            .collect();
        let mut from = |emit: &mut dyn FnMut(Row)| all.iter().cloned().for_each(emit);
        let first = window(Order::Name, None, &mut from);
        assert_eq!(first.rows.len(), WINDOW);
        assert!(first.more);
        assert_eq!(titles(&first)[0], "n000");
        let after = Order::Name.key(first.rows.last().unwrap());
        let second = window(Order::Name, Some(&after), &mut from);
        assert_eq!(titles(&second)[0], "n100");
        let after = Order::Name.key(second.rows.last().unwrap());
        let third = window(Order::Name, Some(&after), &mut from);
        assert_eq!((third.rows.len(), third.more), (50, false));
        assert_eq!(titles(&third).last(), Some(&"n249"));
    }

    #[test]
    fn folders_come_first_and_the_latest_change_first_by_recent() {
        let mut rows = alloc::vec![note("b", 1), note("a", 5)];
        rows.push(Row {
            folder: true,
            title: "Zed".into(),
            path: "v/Zed".into(),
            modified: 0,
        });
        let mut from = |emit: &mut dyn FnMut(Row)| rows.iter().cloned().for_each(emit);
        assert_eq!(
            titles(&window(Order::Name, None, &mut from)),
            ["Zed", "a", "b"]
        );
        let recent = window(Order::Recent, None, &mut from);
        assert_eq!(titles(&recent), ["Zed", "a", "b"]);
        let only_notes = |emit: &mut dyn FnMut(Row)| {
            rows.iter().filter(|r| !r.folder).cloned().for_each(emit);
        };
        let mut only_notes = only_notes;
        assert_eq!(
            titles(&window(Order::Recent, None, &mut only_notes)),
            ["a", "b"]
        );
    }

    #[test]
    fn case_does_not_order_and_equal_entries_come_once() {
        let rows = [note("b", 0), note("A", 0), note("a2", 0), note("A", 0)];
        let mut from = |emit: &mut dyn FnMut(Row)| rows.iter().cloned().for_each(emit);
        assert_eq!(
            titles(&window(Order::Name, None, &mut from)),
            ["A", "a2", "b"]
        );
    }
}
