//! Harvest runs: from a [`JobSpec`] to candidate rows to edits.
//! See `docs/ARCHITECTURE.md` for the lifecycle.

pub mod active;
pub mod job;
pub mod load;
pub mod pipeline;
pub mod spec;
pub mod status;
pub mod worker;

pub use active::{ActiveRuns, Claim, ClaimError};
pub use job::{Job, JobError};
pub use spec::{JobSpec, SkipIf};
pub use status::{RowStatus, RunStatus};
