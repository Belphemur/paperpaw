//! Which files are pictures, and in what order they are read.

use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use paperpaw_sdk::Storage;

use crate::{MAX_NAME, MAX_PAGES};

/// Whether `name` is a picture PaperPaw shows (hidden files, such as the
/// `._` copies macOS leaves on cards, are not).
pub(crate) fn is_picture(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !lower.starts_with('.')
        && [".jpg", ".jpeg", ".png", ".gif"]
            .iter()
            .any(|extension| lower.ends_with(extension))
}

/// Names in reading order: numbers within them by value, so `2.jpg` comes
/// before `10.jpg`; letters without regard to case.
pub(crate) fn natural(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, _) => return Ordering::Less,
            (_, None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let digits = |s: &[u8]| s.iter().take_while(|c| c.is_ascii_digit()).count();
                let (n, m) = (digits(a), digits(b));
                let zeros = |s: &[u8]| s.iter().take_while(|&&c| c == b'0').count();
                let (x0, y0) = (zeros(&a[..n]).min(n - 1), zeros(&b[..m]).min(m - 1));
                let (x, y) = (&a[x0..n], &b[y0..m]);
                let order = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[n..];
                b = &b[m..];
            }
            (Some(x), Some(y)) => {
                let order = x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase());
                if order != Ordering::Equal {
                    return order;
                }
                a = &a[1..];
                b = &b[1..];
            }
        }
    }
}

/// The order of the pages: by [`natural`], names that read alike by their
/// letters, so it is total.
fn order(a: &str, b: &str) -> Ordering {
    natural(a, b).then_with(|| a.cmp(b))
}

/// A page: the name of its file, and its size.
pub(crate) type Page = (String, u64);

/// The pictures of the folder `dir` that come from the picture `first` on
/// (it first), in reading order: at most [`MAX_PAGES`], the first of them,
/// whatever the folder holds. Names wait in no more than twice that many
/// pieces of memory; a folder that cannot be read has `first` alone.
pub(crate) fn pages_from(storage: &dyn Storage, dir: &str, first: &Page) -> Vec<Page> {
    let mut pages: Vec<Page> = Vec::new();
    let keep = |pages: &mut Vec<Page>| {
        pages.sort_by(|a, b| order(&a.0, &b.0));
        pages.truncate(MAX_PAGES);
    };
    let listed = storage.list(dir, &mut |entry| {
        if entry.is_dir
            || entry.name.len() > MAX_NAME
            || !is_picture(entry.name)
            || order(entry.name, &first.0) == Ordering::Less
            || entry.name == first.0
        {
            return;
        }
        pages.push((String::from(entry.name), entry.size));
        if pages.len() > 2 * MAX_PAGES {
            keep(&mut pages);
        }
    });
    if listed.is_err() {
        pages.clear();
    }
    pages.push(first.clone());
    keep(&mut pages);
    pages
}
