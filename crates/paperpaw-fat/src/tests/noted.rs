//! Folders of many entries read whole are noted: a name is looked for
//! from where the last one was found, so names taken out of such a folder
//! one after the other as it lists them take a few blocks each, not the
//! folder up to them; never where a file a write goes through may be in
//! the folder, nor once an entry was added to it.

use super::*;

/// The name of the `n`th file of a big folder: longer than 8.3, as the
/// library's (`<key>.ini`).
fn long(n: usize) -> String {
    format!("{n:016x}.ini")
}

/// A card with `count` files in folder `big`, as on the X4: of 4 MB (as
/// [`assert_nothing_lost`] takes), or 16 MB for more than a thousand.
fn big(count: usize) -> Cut {
    let size = if count > 1000 { 16 << 20 } else { 4 << 20 };
    let card = Cut::formatted_of(size, FormatVolumeOptions::new());
    let s = card.storage();
    for n in 0..count {
        s.write(&format!("big/{}", long(n)), long(n).as_bytes())
            .unwrap();
    }
    s.sync().unwrap();
    card
}

/// The names of folder `dir`, as it lists them.
fn listed(s: &dyn Storage, dir: &str) -> Vec<String> {
    let mut names = Vec::new();
    s.list(dir, &mut |e| names.push(e.name.to_string()))
        .unwrap();
    names
}

#[test]
fn a_big_folder_read_whole_is_noted_until_an_entry_is_added() {
    let card = big(BIG + 4);
    let s = card.storage();
    assert_eq!(s.dir("big").unwrap().note(), None);
    assert_eq!(listed(&s, "big").len(), BIG + 4);
    assert_eq!(s.dir("big").unwrap().note(), Some(0));
    // Found, in any order, and not found.
    for n in [3, 200, 1, BIG + 3, 0] {
        assert_eq!(
            whole(&s, &format!("big/{}", long(n))),
            Ok(long(n).into_bytes())
        );
    }
    assert_eq!(
        s.size("big/0123456789abcdef.ini"),
        Err(StorageError::NotFound)
    );
    // Removed: still noted.
    s.remove(&format!("big/{}", long(5))).unwrap();
    assert!(s.dir("big").unwrap().note().is_some());
    assert_eq!(
        s.size(&format!("big/{}", long(5))),
        Err(StorageError::NotFound)
    );
    // Written (through files beside it): no longer.
    s.write(&format!("big/{}", long(BIG + 9)), b"new").unwrap();
    assert_eq!(s.dir("big").unwrap().note(), None);
    assert_eq!(
        whole(&s, &format!("big/{}", long(BIG + 9))),
        Ok(b"new".to_vec())
    );
}

#[test]
fn small_folders_and_folders_of_temporary_files_are_not_noted() {
    // With `.` and `..`, one short.
    let card = big(BIG - 3);
    let s = card.storage();
    listed(&s, "big");
    assert_eq!(s.dir("big").unwrap().note(), None);
    // A file a write goes through, left by a cut.
    let card = big(BIG + 1);
    let s = card.storage();
    s.dir("big")
        .unwrap()
        .create_file(Temps::of("x.txt").ready())
        .unwrap();
    listed(&s, "big");
    assert_eq!(s.dir("big").unwrap().note(), None);
}

#[test]
fn a_folder_written_to_while_it_is_listed_is_not_noted() {
    let card = big(BIG + 1);
    let s = card.storage();
    let mut first = true;
    s.list("big", &mut |_| {
        if first {
            first = false;
            s.write("other/a.txt", b"a").unwrap();
        }
    })
    .unwrap();
    assert_eq!(s.dir("big").unwrap().note(), None);
}

/// Blocks read moving the files of a folder of `count` out of it, one
/// after the other as it lists them.
fn read_moving_out(count: usize) -> usize {
    let card = big(count);
    let s = card.storage();
    let names = listed(&s, "big");
    let before = card.reads.get();
    for name in &names {
        let to = format!("split/{}/{name}", &name[14..16]);
        s.rename(&format!("big/{name}"), &to).unwrap();
    }
    let read = card.reads.get() - before;
    for name in &names {
        assert_eq!(
            whole(&s, &format!("split/{}/{name}", &name[14..16])),
            Ok(name.clone().into_bytes())
        );
    }
    assert!(listed(&s, "big").is_empty());
    read
}

#[test]
fn files_moved_out_of_a_big_folder_in_its_order_take_a_few_blocks_each() {
    let (few, many) = (read_moving_out(400), read_moving_out(1200));
    // Each found where the last one was, not by reading the folder up
    // to it: as many blocks a move whatever the folder's size.
    let (per_few, per_many) = (few / 400, many / 1200);
    // (Most of them the folder of the parts, on this card of small
    // clusters, which a folder spans many of.)
    assert!(
        per_many <= per_few * 5 / 4 + 4,
        "{per_few} then {per_many} blocks a move"
    );
}

/// Files moved out of a noted folder, cut off at any point, are whole
/// at their old place or their new one, or at both until the next change
/// settles it; settled, no two files share a cluster and none is lost.
#[test]
fn files_moved_out_of_a_noted_folder_are_found_after_any_cut() {
    let card = big(BIG + 8);
    let image = card.image();
    let names: Vec<String> = (0..3).map(long).collect();
    let cuts = cut_images(image, |s| {
        assert_eq!(listed(s, "big").len(), BIG + 8);
        for name in &names {
            s.rename(&format!("big/{name}"), &format!("split/{name}"))
                .unwrap();
        }
    });
    let last = cuts.len() - 1;
    for (n, image) in cuts.into_iter().enumerate() {
        let s = mounted(image);
        // The next change settles a move cut off.
        s.write("other.txt", b"other").unwrap();
        for name in &names {
            let at = [
                whole(&s, &format!("big/{name}")).ok(),
                whole(&s, &format!("split/{name}")).ok(),
            ];
            let content = Some(name.clone().into_bytes());
            assert!(
                at == [content.clone(), None] || at == [None, content.clone()],
                "{name} after cut {n}: {at:?}"
            );
            if n == last {
                assert_eq!(at, [None, content], "{name}");
            }
        }
        // But a cluster of the folder made for them, cut off while it is.
        assert_nothing_lost(&s, s.fs.cluster_size(), n);
    }
}
