//! What it costs to read Literata's bold from the card instead of the
//! firmware (#69): a bold-heavy page drawn from the pack through a
//! storage that counts its reads, cold (nothing cached) and warm, beside
//! the same pack kept in memory as the firmware keeps its own. The reads
//! are what the SD card is asked for; the host's time says nothing of the
//! card's, which the reads times the X4's cost for one read do.

#![cfg(feature = "outline")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stderr,
    reason = "a measurement that says what it found, and tests panic on purpose"
)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use paperpaw_graphics::Framebuffer;
use paperpaw_platform::{Color, PixelFormat};
use paperpaw_sdk::{DirEntry, Font, FontFace, Fonts, Id, MemoryStorage, Storage, StorageError};
use paperpaw_text::charset::{chars, named};
use paperpaw_text::{OutlineFont, build_pack};

const LITERATA: &[u8] = include_bytes!("../fonts/Literata.ttf");
/// The sizes the pack has (the firmware's reading sizes up to 32 px).
const SIZES: [f32; 4] = [20.0, 24.0, 28.0, 32.0];
const SIZE: f32 = 24.0;

/// A storage that counts the reads asked of it.
struct Counting {
    inner: MemoryStorage,
    reads: Cell<u32>,
    bytes: Cell<u64>,
}

impl Storage for Counting {
    fn read_at(&self, path: &str, offset: u64, buf: &mut [u8]) -> Result<usize, StorageError> {
        let n = self.inner.read_at(path, offset, buf)?;
        self.reads.set(self.reads.get() + 1);
        self.bytes.set(self.bytes.get() + n as u64);
        Ok(n)
    }
    fn size(&self, path: &str) -> Result<u64, StorageError> {
        self.inner.size(path)
    }
    fn write(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.inner.write(path, data)
    }
    fn append(&self, path: &str, data: &[u8]) -> Result<(), StorageError> {
        self.inner.append(path, data)
    }
    fn list(&self, dir: &str, visit: &mut dyn FnMut(DirEntry<'_>)) -> Result<(), StorageError> {
        self.inner.list(dir, visit)
    }
    fn remove(&self, path: &str) -> Result<(), StorageError> {
        self.inner.remove(path)
    }
    fn exists(&self, path: &str) -> bool {
        self.inner.exists(path)
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        self.inner.rename(from, to)
    }
    fn create_dir(&self, path: &str) -> Result<(), StorageError> {
        self.inner.create_dir(path)
    }
    fn remove_dir_all(&self, path: &str) -> Result<(), StorageError> {
        self.inner.remove_dir_all(path)
    }
    fn free_space(&self) -> Option<u64> {
        None
    }
}

/// The bold pack, as `literata-styles` ships it (`tools/literata-packs.py`).
fn bold_pack() -> Vec<u8> {
    let font = OutlineFont::new(LITERATA.to_vec(), &[("wght", 700.0)]).unwrap();
    let latin = chars(named("latin").unwrap());
    let ligatures = chars(named("ligatures").unwrap()).filter(|c| font.has_char(*c));
    build_pack(&font, &SIZES, latin.chain(ligatures))
}

/// A page of bold text: 38 lines of about 36 letters, all of the Latin
/// alphabet in use, some digits and punctuation (a bold-heavy page, such
/// as a glossary or a book set in bold).
fn page() -> Vec<String> {
    let words = "The quick brown fox jumps over the lazy dog while Zoe, Max & Quincy \
        examine 1,024 vexing, rather sharp puzzles; 'Wait!' said Jürgen (naïve) -- \
        Ångström, Çelik and Łukasz agreed: café, déjà vu, über, façade, œuvre."
        .split(' ')
        .collect::<Vec<_>>();
    let mut at = 0;
    (0..38)
        .map(|_| {
            let mut line = String::new();
            while line.len() < 36 {
                line.push_str(words[at % words.len()]);
                line.push(' ');
                at += 1;
            }
            line
        })
        .collect()
}

fn draw(font: paperpaw_sdk::FontHandle<'_>, lines: &[String]) -> std::time::Duration {
    let mut frame = Framebuffer::new(480, 800, PixelFormat::Gray8).unwrap();
    let started = Instant::now();
    let mut canvas = frame.canvas();
    for (n, line) in lines.iter().enumerate() {
        font.draw(
            &mut canvas,
            SIZE,
            10.0,
            30 + 20 * n as i32,
            line,
            Color::BLACK,
        );
    }
    started.elapsed()
}

#[test]
fn a_bold_page_from_the_card_asks_for_few_reads() {
    let bytes = bold_pack();
    let lines = page();

    // In memory, as the firmware keeps its packs.
    let mut resident = Fonts::new(12 * 1024);
    resident
        .add(paperpaw_sdk::FontSpec::new(
            Id::new_static("a.bold"),
            Font::from_data(bytes.clone(), &[]).unwrap(),
        ))
        .unwrap();
    let cold_resident = draw(resident.get("a.bold").unwrap(), &lines);
    let warm_resident = draw(resident.get("a.bold").unwrap(), &lines);

    // From the card, a glyph at a time, through the glyph cache.
    let counting = Rc::new(Counting {
        inner: MemoryStorage::new().with("p/bold.ppfp", bytes),
        reads: Cell::new(0),
        bytes: Cell::new(0),
    });
    let storage: Rc<dyn Storage> = counting.clone();
    let mut fonts = Fonts::new(12 * 1024);
    fonts
        .add_face(FontFace::new(
            Id::new_static("p.bold"),
            "p/bold.ppfp",
            "Literata",
            true,
            false,
        ))
        .unwrap();
    fonts.set_family(Some("Literata"));
    assert!(fonts.load(&storage).is_empty());
    let (open_reads, open_bytes) = (counting.reads.get(), counting.bytes.get());
    let cold = draw(fonts.get("p.bold").unwrap(), &lines);
    let (cold_reads, cold_bytes) = (counting.reads.get(), counting.bytes.get());
    let warm = draw(fonts.get("p.bold").unwrap(), &lines);
    let warm_reads = counting.reads.get() - cold_reads;

    eprintln!(
        "bold page, {} lines at {SIZE} px: opening the pack {open_reads} reads / {open_bytes} B; \
         first page {} reads / {} B; the same page again {warm_reads} reads; \
         host time: resident {cold_resident:?} then {warm_resident:?}, from the card {cold:?} then {warm:?}",
        lines.len(),
        cold_reads - open_reads,
        cold_bytes - open_bytes,
    );
    // A guard, not a promise: a page of the whole alphabet in bold costs
    // some hundred reads the first time (a glyph each, the cache is
    // 12 KB), and none again.
    assert!(
        cold_reads - open_reads < 600,
        "{} reads",
        cold_reads - open_reads
    );
    assert!(warm_reads < 50, "{warm_reads} reads again");
}
