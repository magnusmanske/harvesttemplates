//! Just enough wikitext handling to read template parameters reliably.
//! Deliberately not a full parser: no expansion, no parser functions.

use regex::Regex;
use std::sync::LazyLock;

mod clean;
mod scan;
mod template;

pub use clean::{clean_value, clean_value_with};
pub use template::{TemplateMatcher, TemplateParams};

static HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^=+[^=\n]+=+[ \t]*$").unwrap());

/// The text before the first section heading: where infoboxes about the page subject live (#122).
pub fn lead_section(wikitext: &str) -> &str {
    HEADING.find(wikitext).map_or(wikitext, |m| &wikitext[..m.start()])
}

/// MediaWiki's first-letter capitalisation of titles: `foo bar` → `Foo bar`.
pub fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lead() {
        assert_eq!(lead_section("{{Infobox}}\nText\n== History ==\n{{Infobox radar}}"), "{{Infobox}}\nText\n");
        assert_eq!(lead_section("no headings, a = b = c"), "no headings, a = b = c");
    }
}
