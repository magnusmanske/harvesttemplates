//! Harvest runs: from a [`JobSpec`] to candidate rows to edits.

pub mod job;
pub mod pipeline;
pub mod spec;
pub mod status;

pub use job::{Job, JobError};
pub use spec::{JobSpec, SkipIf};
pub use status::{RowStatus, RunStatus};
