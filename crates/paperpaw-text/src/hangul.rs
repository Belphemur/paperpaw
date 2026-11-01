//! Hangul written in jamo (#61): a syllable decomposed (Unicode's NFD, as
//! some books and Mac file names have it) is a leading consonant, a vowel
//! and maybe a final consonant, each a character of its own. Fonts hold
//! the 11,172 syllables whole (U+AC00 to U+D7A3), most no jamo, so the
//! text engine sets such a sequence as its syllable, by Unicode's
//! arithmetic (chapter 3.12): no table. Old jamo, which no syllable
//! holds, stay as they are.

/// The first syllable: 가.
pub(crate) const FIRST_SYLLABLE: char = '\u{AC00}';

/// Leading consonants that start a modern syllable.
const LEADING: core::ops::RangeInclusive<u32> = 0x1100..=0x1112;
/// Vowels of a modern syllable.
const VOWELS: core::ops::RangeInclusive<u32> = 0x1161..=0x1175;
/// Final consonants of a modern syllable.
const FINALS: core::ops::RangeInclusive<u32> = 0x11A8..=0x11C2;

/// Whether `ch` is a conjoining jamo, old or modern: text with one is
/// set by [`lay`](crate::around) rather than character by character.
pub(crate) fn is_jamo(ch: char) -> bool {
    matches!(ch, '\u{1100}'..='\u{11FF}' | '\u{A960}'..='\u{A97F}' | '\u{D7B0}'..='\u{D7FF}')
}

/// Whether `text` holds Hangul written in jamo: set as its syllables
/// ([`measure_in`](crate::measure_in), [`draw_text_in`](crate::draw_text_in)),
/// not character by character.
pub fn has_jamo(text: &str) -> bool {
    !text.is_ascii() && text.chars().any(is_jamo)
}

/// Whether `ch` is a jamo of a modern syllable: a font that holds the
/// syllables draws it as part of one.
pub(crate) fn composes(ch: char) -> bool {
    let c = u32::from(ch);
    LEADING.contains(&c) || VOWELS.contains(&c) || FINALS.contains(&c)
}

/// The syllable of leading consonant `lead`, vowel `vowel` and maybe a
/// final consonant `last`; `None` where they make none.
pub(crate) fn syllable(lead: char, vowel: char, last: Option<char>) -> Option<char> {
    let (l, v) = (u32::from(lead), u32::from(vowel));
    if !LEADING.contains(&l) || !VOWELS.contains(&v) {
        return None;
    }
    let t = match last.map(u32::from) {
        Some(t) if FINALS.contains(&t) => t - 0x11A7,
        Some(_) => return None,
        None => 0,
    };
    let index = (l - 0x1100) * 588 + (v - 0x1161) * 28 + t;
    char::from_u32(u32::from(FIRST_SYLLABLE) + index)
}

/// Sets the jamo among `keys` (one per character) as their syllables
/// where `has` says the font holds them: the syllable takes the leading
/// consonant's key, the vowel and the final draw nothing (`none`), so
/// every character keeps one key. Where the font lacks the syllable with
/// its final consonant, the one without it is set and the final stays a
/// jamo of its own.
pub(crate) fn compose(keys: &mut [char], none: char, has: impl Fn(char) -> bool) {
    let mut i = 0;
    while i + 1 < keys.len() {
        let (lead, vowel) = (keys[i], keys[i + 1]);
        let last = keys
            .get(i + 2)
            .copied()
            .filter(|&t| FINALS.contains(&u32::from(t)));
        let found = syllable(lead, vowel, last)
            .filter(|&s| has(s))
            .map(|s| (s, last.is_some()))
            .or_else(|| {
                last?;
                syllable(lead, vowel, None)
                    .filter(|&s| has(s))
                    .map(|s| (s, false))
            });
        let Some((whole, with_last)) = found else {
            i += 1;
            continue;
        };
        keys[i] = whole;
        keys[i + 1] = none;
        if with_last {
            keys[i + 2] = none;
            i += 3;
        } else {
            i += 2;
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests panic on purpose when something is wrong"
)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;

    /// Composes `text` as a font holding every syllable would.
    fn composed(text: &str) -> String {
        let mut keys: Vec<char> = text.chars().collect();
        compose(&mut keys, '\0', |_| true);
        keys.into_iter().filter(|&c| c != '\0').collect()
    }

    /// A font without 한 but with 하: 하 and the final ᆫ as a jamo.
    #[test]
    fn a_missing_syllable_with_a_final_falls_back_to_the_one_without() {
        let mut keys: Vec<char> = "\u{1112}\u{1161}\u{11AB}".chars().collect();
        compose(&mut keys, '\0', |s| s != '한');
        assert_eq!(keys, ['하', '\0', '\u{11AB}']);
        // Neither: the jamo stay as they are.
        let mut keys: Vec<char> = "\u{1112}\u{1161}\u{11AB}".chars().collect();
        compose(&mut keys, '\0', |_| false);
        assert_eq!(keys, ['\u{1112}', '\u{1161}', '\u{11AB}']);
    }

    #[test]
    fn decomposed_syllables_come_together() {
        // 한국어 decomposed (NFD): ᄒ ᅡ ᆫ, ᄀ ᅮ ᆨ, ᄋ ᅥ.
        let nfd = "\u{1112}\u{1161}\u{11AB}\u{1100}\u{116E}\u{11A8}\u{110B}\u{1165}";
        assert_eq!(composed(nfd), "한국어");
        assert_eq!(syllable('\u{1100}', '\u{1161}', None), Some('가'));
        assert_eq!(
            syllable('\u{1112}', '\u{1175}', Some('\u{11C2}')),
            Some('\u{D7A3}')
        );
        // Mixed with composed text and others, which stay.
        assert_eq!(composed("a\u{1100}\u{1161}가 b"), "a가가 b");
    }

    #[test]
    fn what_makes_no_syllable_stays() {
        // A lone vowel, a lone final, old jamo, a vowel before its lead.
        for text in [
            "\u{1161}",
            "\u{11A8}",
            "\u{1113}\u{1161}",
            "\u{1161}\u{1100}",
        ] {
            assert_eq!(composed(text), text);
        }
        // A font without the syllable: the jamo stay.
        let mut keys = ['\u{1100}', '\u{1161}'];
        compose(&mut keys, '\0', |_| false);
        assert_eq!(keys, ['\u{1100}', '\u{1161}']);
        assert!(composes('\u{1100}') && composes('\u{11C2}') && !composes('\u{1113}'));
        assert!(is_jamo('\u{1113}') && is_jamo('\u{D7B0}') && !is_jamo('가'));
    }

    proptest::proptest! {
        /// Any text: one key per character, never a panic, and only jamo
        /// change.
        #[test]
        fn any_text_keeps_its_length(text in "[\u{1100}-\u{11FF}a가 ]{0,40}") {
            let before: Vec<char> = text.chars().collect();
            let mut keys = before.clone();
            compose(&mut keys, '\0', |_| true);
            proptest::prop_assert_eq!(keys.len(), before.len());
            for (k, c) in keys.iter().zip(&before) {
                proptest::prop_assert!(k == c || is_jamo(*c));
            }
        }
    }
}
