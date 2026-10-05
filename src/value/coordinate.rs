//! Globe coordinates (#16): decimal pairs, degrees/minutes/seconds with
//! hemispheres, and `{{coord}}`-style templates.

use super::ValueError;
use super::numerals::to_ascii_digits;
use crate::wikitext::{TemplateMatcher, clean_value};
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Coordinate {
    pub latitude: f64,
    pub longitude: f64,
    /// In degrees, as Wikibase expects: 1, 1/60, 1/3600, 0.01, …
    pub precision: f64,
}

/// A coordinate from a template value. A nested `{{coord|…}}` (or a template
/// with that name in the wiki's language, via `template_prefixes`) is read
/// from its unnamed parameters; anything else is read as text.
pub fn parse_coordinate(raw: &str, template_prefixes: &[String]) -> Result<Coordinate, ValueError> {
    let matcher = TemplateMatcher::new(["Coord", "Coordinate", "Coordinates"], template_prefixes, true);
    let text = match matcher.find(raw) {
        Some(params) => (1..)
            .map_while(|i| params.get(&i.to_string()))
            .take_while(|v| is_coordinate_part(v))
            .collect::<Vec<_>>()
            .join(" "),
        None => clean_value(raw),
    };
    from_tokens(&tokens(&text))
}

/// `{{coord}}` options like `type:city(3500000)` follow the numbers; they are not part of the position.
fn is_coordinate_part(value: &str) -> bool {
    let value = to_ascii_digits(value.trim());
    value.parse::<f64>().is_ok() || ["N", "S", "E", "W"].contains(&value.to_uppercase().as_str())
}

/// A coordinate from separate latitude and longitude values.
pub fn parse_coordinate_parts(latitude: &str, longitude: &str) -> Result<Coordinate, ValueError> {
    let (lat, lon) = (tokens(&clean_value(latitude)), tokens(&clean_value(longitude)));
    from_tokens(&[lat, lon].concat())
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(String),
    Hemisphere(char),
}

/// Numbers and stand-alone N/S/E/W; everything else separates.
fn tokens(text: &str) -> Vec<Token> {
    let text = to_ascii_digits(text).replace('−', "-");
    let chars: Vec<char> = text.chars().collect();
    let mut out = vec![];
    let mut number = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let is_number_char = c.is_ascii_digit() || (c == '.' && !number.is_empty()) || (c == '-' && number.is_empty());
        if is_number_char {
            number.push(c);
            continue;
        }
        if !number.is_empty() {
            out.push(Token::Number(std::mem::take(&mut number)));
        }
        let letter_around = |j: Option<usize>| j.and_then(|j| chars.get(j)).is_some_and(|c| c.is_alphabetic());
        let alone = !letter_around(i.checked_sub(1)) && !letter_around(Some(i + 1));
        if alone && "NSEWnsew".contains(c) {
            out.push(Token::Hemisphere(c.to_ascii_uppercase()));
        }
    }
    if !number.is_empty() {
        out.push(Token::Number(number));
    }
    out
}

fn from_tokens(tokens: &[Token]) -> Result<Coordinate, ValueError> {
    let hemispheres: Vec<usize> =
        tokens.iter().enumerate().filter(|(_, t)| matches!(t, Token::Hemisphere(_))).map(|(i, _)| i).collect();
    let (lat, lon) = match hemispheres[..] {
        [] if tokens.len() == 2 => (angle(&tokens[..1], 'N')?, angle(&tokens[1..], 'E')?),
        [a, b] if b == tokens.len() - 1 => {
            let (first, second) = (angle(&tokens[..=a], 'N')?, angle(&tokens[a + 1..=b], 'E')?);
            if is_longitude(&tokens[a]) { (second, first) } else { (first, second) }
        }
        _ => return Err(ValueError::NoCoordinate),
    };
    let precision = lat.1.min(lon.1);
    let valid = lat.0.abs() <= 90.0 && lon.0.abs() <= 180.0;
    valid.then_some(Coordinate { latitude: lat.0, longitude: lon.0, precision }).ok_or(ValueError::NoCoordinate)
}

const fn is_longitude(token: &Token) -> bool {
    matches!(token, Token::Hemisphere('E' | 'W'))
}

/// Degrees (and precision) from 1–3 numbers, optionally ending with a hemisphere.
fn angle(tokens: &[Token], default_hemisphere: char) -> Result<(f64, f64), ValueError> {
    let (numbers, hemisphere) = match tokens.split_last() {
        Some((Token::Hemisphere(h), rest)) => (rest, *h),
        _ => (tokens, default_hemisphere),
    };
    let numbers: Vec<&str> = numbers
        .iter()
        .map(|t| if let Token::Number(n) = t { Ok(n.as_str()) } else { Err(ValueError::NoCoordinate) })
        .collect::<Result<_, _>>()?;
    if numbers.is_empty() || numbers.len() > 3 {
        return Err(ValueError::NoCoordinate);
    }
    let mut degrees = 0.0;
    let mut precision = 1.0;
    for (i, n) in numbers.iter().enumerate() {
        let value: f64 = n.parse().map_err(|_| ValueError::NoCoordinate)?;
        let unit = 60f64.powi(i as i32);
        if i > 0 && !(0.0..60.0).contains(&value) {
            return Err(ValueError::NoCoordinate);
        }
        let decimals = n.split_once('.').map_or(0, |(_, d)| d.len());
        precision = 10f64.powi(-(decimals as i32)) / unit;
        degrees += value.abs() / unit;
    }
    let negative = numbers[0].starts_with('-') ^ matches!(hemisphere, 'S' | 'W');
    Ok((if negative { -degrees } else { degrees }, precision))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<(f64, f64, f64), ValueError> {
        parse_coordinate(raw, &["Template".to_string(), "Vorlage".to_string()])
            .map(|c| (c.latitude, c.longitude, c.precision))
    }

    fn close(a: (f64, f64, f64), b: (f64, f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9 && (a.2 - b.2).abs() < 1e-12
    }

    #[test]
    fn formats() {
        let cases = [
            ("52.52, 13.405", (52.52, 13.405, 0.001)),
            ("-33.86; 151.21", (-33.86, 151.21, 0.01)),
            ("52°31′12″N 13°24′36″E", (52.52, 13.41, 1.0 / 3600.0)),
            ("52°31'N, 13°24'E", (52.0 + 31.0 / 60.0, 13.4, 1.0 / 60.0)),
            ("33°52′S 151°12′E", (-(33.0 + 52.0 / 60.0), 151.2, 1.0 / 60.0)),
            ("12.5 S 77.03 W", (-12.5, -77.03, 0.01)),
            ("{{coord|52|31|12|N|13|24|36|E|display=inline,title}}", (52.52, 13.41, 1.0 / 3600.0)),
            ("{{Coord|48.8584|2.2945|type:landmark}}", (48.8584, 2.2945, 0.0001)),
            (
                "Berlin {{coord|52|31|N|13|24|E|region:DE-BE_type:city(3500000)}}",
                (52.0 + 31.0 / 60.0, 13.4, 1.0 / 60.0),
            ),
            ("٥٢, ١٣", (52.0, 13.0, 1.0)),
        ];
        for (raw, expected) in cases {
            let got = parse(raw).unwrap_or_else(|e| panic!("{raw}: {e}"));
            assert!(close(got, expected), "{raw}: got {got:?}, expected {expected:?}");
        }
    }

    #[test]
    fn rejections() {
        for raw in ["", "somewhere", "95, 13", "52, 190", "52 61 N 13 E", "52 N", "1 2 3"] {
            assert_eq!(parse(raw), Err(ValueError::NoCoordinate), "{raw}");
        }
    }

    #[test]
    fn separate_parts() {
        let c = parse_coordinate_parts("52.52", "13.405").unwrap();
        assert_eq!((c.latitude, c.longitude), (52.52, 13.405));
        let c = parse_coordinate_parts("33 52 S", "151 12 E").unwrap();
        assert!(c.latitude < 0.0 && c.longitude > 151.0);
    }
}
