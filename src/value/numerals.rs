/// First code point of each decimal digit block we convert (digits are contiguous).
const ZEROS: [char; 15] = [
    '\u{0660}', // Arabic-Indic
    '\u{06F0}', // Extended Arabic-Indic (Persian, Urdu)
    '\u{0966}', // Devanagari
    '\u{09E6}', // Bengali, Assamese
    '\u{0AE6}', // Gujarati
    '\u{0BE6}', // Tamil
    '\u{0C66}', // Telugu
    '\u{0CE6}', // Kannada
    '\u{0D66}', // Malayalam
    '\u{0E50}', // Thai
    '\u{0ED0}', // Lao
    '\u{1040}', // Myanmar
    '\u{17E0}', // Khmer
    '\u{FF10}', // Fullwidth
    '\u{0A66}', // Gurmukhi
];

const CJK: [(char, char); 12] = [
    ('〇', '0'),
    ('○', '0'),
    ('零', '0'),
    ('一', '1'),
    ('二', '2'),
    ('三', '3'),
    ('四', '4'),
    ('五', '5'),
    ('六', '6'),
    ('七', '7'),
    ('八', '8'),
    ('九', '9'),
];

/// Replace digits of other scripts with ASCII digits. CJK numerals are mapped
/// digit by digit (`二〇一五` → `2015`); positional forms like `十` are not supported.
pub fn to_ascii_digits(s: &str) -> String {
    s.chars().map(ascii_digit).collect()
}

fn ascii_digit(c: char) -> char {
    let block = ZEROS.iter().find_map(|&zero| {
        let offset = (c as u32).checked_sub(zero as u32)?;
        (offset < 10).then(|| char::from(b'0' + offset as u8))
    });
    block.or_else(|| CJK.iter().find(|(k, _)| *k == c).map(|(_, v)| *v)).unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts() {
        assert_eq!(to_ascii_digits("١٩٥٠"), "1950");
        assert_eq!(to_ascii_digits("۱۹۵۰"), "1950");
        assert_eq!(to_ascii_digits("१९५०"), "1950");
        assert_eq!(to_ascii_digits("౧౯౫౦"), "1950");
        assert_eq!(to_ascii_digits("໑໙໗໐"), "1970");
        assert_eq!(to_ascii_digits("１９５０年"), "1950年");
        assert_eq!(to_ascii_digits("二〇一五年"), "2015年");
        assert_eq!(to_ascii_digits("abc 12"), "abc 12");
    }
}
