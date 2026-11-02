//! The calls of shared functions the table mode tests
//! (`../../native_plugin.rs`): run in the plugin (whose shared functions
//! call the shell's copies through the table) and in the test directly
//! (the functions' own bodies), they must come out the same. Included by
//! both with `#[path]`.

#![allow(
    clippy::unwrap_used,
    reason = "a test's probe: the pack and the canvas are the test's own"
)]

extern crate alloc;

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;

use paperpaw_graphics::Framebuffer;
use paperpaw_platform::{Color, PixelFormat};
use paperpaw_text::{
    Around, BitmapFont, Font, GlyphCache, PackSource, StoredPack, TextStyle, draw_text,
    draw_text_in, is_mark, measure, measure_in, wrap_lines_kerned,
};

pub const PACK: &[u8] = include_bytes!("../../../../paperpaw-text/testdata/pack.ppfp.v1");

/// What the calls came to.
#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct TextProbe {
    /// `measure`, a method of `Font` (`&self`), `kern` and `advance`.
    pub width: f32,
    /// `draw_text` (a `&mut` cache, a canvas, a borrowed style): the advance.
    pub advance: f32,
    /// The sum of the canvas's gray levels after it.
    pub gray: u64,
    /// `wrap_lines_kerned` with closures crossing as `dyn`: the lines' ends.
    pub lines: Vec<usize>,
    pub mark: bool,
    /// `measure_in` and `draw_text_in` of a text that needs the bidi code
    /// (right-to-left letters, brackets): its width and ink.
    pub width_in: f32,
    pub gray_in: u64,
    /// Every method of a pack read from a source and of a glyph cache,
    /// once: the handles' tables are the maker's, so items in another
    /// order than the plugin's SDK has them would come out wrong here.
    pub handles: Vec<f32>,
}

/// The pack, read as from a card.
struct Bytes;

impl PackSource for Bytes {
    fn read_at(&self, at: u64, buf: &mut [u8]) -> bool {
        let at = at as usize;
        match PACK.get(at..at + buf.len()) {
            Some(bytes) => {
                buf.copy_from_slice(bytes);
                true
            }
            None => false,
        }
    }
}

/// Calls every wrapper of `StoredPack` and `GlyphCache`.
fn handles() -> Vec<f32> {
    let flag = |b: bool| if b { 1.0 } else { 0.0 };
    let pack = StoredPack::open(Rc::new(Bytes)).unwrap();
    let glyph = pack.render('A', 12.0).unwrap();
    let mut out = alloc::vec![
        pack.largest_size().unwrap_or(0.0),
        flag(pack.has_char('A')),
        flag(pack.has_char('\u{5d0}')),
        pack.line_metrics(12.0).line_height,
        pack.advance('A', 12.0),
        f32::from(glyph.width),
        glyph.advance,
        flag(pack.table(BitmapFont::ANCHORS).is_some()),
        flag(pack.bytes() > 0),
        pack.failed_reads() as f32,
    ];
    pack.trim();
    let font = Font::Stored(pack);
    let mut cache = GlyphCache::new(1024);
    out.push(cache.glyph(0, &font, 'A', 12.0).advance);
    out.push(flag(cache.bytes() > 0));
    cache.set_capacity(2048);
    out.push(cache.capacity() as f32);
    cache.trim(0);
    out.push(cache.bytes() as f32);
    out.push(cache.glyph(0, &font, 'A', 12.0).advance);
    cache.clear();
    out.push(cache.bytes() as f32);
    out
}

pub fn probe() -> TextProbe {
    let font = Font::from_data(PACK, &[]).unwrap();
    let width = measure(&font, 12.0, "AAAA A");
    let mut frame = Framebuffer::new(64, 32, PixelFormat::Gray8).unwrap();
    let advance;
    {
        let mut canvas = frame.canvas();
        canvas.clear(Color::WHITE);
        let mut cache = GlyphCache::new(1024);
        let style = TextStyle::new(&font, 0, 12.0, Color::BLACK).bolder(1);
        advance = draw_text(&mut canvas, &mut cache, &style, 4.0, 20, "AA A");
    }
    let around = Around::NONE;
    let rtl = "A (\u{05d0}\u{05d1}) B";
    let width_in = measure_in(&font, 12.0, rtl, &around);
    let mut again = Framebuffer::new(64, 32, PixelFormat::Gray8).unwrap();
    {
        let mut canvas = again.canvas();
        canvas.clear(Color::WHITE);
        let mut cache = GlyphCache::new(1024);
        let style = TextStyle::new(&font, 0, 12.0, Color::BLACK);
        draw_text_in(&mut canvas, &mut cache, &style, 2.0, 20, rtl, &around);
    }
    let view = again.view();
    let mut gray_in = 0u64;
    for y in 0..32 {
        for x in 0..64 {
            gray_in += u64::from(view.pixel(x, y).r);
        }
    }
    let view = frame.view();
    let mut gray = 0u64;
    for y in 0..32 {
        for x in 0..64 {
            gray += u64::from(view.pixel(x, y).r);
        }
    }
    let lines = wrap_lines_kerned(
        "AAAA AAA AA A AAAAA",
        |_| 30.0,
        |_, c| if c == ' ' { 3.0 } else { 7.5 },
        |_, _, _| 0.0,
        None,
    )
    .iter()
    .map(|line| line.range.end)
    .collect();
    TextProbe {
        width,
        advance,
        gray,
        lines,
        mark: is_mark('\u{301}'),
        width_in,
        gray_in,
        handles: handles(),
    }
}

paperpaw_sdk::service_type!(TextProbe, "hello.text-probe/1");

/// What calls of the families after text came to: ui, data, graphics.
#[repr(C)]
#[derive(Debug, PartialEq)]
pub struct FamilyProbe {
    /// ui: a list view's moves, pages, rows and hits, and `list_step`.
    pub list: Vec<String>,
    /// data: a translator without catalogs, JSON, INI and XML.
    pub data: Vec<String>,
    /// graphics: the sum of the gray levels after shapes, and `round`.
    pub gray: u64,
    pub round: Vec<i32>,
}

fn list() -> Vec<String> {
    use paperpaw_platform::{Point, Rect};
    use paperpaw_sdk::ui::{ListView, list_step};
    let mut view = ListView::new(Rect::new(0, 0, 100, 200), 40);
    let mut out = alloc::vec![
        format!("{:?}", view.set_focus(3)),
        format!("{:?}", view.move_focus(4, 12)),
        format!("{:?}", view.focus_change()),
        format!("{:?}", view.paging(12)),
        format!("{:?}", view.row_rect(2)),
        format!("{:?}", view.hit(Point::new(10, 90), 12)),
    ];
    out.push(format!(
        "{:?}",
        list_step(&paperpaw_sdk::builtin::actions::NEXT_PAGE, 2)
    ));
    out
}

fn data() -> Vec<String> {
    use paperpaw_sdk::{Arg, Translator, config, json, xml};
    let tr = Translator::new();
    let mut out = alloc::vec![
        format!("{:?}", tr.get("probe.key")),
        tr.format("probe.{name}", &[("name", Arg::Str("x"))]),
        tr.plural("probe.count", 2, &[]),
    ];
    let members = json::object(r#"{"a": "b\u00e9", "n": 4.5, "l": ["x", "y"]}"#);
    out.push(format!(
        "{:?} {:?}",
        json::text(&members, "a"),
        json::number(&members, "n")
    ));
    out.push(json::string("q\"\u{7}"));
    out.push(format!("{:?}", json::raw_member(r#"{"l": [1, 2]}"#, "l")));
    out.push(format!("{:?}", json::unquote("\"a\\nb\"")));
    out.push(format!("{:?}", json::string_array(r#"["x", "y"]"#)));
    let mut events = String::new();
    let mut parser = json::stream::Parser::default();
    parser.feed(br#"{"k": [true, "v"]"#, &mut |e| {
        events.push_str(&format!("{e:?};"))
    });
    parser.finish(&mut |e| events.push_str(&format!("{e:?};")));
    out.push(events);
    let ini = "[s]\nk = \"v w\"\nn = 7\n";
    for entry in config::parse(ini) {
        out.push(format!("{entry:?}"));
    }
    out.push(config::set(ini, "s", "k", "z"));
    out.push(config::remove(ini, "s", "n"));
    out.push(format!("{}", config::quote("a b")));
    let mut tokens = String::new();
    xml::Tokenizer::parse(b"<a b='&amp;c'>d&lt;</a>", &mut |t| {
        tokens.push_str(&format!("{t:?};"));
    });
    out.push(tokens);
    out.push(format!("{}", xml::decode("&quot;&#65;")));
    out
}

fn graphics() -> (u64, Vec<i32>) {
    use paperpaw_platform::{Point, Rect};
    use paperpaw_sdk::round;
    let mut frame = Framebuffer::new(64, 32, PixelFormat::Gray8).unwrap();
    {
        let mut canvas = frame.canvas();
        canvas.clear(Color::WHITE);
        canvas.fill_round(Rect::new(2, 2, 20, 12), 4, Color::BLACK);
        canvas.stroke_round_rect(24, 2, 30, 20, 2, 6, Color::BLACK);
        canvas.draw_line(Point::new(0, 31), Point::new(63, 16), 2, Color::BLACK);
        canvas.fill_ring(Point::new(40, 20), 9, 3, 30, 200, Color::BLACK);
        canvas.fill_tint(Rect::new(0, 20, 30, 10), Color::BLACK);
    }
    let view = frame.view();
    let mut gray = 0u64;
    for y in 0..32 {
        for x in 0..64 {
            gray += u64::from(view.pixel(x, y).r);
        }
    }
    let (sin, cos) = round::sin_cos(30);
    let at = round::point_at(Point::new(10, 10), 8, 45);
    (
        gray,
        alloc::vec![sin, cos, at.x, at.y, round::corner_inset(6, 1, 20) as i32],
    )
}

pub fn family_probe() -> FamilyProbe {
    let (gray, round) = graphics();
    FamilyProbe {
        list: list(),
        data: data(),
        gray,
        round,
    }
}

paperpaw_sdk::service_type!(FamilyProbe, "hello.family-probe/1");
