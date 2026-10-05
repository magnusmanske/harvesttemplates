//! Just enough wikitext handling to read template parameters reliably.
//! Deliberately not a full parser: no expansion, no parser functions.

mod clean;
mod scan;
mod template;

pub use clean::clean_value;
pub use template::{TemplateMatcher, TemplateParams};

/// MediaWiki's first-letter capitalisation of titles: `foo bar` → `Foo bar`.
pub fn uppercase_first(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
