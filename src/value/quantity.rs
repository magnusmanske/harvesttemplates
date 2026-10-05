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

static THOUSANDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9])(?:&nbsp;|\s|'|’|\u{202f})([0-9])").unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^([+-]?)0*([0-9]+(?:\.[0-9]+)?)$").unwrap());

/// The number is greedy and ends in a digit, so `2,4 kg` does not split as `2` + `,4 kg`.
static NUMBER_AND_UNIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*([+\-−]?[0-9](?:(?:[0-9.,'’ \u{a0}\u{202f}]|&nbsp;)*[0-9])?)\s*(\S.*)?$").unwrap()
});

/// Split `2,4 kg` into the number and the unit text (#136). Without a match,
/// the whole text is the number (and will fail to parse).
pub fn split_unit(text: &str) -> (String, Option<String>) {
    let text = to_ascii_digits(text);
    match NUMBER_AND_UNIT.captures(&text) {
        Some(caps) => (caps[1].to_string(), caps.get(2).map(|u| u.as_str().trim().to_string())),
        None => (text, None),
    }
}

/// How unit names and suffixes are compared: `[[Kilogram|kg]]` → `kg`, `Kg.` → `kg`.
pub fn unit_key(name: &str) -> String {
    let name = name.trim().trim_start_matches("[[").trim_end_matches("]]");
    let name = name.rsplit('|').next().unwrap_or(name);
    name.trim().trim_end_matches('.').to_lowercase()
}

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
    let caps = NUMBER.captures(text.trim()).ok_or(ValueError::UnclearNumber)?;
    let sign = if &caps[1] == "-" { "-" } else { "+" };
    let digits = &caps[2];
    let digits = if digits.starts_with('.') { format!("0{digits}") } else { digits.to_string() };
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
    fn number_and_unit() {
        let cases = [
            ("82 g", "82", Some("g")),
            ("2,4 kg", "2,4", Some("kg")),
            ("1 234 567 m", "1 234 567", Some("m")),
            ("−12 °C", "−12", Some("°C")),
            ("1.5 [[kilogram]]", "1.5", Some("[[kilogram]]")),
            ("62", "62", None),
            ("1&nbsp;000 t", "1&nbsp;000", Some("t")),
        ];
        for (text, number, unit) in cases {
            assert_eq!(split_unit(text), (number.to_string(), unit.map(String::from)), "{text}");
        }
        assert_eq!(unit_key("[[Kilogram|kg]]"), "kg");
        assert_eq!(unit_key(" Kg. "), "kg");
    }

    #[test]
    fn unclear() {
        for raw in ["82 g", "1.2.3", "", ".", "abc", "1-2"] {
            assert_eq!(parse_amount(raw, Point), Err(ValueError::UnclearNumber), "{raw}");
        }
    }
}
