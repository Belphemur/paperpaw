//! Hangul from jamo: the two-set (dubeolsik) keyboard types the letters
//! (compatibility jamo, U+3131 to U+3163), and the text joins them to
//! syllables as they come, like a Korean phone: a consonant and a vowel
//! make a syllable, a consonant after it ends it, a vowel after that takes
//! the ending for its own start. *Delete* takes back one jamo.
//!
//! It works on the end of the text and nothing else (no state), so a
//! layout needs only its letters as data: any layout with jamo composes.

use alloc::string::String;

/// The jamo a syllable starts with (19), its vowels (21) and its endings
/// (27, after none), in the order of Unicode's syllable blocks.
const INITIAL: &str = "ㄱㄲㄴㄷㄸㄹㅁㅂㅃㅅㅆㅇㅈㅉㅊㅋㅌㅍㅎ";
const MEDIAL: &str = "ㅏㅐㅑㅒㅓㅔㅕㅖㅗㅘㅙㅚㅛㅜㅝㅞㅟㅠㅡㅢㅣ";
const FINAL: &str = "ㄱㄲㄳㄴㄵㄶㄷㄹㄺㄻㄼㄽㄾㄿㅀㅁㅂㅄㅅㅆㅇㅈㅊㅋㅌㅍㅎ";

/// Two endings that make one (the second after the first), and the one
/// they make.
const ENDINGS: [(char, char, char); 11] = [
    ('ㄱ', 'ㅅ', 'ㄳ'),
    ('ㄴ', 'ㅈ', 'ㄵ'),
    ('ㄴ', 'ㅎ', 'ㄶ'),
    ('ㄹ', 'ㄱ', 'ㄺ'),
    ('ㄹ', 'ㅁ', 'ㄻ'),
    ('ㄹ', 'ㅂ', 'ㄼ'),
    ('ㄹ', 'ㅅ', 'ㄽ'),
    ('ㄹ', 'ㅌ', 'ㄾ'),
    ('ㄹ', 'ㅍ', 'ㄿ'),
    ('ㄹ', 'ㅎ', 'ㅀ'),
    ('ㅂ', 'ㅅ', 'ㅄ'),
];

/// Two vowels that make one, and the one they make.
const VOWELS: [(char, char, char); 7] = [
    ('ㅗ', 'ㅏ', 'ㅘ'),
    ('ㅗ', 'ㅐ', 'ㅙ'),
    ('ㅗ', 'ㅣ', 'ㅚ'),
    ('ㅜ', 'ㅓ', 'ㅝ'),
    ('ㅜ', 'ㅔ', 'ㅞ'),
    ('ㅜ', 'ㅣ', 'ㅟ'),
    ('ㅡ', 'ㅣ', 'ㅢ'),
];

/// The letters *Shift* brings on a two-set keyboard.
const SHIFTED: [(char, char); 7] = [
    ('ㅂ', 'ㅃ'),
    ('ㅈ', 'ㅉ'),
    ('ㄷ', 'ㄸ'),
    ('ㄱ', 'ㄲ'),
    ('ㅅ', 'ㅆ'),
    ('ㅐ', 'ㅒ'),
    ('ㅔ', 'ㅖ'),
];

const FIRST_SYLLABLE: u32 = 0xAC00;
const LAST_SYLLABLE: u32 = 0xD7A3;
/// Vowels, and endings including none, a syllable block holds.
const MEDIALS: u32 = 21;
const FINALS: u32 = 28;

/// Whether `c` is a jamo of the keyboard (a compatibility jamo).
pub(crate) fn is_jamo(c: char) -> bool {
    ('\u{3131}'..='\u{3163}').contains(&c)
}

/// What *Shift* makes of `c`, where it makes another letter.
pub(crate) fn shifted(c: char) -> Option<char> {
    SHIFTED.iter().find(|(a, _)| *a == c).map(|(_, b)| *b)
}

/// The place of `c` among `of` (0 first).
fn place(of: &str, c: char) -> Option<u32> {
    of.chars().position(|x| x == c).map(|p| p as u32)
}

/// A syllable's start, vowel and ending (0 for none): the places of its
/// jamo.
fn parts(c: char) -> Option<(u32, u32, u32)> {
    let code = u32::from(c);
    if !(FIRST_SYLLABLE..=LAST_SYLLABLE).contains(&code) {
        return None;
    }
    let block = code - FIRST_SYLLABLE;
    Some((
        block / (MEDIALS * FINALS),
        block / FINALS % MEDIALS,
        block % FINALS,
    ))
}

/// The syllable with these places.
fn syllable(initial: u32, medial: u32, last: u32) -> Option<char> {
    char::from_u32(FIRST_SYLLABLE + (initial * MEDIALS + medial) * FINALS + last)
}

fn nth(of: &str, n: u32) -> Option<char> {
    of.chars().nth(n as usize)
}

/// The two jamo `c` is made of, in `table`.
fn split(table: &[(char, char, char)], c: char) -> Option<(char, char)> {
    table
        .iter()
        .find(|(_, _, both)| *both == c)
        .map(|(a, b, _)| (*a, *b))
}

/// The jamo `a` and `b` make, in `table`.
fn join(table: &[(char, char, char)], a: char, b: char) -> Option<char> {
    table
        .iter()
        .find(|(x, y, _)| *x == a && *y == b)
        .map(|(_, _, both)| *both)
}

/// The place of an ending among the endings, counting none as 0.
fn ending(c: char) -> Option<u32> {
    place(FINAL, c).map(|p| p + 1)
}

/// What `jamo` makes of `last`, the text's last letter: the letter that
/// replaces it, and a letter after it where the jamo starts a new
/// syllable. None where they do not join (the jamo then stands after it).
fn combine(last: char, jamo: char) -> Option<(char, Option<char>)> {
    let vowel = place(MEDIAL, jamo);
    if let Some((start, middle, end)) = parts(last) {
        if end == 0 {
            if let Some(more) = vowel {
                // A second vowel makes a compound one (ㅗ and ㅏ: ㅘ).
                let both = join(&VOWELS, nth(MEDIAL, middle)?, nth(MEDIAL, more)?)?;
                return Some((syllable(start, place(MEDIAL, both)?, 0)?, None));
            }
            return Some((syllable(start, middle, ending(jamo)?)?, None));
        }
        let kept = nth(FINAL, end - 1)?;
        if let Some(more) = vowel {
            // The ending goes to the vowel: all of it, or (a compound
            // one) its second letter.
            let (stays, moves) =
                split(&ENDINGS, kept).map_or((0, kept), |(a, b)| (ending(a).unwrap_or(0), b));
            let held = syllable(start, middle, stays)?;
            return Some((held, syllable(place(INITIAL, moves)?, more, 0)));
        }
        let both = join(&ENDINGS, kept, jamo)?;
        return Some((syllable(start, middle, ending(both)?)?, None));
    }
    if let (Some(start), Some(more)) = (place(INITIAL, last), vowel) {
        return Some((syllable(start, more, 0)?, None));
    }
    if vowel.is_some() {
        return Some((join(&VOWELS, last, jamo)?, None));
    }
    None
}

/// Adds `jamo` at the end of `text`, joined to its last letter where they
/// make one syllable.
pub(crate) fn push(text: &mut String, jamo: char) {
    let joined = text
        .chars()
        .next_back()
        .and_then(|last| combine(last, jamo));
    match joined {
        Some((last, more)) => {
            text.pop();
            text.push(last);
            text.extend(more);
        }
        None => text.push(jamo),
    }
}

/// Takes the last jamo off `text`: of a syllable, its ending, else its
/// vowel (one of a compound one), else it is its start. Whether anything
/// was there to take.
pub(crate) fn pop(text: &mut String) -> bool {
    let Some(last) = text.pop() else {
        return false;
    };
    let rest = if let Some((start, middle, end)) = parts(last) {
        if end > 0 {
            let kept = nth(FINAL, end - 1).and_then(|c| split(&ENDINGS, c).map(|(a, _)| a));
            syllable(start, middle, kept.and_then(ending).unwrap_or(0))
        } else {
            let first = nth(MEDIAL, middle)
                .and_then(|c| split(&VOWELS, c))
                .and_then(|(a, _)| place(MEDIAL, a));
            match first {
                Some(first) => syllable(start, first, 0),
                None => nth(INITIAL, start),
            }
        }
    } else {
        split(&VOWELS, last).map(|(a, _)| a)
    };
    text.extend(rest);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec::Vec;

    fn typed(jamo: &str) -> String {
        let mut text = String::new();
        for c in jamo.chars() {
            push(&mut text, c);
        }
        text
    }

    /// The jamo a syllable is typed with.
    fn spelled(c: char) -> Vec<char> {
        let (start, middle, end) = parts(c).expect("a syllable");
        let mut jamo = alloc::vec![nth(INITIAL, start).unwrap()];
        let vowel = nth(MEDIAL, middle).unwrap();
        jamo.extend(split(&VOWELS, vowel).map_or(alloc::vec![vowel], |(a, b)| alloc::vec![a, b]));
        if end > 0 {
            let last = nth(FINAL, end - 1).unwrap();
            jamo.extend(
                split(&ENDINGS, last).map_or(alloc::vec![last], |(a, b)| alloc::vec![a, b]),
            );
        }
        jamo
    }

    #[test]
    fn words_join_as_they_are_typed() {
        assert_eq!(typed("ㅎㅏㄴㄱㅡㄹ"), "한글");
        assert_eq!(typed("ㅇㅏㄴㄴㅕㅇㅎㅏㅅㅔㅇㅛ"), "안녕하세요");
        assert_eq!(typed("ㅅㅏㄹㅏㅇ"), "사랑");
        assert_eq!(typed("ㄱㅏㅂㅅ"), "값");
    }

    #[test]
    fn a_vowel_takes_the_ending_for_its_start() {
        assert_eq!(typed("ㅇㅣㄹㄱㅓ"), "일거");
        assert_eq!(typed("ㅇㅣㄹㄱㅅㅓ"), "읽서");
        assert_eq!(typed("ㅁㅏㄱㅇㅓ"), "막어");
        assert_eq!(typed("ㅇㅏㄴㅈㅏ"), "안자");
        assert_eq!(typed("ㅇㅣㄹㄱ"), "읽");
        assert_eq!(typed("ㅇㅣㄹㄱㅇㅓ"), "읽어");
    }

    #[test]
    fn compound_vowels_and_letters_that_cannot_end() {
        assert_eq!(typed("ㅇㅗㅏ"), "와");
        assert_eq!(typed("ㅇㅜㅓ"), "워");
        assert_eq!(typed("ㅇㅡㅣ"), "의");
        assert_eq!(typed("ㅇㅏㄸ"), "아ㄸ");
        assert_eq!(typed("ㅇㅏㅃㅏ"), "아빠");
    }

    #[test]
    fn lone_letters_stay_apart() {
        assert_eq!(typed("ㅏㅏ"), "ㅏㅏ");
        assert_eq!(typed("ㄱㄴ"), "ㄱㄴ");
        assert_eq!(typed("ㅗㅏ"), "ㅘ");
        assert_eq!(typed("ㅏㄱ"), "ㅏㄱ");
        assert_eq!(typed("aㅏ"), "aㅏ");
    }

    #[test]
    fn delete_takes_one_jamo() {
        let mut text = typed("ㅎㅏㄴㄱㅡㄹ");
        let mut seen = Vec::new();
        while pop(&mut text) {
            seen.push(text.clone());
        }
        assert_eq!(seen, ["한그", "한ㄱ", "한", "하", "ㅎ", ""]);
        let mut text = typed("ㄱㅏㅂㅅ");
        assert!(pop(&mut text) && text == "갑");
        let mut text = typed("ㅇㅗㅏ");
        assert!(pop(&mut text) && text == "오");
        let mut text = String::from("a");
        assert!(pop(&mut text) && text.is_empty());
        assert!(!pop(&mut text));
    }

    #[test]
    fn every_syllable_round_trips() {
        for code in FIRST_SYLLABLE..=LAST_SYLLABLE {
            let c = char::from_u32(code).unwrap();
            let jamo = spelled(c);
            let mut text = String::new();
            for j in &jamo {
                push(&mut text, *j);
            }
            assert_eq!(text, c.to_string(), "{jamo:?}");
            // And back, one jamo at a time, to nothing.
            for _ in 0..jamo.len() {
                assert!(pop(&mut text));
            }
            assert!(text.is_empty(), "{c}");
        }
    }

    #[test]
    fn shift_doubles_what_has_a_double() {
        assert_eq!(shifted('ㅂ'), Some('ㅃ'));
        assert_eq!(shifted('ㅔ'), Some('ㅖ'));
        assert_eq!(shifted('ㅁ'), None);
        assert!(SHIFTED.iter().all(|(_, b)| is_jamo(*b)));
    }

    #[test]
    fn the_tables_are_whole() {
        assert_eq!(INITIAL.chars().count(), 19);
        assert_eq!(MEDIAL.chars().count() as u32, MEDIALS);
        assert_eq!(FINAL.chars().count() as u32, FINALS - 1);
        for (a, b, both) in ENDINGS {
            assert!(FINAL.contains(a) && FINAL.contains(b) && FINAL.contains(both));
        }
        for (a, b, both) in VOWELS {
            assert!(MEDIAL.contains(a) && MEDIAL.contains(b) && MEDIAL.contains(both));
        }
        // Every simple ending starts a syllable too.
        for c in FINAL.chars().filter(|c| split(&ENDINGS, *c).is_none()) {
            assert!(INITIAL.contains(c), "{c}");
        }
    }

    proptest::proptest! {
        /// Any jamo in any order joins without a panic, a jamo a push and a
        /// jamo a delete: all of them taken back leaves nothing.
        #[test]
        fn any_order_comes_back_to_nothing(
            jamo in proptest::collection::vec(0x3131u32..=0x3163, 0..40),
        ) {
            let mut text = String::new();
            let mut count = 0;
            for code in jamo {
                // The keys' letters: the compatibility block holds more, and
                // the compound vowels are two keys.
                let c = char::from_u32(code).unwrap();
                if place(INITIAL, c).is_some()
                    || (place(MEDIAL, c).is_some() && split(&VOWELS, c).is_none())
                {
                    push(&mut text, c);
                    count += 1;
                }
            }
            for _ in 0..count {
                proptest::prop_assert!(pop(&mut text));
            }
            proptest::prop_assert!(text.is_empty(), "{text}");
        }
    }
}
