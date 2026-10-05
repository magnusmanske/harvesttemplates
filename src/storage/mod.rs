//! Runs, their rows, and shared queries, in ToolsDB. All SQL lives here.

use crate::config::{DbConfig, DbUser};
use crate::harvest::JobSpec;
use crate::harvest::status::{RowStatus, RunStatus};
use crate::wiki::Page;
use anyhow::{Context, Result};
use mysql_async::prelude::{FromValue, Queryable};
use mysql_async::{Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, Row, Value};
use serde::Serialize;
use serde_json::Value as Json;
use std::collections::HashMap;

const INSERT_CHUNK: usize = 500;

/// Who owns a run or share. The id is the Wikidata user id, which survives renames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owner {
    pub id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RunRecord {
    pub id: u64,
    #[serde(skip)]
    pub user_id: u64,
    pub user_name: String,
    pub share_id: Option<u64>,
    pub spec: JobSpec,
    pub status: RunStatus,
    pub editgroup: String,
    /// Candidates left out while loading, by reason.
    pub excluded: Option<Json>,
    pub message: Option<String>,
    pub created: i64,
    pub started: Option<i64>,
    pub finished: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RowRecord {
    pub seq: u32,
    pub page_id: u64,
    pub title: String,
    pub item: Option<String>,
    pub status: RowStatus,
    pub raw_value: Option<String>,
    pub value: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RowUpdate<'a> {
    pub status: RowStatus,
    pub item: Option<&'a str>,
    pub raw_value: Option<&'a str>,
    pub value: Option<&'a str>,
    pub message: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub pending: u64,
    pub ready: u64,
    pub done: u64,
    pub skipped: u64,
    pub error: u64,
}

impl Counts {
    pub const fn total(&self) -> u64 {
        self.pending + self.ready + self.done + self.skipped + self.error
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ShareRecord {
    pub id: u64,
    #[serde(skip)]
    pub user_id: u64,
    pub user_name: String,
    pub title: String,
    pub spec: JobSpec,
    pub created: i64,
    pub last_run_id: Option<u64>,
    pub last_completed: Option<i64>,
    pub last_done: Option<u64>,
    pub last_errors: Option<u64>,
    pub tags: Vec<String>,
}

#[derive(Clone)]
pub struct Store {
    pool: Pool,
    opts: Opts,
}

/// Connection options hold the password, so they stay out of `Debug`.
impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("database", &self.opts.db_name()).finish_non_exhaustive()
    }
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl Store {
    pub fn new(config: &DbConfig, user: &DbUser) -> Self {
        let opts = OptsBuilder::default()
            .ip_or_hostname(&config.host)
            .tcp_port(config.port)
            .user(Some(&user.name))
            .pass(Some(user.password.expose()))
            .db_name(Some(&config.database));
        Self::with_opts(opts.into(), config.max_connections)
    }

    pub fn from_url(url: &str, max_connections: usize) -> Result<Self> {
        Ok(Self::with_opts(Opts::from_url(url).context("invalid database URL")?, max_connections))
    }

    fn with_opts(opts: Opts, max_connections: usize) -> Self {
        let constraints = PoolConstraints::new(0, max_connections.max(1)).unwrap_or_default();
        let pool_opts =
            OptsBuilder::from_opts(opts.clone()).pool_opts(PoolOpts::default().with_constraints(constraints));
        Self { pool: Pool::new(pool_opts), opts }
    }

    async fn conn(&self) -> Result<Conn> {
        self.pool.get_conn().await.context("cannot connect to the tool database")
    }

    /// Create the database if needed (Toolforge users may create `<user>__…`), then the tables.
    pub async fn migrate(&self) -> Result<()> {
        self.create_database().await?;
        let mut conn = self.conn().await?;
        for statement in sql_statements(include_str!("schema.sql")) {
            conn.query_drop(statement).await?;
        }
        Ok(())
    }

    async fn create_database(&self) -> Result<()> {
        let name = self.opts.db_name().context("no database configured")?;
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            anyhow::bail!("invalid database name '{name}'");
        }
        let opts = OptsBuilder::from_opts(self.opts.clone()).db_name(None::<String>);
        let mut conn = Conn::new(opts).await.context("cannot connect to the tool database server")?;
        conn.query_drop(format!("CREATE DATABASE IF NOT EXISTS `{name}`")).await?;
        conn.disconnect().await?;
        Ok(())
    }

    /// After a restart nothing is running: interrupted edits can be resumed, loads cannot.
    pub async fn recover_after_restart(&self) -> Result<()> {
        let mut conn = self.conn().await?;
        let sql = "UPDATE run SET status = IF(status = 'loading', 'failed', 'paused'),
                   message = IF(status = 'loading', 'interrupted by a restart; please load again', message)
                   WHERE status IN ('loading', 'previewing', 'editing')";
        conn.query_drop(sql).await?;
        Ok(())
    }

    // ---------- Runs ----------

    pub async fn create_run(&self, owner: &Owner, spec: &JobSpec, share_id: Option<u64>) -> Result<u64> {
        let editgroup: String = (0..12).map(|_| char::from(b"0123456789abcdef"[rand::random_range(0..16)])).collect();
        let sql = "INSERT INTO run (user_id, user_name, share_id, spec, status, editgroup, created)
                   VALUES (?, ?, ?, ?, 'loading', ?, ?)";
        let mut conn = self.conn().await?;
        let params = (owner.id, &owner.name, share_id, serde_json::to_string(spec)?, editgroup, now());
        conn.exec_drop(sql, params).await?;
        conn.last_insert_id().context("no run id")
    }

    pub async fn set_status(&self, id: u64, status: RunStatus, message: Option<&str>) -> Result<()> {
        let sql = "UPDATE run SET status = ?, message = ?,
                   started = IF(? AND started IS NULL, UNIX_TIMESTAMP(), started),
                   finished = IF(?, UNIX_TIMESTAMP(), NULL)
                   WHERE id = ?";
        let started = matches!(status, RunStatus::Previewing | RunStatus::Editing);
        let finished = matches!(status, RunStatus::Done | RunStatus::Failed);
        self.conn().await?.exec_drop(sql, (status.as_str(), message, started, finished, id)).await?;
        Ok(())
    }

    pub async fn set_excluded(&self, id: u64, excluded: &Json) -> Result<()> {
        let sql = "UPDATE run SET excluded = ? WHERE id = ?";
        self.conn().await?.exec_drop(sql, (excluded.to_string(), id)).await?;
        Ok(())
    }

    pub async fn add_rows(&self, id: u64, pages: &[Page]) -> Result<()> {
        let mut conn = self.conn().await?;
        for (chunk_no, chunk) in pages.chunks(INSERT_CHUNK).enumerate() {
            let placeholders = vec!["(?, ?, ?, ?, ?, 'pending')"; chunk.len()].join(", ");
            let sql = format!("INSERT INTO run_row (run_id, seq, page_id, title, item, status) VALUES {placeholders}");
            let mut values: Vec<Value> = Vec::with_capacity(chunk.len() * 5);
            for (i, page) in chunk.iter().enumerate() {
                let seq = (chunk_no * INSERT_CHUNK + i) as u32;
                let item = page.item.map(|q| q.to_string());
                values.extend([id.into(), seq.into(), page.id.into(), page.title.clone().into(), item.into()]);
            }
            conn.exec_drop(sql, values).await?;
        }
        Ok(())
    }

    pub async fn run(&self, id: u64) -> Result<Option<RunRecord>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM run WHERE id = ?");
        let row: Option<Row> = self.conn().await?.exec_first(sql, (id,)).await?;
        row.map(run_from_row).transpose()
    }

    pub async fn runs_of(&self, user_id: u64, limit: u32) -> Result<Vec<RunRecord>> {
        let sql = format!("SELECT {RUN_COLUMNS} FROM run WHERE user_id = ? ORDER BY id DESC LIMIT ?");
        let rows: Vec<Row> = self.conn().await?.exec(sql, (user_id, limit)).await?;
        rows.into_iter().map(run_from_row).collect()
    }

    pub async fn counts(&self, id: u64) -> Result<Counts> {
        let sql = "SELECT status, COUNT(*) FROM run_row WHERE run_id = ? GROUP BY status";
        let rows: Vec<(String, u64)> = self.conn().await?.exec(sql, (id,)).await?;
        let mut counts = Counts::default();
        for (status, n) in rows {
            let slot = match RowStatus::parse(&status) {
                Some(RowStatus::Pending) => &mut counts.pending,
                Some(RowStatus::Ready) => &mut counts.ready,
                Some(RowStatus::Done) => &mut counts.done,
                Some(RowStatus::Skipped) => &mut counts.skipped,
                Some(RowStatus::Error) | None => &mut counts.error,
            };
            *slot += n;
        }
        Ok(counts)
    }

    /// A page of rows for display, optionally of one status.
    pub async fn rows(&self, id: u64, status: Option<RowStatus>, offset: u32, limit: u32) -> Result<Vec<RowRecord>> {
        let sql = format!(
            "SELECT {ROW_COLUMNS} FROM run_row WHERE run_id = ? AND (? IS NULL OR status = ?) ORDER BY seq LIMIT ? OFFSET ?"
        );
        let status = status.map(RowStatus::as_str);
        let rows: Vec<Row> = self.conn().await?.exec(sql, (id, status, status, limit, offset)).await?;
        rows.into_iter().map(row_from_row).collect()
    }

    /// The next rows to work on, in order, after `after_seq`.
    pub async fn rows_to_process(
        &self,
        id: u64,
        statuses: &[RowStatus],
        after_seq: Option<u32>,
        limit: u32,
    ) -> Result<Vec<RowRecord>> {
        let placeholders = vec!["?"; statuses.len()].join(", ");
        let sql = format!(
            "SELECT {ROW_COLUMNS} FROM run_row WHERE run_id = ? AND seq > ? AND status IN ({placeholders}) ORDER BY seq LIMIT ?"
        );
        let mut values: Vec<Value> = vec![id.into(), after_seq.map_or(-1, i64::from).into()];
        values.extend(statuses.iter().map(|s| s.as_str().into()));
        values.push(limit.into());
        let rows: Vec<Row> = self.conn().await?.exec(sql, values).await?;
        rows.into_iter().map(row_from_row).collect()
    }

    pub async fn update_row(&self, id: u64, seq: u32, update: &RowUpdate<'_>) -> Result<()> {
        let sql = "UPDATE run_row SET status = ?, item = COALESCE(?, item), raw_value = ?, value = ?, message = ?
                   WHERE run_id = ? AND seq = ?";
        let u = update;
        let params = (u.status.as_str(), u.item, u.raw_value, u.value, u.message, id, seq);
        self.conn().await?.exec_drop(sql, params).await?;
        Ok(())
    }

    // ---------- Shares ----------

    pub async fn create_share(&self, owner: &Owner, title: &str, spec: &JobSpec) -> Result<u64> {
        let sql = "INSERT INTO share (user_id, user_name, title, spec, created) VALUES (?, ?, ?, ?, ?)";
        let mut conn = self.conn().await?;
        conn.exec_drop(sql, (owner.id, &owner.name, title, serde_json::to_string(spec)?, now())).await?;
        conn.last_insert_id().context("no share id")
    }

    pub async fn shares(&self) -> Result<Vec<ShareRecord>> {
        let mut conn = self.conn().await?;
        let rows: Vec<Row> = conn.query(format!("SELECT {SHARE_COLUMNS} FROM share ORDER BY id DESC")).await?;
        let mut tags: HashMap<u64, Vec<String>> = HashMap::new();
        for (share, tag) in conn.query::<(u64, String), _>("SELECT share_id, tag FROM share_tag ORDER BY tag").await? {
            tags.entry(share).or_default().push(tag);
        }
        let mut shares: Vec<ShareRecord> = rows.into_iter().map(share_from_row).collect::<Result<_>>()?;
        for share in &mut shares {
            share.tags = tags.remove(&share.id).unwrap_or_default();
        }
        Ok(shares)
    }

    pub async fn share(&self, id: u64) -> Result<Option<ShareRecord>> {
        let mut conn = self.conn().await?;
        let row: Option<Row> =
            conn.exec_first(format!("SELECT {SHARE_COLUMNS} FROM share WHERE id = ?"), (id,)).await?;
        let Some(mut share) = row.map(share_from_row).transpose()? else { return Ok(None) };
        share.tags = conn.exec("SELECT tag FROM share_tag WHERE share_id = ? ORDER BY tag", (id,)).await?;
        Ok(Some(share))
    }

    /// Replace a share's tags (#174). Ownership is the caller's to check.
    pub async fn set_tags(&self, id: u64, tags: &[String]) -> Result<()> {
        let mut tx = self.pool.start_transaction(mysql_async::TxOpts::default()).await?;
        tx.exec_drop("DELETE FROM share_tag WHERE share_id = ?", (id,)).await?;
        tx.exec_batch("INSERT INTO share_tag (share_id, tag) VALUES (?, ?)", tags.iter().map(|t| (id, t))).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Only the owner can delete. Returns whether a share was deleted.
    pub async fn delete_share(&self, id: u64, user_id: u64) -> Result<bool> {
        let mut conn = self.conn().await?;
        conn.exec_drop("DELETE FROM share WHERE id = ? AND user_id = ?", (id, user_id)).await?;
        let deleted = conn.affected_rows() > 0;
        if deleted {
            conn.exec_drop("DELETE FROM share_tag WHERE share_id = ?", (id,)).await?;
        }
        Ok(deleted)
    }

    /// Show how the last complete run of a shared query went (#137, #141).
    pub async fn record_share_run(&self, share_id: u64, run_id: u64, counts: &Counts) -> Result<()> {
        let sql = "UPDATE share SET last_run_id = ?, last_completed = ?, last_done = ?, last_errors = ? WHERE id = ?";
        let params = (run_id, now(), counts.done, counts.error, share_id);
        self.conn().await?.exec_drop(sql, params).await?;
        Ok(())
    }
}

const RUN_COLUMNS: &str =
    "id, user_id, user_name, share_id, spec, status, editgroup, excluded, message, created, started, finished";
const ROW_COLUMNS: &str = "seq, page_id, title, item, status, raw_value, value, message";
const SHARE_COLUMNS: &str =
    "id, user_id, user_name, title, spec, created, last_run_id, last_completed, last_done, last_errors";

/// Statements of an SQL script, without `--` comment lines.
fn sql_statements(script: &str) -> Vec<String> {
    let code: Vec<&str> = script.lines().filter(|l| !l.trim_start().starts_with("--")).collect();
    code.join("\n").split(';').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
}

fn take<T: FromValue>(row: &mut Row, column: &str) -> Result<T> {
    row.take_opt(column)
        .with_context(|| format!("column {column} missing"))?
        .with_context(|| format!("column {column} has an unexpected type"))
}

fn run_from_row(mut row: Row) -> Result<RunRecord> {
    let status: String = take(&mut row, "status")?;
    let spec: String = take(&mut row, "spec")?;
    let excluded: Option<String> = take(&mut row, "excluded")?;
    Ok(RunRecord {
        id: take(&mut row, "id")?,
        user_id: take(&mut row, "user_id")?,
        user_name: take(&mut row, "user_name")?,
        share_id: take(&mut row, "share_id")?,
        spec: serde_json::from_str(&spec)?,
        status: RunStatus::parse(&status).unwrap_or(RunStatus::Failed),
        editgroup: take(&mut row, "editgroup")?,
        excluded: excluded.and_then(|e| serde_json::from_str(&e).ok()),
        message: take(&mut row, "message")?,
        created: take(&mut row, "created")?,
        started: take(&mut row, "started")?,
        finished: take(&mut row, "finished")?,
    })
}

fn row_from_row(mut row: Row) -> Result<RowRecord> {
    let status: String = take(&mut row, "status")?;
    Ok(RowRecord {
        seq: take(&mut row, "seq")?,
        page_id: take(&mut row, "page_id")?,
        title: take(&mut row, "title")?,
        item: take(&mut row, "item")?,
        status: RowStatus::parse(&status).unwrap_or(RowStatus::Error),
        raw_value: take(&mut row, "raw_value")?,
        value: take(&mut row, "value")?,
        message: take(&mut row, "message")?,
    })
}

fn share_from_row(mut row: Row) -> Result<ShareRecord> {
    let spec: String = take(&mut row, "spec")?;
    Ok(ShareRecord {
        id: take(&mut row, "id")?,
        user_id: take(&mut row, "user_id")?,
        user_name: take(&mut row, "user_name")?,
        title: take(&mut row, "title")?,
        spec: serde_json::from_str(&spec)?,
        created: take(&mut row, "created")?,
        last_run_id: take(&mut row, "last_run_id")?,
        last_completed: take(&mut row, "last_completed")?,
        last_done: take(&mut row, "last_done")?,
        last_errors: take(&mut row, "last_errors")?,
        tags: vec![],
    })
}

#[cfg(test)]
mod tests;
