//! A damaged folder (#388): entries of names no file system gives and
//! sizes past the card, as an X4 card showed after resets. Removed with
//! its folder, each where it is found; what they point at stays.

use super::*;

/// Where the short entry of name `short` (as FAT keeps it) is on `card`:
/// the only one.
fn short_entry(card: &Cut, short: &[u8; 11]) -> usize {
    let image = card.image();
    let mut found = image
        .chunks(32)
        .enumerate()
        .filter(|(_, entry)| &entry[..11] == short)
        .map(|(i, _)| i * 32);
    let at = found.next().expect("no such entry");
    assert_eq!(found.next(), None, "two such entries");
    at
}

/// The first cluster of the entry at `at` (FAT12 or FAT16).
fn first_cluster(card: &Cut, at: usize) -> u16 {
    let image = card.image();
    u16::from_le_bytes([image[at + 26], image[at + 27]])
}

/// Overwrites the entry at `at`: name, attributes, first cluster, size.
fn garble(card: &Cut, at: usize, name: &[u8; 11], attributes: u8, first: u16, size: u32) {
    let mut data = card.data.borrow_mut();
    data[at..at + 11].copy_from_slice(name);
    data[at + 11] = attributes;
    data[at + 26..at + 28].copy_from_slice(&first.to_le_bytes());
    data[at + 28..at + 32].copy_from_slice(&size.to_le_bytes());
}

fn kept() -> Vec<u8> {
    (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect()
}

/// A folder of a book's cache whose entries were overwritten: two point
/// at the clusters of a file kept elsewhere (one taken for a folder),
/// one past the card. The folder and the one above it are removed, the
/// file kept elsewhere stays whole, also once the card is filled.
#[test]
fn a_damaged_folder_is_removed_and_what_it_points_at_stays() {
    let card = Cut::formatted(FormatVolumeOptions::new());
    let s = card.storage();
    s.write("keep/other.bin", &kept()).unwrap();
    for name in ["aaaa.bin", "bbbb.bin", "cccc.bin", "dddd.bin"] {
        s.write(&format!("cache/1e/book/{name}"), b"cached")
            .unwrap();
    }
    s.write("cache/1e/next/toc.dat", b"toc").unwrap();
    s.sync().unwrap();
    let other = first_cluster(&card, short_entry(&card, b"OTHER   BIN"));
    // As the X4 card showed them: bytes FAT does not read, and a size
    // of 2,499,805,184.
    let mut name = [0xbd; 11];
    name[9] = b'@';
    name[10] = 0;
    garble(
        &card,
        short_entry(&card, b"AAAA    BIN"),
        &name,
        0,
        other,
        2_499_805_184,
    );
    garble(
        &card,
        short_entry(&card, b"BBBB    BIN"),
        &[0xbf; 11],
        0x10,
        other,
        64,
    );
    garble(
        &card,
        short_entry(&card, b"CCCC    BIN"),
        b"PAST    BIN",
        0,
        0xfff0,
        10,
    );

    let s = card.storage();
    let mut listed = Vec::new();
    s.list("cache/1e/book", &mut |e| listed.push(e.name.to_string()))
        .unwrap();
    assert_eq!(listed.len(), 4, "{listed:?}");
    assert_eq!(s.remove_dir_all("cache/1e"), Ok(()));
    assert_eq!(s.list("cache/1e", &mut |_| {}), Err(StorageError::NotFound));
    assert_eq!(whole(&s, "keep/other.bin").unwrap(), kept());
    // The card filled: no file is given the clusters kept.
    let fill = vec![0x55; 64 << 10];
    let mut n = 0;
    while s.write(&format!("fill/{n}.bin"), &fill).is_ok() {
        n += 1;
    }
    assert!(n > 10, "{n}");
    let s = card.storage();
    assert_eq!(whole(&s, "keep/other.bin").unwrap(), kept());
}

/// Names PaperPaw does not take, made elsewhere (`nul.txt`): removed with
/// their folder, their clusters freed.
#[test]
fn names_not_taken_go_with_their_folder() {
    let card = Cut::formatted(FormatVolumeOptions::new());
    let s = card.storage();
    s.write("odd/good.txt", b"good").unwrap();
    s.sync().unwrap();
    let free = counted(&s);
    let dir = s.fs.root_dir().open_dir("odd").unwrap();
    let mut file = dir.create_file("nul.txt").unwrap();
    file.write_all(&kept()).unwrap();
    drop(file);
    s.fs.flush().unwrap();
    assert!(counted(&s) < free);
    assert_eq!(s.remove_dir_all("odd"), Ok(()));
    assert_eq!(s.list("odd", &mut |_| {}), Err(StorageError::NotFound));
    let s = card.storage();
    assert!(counted(&s) > free, "{} {free}", counted(&s));
}
