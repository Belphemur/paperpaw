use alloc::string::String;
use alloc::vec::Vec;
use paperpaw_sdk::{MemoryStorage, Storage};

use crate::codec::*;
use crate::header::*;
use crate::render::render;

#[test]
fn base64_in_pieces_and_whole() {
    assert_eq!(decode_base64(b"SGVsbG8gV29ybGQ="), b"Hello World");
    assert_eq!(decode_base64(b"SGVs\r\nbG8="), b"Hello");
    assert_eq!(decode_base64(b"SGVsbG8"), b"Hello");
    let mut b = Base64::default();
    let mut out = Vec::new();
    for piece in [&b"SGV"[..], b"sb", b"G8g", b"V29ybGQ="] {
        b.push(piece, &mut out);
    }
    b.finish(&mut out);
    assert_eq!(out, b"Hello World");
    assert_eq!(decode_base64(b"!!!"), b"");
    assert_eq!(encode_base64(b"Hello World"), "SGVsbG8gV29ybGQ=\r\n");
    assert_eq!(base64_line(b"\0user\0pass"), "AHVzZXIAcGFzcw==");
    let long = encode_base64(&[7u8; 100]);
    assert!(long.lines().all(|l| l.len() <= 76));
    assert_eq!(decode_base64(long.as_bytes()), [7u8; 100]);
}

#[test]
fn quoted_printable() {
    assert_eq!(
        decode_qp(b"caf=C3=A9 =\r\nau lait\r\n", false),
        "café au lait\r\n".as_bytes()
    );
    assert_eq!(decode_qp(b"a=3Db=\n", false), b"a=b");
    assert_eq!(decode_qp(b"x=ZZy=", false), b"x=ZZy");
    assert_eq!(decode_qp(b"Hello_World", true), b"Hello World");
    assert_eq!(decode_qp(b"Hello_World", false), b"Hello_World");
    let text = "Grüße, café = ok\nzweite Zeile  \n";
    let encoded = encode_qp(text);
    assert!(encoded.lines().all(|l| l.len() <= 76), "{encoded}");
    let back = decode_qp(encoded.as_bytes(), false);
    assert_eq!(String::from_utf8(back).unwrap().replace("\r\n", "\n"), text);
    let long = "ä".repeat(100);
    let encoded = encode_qp(&long);
    assert!(encoded.lines().all(|l| l.len() <= 76));
    assert_eq!(
        String::from_utf8(decode_qp(encoded.as_bytes(), false))
            .unwrap()
            .trim_end(),
        long
    );
}

#[test]
fn charsets() {
    assert_eq!(
        text_of(Charset::of("iso-8859-1").unwrap(), b"caf\xe9"),
        "café"
    );
    assert_eq!(
        text_of(Charset::of("ISO-8859-15").unwrap(), b"\xa4 5"),
        "€ 5"
    );
    assert_eq!(
        text_of(Charset::of("windows-1252").unwrap(), b"\x93hi\x94 \x80"),
        "“hi” €"
    );
    assert_eq!(
        text_of(
            Charset::of("windows-1251").unwrap(),
            b"\xcf\xf0\xe8\xe2\xe5\xf2"
        ),
        "Привет"
    );
    assert_eq!(
        text_of(Charset::of("koi8-r").unwrap(), b"\xf0\xd2\xc9\xd7\xc5\xd4"),
        "Привет"
    );
    assert_eq!(
        text_of(Charset::of("iso-8859-2").unwrap(), b"\xb1\xe6\xea"),
        "ąćę"
    );
    assert_eq!(
        text_of(Charset::of("windows-1250").unwrap(), b"\xb9\xe6\xea"),
        "ąćę"
    );
    assert_eq!(Charset::of("shift_jis"), None);
    assert_eq!(Charset::of("UTF-8*de"), Some(Charset::Utf8));
    // UTF-8 cut anywhere.
    let bytes = "äöü€😀".as_bytes();
    for cut in 0..bytes.len() {
        let mut d = Decoder::new(Charset::Utf8);
        let mut out = String::new();
        d.push(&bytes[..cut], &mut out);
        d.push(&bytes[cut..], &mut out);
        d.finish(&mut out);
        assert_eq!(out, "äöü€😀", "cut at {cut}");
    }
    assert_eq!(text_of(Charset::Utf8, b"a\xffb"), "a\u{fffd}b");
}

#[test]
fn encoded_words() {
    assert_eq!(decode_words(b"=?UTF-8?B?w6TDtsO8?="), "äöü");
    assert_eq!(
        decode_words(b"=?iso-8859-1?Q?Caf=E9_au_lait?="),
        "Café au lait"
    );
    // White space between two encoded words goes, elsewhere it stays.
    assert_eq!(decode_words(b"=?utf-8?Q?a?= =?utf-8?Q?b?= c  d"), "ab c  d");
    assert_eq!(decode_words(b"plain \xc3\xa4"), "plain ä");
    assert_eq!(decode_words(b"plain \xe4"), "plain ä");
    assert_eq!(decode_words(b"=?bogus"), "=?bogus");
    assert_eq!(decode_words(b"=?x?Z?abc?="), "=?x?Z?abc?=");
    let encoded = encode_words("Grüße aus Köln, ein sehr langer Betreff");
    assert!(encoded.is_ascii());
    assert!(encoded.split("\r\n").all(|l| l.len() < 76), "{encoded}");
    assert_eq!(
        decode_words(encoded.replace("\r\n ", " ").as_bytes()),
        "Grüße aus Köln, ein sehr langer Betreff"
    );
    assert_eq!(encode_words("plain"), "plain");
}

#[test]
fn headers_and_addresses() {
    let raw = b"From: =?utf-8?Q?J=C3=BCrgen?= <j@example.org>\r\nSubject: A very\r\n long subject\r\nX-Odd\r\nTo: \"Doe, Jane\" <jane@example.org>, bob@example.org (Bob)\r\n\r\nbody";
    let h = Headers::parse(raw);
    assert_eq!(h.get("subject"), Some("A very long subject"));
    let from = addresses(h.get("from").unwrap());
    assert_eq!(
        from,
        [Address {
            name: "Jürgen".into(),
            address: "j@example.org".into()
        }]
    );
    let to = addresses(h.get("to").unwrap());
    assert_eq!(to.len(), 2);
    assert_eq!(to[0].name, "Doe, Jane");
    assert_eq!(to[1].shown(), "Bob");
    assert_eq!(to[1].address, "bob@example.org");
    assert!(addresses("undisclosed-recipients:;").is_empty());
    assert_eq!(addresses("Team: a@b.c, d@e.f;").len(), 2);
    assert_eq!(h.get("body"), None);
}

#[test]
fn content_types() {
    let c = ContentType::parse(Some(
        "Multipart/Alternative; boundary=\"b;1\"; charset=utf-8",
    ));
    assert_eq!(c.kind, "multipart/alternative");
    assert_eq!(c.param("boundary"), Some("b;1"));
    assert!(c.is_multipart());
    assert_eq!(ContentType::parse(None).kind, "text/plain");
    assert_eq!(ContentType::parse(Some("garbage")).kind, "text/plain");
    let f = ContentType::parse(Some("attachment; filename*=utf-8''J%C3%BCrgen.pdf"));
    assert_eq!(f.param("filename"), Some("Jürgen.pdf"));
    let pieces = ContentType::parse(Some("x/y; name*0=\"long\"; name*1=\"name.txt\""));
    assert_eq!(pieces.param("name"), Some("longname.txt"));
}

#[test]
fn dates() {
    assert_eq!(date_seconds("Thu, 1 Jan 1970 00:00:00 +0000"), Some(0));
    assert_eq!(
        date_seconds("Tue, 3 Nov 2026 09:15:00 +0100"),
        Some(1_793_693_700)
    );
    assert_eq!(
        date_seconds("3 Nov 2026 09:15 GMT (comment)"),
        date_seconds("Tue, 03 Nov 2026 09:15:00 +0000")
    );
    assert_eq!(
        date_seconds("Mon, 29 Feb 2016 23:59:59 -0800"),
        Some(1_456_819_199)
    );
    assert_eq!(date_seconds("nonsense"), None);
    assert_eq!(date_seconds("32 Jan 2026 00:00:00 +0000"), None);
    assert_eq!(civil_from_days(days_from_civil(2026, 11, 3)), (2026, 11, 3));
    assert_eq!(reply_subject("Hello"), "Re: Hello");
    assert_eq!(reply_subject("RE: Hello"), "RE: Hello");
}

fn rendered(message: &str, limit: usize) -> (crate::render::Rendered, String) {
    let storage = MemoryStorage::new().with("m.eml", message);
    let info = render(&storage, "m.eml", "m.txt", limit).unwrap();
    let text =
        String::from_utf8(paperpaw_sdk::read_file(&storage, "m.txt", 1 << 20).unwrap()).unwrap();
    (info, text)
}

#[test]
fn a_plain_message() {
    let (info, text) = rendered(
        "From: Ada <ada@example.org>\r\nTo: me@example.org\r\nSubject: =?utf-8?B?SGFsbG8g8J+RiyE=?=\r\nDate: Tue, 3 Nov 2026 09:15:00 +0100\r\nMessage-ID: <1@x>\r\n\r\nLine one\r\nLine two\r\n",
        1000,
    );
    assert_eq!(info.subject, "Hallo 👋!");
    assert_eq!(info.from[0].shown(), "Ada");
    assert_eq!(info.message_id, "<1@x>");
    assert!(info.date.is_some());
    assert_eq!(text, "Line one\nLine two\n");
}

#[test]
fn alternative_prefers_the_plain_text_and_attachments_are_named() {
    let message = "From: a@b.c\r\nContent-Type: multipart/mixed; boundary=outer\r\n\r\npreamble\r\n--outer\r\nContent-Type: multipart/alternative; boundary=inner\r\n\r\n--inner\r\nContent-Type: text/plain; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\ncaf=E9 =\r\nau lait\r\n--inner\r\nContent-Type: text/html\r\n\r\n<p>html version</p>\r\n--inner--\r\n--outer\r\nContent-Type: application/pdf; name=\"plan.pdf\"\r\nContent-Transfer-Encoding: base64\r\nContent-Disposition: attachment; filename=\"plan.pdf\"\r\n\r\nJVBERi0xLjQK\r\n--outer--\r\nepilogue\r\n";
    let (info, text) = rendered(message, 1000);
    assert_eq!(text, "café au lait\n");
    assert_eq!(info.attachments, ["plan.pdf (application/pdf)"]);
}

#[test]
fn html_only_and_base64_text() {
    let message = "Content-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nPHA+SGFsbG8gJmFtcDsgV2VsdDwvcD48cD5Ew7xyZjwvcD4=\r\n";
    let (_, text) = rendered(message, 1000);
    assert_eq!(text.trim(), "Hallo & Welt\n\nDürf");
}

#[test]
fn a_long_message_is_cut_and_says_so() {
    let body = "word ".repeat(2000);
    let message = alloc::format!("Subject: x\r\n\r\n{body}\r\n");
    let (info, text) = rendered(&message, 500);
    assert!(info.truncated);
    assert!(text.len() <= 500);
    assert_eq!(info.text_len, text.len());
}

#[test]
fn hostile_messages_do_not_panic() {
    for message in [
        "",
        "\r\n\r\n",
        "Content-Type: multipart/mixed\r\n\r\nx",
        "Content-Type: multipart/mixed; boundary=\r\n\r\n--\r\n--",
        "Content-Type: multipart/mixed; boundary=a\r\n\r\n--a\r\nContent-Type: multipart/mixed; boundary=a\r\n\r\n--a\r\n--a--\r\n--a--",
        "Content-Transfer-Encoding: base64\r\n\r\n====\r\n!!!!",
        "Content-Type: text/html\r\n\r\n<<<<script>&&&;;&#99999999999;</style>",
    ] {
        let _ = rendered(message, 100);
    }
    let deep = (0..50)
        .map(|i| {
            alloc::format!(
                "--b{i}\r\nContent-Type: multipart/mixed; boundary=b{}\r\n\r\n",
                i + 1
            )
        })
        .collect::<String>();
    let _ = rendered(
        &alloc::format!("Content-Type: multipart/mixed; boundary=b0\r\n\r\n{deep}"),
        100,
    );
    let _ = MemoryStorage::new().exists("x");
}
