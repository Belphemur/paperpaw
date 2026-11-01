use alloc::string::String;
use core::fmt::Write;

/// A value substituted into a `{placeholder}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arg<'a> {
    Str(&'a str),
    Int(i64),
}

/// Replaces `{name}` placeholders. `{{` and `}}` produce literal braces;
/// unknown placeholders are kept as written so mistakes stay visible.
/// Numbers are written with `digits` (Latin where `None`).
pub(crate) fn interpolate(
    message: &str,
    args: &[(&str, Arg<'_>)],
    digits: Option<&[char; 10]>,
) -> String {
    interpolate_impl(message, None, args, digits)
}

/// Like [`interpolate`], with `{count}` bound to `count`.
pub(crate) fn interpolate_with_count(
    message: &str,
    count: i64,
    args: &[(&str, Arg<'_>)],
    digits: Option<&[char; 10]>,
) -> String {
    interpolate_impl(message, Some(count), args, digits)
}

/// Writes `number` to `out` with `digits` (Latin where `None`).
pub(crate) fn write_int(out: &mut String, number: i64, digits: Option<&[char; 10]>) {
    let start = out.len();
    let _ = write!(out, "{number}");
    if let Some(digits) = digits {
        let written = with_digits(&out[start..], digits);
        out.truncate(start);
        out.push_str(&written);
    }
}

/// `text` with its Latin digits written as `digits`.
pub(crate) fn with_digits(text: &str, digits: &[char; 10]) -> String {
    text.chars()
        .map(|c| match c.to_digit(10) {
            Some(d) if c.is_ascii_digit() => digits[d as usize],
            _ => c,
        })
        .collect()
}

fn interpolate_impl(
    message: &str,
    count: Option<i64>,
    args: &[(&str, Arg<'_>)],
    digits: Option<&[char; 10]>,
) -> String {
    let mut out = String::with_capacity(message.len() + 16);
    let mut rest = message;
    while let Some(open) = rest.find(['{', '}']) {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        if let Some(after) = tail.strip_prefix("{{") {
            out.push('{');
            rest = after;
            continue;
        }
        if let Some(after) = tail.strip_prefix("}}") {
            out.push('}');
            rest = after;
            continue;
        }
        if tail.starts_with('{')
            && let Some(close) = tail.find('}')
        {
            let name = &tail[1..close];
            let value = args
                .iter()
                .find(|(arg_name, _)| *arg_name == name)
                .map(|(_, value)| *value)
                .or_else(|| (name == "count").then_some(count).flatten().map(Arg::Int));
            match value {
                Some(Arg::Str(s)) => out.push_str(s),
                Some(Arg::Int(i)) => write_int(&mut out, i, digits),
                None => out.push_str(&tail[..=close]),
            }
            rest = &tail[close + 1..];
            continue;
        }
        // A lone brace: keep it.
        out.push_str(&tail[..1]);
        rest = &tail[1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_placeholders() {
        let args = [("name", Arg::Str("Ada")), ("n", Arg::Int(-3))];
        assert_eq!(interpolate("{name} has {n}", &args, None), "Ada has -3");
    }

    #[test]
    fn keeps_unknown_placeholders_and_lone_braces() {
        assert_eq!(interpolate("{missing} } {", &[], None), "{missing} } {");
    }

    #[test]
    fn escaped_braces() {
        assert_eq!(interpolate("{{literal}}", &[], None), "{literal}");
    }

    #[test]
    fn count_is_implicit() {
        assert_eq!(
            interpolate_with_count("{count} books", 4, &[], None),
            "4 books"
        );
    }

    #[test]
    fn numbers_are_written_in_the_digits_given() {
        let persian = ['۰', '۱', '۲', '۳', '۴', '۵', '۶', '۷', '۸', '۹'];
        let args = [("n", Arg::Int(-3_150)), ("name", Arg::Str("ab12"))];
        assert_eq!(
            interpolate("{name}: {n}", &args, Some(&persian)),
            "ab12: -۳۱۵۰",
            "a text argument as it is"
        );
        assert_eq!(
            interpolate_with_count("{count} / 7", 40, &[], Some(&persian)),
            "۴۰ / 7",
            "the message's own text as it is"
        );
        let mut out = String::from("x");
        write_int(&mut out, i64::MIN, Some(&persian));
        assert_eq!(out, "x-۹۲۲۳۳۷۲۰۳۶۸۵۴۷۷۵۸۰۸");
        assert_eq!(with_digits("p. 12 ٣", &persian), "p. ۱۲ ٣");
    }
}
