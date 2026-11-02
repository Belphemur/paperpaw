//! The Markdown notes are written in: every construct (and what is not
//! one), constructs left unclosed, files changed after they were
//! converted, and the markup of notes (wikilinks, embeds, callouts, tags,
//! tables, front matter) left open, nested or huge.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests panic on purpose when something is wrong"
)]

use paperpaw_plugin_notes_vault::MarkdownDocument;
use paperpaw_sdk::document::{Block, BlockKind, Document, Locator};
use paperpaw_sdk::{MemoryStorage, Storage};

/// Every block of every section, read as a reader does.
fn blocks(doc: &dyn Document, storage: &dyn Storage) -> Vec<(u32, Block)> {
    let mut out = Vec::new();
    for section in 0..doc.sections() {
        let mut at = Locator::new(section, 0);
        let mut count = 0;
        while let Some(block) = doc.block_at(storage, at).expect("readable") {
            assert!(block.end > at.offset, "no progress at {at:?}");
            at = Locator::new(section, block.end);
            out.push((section, block));
            count += 1;
            assert!(count < 1_000_000, "endless section {section}");
        }
    }
    out
}

/// The words of all blocks, in order.
fn words(doc: &dyn Document, storage: &dyn Storage) -> Vec<String> {
    blocks(doc, storage)
        .iter()
        .flat_map(|(_, b)| {
            b.text
                .split_whitespace()
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The text of all blocks, a line each.
fn text(doc: &dyn Document, storage: &dyn Storage) -> String {
    blocks(doc, storage)
        .iter()
        .map(|(_, b)| b.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn markdown(md: &str) -> (MemoryStorage, MarkdownDocument) {
    let storage = MemoryStorage::new().with("notes/n.md", md.as_bytes().to_vec());
    let doc = MarkdownDocument::open(&storage, "notes/n.md", true).unwrap();
    (storage, doc)
}

/// Kind, text and whether its first character is bold and italic, per
/// block.
fn shown(doc: &dyn Document, storage: &dyn Storage) -> Vec<(BlockKind, String)> {
    blocks(doc, storage)
        .into_iter()
        .map(|(_, b)| (b.kind, b.text))
        .collect()
}

#[test]
fn markdown_headings_of_every_level_and_what_is_no_heading() {
    let md = "# One\n## Two ##\n### Three\n#### Four\n##### Five\n###### Six\n\
              ####### Seven\n#NoSpace\n#\n";
    let (storage, doc) = markdown(md);
    let got = shown(&doc, &storage);
    let headings: Vec<(BlockKind, &str)> = got.iter().map(|(k, t)| (*k, t.as_str())).collect();
    assert_eq!(
        headings,
        [
            (BlockKind::Heading(1), "One"),
            (BlockKind::Heading(2), "Two"),
            (BlockKind::Heading(3), "Three"),
            (BlockKind::Heading(4), "Four"),
            (BlockKind::Heading(5), "Five"),
            (BlockKind::Heading(6), "Six"),
            (BlockKind::Paragraph, "####### Seven #NoSpace #"),
        ]
    );
    assert_eq!(doc.metadata().title.as_deref(), Some("n"));
}

#[test]
fn markdown_lists_quotes_rules_and_code() {
    let md = "- a\n* b\n+ c\n  - nested\n    1. deep\n\n1. one\n22. two\ngoes on\n\n\
              1234567890. not a number\n\n> quote one\n> quote two\n\n---\n***\n_ _ _\n--\n\n\
              ```rust\nfn main() {\n    let x = 1;  \n}\n```\n~~~\n# not a heading\n~~~\n";
    let (storage, doc) = markdown(md);
    let got = shown(&doc, &storage);
    let texts: Vec<&str> = got.iter().map(|(_, t)| t.as_str()).collect();
    assert_eq!(
        texts,
        [
            "•\u{a0}a",
            "•\u{a0}b",
            "•\u{a0}c",
            "•\u{a0}nested",
            // Numbered, in the one list Markdown here makes of them.
            "1.\u{a0}deep",
            "2.\u{a0}one",
            // A line after an item goes on with it.
            "3.\u{a0}two goes on",
            "1234567890. not a number",
            "quote one quote two",
            "* * *",
            "* * *",
            "* * *",
            "--",
            "fn main() {\n    let x = 1;\n}",
            "# not a heading",
        ]
    );
    let code = got
        .iter()
        .filter(|(k, _)| *k == BlockKind::Preformatted)
        .count();
    assert_eq!(code, 2);
}

#[test]
fn markdown_inline_markup_nests_and_closes() {
    let md = "Plain **bold *both* bold** and ***all*** \\*not\\* `code **not bold**` \
              [link *it*](https://x.org) ![pic](https://x.org/p.png) under_scored_word \
              **open bold\n\nnext *open italic\n\nunclosed `code and [unclosed link](x\n";
    let (storage, doc) = markdown(md);
    let read = blocks(&doc, &storage);
    let first = &read[0].1;
    assert_eq!(
        first.text,
        "Plain bold both bold and all *not* code **not bold** link it pic under_scored_word open bold"
    );
    let style = |word: &str| first.style_at(first.text.find(word).unwrap());
    assert!(style("bold both").bold() && !style("bold both").italic());
    assert!(style("both").bold() && style("both").italic());
    assert!(style("all").bold() && style("all").italic());
    assert!(!style("*not*").bold() && !style("*not*").italic());
    assert!(!style("**not bold**").bold());
    assert!(style("it pic").italic());
    assert!(!style("under_scored").italic());
    // Emphasis left open ends with its line.
    assert!(style("open bold").bold());
    assert_eq!(read[1].1.text, "next open italic");
    assert!(!read[2].1.style_at(0).bold() && !read[2].1.style_at(0).italic());
    assert_eq!(read[2].1.text, "unclosed `code and [unclosed link](x");
    let links = doc.links(&storage, 0, 0, u32::MAX);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].href, "https://x.org");
}

#[test]
fn markdown_tables_are_rows_and_html_is_kept_as_text() {
    let md = "| a | b |\n|---|---|\n| 1 | 2 |\n\n<div>raw <b>html</b></div>\n";
    let (storage, doc) = markdown(md);
    let got = text(&doc, &storage);
    // A row is a line of cells, set apart; the row of dashes is no text.
    assert!(got.contains("a · b") && got.contains("1 · 2"), "{got}");
    assert!(!got.contains("---"), "{got}");
    assert!(got.contains("<div>raw <b>html</b></div>"), "{got}");
}

#[test]
fn markdown_line_ends_of_every_kind() {
    // Old Mac line ends are no line ends: the text goes on.
    let (storage, doc) =
        markdown("# Title\r\n\r\nFirst line\r\nsecond line\r\n\r\nLast\rstill last");
    let read = blocks(&doc, &storage);
    let texts: Vec<&str> = read.iter().map(|(_, b)| b.text.as_str()).collect();
    assert_eq!(
        texts,
        ["Title", "First line second line", "Last still last"]
    );
}

#[test]
fn markdown_two_spaces_break_the_line_after_them() {
    for md in ["one  \ntwo\nthree\n", "one  \r\ntwo\r\nthree\r\n"] {
        let (storage, doc) = markdown(md);
        assert_eq!(text(&doc, &storage), "one\ntwo three", "{md:?}");
    }
}

#[test]
fn markdown_a_fence_left_open_keeps_the_rest_as_code() {
    let (storage, doc) = markdown("Before\n\n```\ncode one\n\n# no heading\n- no item\n");
    let read = blocks(&doc, &storage);
    assert_eq!(read[0].1.text, "Before");
    assert_eq!(read[1].1.kind, BlockKind::Preformatted);
    assert_eq!(read[1].1.text, "code one\n\n# no heading\n- no item");
}

#[test]
fn markdown_that_is_empty_or_not_text_opens() {
    for md in [
        &b""[..],
        b"\n\n\n",
        b"\xff\xfe\x00#\x00 \x00H\x00",
        b"\0\0\0",
        b"[[[[]]]]((((",
    ] {
        let storage = MemoryStorage::new().with("x.md", md.to_vec());
        let doc = MarkdownDocument::open(&storage, "x.md", true).unwrap();
        blocks(&doc, &storage);
        assert_eq!(doc.metadata().title.as_deref(), Some("x"));
    }
}

#[test]
fn markdown_changed_is_converted_anew() {
    let storage = MemoryStorage::new().with("n.md", b"Old words\n".to_vec());
    let doc = MarkdownDocument::open(&storage, "n.md", true).unwrap();
    assert_eq!(text(&doc, &storage), "Old words");
    // Same size, other words.
    storage.write("n.md", b"New words\n").unwrap();
    let doc = MarkdownDocument::open(&storage, "n.md", true).unwrap();
    assert_eq!(text(&doc, &storage), "New words");
}

/// Runs `work` on another thread; `None` if it takes longer than
/// `seconds`.
fn within<T: Send + 'static>(seconds: u64, work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (send, receive) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = send.send(work());
    });
    receive
        .recv_timeout(std::time::Duration::from_secs(seconds))
        .ok()
}

#[test]
fn markdown_long_lines_convert() {
    // A long line of words and markup, as an export without line breaks.
    let line = "word **bold** *it* `c` ".repeat(10_000);
    let done = within(60, move || {
        let (storage, doc) = markdown(&line);
        words(&doc, &storage).len()
    });
    assert_eq!(done, Some(40_000));
}

#[test]
fn markdown_lines_of_brackets_convert_in_linear_time() {
    // A line of 200,000 brackets once took hours, as each `[` or `![`
    // copied the rest of the line to look for its link. The text comes in
    // blocks of at most 4000 characters, so the brackets are counted.
    let line = "[".repeat(200_000);
    let done = within(20, move || {
        let (storage, doc) = markdown(&line);
        text(&doc, &storage).matches('[').count()
    });
    assert_eq!(done, Some(200_000));
}

#[test]
fn markdown_wikilinks_embeds_callouts_and_tags_in_hostile_text_convert_in_linear_time() {
    // Lines of every opening of a note's markup, never closed, deeply
    // nested or huge: converted in time, nothing panics, the text stays.
    let pieces = [
        "[[",
        "![[",
        "[[a|",
        "[[a#",
        "![[a|",
        "> [!",
        "> > > > [!note]",
        "---\ntags: [",
        "#",
        "|",
        "\\",
    ];
    for piece in pieces {
        let md = format!("{}\n", piece.repeat(100_000));
        let done = within(20, move || {
            let (storage, doc) = markdown(&md);
            text(&doc, &storage).len()
        });
        assert!(done.is_some(), "{piece:?} took too long");
    }
    let nested = format!("{}x{}\n", "[[a|".repeat(50_000), "]]".repeat(50_000));
    assert!(
        within(20, move || {
            let (storage, doc) = markdown(&nested);
            text(&doc, &storage).len()
        })
        .is_some()
    );
    // Broken links lead nowhere and are shown as typed.
    let (storage, doc) = markdown("see [[ ]] and [[|]] and [[#]] and ![[]] and [[a]b]]\n");
    let got = text(&doc, &storage);
    assert!(got.contains("see "), "{got}");
    let _ = doc.links(&storage, 0, 0, u32::MAX);
}
