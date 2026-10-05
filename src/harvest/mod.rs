//! Harvest runs: from a [`JobSpec`] to candidate rows to edits.

pub mod job;
pub mod pipeline;
pub mod spec;

pub use job::{Job, JobError};
pub use spec::{JobSpec, SkipIf};
