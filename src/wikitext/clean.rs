use super::scan::{Call, find_top_level, scan_call};
use regex::Regex;
use std::sync::LazyLock;

static LINK_LABEL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[\[([^\[\]|]*)\|[^\[\]]*\]\]").unwrap());
static BOLD_ITALIC: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"'{2,}").unwrap());

/// Reduce a raw parameter value to the text a value parser should see:
/// nested templates removed, `[[target|label]]` reduced to `[[target]]`,
/// bold/italic markup and `&nbsp;` gone, whitespace collapsed.
/// `{{!}}` counts as a pipe; outside links, only the part before it is kept.
/// Punctuation alone (`、`, `–`, `?`) counts as no value (#147).
pub fn clean_value(raw: &str) -> String {
    clean_value_with(raw, false)
}

/// Like [`clean_value`]; with `unwrap`, a nested template is replaced by its
/// first unnamed parameter instead of being removed: `{{URL|example.org}}` → `example.org` (#2).
pub fn clean_value_with(raw: &str, unwrap: bool) -> String {
    let value = raw.replace("{{!}}", "|");
    let value = remove_templates(&value, unwrap);
    let value = LINK_LABEL.replace_all(&value, "[[$1]]");
    let value = BOLD_ITALIC.replace_all(&value, "");
    let value = value.replace("&nbsp;", " ");
    let value = match find_top_level(&value, b'|') {
        Some(pipe) => &value[..pipe],
        None => &value,
    };
    let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if value.chars().any(char::is_alphanumeric) { value } else { String::new() }
}

fn remove_templates(text: &str, unwrap: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    while let Some(offset) = text[pos..].find("{{") {
        let start = pos + offset;
        out.push_str(&text[pos..start]);
        pos = match scan_call(text, start) {
            Some(call) => {
                if unwrap {
                    out.push_str(&first_unnamed(text, &call));
                }
                call.end
            }
            None => start + 2,
        };
    }
    out.push_str(&text[pos..]);
    out
}

fn first_unnamed(text: &str, call: &Call) -> String {
    let parts = call.parts[1..].iter().map(|r| &text[r.clone()]);
    parts
        .into_iter()
        .find(|p| find_top_level(p, b'=').is_none())
        .map_or_else(String::new, |p| remove_templates(p.trim(), true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases() {
        let cases = [
            ("[[Paris|the city]]", "[[Paris]]"),
            ("[[File:A b.jpg|300px]]<br/>caption", "[[File:A b.jpg]]<br/>caption"),
            ("'''bold''' and ''italic''", "bold and italic"),
            ("1&nbsp;000", "1 000"),
            ("{{flag|FR}} [[France]]", "[[France]]"),
            ("{{a|{{b}}}}x", "x"),
            ("  many \n  spaces ", "many spaces"),
            // #132
            ("[[Target{{!}}Label]]", "[[Target]]"),
            ("Page{{!}}Text", "Page"),
            // #147
            ("{{NCID|BN1028867X}}、{{NCID|BA90640025}}", ""),
            ("–", ""),
        ];
        for (raw, expected) in cases {
            assert_eq!(clean_value(raw), expected, "input: {raw}");
        }
    }

    #[test]
    fn unwrapping() {
        let cases = [
            ("{{URL|brenntag.com}}", "brenntag.com"),
            ("{{URL|1=example.org|name}}", "name"),
            ("{{lang|fr|{{nobr|Jean}}}}", "fr"),
            ("{{NCID|BN1028867X}}、{{NCID|BA90640025}}", "BN1028867X、BA90640025"),
            ("{{flag}} France", "France"),
        ];
        for (raw, expected) in cases {
            assert_eq!(clean_value_with(raw, true), expected, "input: {raw}");
        }
    }
}
