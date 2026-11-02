//! Folders listed a piece at a time (`Storage::list_from`): the same
//! entries as listed whole, each piece going on where the last stopped
//! (a big folder read about twice in all, not once a piece), and never
//! an entry that is not there where the folder changed between pieces.

use super::*;

/// A card with `count` files of long names in folder `big` (FAT32, as
/// the X4's cards; clusters of one block, so the folder's chain is long).
fn big(count: usize) -> Cut {
    let options = FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat32)
        .bytes_per_cluster(512);
    let card = Cut::formatted_of(40 << 20, options);
    let s = card.storage();
    for n in 0..count {
        s.write(&name(n), b"x").unwrap();
    }
    s.sync().unwrap();
    card
}

/// The path of the `n`th file: names of different lengths, so that their
/// entries take different numbers of slots.
fn name(n: usize) -> String {
    format!("big/{n:04} {}.epub", "book ".repeat(n % 7))
}

/// The names of `dir` as listed whole.
fn whole_list(s: &dyn Storage, dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    s.list(dir, &mut |e| names.push(e.name.to_string()))
        .unwrap();
    names
}

/// The names of `dir` listed `piece` at a time; how many pieces.
fn in_pieces(s: &dyn Storage, dir: &str, piece: usize) -> (Vec<String>, usize) {
    let (mut names, mut pieces, mut from) = (Vec::new(), 0, Some(0));
    while let Some(at) = from {
        pieces += 1;
        let mut taken = 0;
        from = s
            .list_from(dir, at, &mut |e| {
                if taken == piece {
                    return false;
                }
                taken += 1;
                names.push(e.name.to_string());
                true
            })
            .unwrap();
        assert!(pieces <= names.len() + 2, "no progress at {at}");
    }
    (names, pieces)
}

#[test]
fn a_folder_in_pieces_lists_as_whole() {
    let card = big(300);
    let s = card.storage();
    s.write("small.txt", b"x").unwrap();
    for dir in ["big", "", "none-here-yet"] {
        let all = if dir == "none-here-yet" {
            Vec::new()
        } else {
            whole_list(&s, dir)
        };
        for piece in [1, 7, 64, 299, 300, 1000] {
            if dir == "none-here-yet" {
                assert_eq!(
                    s.list_from(dir, 0, &mut |_| true),
                    Err(StorageError::NotFound)
                );
                continue;
            }
            let (names, pieces) = in_pieces(&s, dir, piece);
            assert_eq!(names, all, "{dir:?} by {piece}");
            assert_eq!(
                pieces,
                all.len().div_ceil(piece).max(1),
                "{dir:?} by {piece}"
            );
        }
    }
    // A piece refused at once: the first entry's place, which lists it.
    let at = s.list_from("big", 0, &mut |_| false).unwrap().unwrap();
    let mut again = None;
    s.list_from("big", at, &mut |e| {
        again.get_or_insert(e.name.to_string());
        false
    })
    .unwrap();
    assert_eq!(again.as_deref(), Some(&whole_list(&s, "big")[0][..]));
    let mut first = None;
    let next = s
        .list_from("big", 0, &mut |e| {
            if first.is_some() {
                return false;
            }
            first = Some(e.name.to_string());
            true
        })
        .unwrap()
        .unwrap();
    assert_eq!(s.list_from("big", next, &mut |_| false), Ok(Some(next)));
    assert_eq!(first.as_deref(), Some(&whole_list(&s, "big")[0][..]));
}

/// Blocks read listing a folder of `count`, 20 entries a piece.
fn read_in_pieces(count: usize) -> usize {
    let card = big(count);
    let s = card.storage();
    let before = card.reads.get();
    let (names, _) = in_pieces(&s, "big", 20);
    assert_eq!(names.len(), count);
    card.reads.get() - before
}

/// Blocks read listing it whole once.
fn read_whole(count: usize) -> usize {
    let card = big(count);
    let s = card.storage();
    let before = card.reads.get();
    whole_list(&s, "big");
    card.reads.get() - before
}

#[test]
fn a_big_folder_in_pieces_is_read_about_twice_not_once_a_piece() {
    for count in [400, 1600] {
        let (pieces, once) = (read_in_pieces(count), read_whole(count));
        // The first piece reads it whole (to note it), the others go on.
        assert!(
            pieces <= once * 5 / 2,
            "{count}: {pieces} in pieces, {once} once"
        );
    }
    let (few, many) = (read_in_pieces(400), read_in_pieces(1600));
    assert!(many <= few * 4 * 5 / 4, "{few} then {many}");
}

#[test]
fn a_big_folder_listed_in_pieces_is_noted() {
    let card = big(300);
    let s = card.storage();
    assert_eq!(s.dir("big").unwrap().note(), None);
    in_pieces(&s, "big", 50);
    assert!(s.dir("big").unwrap().note().is_some());
}

/// Between pieces, entries are removed and others written into their
/// slots, of longer names (so that a place may fall inside one's
/// slots): no piece shows an entry that is not there, and every entry
/// there all along is shown.
#[test]
fn a_folder_changed_between_pieces_shows_no_entry_that_is_not_there() {
    for piece in [3, 10, 33] {
        let card = big(120);
        let s = card.storage();
        let (mut shown, mut from, mut round) = (Vec::new(), Some(0), 0);
        let mut changed = Vec::new();
        while let Some(at) = from {
            let mut taken = 0;
            from = s
                .list_from("big", at, &mut |e| {
                    if taken == piece {
                        return false;
                    }
                    taken += 1;
                    shown.push(e.name.to_string());
                    true
                })
                .unwrap();
            // The entry next in line and the one before it go; a longer
            // name may take their slots.
            round += 1;
            for n in [round * piece, round * piece + 1] {
                if n < 120 && s.remove(&name(n)).is_ok() {
                    changed.push(name(n));
                }
            }
            let new = format!("big/new {round} {}.epub", "longer ".repeat(round % 9 + 1));
            s.write(&new, b"y").unwrap();
            changed.push(new);
            assert!(round < 200);
        }
        let there = whole_list(&s, "big");
        for name in &shown {
            let path = format!("big/{name}");
            assert!(
                there.contains(name) || changed.contains(&path),
                "{piece}: {name} shown, never there"
            );
        }
        for n in 0..120 {
            let path = name(n);
            if !changed.contains(&path) {
                let name = &path["big/".len()..];
                assert!(shown.iter().any(|s| s == name), "{piece}: {name} not shown");
            }
        }
    }
}

#[test]
fn entries_begin_at_their_first_slot_only() {
    let card = big(40);
    let s = card.storage();
    let dir = s.dir("big").unwrap();
    let mut inside = 0;
    for entry in dir.iter() {
        let (start, end) = entry.unwrap().offset_range();
        assert_eq!(dir.begins_entry(start).ok(), Some(true), "{start}");
        // Its other slots: of its long name, and its short entry last.
        for at in (start + 32..end).step_by(32) {
            assert_eq!(
                dir.begins_entry(at).ok(),
                Some(false),
                "{at} in {start}..{end}"
            );
            inside += 1;
        }
        assert_eq!(dir.begins_entry(start + 1).ok(), Some(false));
    }
    assert!(inside > 40, "long names take more than a slot");
    // Past the folder's slots: its end.
    assert_eq!(dir.begins_entry(64 << 20).ok(), Some(true));
}

/// A place inside an entry's slots (where the entry that began there was
/// removed and a longer one written over it): the listing goes on from
/// as many entries shown, counted from the start.
#[test]
fn a_place_inside_an_entry_goes_on_by_its_count() {
    let card = big(40);
    let s = card.storage();
    let all = whole_list(&s, "big");
    let dir = s.dir("big").unwrap();
    let entry = dir
        .iter()
        .map(Result::unwrap)
        .find(|e| e.file_name() == all[10])
        .unwrap();
    let (start, end) = entry.offset_range();
    assert!(end - start > 32);
    let inside = 10 << 32 | (start / 32 + 1);
    let mut names = Vec::new();
    let next = s
        .list_from("big", inside, &mut |e| {
            names.push(e.name.to_string());
            true
        })
        .unwrap();
    assert_eq!(next, None);
    assert_eq!(names, all[10..]);
}
