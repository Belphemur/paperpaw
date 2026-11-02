//! The parts of a certificate the shell reads itself: its key's
//! SubjectPublicKeyInfo (what a plugin pins, hashed) and the dates it is
//! valid between. A walk through the DER, bounded and without panics: a
//! certificate that is not as expected is `None`.

/// What of a certificate is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Parts<'a> {
    /// The whole SubjectPublicKeyInfo, tag and length included.
    pub spki: &'a [u8],
    /// Seconds since 1970 it is valid from, and to.
    pub not_before: i64,
    pub not_after: i64,
}

/// One DER element: its tag, its content, and what follows it.
struct Element<'a> {
    tag: u8,
    whole: &'a [u8],
    content: &'a [u8],
    rest: &'a [u8],
}

fn element(data: &[u8]) -> Option<Element<'_>> {
    let (&tag, after) = data.split_first()?;
    // A tag of more than a byte is not in a certificate's top.
    if tag & 0x1f == 0x1f {
        return None;
    }
    let (&first, after) = after.split_first()?;
    let (length, header) = if first < 0x80 {
        (usize::from(first), 2)
    } else {
        let count = usize::from(first & 0x7f);
        if count == 0 || count > 3 || after.len() < count {
            return None;
        }
        let length = after[..count]
            .iter()
            .fold(0usize, |n, &b| (n << 8) | usize::from(b));
        (length, 2 + count)
    };
    let end = header.checked_add(length)?;
    if data.len() < end {
        return None;
    }
    Some(Element {
        tag,
        whole: &data[..end],
        content: &data[header..end],
        rest: &data[end..],
    })
}

/// The elements of `content`, one after the other.
fn children(mut content: &[u8]) -> impl Iterator<Item = Element<'_>> {
    core::iter::from_fn(move || {
        let next = element(content)?;
        content = next.rest;
        Some(next)
    })
}

/// Seconds since 1970 of a UTCTime or GeneralizedTime in Zulu (`Z`), as
/// certificates write them; `None` for any other form.
fn time(tag: u8, text: &[u8]) -> Option<i64> {
    let digits = |from: usize, to: usize| -> Option<i64> {
        text.get(from..to)?.iter().try_fold(0i64, |n, &b| {
            b.is_ascii_digit().then(|| n * 10 + i64::from(b - b'0'))
        })
    };
    let (year, rest) = match tag {
        // UTCTime: YYMMDDHHMMSSZ, 50..99 is 19xx.
        0x17 => {
            let yy = digits(0, 2)?;
            (if yy >= 50 { 1900 + yy } else { 2000 + yy }, 2)
        }
        // GeneralizedTime: YYYYMMDDHHMMSSZ.
        0x18 => (digits(0, 4)?, 4),
        _ => return None,
    };
    let (month, day, hour, minute, second) = (
        digits(rest, rest + 2)?,
        digits(rest + 2, rest + 4)?,
        digits(rest + 4, rest + 6)?,
        digits(rest + 6, rest + 8)?,
        digits(rest + 8, rest + 10)?,
    );
    if text.get(rest + 10) != Some(&b'Z') || text.len() != rest + 11 {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Days since 1970-01-01 of a date of the Gregorian calendar.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted = (month + 9) % 12;
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The parts of the certificate `der` that are read, if it is one.
pub(crate) fn parts(der: &[u8]) -> Option<Parts<'_>> {
    let certificate = element(der).filter(|e| e.tag == 0x30)?;
    let tbs = element(certificate.content).filter(|e| e.tag == 0x30)?;
    let mut fields = children(tbs.content);
    let mut field = fields.next()?;
    // The version, [0] EXPLICIT, is there or not.
    if field.tag == 0xa0 {
        field = fields.next()?;
    }
    // `field` is the serial number; then the signature algorithm and the
    // issuer.
    if field.tag != 0x02 {
        return None;
    }
    let (_algorithm, _issuer) = (fields.next()?, fields.next()?);
    let validity = fields.next().filter(|e| e.tag == 0x30)?;
    let mut dates = children(validity.content);
    let (from, to) = (dates.next()?, dates.next()?);
    let _subject = fields.next()?;
    let spki = fields.next().filter(|e| e.tag == 0x30)?;
    Some(Parts {
        spki: spki.whole,
        not_before: time(from.tag, from.content)?,
        not_after: time(to.tag, to.content)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_read_as_certificates_write_them() {
        // 2025-11-02 04:00:00 UTC.
        let at = days_from_civil(2025, 11, 2) * 86_400 + 4 * 3_600;
        assert_eq!(time(0x17, b"251102040000Z"), Some(at));
        assert_eq!(time(0x18, b"20251102040000Z"), Some(at));
        assert_eq!(time(0x18, b"19700101000000Z"), Some(0));
        // 50 and over is the last century, below the next.
        assert_eq!(time(0x17, b"491231235959Z"), Some(2_524_607_999));
        assert_eq!(time(0x17, b"500101000000Z"), Some(-631_152_000));
        // Not Zulu, not digits, too short, long, or another type.
        assert_eq!(time(0x17, b"2511020400+0100"), None);
        assert_eq!(time(0x17, b"25110204000aZ"), None);
        assert_eq!(time(0x17, b"2511020400Z"), None);
        assert_eq!(time(0x17, b"251102040000Zx"), None);
        assert_eq!(time(0x13, b"251102040000Z"), None);
        assert_eq!(time(0x17, b"251302040000Z"), None);
    }

    #[test]
    fn a_leap_year_and_the_end_of_february_count() {
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
        assert_eq!(
            days_from_civil(2100, 3, 1) - days_from_civil(2100, 2, 28),
            1
        );
    }

    #[test]
    fn things_that_are_no_certificates_are_nothing() {
        assert!(parts(&[]).is_none());
        assert!(parts(&[0x30]).is_none());
        assert!(parts(&[0x30, 0x05, 0x01]).is_none());
        assert!(parts(&[0x30, 0x80]).is_none());
        assert!(parts(&[0x30, 0x84, 0xff, 0xff, 0xff, 0xff]).is_none());
        assert!(parts(&[0x31, 0x00]).is_none());
        assert!(parts(&[0x30, 0x03, 0x30, 0x01, 0x00]).is_none());
        // Nothing read goes past its element, whatever the bytes.
        for seed in 0..300u32 {
            let bytes: Vec<u8> = (0..64u32)
                .map(|i| (seed.wrapping_mul(2_654_435_761).wrapping_add(i * 40_503) >> 7) as u8)
                .collect();
            let _ = parts(&bytes);
        }
    }
}
