//! Health and metrics for the Buggregator service dashboard (see `wikimisc::telemetry`).
//! Metric names live here so the set stays small and every label bounded.

use wikimisc::metrics::{counter, gauge};
use wikimisc::telemetry::Telemetry;

/// Starts pushing to the dashboard (token from `$DASHBOARD_TOKEN`). Optional: without a token,
/// or on any error, the service just runs without it. On SIGTERM/SIGINT it sends a final push
/// and exits; interrupted runs are paused by `recover_after_restart` as before.
pub fn start() {
    Telemetry::builder("web").version(env!("CARGO_PKG_VERSION")).install_or_log();
}

/// A run's task ended. `phase` is `load`, `preview` or `edit`; `status` a `RunStatus` name.
pub fn record_run(phase: &'static str, status: &'static str) {
    counter!("ht_runs_total", "phase" => phase, "status" => status).increment(1);
}

/// A row was processed; with `mode` `edit` and `status` `done`, that is an edit to Wikidata.
pub fn record_row(mode: &'static str, status: &'static str) {
    counter!("ht_rows_total", "mode" => mode, "status" => status).increment(1);
}

/// Wikidata asked us to wait (maxlag, rate limit) before editing again.
pub fn record_edit_wait() {
    counter!("ht_edit_waits_total").increment(1);
}

/// Runs a task is working on right now.
pub fn set_active_runs(count: usize) {
    gauge!("ht_runs_active").set(count as f64);
}
