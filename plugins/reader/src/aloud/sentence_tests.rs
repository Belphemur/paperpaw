//! Sentences as read aloud, for any words: a sentence starting inside the
//! words ends after it and within the most words said at once, at the
//! first word that ends a sentence; the text spoken is the words in order.

extern crate std;

use alloc::string::String;
use alloc::vec::Vec;

use proptest::prelude::*;

use super::*;

fn words() -> impl Strategy<Value = Vec<String>> {
    proptest::collection::vec("[a-zA-Z]{1,6}[.!?…,;:\"”’)\\]-]{0,2}", 0..200)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(std::env::var("PROPTEST_CASES").ok().and_then(|n| n.parse().ok()).unwrap_or(256)))]

    #[test]
    fn a_sentence_ends_at_its_first_end(words in words(), from in 0usize..220) {
        let words: Vec<&str> = words.iter().map(String::as_str).collect();
        prop_assume!(!words.is_empty());
        let from = from % words.len();
        let end = sentence_end(&words, from);
        prop_assert!(end > from && end <= words.len());
        prop_assert!(end - from <= MAX_WORDS);
        // No end of a sentence before its last word.
        prop_assert!(words[from..end - 1].iter().all(|w| !ends_sentence(w)));
        // It ends with one, or at the most words, or at the last word.
        prop_assert!(
            ends_sentence(words[end - 1]) || end - from == MAX_WORDS || end == words.len()
        );
    }

    #[test]
    fn the_text_spoken_is_the_words_in_order(
        words in words(),
        lines in proptest::collection::vec(any::<bool>(), 200),
    ) {
        let parts: Vec<(&str, bool)> =
            words.iter().zip(&lines).map(|(w, &l)| (w.as_str(), l)).collect();
        let text = spoken(parts.iter().copied());
        // Joined again where a hyphen split a word, else with spaces: the
        // same letters in the same order.
        let letters = |s: &str| s.chars().filter(|c| !matches!(c, ' ' | '-')).collect::<String>();
        let all: String = words.iter().map(|w| letters(w)).collect();
        prop_assert_eq!(letters(&text), all);
        prop_assert!(!text.starts_with(' ') && !text.ends_with(' '));
        prop_assert!(!text.contains("  "));
    }
}

#[test]
fn a_sentence_without_an_end_stops_at_the_most_words() {
    let many: Vec<&str> = core::iter::repeat_n("word", 200).collect();
    assert_eq!(sentence_end(&many, 0), MAX_WORDS);
    assert_eq!(sentence_end(&many, 150), 200);
}

#[test]
fn nothing_is_spoken_of_nothing() {
    assert_eq!(spoken(core::iter::empty()), "");
    assert_eq!(spoken([("-", true), ("a", true)].into_iter()), "- a");
}
