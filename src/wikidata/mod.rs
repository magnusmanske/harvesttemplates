//! Reading from Wikidata and the query service, and building statements.
//! Edits are in [`crate::auth::edit`], since they need the user's OAuth token.

pub mod entity;
pub mod statement;
pub mod wdqs;

pub use entity::{ConstraintDef, ConstraintStatus, Entity, HOST, PropertyInfo, Sitelink, Wikidata};
pub use statement::{Qualifier, Source, statement};
pub use wdqs::Wdqs;
