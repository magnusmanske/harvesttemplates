use super::scan::{find_top_level, scan_call};
use regex::Regex;
use std::sync::LazyLock;

static LINK_LABEL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\[\[([^\[\]|]*)\|[^\[\]]*\]\]").unwrap());
static BOLD_ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'{2,}").unwrap());

/// Reduce a raw parameter value to the text a value parser should see:
/// nested templates removed, `[[target|label]]` reduced to `[[target]]`,
/// bold/italic markup and `&nbsp;` gone, whitespace collapsed.
/// `{{!}}` counts as a pipe; outside links, only the part before it is kept.
pub fn clean_value(raw: &str) -> String {
    let value = raw.replace("{{!}}", "|");
    let value = remove_templates(&value);
    let value = LINK_LABEL.replace_all(&value, "[[$1]]");
    let value = BOLD_ITALIC.replace_all(&value, "");
    let value = value.replace("&nbsp;", " ");
    let value = match find_top_level(&value, b'|') {
        Some(pipe) => &value[..pipe],
        None => &value,
    };
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn remove_templates(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(offset) = text[pos..].find("{{") {
        let start = pos + offset;
        out.push_str(&text[pos..start]);
        pos = scan_call(text, start).map_or(start + 2, |call| call.end);
    }
    out.push_str(&text[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases() {
        let cases = [
            ("[[Paris|the city]]", "[[Paris]]"),
            (
                "[[File:A b.jpg|300px]]<br/>caption",
                "[[File:A b.jpg]]<br/>caption",
            ),
            ("'''bold''' and ''italic''", "bold and italic"),
            ("1&nbsp;000", "1 000"),
            ("{{flag|FR}} [[France]]", "[[France]]"),
            ("{{a|{{b}}}}x", "x"),
            ("  many \n  spaces ", "many spaces"),
            // #132
            ("[[Target{{!}}Label]]", "[[Target]]"),
            ("Page{{!}}Text", "Page"),
        ];
        for (raw, expected) in cases {
            assert_eq!(clean_value(raw), expected, "input: {raw}");
        }
    }
}
