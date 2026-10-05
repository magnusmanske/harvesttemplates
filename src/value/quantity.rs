use super::ValueError;
use super::numerals::to_ascii_digits;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DecimalMark {
    #[default]
    #[serde(rename = ".")]
    Point,
    #[serde(rename = ",")]
    Comma,
}

static THOUSANDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([0-9])(?:&nbsp;|\s|'|’|\u{202f})([0-9])").unwrap());
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([+-]?)0*([0-9]+(?:\.[0-9]+)?)$").unwrap());

/// Parse a decimal number into a Wikibase amount string such as `+1234.5`.
pub fn parse_amount(raw: &str, mark: DecimalMark) -> Result<String, ValueError> {
    let text = to_ascii_digits(raw).replace('−', "-");
    // Twice, so overlapping separators like `1 000 000` all go.
    let text = THOUSANDS.replace_all(&text, "$1$2");
    let text = THOUSANDS.replace_all(&text, "$1$2");
    let text = match mark {
        DecimalMark::Point => text.replace(',', ""),
        DecimalMark::Comma => text.replace('.', "").replace(',', "."),
    };
    let caps = NUMBER
        .captures(text.trim())
        .ok_or(ValueError::UnclearNumber)?;
    let sign = if &caps[1] == "-" { "-" } else { "+" };
    let digits = &caps[2];
    let digits = if digits.starts_with('.') {
        format!("0{digits}")
    } else {
        digits.to_string()
    };
    Ok(format!("{sign}{digits}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use DecimalMark::{Comma, Point};

    #[test]
    fn amounts() {
        let cases = [
            ("1234", Point, "+1234"),
            ("1,234,567.5", Point, "+1234567.5"),
            ("1.234.567,5", Comma, "+1234567.5"),
            ("1 234 567", Point, "+1234567"),
            ("1&nbsp;234", Point, "+1234"),
            ("1'234", Point, "+1234"),
            ("−12", Point, "-12"),
            ("-0.5", Point, "-0.5"),
            ("007", Point, "+7"),
            ("0", Point, "+0"),
            ("٣٤", Point, "+34"),
        ];
        for (raw, mark, expected) in cases {
            assert_eq!(parse_amount(raw, mark).as_deref(), Ok(expected), "{raw}");
        }
    }

    #[test]
    fn unclear() {
        for raw in ["82 g", "1.2.3", "", ".", "abc", "1-2"] {
            assert_eq!(
                parse_amount(raw, Point),
                Err(ValueError::UnclearNumber),
                "{raw}"
            );
        }
    }
}
