//! Punctuation hung into the margins (`ScriptOptions::HANGING_PUNCTUATION`):
//! where lines are set, never where they break. Lines are ten units wide,
//! a character one unit.

use super::*;

const HANG: ScriptOptions = ScriptOptions::HANGING_PUNCTUATION;

/// The page of the one block `block`, laid out in `style`.
fn page(block: Block, style: &Style, hyphenate: Option<Hyphenate<'_>>) -> Page {
    let end = block.end;
    let doc = Blocks::new(move |at: Locator| (at.offset < end).then(|| block.clone()));
    let storage = paperpaw_document::NoStorage;
    let cx = LayoutContext::new(&doc, &storage, 10.0, style, &Units).hyphenate(hyphenate);
    layout_page(&cx, Locator::START, 100.0).unwrap()
}

fn of(text: &str) -> Block {
    block(text, 0, text.len() as u32)
}

/// Where each line starts and the extra width of its spaces.
fn placed(page: &Page) -> Vec<(f32, f32)> {
    page.lines.iter().map(|l| (l.x, l.word_spacing)).collect()
}

#[test]
fn justified_lines_hang_their_quotes_stops_and_commas() {
    let text = "\"aaa bbb, ccc ddd. eee";
    let justified = Style::new(0.0, 0.0, 10.0, true);
    let plain = page(of(text), &justified, None);
    let hung = page(of(text), &justified.script_options(HANG), None);
    // The same lines and the same page.
    assert_eq!(texts(&hung), ["\"aaa bbb,", "ccc ddd.", "eee"]);
    assert_eq!(texts(&hung), texts(&plain));
    assert_eq!((hung.start, hung.end), (plain.start, plain.end));
    assert_eq!(placed(&plain), [(0.0, 1.0), (0.0, 2.0), (0.0, 0.0)]);
    // The quote one unit out at the left, the comma and the stop one at
    // the right: the spaces take what they leave.
    assert_eq!(placed(&hung), [(-1.0, 3.0), (0.0, 3.0), (0.0, 0.0)]);
}

#[test]
fn ragged_lines_hang_only_at_the_edge_they_are_set_to() {
    let text = "\"aaa bbb.";
    let hung = STYLE.script_options(HANG);
    // Set to their start: the quote hangs, the stop has room.
    assert_eq!(placed(&page(of(text), &hung, None)), [(-1.0, 0.0)]);
    // Set to their end: the stop hangs.
    let end = of(text).align(Align::End);
    assert_eq!(placed(&page(end.clone(), &STYLE, None)), [(1.0, 0.0)]);
    assert_eq!(placed(&page(end, &hung, None)), [(2.0, 0.0)]);
    // Centred: nothing.
    let centred = of(text).align(Align::Center);
    assert_eq!(
        placed(&page(centred.clone(), &hung, None)),
        placed(&page(centred, &STYLE, None))
    );
}

#[test]
fn an_indented_first_line_keeps_its_quote() {
    let indented = Style::new(0.0, 2.0, 10.0, false).script_options(HANG);
    let page = page(of("\"aa bb \"cc dd"), &indented, None);
    assert_eq!(texts(&page), ["\"aa bb", "\"cc dd"]);
    // The first line in its indent; the second hangs.
    assert_eq!(placed(&page), [(2.0, 0.0), (-1.0, 0.0)]);
}

#[test]
fn the_hyphen_of_a_broken_word_hangs() {
    let anywhere: Hyphenate<'_> = &|word: &str, at: &mut dyn FnMut(usize)| {
        (1..word.len()).for_each(at);
    };
    let justified = Style::new(0.0, 0.0, 10.0, true);
    let text = "aa bbbbbbbbbbbb";
    let plain = page(of(text), &justified, Some(anywhere));
    let hung = page(of(text), &justified.script_options(HANG), Some(anywhere));
    assert_eq!(texts(&hung), texts(&plain));
    assert!(hung.lines[0].text.ends_with('-'), "{:?}", texts(&hung));
    let (x, plain_gap) = placed(&plain)[0];
    assert_eq!(placed(&hung)[0], (x, plain_gap + 1.0));
}

#[test]
fn right_to_left_lines_do_not_hang() {
    let hebrew = "\u{5d0}\u{5d1} \u{5d2}\u{5d3}.";
    let hung = STYLE.script_options(HANG);
    for dir in [Direction::Auto, Direction::Rtl] {
        let block = of(hebrew).dir(dir).align(Align::End);
        assert_eq!(
            placed(&page(block.clone(), &hung, None)),
            placed(&page(block, &STYLE, None))
        );
    }
}
