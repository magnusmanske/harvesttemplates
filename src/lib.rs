//! HarvestTemplates: copy template parameters from Wikimedia wikis into Wikidata.
//! See `docs/ARCHITECTURE.md` for the big picture.

#![forbid(unsafe_code)]
#![warn(
    clippy::cognitive_complexity,
    clippy::dbg_macro,
    clippy::missing_const_for_fn,
    clippy::print_stderr,
    clippy::print_stdout,
    clippy::semicolon_if_nothing_returned,
    clippy::unused_self,
    clippy::wildcard_imports,
    missing_debug_implementations,
    unused_extern_crates
)]

pub mod api;
pub mod app_state;
pub mod auth;
pub mod config;
pub mod constraints;
pub mod harvest;
pub mod http;
pub mod ids;
pub mod storage;
#[cfg(test)]
pub(crate) mod test_support;
pub mod value;
pub mod wiki;
pub mod wikidata;
pub mod wikitext;
