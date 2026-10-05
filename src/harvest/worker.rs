//! Background tasks: loading a run's candidates, and working through its rows.

use super::active::Claim;
use super::job::Job;
use super::load::candidates;
use super::pipeline::{Outcome, PlannedEdit, Rejection, evaluate, summary};
use super::status::{RowStatus, RunStatus};
use crate::app_state::AppState;
use crate::auth::{EditError, Editor};
use crate::storage::{RowRecord, RowUpdate, RunRecord};
use crate::wiki::Page;
use crate::wiki::content::{self, Revision};
use anyhow::{Result, bail};
use std::sync::Arc;
use std::time::Duration;

const BATCH: u32 = 50;
/// Network failures in a row before we stop rather than mark every row as failed.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Collect a new run's candidates, then mark it ready (or failed, with the reason).
pub async fn load(app: Arc<AppState>, run_id: u64, job: Job, claim: Claim) {
    let status = match load_rows(&app, run_id, &job).await {
        Ok(()) => (RunStatus::Ready, None),
        Err(e) => (RunStatus::Failed, Some(format!("{e:#}"))),
    };
    set_final_status(&app, run_id, status).await;
    drop(claim);
}

async fn load_rows(app: &AppState, run_id: u64, job: &Job) -> Result<()> {
    let (pages, excluded) = candidates(job, &app.clients, app.pages.as_ref(), app.limits()).await?;
    app.store.add_rows(run_id, &pages).await?;
    app.store.set_excluded(run_id, &serde_json::to_value(excluded)?).await
}

/// Errors here can only be logged: there is no one left to tell.
async fn set_final_status(app: &AppState, run_id: u64, (status, message): (RunStatus, Option<String>)) {
    tracing::info!(
        "run {run_id}: {} {}",
        status.as_str(),
        message.as_deref().unwrap_or_default()
    );
    if let Err(e) = app.store.set_status(run_id, status, message.as_deref()).await {
        tracing::error!("run {run_id}: cannot store status: {e:#}");
    }
}

#[derive(Debug)]
pub enum Mode {
    /// Evaluate rows without editing.
    Preview,
    Edit(Box<Editor>),
}

impl Mode {
    const fn statuses(&self) -> &'static [RowStatus] {
        match self {
            Self::Preview => &[RowStatus::Pending],
            Self::Edit(_) => &[RowStatus::Pending, RowStatus::Ready],
        }
    }

    const fn running(&self) -> RunStatus {
        match self {
            Self::Preview => RunStatus::Previewing,
            Self::Edit(_) => RunStatus::Editing,
        }
    }

    const fn finished(&self) -> RunStatus {
        match self {
            Self::Preview => RunStatus::Ready,
            Self::Edit(_) => RunStatus::Done,
        }
    }
}

enum Flow {
    Continue,
    Stop,
}

enum Saved {
    Ok(RowStatus),
    Rejected(String),
    Stopped,
}

#[derive(Debug)]
pub struct Worker {
    app: Arc<AppState>,
    run: RunRecord,
    job: Job,
    mode: Mode,
    claim: Claim,
    failures: u32,
}

impl Worker {
    pub const fn new(app: Arc<AppState>, run: RunRecord, job: Job, mode: Mode, claim: Claim) -> Self {
        Self {
            app,
            run,
            job,
            mode,
            claim,
            failures: 0,
        }
    }

    pub async fn run(mut self) {
        let id = self.run.id;
        let (status, message) = match self.process().await {
            Ok(Flow::Continue) => (self.mode.finished(), None),
            Ok(Flow::Stop) => (RunStatus::Paused, None),
            Err(e) => (RunStatus::Failed, Some(format!("{e:#}"))),
        };
        tracing::info!("run {id}: {}", status.as_str());
        if let Err(e) = self.finish(status, message.as_deref()).await {
            tracing::error!("run {id}: cannot store status: {e:#}");
        }
    }

    async fn finish(&self, status: RunStatus, message: Option<&str>) -> Result<()> {
        let store = &self.app.store;
        store.set_status(self.run.id, status, message).await?;
        if let (RunStatus::Done, Some(share)) = (status, self.run.share_id) {
            store
                .record_share_run(share, self.run.id, &store.counts(self.run.id).await?)
                .await?;
        }
        Ok(())
    }

    async fn process(&mut self) -> Result<Flow> {
        self.app
            .store
            .set_status(self.run.id, self.mode.running(), None)
            .await?;
        let mut after = None;
        loop {
            let rows = self
                .app
                .store
                .rows_to_process(self.run.id, self.mode.statuses(), after, BATCH)
                .await?;
            let Some(last) = rows.last() else {
                return Ok(Flow::Continue);
            };
            after = Some(last.seq);
            let ids: Vec<u64> = rows.iter().map(|r| r.page_id).collect();
            let revisions = content::revisions(&self.app.clients.mw, &self.job.site, &ids).await?;
            for row in &rows {
                if self.claim.stop_requested() {
                    return Ok(Flow::Stop);
                }
                if let Flow::Stop = self.row(row, revisions.get(&row.page_id)).await? {
                    return Ok(Flow::Stop);
                }
            }
        }
    }

    /// Re-evaluates the row against the current page and item, so nothing stale is written.
    async fn row(&mut self, row: &RowRecord, revision: Option<&Revision>) -> Result<Flow> {
        let item = row.item.as_deref().and_then(|q| q.parse().ok());
        let page = Page {
            id: row.page_id,
            title: row.title.clone(),
            item,
            latest_revision: 0,
        };
        let outcome = match revision {
            Some(revision) => evaluate(&self.job, &self.app.clients, &page, revision).await,
            None => Outcome {
                raw: None,
                value: None,
                result: Err(Rejection::Error("the page no longer exists".into())),
            },
        };
        let (status, message, item) = match &outcome.result {
            Err(Rejection::Skip(m)) => (RowStatus::Skipped, Some(m.clone()), None),
            Err(Rejection::Error(m)) => (RowStatus::Error, Some(m.clone()), None),
            Ok(edit) => match self.save(edit).await? {
                Saved::Ok(status) => (status, None, Some(edit.item.to_string())),
                Saved::Rejected(m) => (RowStatus::Error, Some(m), Some(edit.item.to_string())),
                Saved::Stopped => return Ok(Flow::Stop),
            },
        };
        let update = RowUpdate {
            status,
            item: item.as_deref(),
            raw_value: outcome.raw.as_deref(),
            value: outcome.value.as_deref(),
            message: message.as_deref(),
        };
        self.app.store.update_row(self.run.id, row.seq, &update).await?;
        Ok(Flow::Continue)
    }

    async fn save(&mut self, edit: &PlannedEdit) -> Result<Saved> {
        let Mode::Edit(editor) = &mut self.mode else {
            return Ok(Saved::Ok(RowStatus::Ready));
        };
        let summary = summary(&self.job, &edit.value, &self.run.editgroup);
        let interval = Duration::from_millis(self.app.config.harvest.edit_interval_ms);
        loop {
            match editor
                .add_statements(edit.item, std::slice::from_ref(&edit.statement), &summary)
                .await
            {
                Ok(()) => {
                    self.failures = 0;
                    tokio::time::sleep(interval).await;
                    return Ok(Saved::Ok(RowStatus::Done));
                }
                Err(EditError::Busy(wait)) if !wait_unless_stopped(&self.claim, wait).await => {
                    return Ok(Saved::Stopped);
                }
                Err(EditError::Busy(_)) => {}
                Err(EditError::Rejected(m)) => return Ok(Saved::Rejected(m)),
                Err(EditError::Fatal(m)) => bail!("{m}"),
                Err(EditError::Failed(e)) => {
                    self.failures += 1;
                    if self.failures >= MAX_CONSECUTIVE_FAILURES {
                        bail!("{} edits in a row failed, last: {e:#}", self.failures);
                    }
                    return Ok(Saved::Rejected(format!("edit failed: {e:#}")));
                }
            }
        }
    }
}

/// Sleep, waking every second to honour a stop request. `false` if stopped.
async fn wait_unless_stopped(claim: &Claim, duration: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + duration;
    while tokio::time::Instant::now() < deadline {
        if claim.stop_requested() {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(1).min(duration)).await;
    }
    !claim.stop_requested()
}

#[cfg(test)]
mod tests;
