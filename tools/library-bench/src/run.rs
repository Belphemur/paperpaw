//! The operations, as the library's views do them (`plugins/files`): the
//! walk that makes the catalog, the index of the list, a page of it, and
//! the passes over the whole catalog of the letters, the shelves and the
//! search (`list/find.rs`, `list/shelf.rs`), mirrored here.

use std::path::Path;

use paperpaw_platform::MemoryClass;
use paperpaw_sdk::Env;
use paperpaw_sdk::library::catalog::{self, Record, Show};
use paperpaw_sdk::library::index::{self, letter, sort_key, surname};
use paperpaw_sdk::library::sorted::{self, Sorted, View};
use paperpaw_sdk::library::{self, index::Index};

use super::{copy, measure, report};

const CATALOG: &str = ".paperpaw/library/catalog.txt";

/// Every operation, reported a line each; only the walks with `walks`.
pub(super) fn all(env: &Env<'_>, more: Option<&str>, walks: bool) {
    walk(env, "walk: first catalog (every book opened)");
    walk(env, "walk: again, nothing changed");
    if walks {
        if let Some(more) = more {
            add(env, more);
        }
        return;
    }
    let size = env.storage().size(CATALOG).unwrap_or(0);
    println!("catalog on the card\t{:.1} KiB", size as f64 / 1024.0);
    list(env);
    sorted_list(env);
    passes(env);
    search(env);
    if let Some(more) = more {
        add(env, more);
    }
    split(env);
}

/// Steps of work in one kernel call at most (`work::MAX_ROUNDS`): what a
/// call read is forgotten after it (`recall`), so a walk runs over many
/// calls, never one.
const STEPS_A_CALL: u64 = 4;

/// The catalog walked to its end, a few steps a kernel call.
fn walk(env: &Env<'_>, what: &str) {
    let (steps, took) = measure(|| {
        let mut walk = catalog::Walk::start(env, Vec::new())?;
        let mut steps = 0u64;
        while !walk.step(env)? {
            steps += 1;
            if steps.is_multiple_of(STEPS_A_CALL) {
                // The call ends: what it read is forgotten, as removing a
                // folder makes the kernel do (no such folder is there).
                let _ = env.storage().remove_dir_all(".paperpaw/bench-call");
            }
        }
        Ok::<_, paperpaw_sdk::StorageError>((steps, walk.found(), walk.from_old()))
    });
    let note = match steps {
        Ok((steps, found, old)) => format!("{steps} steps, {found} books, {old} from the old"),
        Err(e) => format!("failed: {e:?}"),
    };
    report(what, &took, &note);
}

/// The list: its index (as the X4 holds it, and whole), ordered, and a
/// page's lines read.
fn list(env: &Env<'_>) {
    let library = library::service(env);
    let small = index::cap(MemoryClass::Small);
    let (index, took) = measure(|| library.index(env, small, ""));
    let note = format!("{} of {} books held", index.items.len(), index.total);
    report("list: index, X4 cap", &took, &note);
    let (whole, took) = measure(|| library.index(env, index::cap(MemoryClass::Large), ""));
    let note = format!("{} of {} books held", whole.items.len(), whole.total);
    report("list: index, every book", &took, &note);
    let (_, took) = measure(|| whole.order(Show::All, true));
    report("list: order by recent", &took, "every book");
    let (order, took) = measure(|| whole.order(Show::All, false));
    report("list: order by title", &took, "every book");
    let (lines, took) = measure(|| page(env, &whole, &order));
    report(
        "list: a page's lines read",
        &took,
        &format!("{lines} lines"),
    );
    let (lines, took) = measure(|| page(env, &whole, &order[order.len() / 2..]));
    report(
        "list: a page in the middle",
        &took,
        &format!("{lines} lines"),
    );
}

/// The list as every device has it now: the books sorted on the card
/// (made as the kernel makes them), opened, paged, a book found.
fn sorted_list(env: &Env<'_>) {
    let (steps, took) = measure(|| {
        let mut build = sorted::Build::start(env, "").ok()?;
        let mut steps = 0u64;
        while !build.step(env).ok()? {
            steps += 1;
        }
        Some(steps)
    });
    let note = steps.map_or_else(|| String::from("failed"), |s| format!("{s} steps"));
    report("sorted: made (kernel)", &took, &note);
    let (opened, took) = measure(|| Sorted::open(env, ""));
    let Some(sorted) = opened else {
        report("sorted: opened", &took, "failed");
        return;
    };
    report(
        "sorted: opened (read through)",
        &took,
        &format!("{} books", sorted.total()),
    );
    let storage = env.storage();
    for (what, view) in [
        ("by recent", View::new(Show::All, true, false)),
        ("by title", View::new(Show::All, false, false)),
        ("covers, stacked", View::new(Show::All, true, true)),
    ] {
        let len = sorted.len(view);
        let (page, took) = measure(|| sorted.page(storage, view, 0, 10));
        report(
            &format!("sorted: first page, {what}"),
            &took,
            &format!("{} of {len}", page.len()),
        );
        let (page, took) = measure(|| sorted.page(storage, view, len / 2, 10));
        report(
            &format!("sorted: middle page, {what}"),
            &took,
            &format!("{} of {len}", page.len()),
        );
        let (page, took) = measure(|| sorted.page(storage, view, len.saturating_sub(10), 10));
        report(
            &format!("sorted: last page, {what}"),
            &took,
            &format!("{} of {len}", page.len()),
        );
        if let Some(entry) = page.last() {
            let (at, took) = measure(|| sorted.find(storage, view, entry.item.hash));
            report(
                &format!("sorted: find a book, {what}"),
                &took,
                &format!("at {at:?}"),
            );
        }
    }
    let (next, took) = measure(|| sorted.next_in_series(storage, true));
    report(
        "sorted: next in series",
        &took,
        &format!("{} series", next.len()),
    );
}

/// The lines of the first ten books of `order`.
fn page(env: &Env<'_>, index: &Index, order: &[u16]) -> usize {
    order
        .iter()
        .take(10)
        .filter_map(|&i| index.record(env.storage(), usize::from(i)))
        .count()
}

/// The passes over the whole catalog: bare, the letters, the authors,
/// series and folders of the shelves.
fn passes(env: &Env<'_>) {
    let library = library::service(env);
    let (count, took) = measure(|| {
        let mut count = 0usize;
        library.each(env, &mut |_, _| count += 1);
        count
    });
    report(
        "pass: every line, nothing done",
        &took,
        &format!("{count} lines"),
    );
    let (letters, took) = measure(|| letters(env));
    report(
        "find: the letters some book starts with",
        &took,
        &format!("{letters} letters"),
    );
    let (shelves, took) = measure(|| shelves(env));
    let note = format!(
        "{} authors, {} series, {} folders",
        shelves.0, shelves.1, shelves.2
    );
    report("shelves: authors, series, folders", &took, &note);
}

/// `list/find.rs`: which letters have books, by title or surname.
fn letters(env: &Env<'_>) -> usize {
    const LETTERS: &[u8; 27] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ#";
    let mut has = [false; 27];
    library::service(env).each(env, &mut |_, line| {
        let by_title = match sort_key(&line.title)[0] {
            b @ b'a'..=b'z' => b.to_ascii_uppercase(),
            _ => b'#',
        };
        let by_author = letter(surname(&line.author));
        for (n, l) in LETTERS.iter().enumerate() {
            has[n] |= *l == by_title || (*l == by_author && by_author != b'#');
        }
    });
    has.iter().filter(|h| **h).count()
}

/// `list.rs` `make_shelves`, simplified: the authors, series and top
/// folders met, each once (kept sorted to find them).
fn shelves(env: &Env<'_>) -> (usize, usize, usize) {
    let mut seen: [Vec<String>; 3] = Default::default();
    library::service(env).each(env, &mut |_, line| {
        let folder = line.path.split('/').nth(1).unwrap_or_default().to_string();
        for (names, name) in seen.iter_mut().zip([&line.author, &line.series, &folder]) {
            if let Err(at) = names.binary_search(name) {
                names.insert(at, name.clone());
            }
        }
    });
    (seen[0].len(), seen[1].len(), seen[2].len())
}

/// `list/shelf.rs` `find` for a search: the words in title, author,
/// series and file name; then by title.
fn found(env: &Env<'_>, query: &str) -> usize {
    let words = catalog::query_words(query);
    let mut keys = Vec::new();
    library::service(env).each(env, &mut |offset, line: Record| {
        let text = format!(
            "{} {} {} {}",
            line.title,
            line.author,
            line.series,
            paperpaw_sdk::files::name(&line.path)
        )
        .to_lowercase();
        if !words.is_empty() && words.iter().all(|w| text.contains(w.as_str())) {
            keys.push((sort_key(&line.title), offset));
        }
    });
    keys.sort_unstable();
    keys.len()
}

/// Searches as typed, and what an index of the titles' first letters held
/// in memory would take for a word's start.
fn search(env: &Env<'_>) {
    for query in ["night", "emma", "zimmermann", "анна", "the glass", "xyzzy"] {
        let (n, took) = measure(|| found(env, query));
        report(
            &format!("search: \"{query}\""),
            &took,
            &format!("{n} found"),
        );
    }
    let whole = library::service(env).index(env, index::cap(MemoryClass::Large), "");
    let (n, took) = measure(|| {
        let start = sort_key("night");
        whole
            .items
            .iter()
            .filter(|i| i.key[..3] == start[..3])
            .count()
    });
    report(
        "search: title start in the index (no card)",
        &took,
        &format!("{n} found"),
    );
}

/// Books copied onto the card later, then the catalog walked again.
fn add(env: &Env<'_>, more: &str) {
    match copy(env.storage(), &Path::new(more).join("books"), "books/added") {
        Ok(n) => {
            walk(env, &format!("walk: after {n} books added"));
        }
        Err(e) => println!("adding books failed: {e}"),
    }
}

/// The library's files and the caches of books laid out as earlier
/// versions kept them (one folder each), then moved into the folders
/// split by keys as the kernel does once ([`library::split`]).
fn split(env: &Env<'_>) {
    let storage = env.storage();
    let (moved, took) = measure(|| unsplit(env));
    report(
        "earlier layout made (not timed on a device)",
        &took,
        &format!("{moved} moved back"),
    );
    let (done, took) = measure(|| {
        let mut split = library::split::Split::new();
        let mut steps = 0u64;
        while !split.step(storage, env.book_data()).ok()? {
            steps += 1;
        }
        Some((steps, split.moved()))
    });
    let note = match done {
        Some((steps, moved)) => format!("{steps} steps, {moved} moved or removed"),
        None => String::from("failed"),
    };
    report("split: earlier layout moved once", &took, &note);
}

/// Moves what is kept in the parts of keys back into the folder itself:
/// details and covers into the folders earlier versions had; how many.
fn unsplit(env: &Env<'_>) -> usize {
    let storage = env.storage();
    let mut folders: Vec<(String, String)> = vec![
        (
            String::from(".paperpaw/library/details"),
            String::from(".paperpaw/library/books"),
        ),
        (
            String::from(".paperpaw/library/thumbs"),
            String::from(".paperpaw/library/covers"),
        ),
    ];
    for data in env.book_data() {
        if data.kind == library::BookDataKind::Cache {
            folders.push((data.folder.to_string(), data.folder.to_string()));
        }
    }
    let mut moved = 0;
    for (new, old) in &folders {
        let _ = storage.create_dir(old);
        let mut parts = Vec::new();
        let _ = storage.list(new, &mut |e| {
            if e.is_dir && e.name.len() == 2 {
                parts.push(e.name.to_string());
            }
        });
        for part in parts {
            let dir = format!("{new}/{part}");
            let mut names = Vec::new();
            let _ = storage.list(&dir, &mut |e| names.push(e.name.to_string()));
            for name in names {
                if storage
                    .rename(&format!("{dir}/{name}"), &format!("{old}/{name}"))
                    .is_ok()
                {
                    moved += 1;
                }
            }
            let _ = storage.remove_dir_all(&dir);
        }
    }
    moved
}
