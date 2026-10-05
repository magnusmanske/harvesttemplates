//! Access to the source wikis: site metadata, the MediaWiki API and the replicas.

pub mod api;
pub mod content;
pub mod pages;
pub mod replica;
pub mod site;

pub use api::MwApi;
pub use pages::{ApiSource, Limits, Page, PageSource, WithFallback};
pub use replica::Replicas;
pub use site::Site;
