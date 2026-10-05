//! Just enough wikitext handling to read template parameters reliably.
//! Deliberately not a full parser: no expansion, no parser functions.

mod clean;
mod scan;
mod template;

pub use clean::clean_value;
pub use template::{TemplateMatcher, TemplateParams};
