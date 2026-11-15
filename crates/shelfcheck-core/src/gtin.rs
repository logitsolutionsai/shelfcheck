//! GS1 GTIN rules (General Specifications 26.0, January 2026).
//!
//! One check-digit algorithm covers GTIN-8, 12, 13 and 14: weight the digits
//! 3, 1, 3, 1 from the right, starting next to the check digit; the check
//! digit brings the sum up to the next multiple of 10.

/// Why a value is not a well-formed GTIN.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormError {
    Empty,
    HasSeparators,
    NotDigits,
    WrongLength(usize),
}

impl FormError {
    pub fn reason(&self) -> String {
        match self {
            FormError::Empty => "it is empty".to_string(),
            FormError::HasSeparators => "it contains spaces or dashes".to_string(),
            FormError::NotDigits => "it contains characters other than digits".to_string(),
            FormError::WrongLength(n) => {
                if *n == 10 {
                    "it has 10 characters; ISBN-10 is not accepted, use the ISBN-13".to_string()
                } else {
                    format!("it has {n} digits; a GTIN has 8, 12, 13 or 14")
                }
            }
        }
    }
}

/// Check the form: digits only, length 8, 12, 13 or 14. Leading zeros count.
pub fn check_form(value: &str) -> Result<(), FormError> {
    let v = value.trim();
    if v.is_empty() {
        return Err(FormError::Empty);
    }
    if v.chars().any(|c| c == ' ' || c == '-' || c == '\u{a0}') {
        return Err(FormError::HasSeparators);
    }
    if !v.chars().all(|c| c.is_ascii_digit()) {
        // An ISBN-10 may end in X; report its length, which says more.
        if v.len() == 10
            && v.get(..9)
                .is_some_and(|h| h.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(FormError::WrongLength(10));
        }
        return Err(FormError::NotDigits);
    }
    match v.len() {
        8 | 12 | 13 | 14 => Ok(()),
        n => Err(FormError::WrongLength(n)),
    }
}

/// The check digit the body (all digits but the last) should end in.
/// `digits` must be ASCII digits only.
pub fn expected_check_digit(digits: &str) -> Option<u8> {
    let body = digits.get(..digits.len().checked_sub(1)?)?;
    let mut sum: u32 = 0;
    for (i, c) in body.chars().rev().enumerate() {
        let d = c.to_digit(10)?;
        sum += if i % 2 == 0 { d * 3 } else { d };
    }
    Some(((10 - (sum % 10)) % 10) as u8)
}

/// Some(expected) when the check digit is wrong, None when it is right or the
/// value is not a well-formed GTIN.
pub fn wrong_check_digit(value: &str) -> Option<u8> {
    let v = value.trim();
    check_form(v).ok()?;
    let expected = expected_check_digit(v)?;
    let actual = v.chars().next_back()?.to_digit(10)? as u8;
    if actual == expected {
        None
    } else {
        Some(expected)
    }
}

/// A GTIN that is well formed and has the right check digit.
pub fn is_valid(value: &str) -> bool {
    check_form(value).is_ok() && wrong_check_digit(value).is_none()
}

/// Normalised 14-digit form, used as an identity key.
pub fn to_gtin14(value: &str) -> Option<String> {
    let v = value.trim();
    if !is_valid(v) {
        return None;
    }
    Some(format!("{v:0>14}"))
}

/// The digits that prefix rules apply to. GTIN-12 and GTIN-14 are brought to
/// the 13-digit form (a leading 0 for GTIN-12; the packaging indicator dropped
/// for GTIN-14). GTIN-8 stays as is.
pub fn prefix_form(value: &str) -> Option<String> {
    let v = value.trim();
    check_form(v).ok()?;
    match v.len() {
        8 | 13 => Some(v.to_string()),
        12 => Some(format!("0{v}")),
        14 => Some(v[1..].to_string()),
        _ => None,
    }
}

/// Does the prefix form start with one of `prefixes`, or fall inside one of
/// the inclusive `ranges` (compared on the range's own length)?
pub fn prefix_matches(
    form: &str,
    prefixes: &[String],
    ranges: &[(String, String)],
) -> Option<String> {
    for p in prefixes {
        if form.starts_with(p.as_str()) {
            return Some(p.clone());
        }
    }
    for (lo, hi) in ranges {
        let n = lo.len();
        if let Some(head) = form.get(..n) {
            if head >= lo.as_str() && head <= hi.as_str() {
                return Some(head.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Examples from the GS1 General Specifications and common references.
    #[test]
    fn check_digits_from_gs1_examples() {
        assert_eq!(expected_check_digit("4006381333931"), Some(1));
        assert!(is_valid("4006381333931"));
        assert!(is_valid("036000291452")); // UPC-A
        assert!(is_valid("96385074")); // GTIN-8
        assert!(is_valid("10614141000415")); // GTIN-14
        assert!(is_valid("9780306406157")); // ISBN-13
        assert_eq!(wrong_check_digit("4006381333932"), Some(1));
    }

    #[test]
    fn form_errors() {
        assert_eq!(check_form("4006 381333931"), Err(FormError::HasSeparators));
        assert_eq!(check_form("400-6381333931"), Err(FormError::HasSeparators));
        assert_eq!(check_form("03600029145"), Err(FormError::WrongLength(11)));
        assert_eq!(check_form("030640615X"), Err(FormError::WrongLength(10)));
        assert_eq!(check_form("ABC"), Err(FormError::NotDigits));
        assert_eq!(check_form(""), Err(FormError::Empty));
    }

    #[test]
    fn prefix_forms() {
        assert_eq!(
            prefix_form("036000291452").as_deref(),
            Some("0036000291452")
        );
        assert_eq!(
            prefix_form("10614141000415").as_deref(),
            Some("0614141000415")
        );
        let prefixes = vec!["02".to_string(), "04".to_string()];
        let ranges = vec![
            ("20".to_string(), "29".to_string()),
            ("0000001".to_string(), "0000099".to_string()),
        ];
        assert_eq!(
            prefix_matches("2512345678900", &prefixes, &ranges).as_deref(),
            Some("25")
        );
        assert_eq!(
            prefix_matches("0412345678903", &prefixes, &ranges).as_deref(),
            Some("04")
        );
        assert_eq!(
            prefix_matches("0000042123456", &prefixes, &ranges).as_deref(),
            Some("0000042")
        );
        assert_eq!(prefix_matches("4006381333931", &prefixes, &ranges), None);
    }

    #[test]
    fn gtin14_normalises() {
        assert_eq!(to_gtin14("036000291452").as_deref(), Some("00036000291452"));
        assert_eq!(to_gtin14("036000291453"), None);
    }

    proptest::proptest! {
        // A valid GTIN with any one digit changed always fails.
        #[test]
        fn one_changed_digit_always_fails(body in "[0-9]{12}", pos in 0usize..13, delta in 1u8..10) {
            let check = expected_check_digit(&format!("{body}0")).unwrap_or(0);
            let valid = format!("{body}{check}");
            proptest::prop_assert!(is_valid(&valid));
            let mut bytes = valid.into_bytes();
            let d = bytes[pos] - b'0';
            bytes[pos] = b'0' + (d + delta) % 10;
            let changed = String::from_utf8(bytes).unwrap_or_default();
            proptest::prop_assert!(!is_valid(&changed));
        }
    }
}
