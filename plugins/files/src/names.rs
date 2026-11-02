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
/// Bytes of names held at most (values of up to a kilobyte each make a
/// few names many bytes).
const SMALL_BYTES: usize = 16 * 1024;
const LARGE_BYTES: usize = 256 * 1024;
/// Different names counted at most, by their hashes.
const SMALL_COUNTED: usize = 2048;
const LARGE_COUNTED: usize = 20_000;

/// How many names a window holds at most, and how many bytes of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Budget {
    pub(crate) names: usize,
    pub(crate) bytes: usize,
}

impl Budget {
    /// The budget of a window on a device of `memory`.
    pub(crate) fn of(memory: MemoryClass) -> Budget {
        if memory == MemoryClass::Small {
            Budget {
                names: SMALL,
                bytes: SMALL_BYTES,
            }
        } else {
            Budget {
                names: LARGE,
                bytes: LARGE_BYTES,
            }
        }
    }

    /// A part of it, for one of `windows` held at once.
    pub(crate) fn shared(self, windows: usize) -> Budget {
        Budget {
            names: self.names,
            bytes: self.bytes / windows.max(1),
        }
    }
}

/// The bytes of a name held counted against a [`Budget`] (beside them,
/// a few words a name, bounded by the number of names).
fn held(name: &str) -> usize {
    name.len()
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

/// The first names (in [`order`]) within a [`Bound`], at most a
/// [`Budget`] of them (one, however long, at least), each counted
/// exactly: a name is taken when it is met first, or passed over for
/// good. A name that would come after the last of a full window is passed
/// over; one before it takes its place, and the last go until it fits
/// (the last only ever moves back in the order, so a name gone never
/// comes back, and every name kept was taken when it was met first).
#[derive(Debug)]
pub(crate) struct Firsts {
    budget: Budget,
    /// Bytes of the names held ([`held`]).
    bytes: usize,
    bound: Bound,
    /// In [`order`], or the other way for [`Bound::Before`].
    names: Vec<Named>,
    /// The first name passed over or let go (the last for
    /// [`Bound::Before`]): it and those after it are not taken any more.
    /// Some where names were left out past the end (before the start for
    /// [`Bound::Before`]).
    passed: Option<String>,
}

impl Firsts {
    pub(crate) fn new(budget: Budget, bound: Bound) -> Firsts {
        Firsts {
            budget: Budget {
                names: budget.names.max(1),
                ..budget
            },
            bytes: 0,
            bound,
            names: Vec::new(),
            passed: None,
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
        if let Some(passed) = &self.passed
            && self.order(name, passed) != Ordering::Less
        {
            return;
        }
        let full = |len: usize, bytes: usize| {
            len > 0 && (len >= self.budget.names || bytes + held(name) > self.budget.bytes)
        };
        // Room made: the last go while it does not fit.
        while self.names.len() > at && full(self.names.len(), self.bytes) {
            if let Some(last) = self.names.pop() {
                self.bytes -= held(&last.name);
                self.passed = Some(last.name);
            }
        }
        if full(self.names.len(), self.bytes) {
            self.passed = Some(name.to_string());
            return;
        }
        self.bytes += held(name);
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
        (names, self.passed.is_some())
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

    /// Every name of `names` in a window of `budget` from `bound`, as
    /// [`Firsts`] keeps them, and as all of them sorted give them: as many
    /// as fit from the bound on.
    fn both(names: &[String], budget: Budget, bound: Bound) -> ((Vec<Named>, bool), Vec<Named>) {
        let mut firsts = Firsts::new(budget, bound.clone());
        let mut all = Firsts::new(unbounded(usize::MAX), Bound::First);
        for (n, name) in names.iter().enumerate() {
            firsts.add(name, n % 3 == 0);
            all.add(name, n % 3 == 0);
        }
        let (all, _) = all.into_names();
        let mut within: Vec<Named> = all
            .into_iter()
            .filter(|n| Firsts::new(budget, bound.clone()).within(&n.name))
            .collect();
        if bound.backwards() {
            within.reverse();
        }
        let mut bytes = 0;
        let mut expected: Vec<Named> = within
            .into_iter()
            .enumerate()
            .take_while(|(n, named)| {
                bytes += held(&named.name);
                *n == 0 || (*n < budget.names && bytes <= budget.bytes)
            })
            .map(|(_, named)| named)
            .collect();
        if bound.backwards() {
            expected.reverse();
        }
        (firsts.into_names(), expected)
    }

    /// A budget of `names` names, of any number of bytes.
    fn unbounded(names: usize) -> Budget {
        Budget {
            names,
            bytes: usize::MAX,
        }
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
                let ((got, cut), expected) = both(&names, unbounded(cap), bound.clone());
                assert_eq!(got, expected, "{bound:?} {cap}");
                let within = names
                    .iter()
                    .filter(|n| Firsts::new(unbounded(1), bound.clone()).within(n))
                    .collect::<alloc::collections::BTreeSet<_>>()
                    .len();
                assert_eq!(cut, within > cap, "{bound:?} {cap}");
            }
        }
    }

    #[test]
    fn a_window_holds_no_more_bytes_than_its_budget() {
        // Names of up to a kilobyte, in a scrambled order: the window
        // holds as many as fit, each counted exactly, one at least.
        let names: Vec<String> = scrambled(500)
            .into_iter()
            .enumerate()
            .map(|(n, name)| {
                let long = "x".repeat((n * 31) % 1024);
                format!("{name} {long}")
            })
            .collect();
        for bound in [
            Bound::First,
            Bound::From("Name 0100".into()),
            Bound::Before("Name 0300".into()),
            Bound::Last,
        ] {
            for bytes in [0, 1024, 16 * 1024, 64 * 1024] {
                let budget = Budget { names: 200, bytes };
                let ((got, cut), expected) = both(&names, budget, bound.clone());
                assert_eq!(got, expected, "{bound:?} {bytes}");
                assert!(!got.is_empty(), "{bound:?} {bytes}");
                let held: usize = got.iter().map(|n| held(&n.name)).sum();
                assert!(got.len() == 1 || held <= bytes, "{bound:?} {bytes}: {held}");
                assert!(cut, "{bound:?} {bytes}");
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
