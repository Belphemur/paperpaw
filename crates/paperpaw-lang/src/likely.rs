//! The script a language is usually written in, for the tags that name
//! none (CLDR's "likely subtags", the script part): `ar`, `fa`, `he` do
//! not say `Arab` or `Hebr`, yet that is what text in them is written in.
//!
//! Data, sorted by language code for a binary search: one more language
//! is one more row, never a change of shape. Languages written in Latin
//! letters are not listed (nor are languages not listed): they have none.

const LIKELY: &[(&str, &str)] = &[
    ("am", "Ethi"),
    ("ar", "Arab"),
    ("as", "Beng"),
    ("be", "Cyrl"),
    ("bg", "Cyrl"),
    ("bn", "Beng"),
    ("bo", "Tibt"),
    ("ckb", "Arab"),
    ("doi", "Deva"),
    ("dv", "Thaa"),
    ("dz", "Tibt"),
    ("el", "Grek"),
    ("fa", "Arab"),
    ("gu", "Gujr"),
    ("he", "Hebr"),
    ("hi", "Deva"),
    ("hy", "Armn"),
    ("ja", "Jpan"),
    ("ka", "Geor"),
    ("kk", "Cyrl"),
    ("km", "Khmr"),
    ("kn", "Knda"),
    ("ko", "Kore"),
    ("kok", "Deva"),
    ("ky", "Cyrl"),
    ("lo", "Laoo"),
    ("mai", "Deva"),
    ("mk", "Cyrl"),
    ("ml", "Mlym"),
    ("mn", "Cyrl"),
    ("mr", "Deva"),
    ("my", "Mymr"),
    ("ne", "Deva"),
    ("or", "Orya"),
    ("pa", "Guru"),
    ("ps", "Arab"),
    ("ru", "Cyrl"),
    ("sa", "Deva"),
    ("sd", "Arab"),
    ("si", "Sinh"),
    ("sr", "Cyrl"),
    ("syr", "Syrc"),
    ("ta", "Taml"),
    ("te", "Telu"),
    ("tg", "Cyrl"),
    ("th", "Thai"),
    ("ti", "Ethi"),
    ("tt", "Cyrl"),
    ("ug", "Arab"),
    ("uk", "Cyrl"),
    ("ur", "Arab"),
    ("yi", "Hebr"),
    ("zh", "Hans"),
];

/// The script of text in the language of `tag` (BCP 47: `ar`, `fa-IR`,
/// `sr-Latn`): the script the tag names (four letters), else the one its
/// language is usually written in; `None` for a language written in Latin
/// letters, one not listed, or no language.
///
/// It reads the tag as it is, without [`Lang::parse`](crate::Lang::parse)
/// and its tables, so a plugin that only needs this does not carry them:
/// the language must be the code a catalog or a tag uses today (`fa`, not
/// `per`; `Lang::parse` knows the old and three-letter spellings).
#[must_use]
pub fn likely_script_of(tag: &str) -> Option<&str> {
    let mut parts = tag.split(['-', '_']);
    let language = parts.next()?;
    if let Some(script) = parts.find(|p| p.len() == 4 && p.bytes().all(|b| b.is_ascii_alphabetic()))
    {
        return Some(script);
    }
    of_code(language)
}

/// The script the language `code` (two or three letters, any case) is
/// usually written in.
pub(crate) fn of_code(code: &str) -> Option<&'static str> {
    let mut lower = [0u8; 3];
    let bytes = code.as_bytes();
    if bytes.len() > lower.len() {
        return None;
    }
    for (slot, b) in lower.iter_mut().zip(bytes) {
        *slot = b.to_ascii_lowercase();
    }
    let lower = lower.get(..bytes.len())?;
    LIKELY
        .binary_search_by(|(language, _)| language.as_bytes().cmp(lower))
        .ok()
        .and_then(|at| LIKELY.get(at))
        .map(|(_, script)| *script)
}

#[cfg(test)]
mod tests {
    use super::{LIKELY, likely_script_of};
    use crate::Lang;

    #[test]
    fn the_table_is_sorted_and_names_scripts_the_tables_know() {
        assert!(LIKELY.windows(2).all(|w| w[0].0 < w[1].0));
        for (language, script) in LIKELY {
            assert!(
                crate::scripts().any(|k| k == *script),
                "{language}: {script}"
            );
            assert_eq!(Lang::parse(language).language().as_str(), *language);
        }
    }

    #[test]
    fn languages_get_their_usual_script_and_a_tag_can_say_otherwise() {
        let script = |tag: &str| Lang::parse(tag).likely_script();
        for tag in ["ar", "fa", "fa-IR", "ur", "ps", "ckb", "ug"] {
            assert_eq!(script(tag), Some("Arab"), "{tag}");
        }
        assert_eq!(script("he"), Some("Hebr"));
        assert_eq!(script("sr"), Some("Cyrl"));
        assert_eq!(script("sr-Latn"), Some("Latn"));
        assert_eq!(script("zh-Hant"), Some("Hant"));
        assert_eq!(script("hi"), Some("Deva"));
        assert_eq!(script("de"), None);
        assert_eq!(script("en-GB"), None);
        assert_eq!(script(""), None);
        assert_eq!(script("not a tag"), None);
        assert_eq!(Lang::NONE.likely_script(), None);
    }

    #[test]
    fn a_tag_gives_the_same_script_without_parsing_it() {
        for tag in [
            "ar",
            "fa",
            "fa-IR",
            "ur",
            "ps",
            "ckb",
            "ug",
            "he",
            "sr",
            "sr-Latn",
            "zh-Hant",
            "hi",
            "de",
            "en-GB",
            "",
            "AR",
            "Fa_ir",
            "ko-KR",
            "ja",
            "not a tag",
            "xyz",
        ] {
            assert_eq!(
                likely_script_of(tag),
                Lang::parse(tag).likely_script(),
                "{tag:?}"
            );
        }
        // What the tag names, even a script this release has no name for.
        assert_eq!(likely_script_of("xx-Qaaa"), Some("Qaaa"));
    }
}
