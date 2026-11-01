//! What Readwise's answers say, read from the files they were saved to as
//! they stream (see `paperpaw_sdk::json::stream`), so an answer of any
//! size is read with little memory. Only a whole answer counts.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::Storage;
use paperpaw_sdk::json::stream::{self, Event as Json};

/// An article of the list: its id and title.
pub type Listed = (String, String);

/// Whether `id` is an id as Readwise gives them (letters and digits,
/// 26 of them): it goes into addresses, lists and names of files.
pub fn is_id(id: &str) -> bool {
    (1..=40).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The articles in the list Readwise sent, at most `max`: those with an
/// id as [`is_id`] says. `None` when the answer is not a whole list (its
/// results never end).
pub fn listed(storage: &dyn Storage, path: &str, max: usize) -> Option<Vec<Listed>> {
    let mut items: Vec<Listed> = Vec::new();
    let mut whole = false;
    let list = ["results"];
    let item = ["results", "[]"];
    stream::read_file(storage, path, &mut |event| match event {
        Json::Open(at) if at == item => items.push((String::new(), String::new())),
        Json::Close(at) if at == list => whole = true,
        Json::Value(at, value) if at.len() == 3 && at[..2] == item => {
            if let Some(last) = items.last_mut() {
                match at[2].as_str() {
                    "id" => last.0 = value.to_string(),
                    "title" => last.1 = value.to_string(),
                    _ => {}
                }
            }
        }
        _ => {}
    });
    items.retain(|(id, _)| is_id(id));
    items.truncate(max);
    whole.then_some(items)
}

/// An article asked about by its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Readwise does not know it any more.
    Gone,
    /// Where it is (`new`, `later`, `archive` …), and who wrote it.
    Known { location: String, author: String },
}

/// What the answer to a list asking for article `id` says; `None` when
/// it is not a whole answer, or about another article.
pub fn asked(storage: &dyn Storage, path: &str, id: &str) -> Option<Answer> {
    let (mut whole, mut count, mut same) = (false, 0, false);
    let (mut location, mut author) = (String::new(), String::new());
    stream::read_file(storage, path, &mut |event| match event {
        Json::Open(at) if at == ["results", "[]"] => count += 1,
        Json::Close(at) if at == ["results"] => whole = true,
        Json::Value(at, value) if at.len() == 3 && at[..2] == ["results", "[]"] && count == 1 => {
            match at[2].as_str() {
                "id" => same = value == id,
                "location" => location = value.to_string(),
                "author" => author = value.to_string(),
                _ => {}
            }
        }
        _ => {}
    });
    match (whole, count) {
        (true, 0) => Some(Answer::Gone),
        (true, _) if same => Some(Answer::Known { location, author }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use paperpaw_sdk::MemoryStorage;

    use super::*;

    fn with(body: &str) -> MemoryStorage {
        MemoryStorage::new().with("a.json", body)
    }

    #[test]
    fn reads_the_list() {
        let list = r#"{"count": 2, "nextPageCursor": null, "results": [
            {"id": "01gm6ceb0", "title": "Slow \"news\"", "tags": {"x": {"id": "no"}}, "location": "later"},
            {"title": "Second", "id": "01gm6cec1", "notes": ""}
        ]}"#;
        assert_eq!(
            listed(&with(list), "a.json", 20).unwrap(),
            [
                (String::from("01gm6ceb0"), String::from("Slow \"news\"")),
                (String::from("01gm6cec1"), String::from("Second")),
            ]
        );
        assert_eq!(listed(&with(list), "a.json", 1).unwrap().len(), 1);
        assert_eq!(
            listed(&with(r#"{"count":0,"results":[]}"#), "a.json", 20),
            Some(Vec::new())
        );
    }

    #[test]
    fn only_a_whole_list_counts() {
        for answer in [
            "<!DOCTYPE html><html><body>Down for maintenance</body></html>",
            r#"{"detail": "Invalid token."}"#,
            r#"{"count": 2, "results": [{"id": "a1", "title": "Newer"}, {"id": "b"#,
            "",
        ] {
            assert_eq!(listed(&with(answer), "a.json", 20), None, "{answer}");
        }
        assert_eq!(listed(&MemoryStorage::new(), "a.json", 20), None);
    }

    #[test]
    fn ids_are_letters_and_digits() {
        let list = r#"{"results": [
            {"id": "x\t.paperpaw/secrets.txt\ny", "title": "Planted"},
            {"id": "../../x", "title": "Out"},
            {"id": "", "title": "None"},
            {"id": "01gm6ceb", "title": "Fine"}
        ]}"#;
        assert_eq!(
            listed(&with(list), "a.json", 20).unwrap(),
            [(String::from("01gm6ceb"), String::from("Fine"))]
        );
    }

    #[test]
    fn an_article_asked_about() {
        let body = |location: &str| {
            alloc::format!(
                r#"{{"count":1,"results":[{{"id":"a1","location":"{location}","author":"Jo","html_content":"{}"}}]}}"#,
                "x".repeat(20_000)
            )
        };
        assert_eq!(
            asked(&with(&body("archive")), "a.json", "a1"),
            Some(Answer::Known {
                location: String::from("archive"),
                author: String::from("Jo")
            })
        );
        assert_eq!(asked(&with(&body("later")), "a.json", "a2"), None);
        assert_eq!(
            asked(&with(r#"{"count":0,"results":[]}"#), "a.json", "a1"),
            Some(Answer::Gone)
        );
        assert_eq!(asked(&with("<html>Login</html>"), "a.json", "a1"), None);
        assert_eq!(
            asked(&with(r#"{"results":[{"id":"a1""#), "a.json", "a1"),
            None
        );
    }

    proptest::proptest! {
        #[test]
        fn any_bytes_are_read_without_harm(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..600)) {
            let storage = MemoryStorage::new().with("a.json", bytes);
            let list = listed(&storage, "a.json", 20);
            proptest::prop_assert!(list.is_none_or(|l| l.len() <= 20 && l.iter().all(|(id, _)| is_id(id))));
            let _ = asked(&storage, "a.json", "a1");
        }
    }
}
