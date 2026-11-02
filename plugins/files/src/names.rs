//! Names met in the catalog (authors, series, tags, folders) held a
//! bounded number at a time, however many there are: the first of them
//! in their order after a name (or the last before one), each with how
//! many books it has, collected in one pass ([`Firsts`]); and how many
//! different names there are, counted up to a bound ([`Distinct`]).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::cmp::Ordering;

use paperpaw_sdk::MemoryClass;

/// Names held at most on devices of little memory (the X4), and on others.
const SMALL: usize = 200;
const LARGE: usize = 2000;
/// Different names counted at most, by their hashes.
const SMALL_COUNTED: usize = 2048;
const LARGE_COUNTED: usize = 20_000;

/// Names held at most on a device of `memory`.
pub(crate) fn cap(memory: MemoryClass) -> usize {
    if memory == MemoryClass::Small {
        SMALL
    } else {
        LARGE
    }
}

/// Different names counted at most on a device of `memory`.
pub(crate) fn counted(memory: MemoryClass) -> usize {
    if memory == MemoryClass::Small {
        SMALL_COUNTED
    } else {
        LARGE_COUNTED
    }
}

/// The order names are shown in: alphabetically in any case, names that
/// differ only in case by their letters.
pub(crate) fn order(a: &str, b: &str) -> Ordering {
    folded(a).cmp(folded(b)).then_with(|| a.cmp(b))
}

fn folded(s: &str) -> impl Iterator<Item = char> + '_ {
    s.chars().flat_map(char::to_lowercase)
}

/// A name and how many books it has, how many of them finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    pub(crate) name: String,
    pub(crate) count: usize,
    pub(crate) finished: usize,
}

/// Where a window of names starts: after a name or at it, or ends before
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Bound {
    /// From the first name.
    First,
    /// The names after this one.
    After(String),
    /// The names from this one on (it included, where it is there).
    From(String),
    /// The names before this one, the last of them.
    Before(String),
    /// The last names.
    Last,
}

impl Bound {
    /// Whether the window is the last names before its bound.
    pub(crate) fn backwards(&self) -> bool {
        matches!(self, Bound::Before(_) | Bound::Last)
    }
}

/// The first names (in [`order`]) within a [`Bound`], at most a number,
/// each counted exactly: a name is taken when it is met first, or
/// passed over for good. A name that would come after the last of a full
/// window is passed over; one before it takes its place, and the last
/// goes (the last only ever moves forward in the order, so a name gone
/// never comes back, and every name kept was taken when it was met
/// first).
#[derive(Debug)]
pub(crate) struct Firsts {
    cap: usize,
    bound: Bound,
    /// In [`order`], or the other way for [`Bound::Before`].
    names: Vec<Named>,
    /// Names were left out past the end (before the start for
    /// [`Bound::Before`]).
    cut: bool,
}

impl Firsts {
    pub(crate) fn new(cap: usize, bound: Bound) -> Firsts {
        Firsts {
            cap: cap.max(1),
            bound,
            names: Vec::new(),
            cut: false,
        }
    }

    /// `a` before `b` in the window's order (backwards before a name).
    fn order(&self, a: &str, b: &str) -> Ordering {
        if self.bound.backwards() {
            order(b, a)
        } else {
            order(a, b)
        }
    }

    /// Whether `name` is within the bound.
    fn within(&self, name: &str) -> bool {
        match &self.bound {
            Bound::First | Bound::Last => true,
            Bound::After(after) => order(name, after) == Ordering::Greater,
            Bound::From(from) => order(name, from) != Ordering::Less,
            Bound::Before(before) => order(name, before) == Ordering::Less,
        }
    }

    /// A book of `name`, `finished` or not.
    pub(crate) fn add(&mut self, name: &str, finished: bool) {
        if !self.within(name) {
            return;
        }
        let at = match self.names.binary_search_by(|n| self.order(&n.name, name)) {
            Ok(at) => {
                let named = &mut self.names[at];
                named.count += 1;
                named.finished += usize::from(finished);
                return;
            }
            Err(at) => at,
        };
        if self.names.len() >= self.cap {
            self.cut = true;
            if at >= self.names.len() {
                return;
            }
            self.names.pop();
        }
        self.names.insert(
            at,
            Named {
                name: name.to_string(),
                count: 1,
                finished: usize::from(finished),
            },
        );
    }

    /// The names, in [`order`], and whether names were left out past the
    /// end (before the start for [`Bound::Before`]).
    pub(crate) fn into_names(self) -> (Vec<Named>, bool) {
        let mut names = self.names;
        if self.bound.backwards() {
            names.reverse();
        }
        (names, self.cut)
    }
}

/// How many different names there are, up to a number (by hashes of
/// them: two names of the same hash count once, which a few thousand
/// names make unlikely).
#[derive(Debug)]
pub(crate) struct Distinct {
    cap: usize,
    hashes: Vec<u32>,
    more: bool,
}

impl Distinct {
    pub(crate) fn new(cap: usize) -> Distinct {
        Distinct {
            cap,
            hashes: Vec::new(),
            more: false,
        }
    }

    pub(crate) fn add(&mut self, name: &str) {
        let hash = paperpaw_sdk::digest::fnv1a64(name) as u32;
        if let Err(at) = self.hashes.binary_search(&hash) {
            if self.hashes.len() < self.cap {
                self.hashes.insert(at, hash);
            } else {
                self.more = true;
            }
        }
    }

    /// How many there are, and whether there are more than that.
    pub(crate) fn count(&self) -> (usize, bool) {
        (self.hashes.len(), self.more)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    /// Every name of `names` in a window of `cap` from `bound`, as
    /// [`Firsts`] keeps them, and as all of them sorted give them.
    fn both(names: &[String], cap: usize, bound: Bound) -> ((Vec<Named>, bool), Vec<Named>) {
        let mut firsts = Firsts::new(cap, bound.clone());
        let mut all = Firsts::new(usize::MAX, Bound::First);
        for (n, name) in names.iter().enumerate() {
            firsts.add(name, n % 3 == 0);
            all.add(name, n % 3 == 0);
        }
        let (all, _) = all.into_names();
        let within: Vec<Named> = all
            .into_iter()
            .filter(|n| Firsts::new(1, bound.clone()).within(&n.name))
            .collect();
        let expected = if bound.backwards() {
            within[within.len().saturating_sub(cap)..].to_vec()
        } else {
            within.into_iter().take(cap).collect()
        };
        (firsts.into_names(), expected)
    }

    /// Names in a scrambled order, many met more than once.
    fn scrambled(count: usize) -> Vec<String> {
        (0..count * 3)
            .map(|n| {
                let k = (n * 7919) % count;
                if k.is_multiple_of(2) {
                    format!("Name {k:04}")
                } else {
                    format!("name {k:04}")
                }
            })
            .collect()
    }

    #[test]
    fn a_window_holds_the_first_names_each_counted_exactly() {
        let names = scrambled(500);
        for bound in [
            Bound::First,
            Bound::After("name 0100".into()),
            Bound::From("Name 0100".into()),
            Bound::Before("Name 0300".into()),
            Bound::Last,
            Bound::After("zzz".into()),
        ] {
            for cap in [1, 7, 50, 499, 600] {
                let ((got, cut), expected) = both(&names, cap, bound.clone());
                assert_eq!(got, expected, "{bound:?} {cap}");
                let within = names
                    .iter()
                    .filter(|n| Firsts::new(1, bound.clone()).within(n))
                    .collect::<alloc::collections::BTreeSet<_>>()
                    .len();
                assert_eq!(cut, within > cap, "{bound:?} {cap}");
            }
        }
    }

    #[test]
    fn names_go_in_order_whatever_their_case() {
        assert_eq!(order("apple", "Banana"), Ordering::Less);
        assert_eq!(
            order("Émile", "Zola"),
            Ordering::Greater,
            "after z, as before"
        );
        assert_eq!(order("ann", "Ann"), Ordering::Greater);
        assert_eq!(order("", "a"), Ordering::Less);
    }

    #[test]
    fn different_names_are_counted_up_to_a_number() {
        let mut distinct = Distinct::new(10);
        for n in 0..30 {
            distinct.add(&format!("author {}", n % 8));
        }
        assert_eq!(distinct.count(), (8, false));
        for n in 0..30 {
            distinct.add(&format!("other {n}"));
        }
        assert_eq!(distinct.count(), (10, true));
    }
}
