//! Dates in the many formats found in templates, in ~250 languages.
//! Month names come from `data/monthnames.json` (ported from PLnode).

use super::ValueError;
use super::numerals::to_ascii_digits;
use crate::ids::ItemId;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Calendar {
    #[default]
    Gregorian,
    Julian,
}

impl Calendar {
    pub const fn item(self) -> ItemId {
        match self {
            Self::Gregorian => ItemId(1_985_727),
            Self::Julian => ItemId(1_985_786),
        }
    }

    const fn is_leap(self, year: i64) -> bool {
        match self {
            Self::Gregorian => year % 4 == 0 && (year % 100 != 0 || year % 400 == 0),
            Self::Julian => year % 4 == 0,
        }
    }
}

/// A date with optional month and day (0 = unknown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Date {
    pub year: i64,
    pub month: u8,
    pub day: u8,
}

impl Date {
    pub fn new(year: i64, month: u8, day: u8, calendar: Calendar) -> Option<Self> {
        let days = match month {
            0 => 0,
            2 if calendar.is_leap(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            1..=12 => 31,
            _ => return None,
        };
        let day_ok = day <= days && (day == 0 || month > 0);
        (year > 0 && day_ok).then_some(Self { year, month, day })
    }

    /// Wikibase precision: 9 = year, 10 = month, 11 = day.
    pub const fn precision(&self) -> u8 {
        match (self.month, self.day) {
            (0, _) => 9,
            (_, 0) => 10,
            _ => 11,
        }
    }

    /// The Wikibase `time` string, e.g. `+1950-01-00T00:00:00Z`.
    pub fn wikibase_time(&self) -> String {
        format!(
            "+{:04}-{:02}-{:02}T00:00:00Z",
            self.year, self.month, self.day
        )
    }

    pub const fn earliest(&self) -> Self {
        Self {
            year: self.year,
            month: if self.month == 0 { 1 } else { self.month },
            day: if self.day == 0 { 1 } else { self.day },
        }
    }

    pub const fn latest(&self) -> Self {
        Self {
            year: self.year,
            month: if self.month == 0 { 12 } else { self.month },
            day: if self.day == 0 { 31 } else { self.day },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Only dates on or after the limit.
    AtLeast,
    /// Only dates before the limit.
    Before,
}

/// Restricts harvested dates, typically to the period where the chosen
/// calendar is certain. An imprecise date passes only if all its days do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateLimit {
    pub relation: Relation,
    pub date: Date,
}

impl DateLimit {
    pub fn accepts(&self, date: &Date) -> bool {
        match self.relation {
            Relation::AtLeast => date.earliest() >= self.date.earliest(),
            Relation::Before => date.latest() < self.date.earliest(),
        }
    }
}

/// Parse a free-text date. `lang` selects month names (the wiki's content language).
pub fn parse_date(raw: &str, lang: &str, calendar: Calendar) -> Result<Date, ValueError> {
    let text = to_ascii_digits(raw)
        .replace(['–', '—', '‐'], "-")
        .replace("[[", "")
        .replace("]]", "");
    if IMPRECISE.is_match(&text) {
        return Err(ValueError::ImpreciseDate);
    }
    let lang = LangPatterns::get(lang);
    for re in pattern_order(lang) {
        if let Some(caps) = re.captures(&text) {
            return date_from(&caps, lang.map(|l| &l.months), calendar)
                .ok_or(ValueError::InvalidDate);
        }
    }
    year_only(&text, calendar)
}

/// Combine separate year/month/day template parameters. The month may be a number,
/// a Roman numeral or a month name.
pub fn parse_date_parts(
    year: &str,
    month: Option<&str>,
    day: Option<&str>,
    lang: &str,
    calendar: Calendar,
) -> Result<Date, ValueError> {
    let year: i64 = to_ascii_digits(year)
        .trim()
        .parse()
        .map_err(|_| ValueError::NoDate)?;
    let months = LangPatterns::get(lang).map(|l| &l.months);
    let month = match month.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => month_number(&to_ascii_digits(m), months).ok_or(ValueError::InvalidDate)?,
        None => 0,
    };
    let day = match day
        .map(|d| to_ascii_digits(d).trim().to_string())
        .filter(|d| !d.is_empty())
    {
        Some(d) => d.parse().map_err(|_| ValueError::InvalidDate)?,
        None => 0,
    };
    Date::new(year, month, day, calendar).ok_or(ValueError::InvalidDate)
}

fn pattern_order(lang: Option<&LangPatterns>) -> Vec<&Regex> {
    let mut order = Vec::with_capacity(8);
    if let Some(l) = lang {
        order.extend([&l.day_month_year, &l.month_day_year, &l.year_month_day]);
    }
    order.extend([&*ISO, &*NUMERIC_YMD, &*NUMERIC_DMY, &*CJK]);
    if let Some(l) = lang {
        order.push(&l.month_year);
    }
    order
}

fn date_from(
    caps: &Captures,
    months: Option<&HashMap<String, u8>>,
    calendar: Calendar,
) -> Option<Date> {
    let year = caps.name("y")?.as_str().parse().ok()?;
    let month = match caps.name("m") {
        Some(m) => month_number(m.as_str(), months)?,
        None => 0,
    };
    let day = caps
        .name("d")
        .map_or(Some(0), |d| d.as_str().parse().ok())?;
    Date::new(year, month, day, calendar)
}

fn month_number(s: &str, months: Option<&HashMap<String, u8>>) -> Option<u8> {
    if let Ok(n) = s.parse() {
        return Some(n);
    }
    let upper = s.to_uppercase();
    if let Some(i) = ROMAN.iter().position(|r| *r == upper) {
        return Some(i as u8 + 1);
    }
    months?.get(&s.to_lowercase()).copied()
}

/// A bare number of up to four digits, or exactly one four-digit year in the text.
fn year_only(text: &str, calendar: Calendar) -> Result<Date, ValueError> {
    let trimmed = text.trim();
    let year = if BARE_YEAR.is_match(trimmed) {
        trimmed.parse().map_err(|_| ValueError::NoDate)?
    } else {
        let mut years: Vec<&str> = FOUR_DIGITS.find_iter(text).map(|m| m.as_str()).collect();
        years.dedup();
        match years[..] {
            [] => return Err(ValueError::NoDate),
            [y] => y.parse().map_err(|_| ValueError::NoDate)?,
            _ => return Err(ValueError::AmbiguousDate),
        }
    };
    Date::new(year, 0, 0, calendar).ok_or(ValueError::InvalidDate)
}

const ROMAN: [&str; 12] = [
    "I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X", "XI", "XII",
];
const ROMAN_ALT: &str = "XII|XI|X|IX|VIII|VII|VI|V|IV|III|II|I";
const YEAR_END: &str = r"(?-u:\b)";

static IMPRECISE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:años|vor|nach|ungefähr|ca|circa|around|about|before|after)\b|\?").unwrap()
});
static BARE_YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{1,4}$").unwrap());
static FOUR_DIGITS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)[0-9]{4}(?-u:\b)").unwrap());
static ISO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?-u:\b)(?P<y>[0-9]{{3,4}})-(?P<m>[0-9]{{1,2}})-(?P<d>[0-9]{{1,2}}){YEAR_END}"
    ))
    .unwrap()
});
static NUMERIC_YMD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?-u:\b)(?P<y>[0-9]{{4}})(?: - |/|\. ?)(?P<m>[0-9]{{1,2}}|{ROMAN_ALT})(?: - |/|\. ?)(?P<d>[0-9]{{1,2}})(?-u:\b)"
    ))
    .unwrap()
});
static NUMERIC_DMY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?-u:\b)(?P<d>[0-9]{{1,2}})(?:[. /-]+| tháng )(?P<m>[0-9]{{1,2}}|{ROMAN_ALT})(?:[., /-]+| năm )(?P<y>[0-9]{{4}}){YEAR_END}"
    ))
    .unwrap()
});
static CJK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<y>[0-9]{3,4})\s*(?:年|년)(?:[〈（(][^）〉)]*[〉）)])?\s*(?:(?P<m>[0-9]{1,2})\s*(?:月|월)\s*(?:(?P<d>[0-9]{1,2})\s*(?:日|일))?)?").unwrap()
});

/// Month-name patterns of one language, compiled on first use.
#[derive(Debug)]
struct LangPatterns {
    /// Lower-cased month name or unambiguous abbreviation → month number.
    months: HashMap<String, u8>,
    day_month_year: Regex,
    month_day_year: Regex,
    year_month_day: Regex,
    month_year: Regex,
}

static MONTH_NAMES: LazyLock<HashMap<String, HashMap<String, u8>>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../data/monthnames.json")).expect("valid monthnames.json")
});
static LANG_PATTERNS: LazyLock<HashMap<&'static str, OnceLock<LangPatterns>>> =
    LazyLock::new(|| {
        MONTH_NAMES
            .keys()
            .map(|k| (k.as_str(), OnceLock::new()))
            .collect()
    });

impl LangPatterns {
    fn get(lang: &str) -> Option<&'static Self> {
        let cell = LANG_PATTERNS.get(lang)?;
        Some(cell.get_or_init(|| Self::build(&MONTH_NAMES[lang])))
    }

    fn build(names: &HashMap<String, u8>) -> Self {
        let months = with_abbreviations(names);
        let mut alternatives: Vec<&String> = months.keys().collect();
        alternatives.sort_by_key(|n| std::cmp::Reverse(n.chars().count()));
        let m = alternatives
            .iter()
            .map(|n| regex::escape(n))
            .collect::<Vec<_>>()
            .join("|");
        let (d, y) = (
            r"(?-u:\b)(?P<d>[0-9]{1,2})",
            format!(r"(?P<y>[0-9]{{3,4}}){YEAR_END}"),
        );
        let compile = |p: String| Regex::new(&format!("(?i){p}")).expect("escaped month names");
        Self {
            day_month_year: compile(format!(
                r"{d}(?: |\. |º |er | - an? de | de | d'| ב)?(?P<m>{m})(?:,| del?|, इ.स.| พ.ศ.)? {y}"
            )),
            month_day_year: compile(format!(r"(?P<m>{m})\.? {d}(?:st|nd|rd|th)?,? {y}")),
            year_month_day: compile(format!(
                r"(?-u:\b)(?P<y>[0-9]{{3,4}})(?:e?ko|\.|,)? (?P<m>{m})(?:aren)? (?P<d>[0-9]{{1,2}})(?:a|ean|an|\.)?(?-u:\b)"
            )),
            month_year: compile(format!(r"(?P<m>{m}) {y}")),
            months,
        }
    }
}

/// Lower-cased names plus three-letter abbreviations that identify a single month.
fn with_abbreviations(names: &HashMap<String, u8>) -> HashMap<String, u8> {
    let mut months: HashMap<String, u8> =
        names.iter().map(|(n, &m)| (n.to_lowercase(), m)).collect();
    let mut abbreviations: HashMap<String, Option<u8>> = HashMap::new();
    for (name, &month) in &months {
        if name.chars().count() > 3 {
            let abbr: String = name.chars().take(3).collect();
            let entry = abbreviations.entry(abbr).or_insert(Some(month));
            if *entry != Some(month) {
                *entry = None;
            }
        }
    }
    for (abbr, month) in abbreviations {
        if let Some(month) = month {
            months.entry(abbr).or_insert(month);
        }
    }
    months
}

#[cfg(test)]
mod tests {
    use super::*;
    use Calendar::{Gregorian, Julian};

    fn date(year: i64, month: u8, day: u8) -> Date {
        Date { year, month, day }
    }

    fn parse(raw: &str, lang: &str) -> Result<Date, ValueError> {
        parse_date(raw, lang, Gregorian)
    }

    #[test]
    fn formats() {
        let cases = [
            ("12 May 1950", "en", date(1950, 5, 12)),
            ("May 12, 1950", "en", date(1950, 5, 12)),
            ("Sep 3rd, 1950", "en", date(1950, 9, 3)),
            ("May 1950", "en", date(1950, 5, 0)),
            ("1950", "en", date(1950, 0, 0)),
            ("[[1950]]", "en", date(1950, 0, 0)),
            ("1950-05-12", "en", date(1950, 5, 12)),
            ("1950–05–12", "en", date(1950, 5, 12)),
            ("12.05.1950", "de", date(1950, 5, 12)),
            ("12. Mai 1950", "de", date(1950, 5, 12)),
            ("1er janvier 1950", "fr", date(1950, 1, 1)),
            ("12 de mayo de 1950", "es", date(1950, 5, 12)),
            ("12 XI 1950", "pl", date(1950, 11, 12)),
            ("1950年5月12日", "ja", date(1950, 5, 12)),
            ("1950年（昭和25年）5月", "ja", date(1950, 5, 0)),
            ("１９５０年", "ja", date(1950, 0, 0)),
            ("1950년 5월 12일", "ko", date(1950, 5, 12)),
            ("1950. május 12.", "hu", date(1950, 5, 12)),
            ("१२ मई १९५०", "hi", date(1950, 5, 12)),
            // #110: short years only when they stand alone
            ("950", "en", date(950, 0, 0)),
            ("3 March 950", "en", date(950, 3, 3)),
        ];
        for (raw, lang, expected) in cases {
            assert_eq!(parse(raw, lang), Ok(expected), "{raw} ({lang})");
        }
    }

    #[test]
    fn rejections() {
        assert_eq!(parse("vor 1888", "de"), Err(ValueError::ImpreciseDate));
        assert_eq!(parse("los años 1410", "es"), Err(ValueError::ImpreciseDate)); // #31
        assert_eq!(parse("c. 1900?", "en"), Err(ValueError::ImpreciseDate));
        assert_eq!(parse("1950–1960", "en"), Err(ValueError::AmbiguousDate));
        assert_eq!(
            parse("31 February 1950", "en"),
            Err(ValueError::InvalidDate)
        );
        assert_eq!(parse("unknown", "en"), Err(ValueError::NoDate));
        assert_eq!(parse("19501", "en"), Err(ValueError::NoDate));
    }

    #[test]
    fn leap_years_depend_on_calendar() {
        assert!(Date::new(1700, 2, 29, Gregorian).is_none());
        assert!(Date::new(1700, 2, 29, Julian).is_some());
        assert!(Date::new(2000, 2, 29, Gregorian).is_some());
    }

    #[test]
    fn abbreviations_skip_ambiguous_prefixes() {
        // Czech: červen (6) / červenec (7) share "čer".
        let months = &LangPatterns::get("cs").unwrap().months;
        assert!(!months.contains_key("čer"));
        assert_eq!(parse("1. července 1950", "cs").map(|d| d.month), Ok(7));
    }

    #[test]
    fn date_parts() {
        let p = |y, m, d| parse_date_parts(y, m, d, "en", Gregorian);
        assert_eq!(p("1950", Some("5"), Some("12")), Ok(date(1950, 5, 12)));
        assert_eq!(p("1950", Some("May"), None), Ok(date(1950, 5, 0)));
        assert_eq!(p("1950", None, None), Ok(date(1950, 0, 0)));
        assert_eq!(p("1950", Some("13"), None), Err(ValueError::InvalidDate));
        assert_eq!(p("x", None, None), Err(ValueError::NoDate));
    }

    #[test]
    fn precision_and_wikibase_format() {
        assert_eq!(date(1950, 5, 0).precision(), 10);
        assert_eq!(date(950, 0, 0).wikibase_time(), "+0950-00-00T00:00:00Z");
    }

    #[test]
    fn limits() {
        let at_least = DateLimit {
            relation: Relation::AtLeast,
            date: date(1582, 10, 15),
        };
        assert!(at_least.accepts(&date(1583, 0, 0)));
        assert!(at_least.accepts(&date(1582, 10, 15)));
        assert!(!at_least.accepts(&date(1582, 0, 0)));
        let before = DateLimit {
            relation: Relation::Before,
            date: date(1926, 0, 0),
        };
        assert!(before.accepts(&date(1925, 0, 0)));
        assert!(!before.accepts(&date(1926, 3, 1)));
    }
}
