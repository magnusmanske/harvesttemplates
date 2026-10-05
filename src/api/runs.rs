use super::{ApiError, ApiResult, SharedState};
use crate::auth::{Editor, User, require_user};
use crate::harvest::worker::{self, Mode, Worker};
use crate::harvest::{Job, JobSpec, RowStatus, RunStatus};
use crate::storage::{Owner, RowRecord, RunRecord};
use crate::wiki::site::host_for;
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_sessions::Session;

const MAX_ROWS_PER_PAGE: u32 = 500;
const CSV_PAGE: u32 = 5000;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(show))
        .route("/{id}/rows", get(rows))
        .route("/{id}/log.csv", get(csv_log))
        .route("/{id}/preview", post(preview))
        .route("/{id}/start", post(start))
        .route("/{id}/stop", post(stop))
}

#[derive(Debug, Deserialize)]
struct CreateRun {
    spec: JobSpec,
    share_id: Option<u64>,
}

/// Validate the spec, then load candidates in the background. Poll `GET /runs/{id}`.
async fn create(State(app): State<SharedState>, session: Session, Json(body): Json<CreateRun>) -> ApiResult<Value> {
    let user = require_user(&session).await?;
    let job = Job::prepare(&app.clients, body.spec.clone()).await?;
    let owner = Owner { id: user.id, name: user.name };
    let id = app.store.create_run(&owner, &body.spec, body.share_id).await?;
    let claim = match app.runs.claim(id, user.id, app.config.harvest.max_active_runs_per_user) {
        Ok(claim) => claim,
        Err(e) => {
            app.store.set_status(id, RunStatus::Failed, Some(&e.to_string())).await?;
            return Err(e.into());
        }
    };
    tokio::spawn(worker::load(app.clone(), id, job, claim));
    Ok(Json(json!({ "id": id })))
}

async fn list(State(app): State<SharedState>, session: Session) -> ApiResult<Vec<RunRecord>> {
    let user = require_user(&session).await?;
    Ok(Json(app.store.runs_of(user.id, 100).await?))
}

async fn show(State(app): State<SharedState>, Path(id): Path<u64>) -> ApiResult<Value> {
    let run = app.store.run(id).await?.ok_or(ApiError::NotFound)?;
    let counts = app.store.counts(id).await?;
    let permalink = run.spec.to_legacy_query();
    let host = host_for(&run.spec.siteid, &run.spec.project).ok();
    let active = app.runs.is_active(id);
    Ok(Json(json!({ "run": run, "counts": counts, "active": active, "permalink": permalink, "host": host })))
}

#[derive(Debug, Deserialize)]
struct RowsQuery {
    status: Option<String>,
    #[serde(default)]
    offset: u32,
    limit: Option<u32>,
}

async fn rows(
    State(app): State<SharedState>,
    Path(id): Path<u64>,
    Query(q): Query<RowsQuery>,
) -> ApiResult<Vec<RowRecord>> {
    let status = match q.status.as_deref() {
        None | Some("") => None,
        Some(s) => Some(RowStatus::parse(s).ok_or_else(|| ApiError::bad_request(format!("unknown status {s}")))?),
    };
    let limit = q.limit.unwrap_or(100).min(MAX_ROWS_PER_PAGE);
    Ok(Json(app.store.rows(id, status, q.offset, limit).await?))
}

async fn csv_log(State(app): State<SharedState>, Path(id): Path<u64>) -> Result<impl IntoResponse, ApiError> {
    app.store.run(id).await?.ok_or(ApiError::NotFound)?;
    let mut csv = csv::Writer::from_writer(vec![]);
    csv.write_record(["page_id", "title", "item", "status", "raw_value", "value", "message"])
        .map_err(anyhow::Error::from)?;
    for offset in (0..).step_by(CSV_PAGE as usize) {
        let rows = app.store.rows(id, None, offset, CSV_PAGE).await?;
        for r in &rows {
            let fields = [
                &r.page_id.to_string(),
                &r.title,
                opt(&r.item),
                r.status.as_str(),
                opt(&r.raw_value),
                opt(&r.value),
                opt(&r.message),
            ];
            csv.write_record(fields).map_err(anyhow::Error::from)?;
        }
        if rows.len() < CSV_PAGE as usize {
            break;
        }
    }
    let body = csv.into_inner().map_err(|e| anyhow::anyhow!("{e}"))?;
    let disposition = format!("attachment; filename=\"harvesttemplates-run-{id}.csv\"");
    Ok((
        [(header::CONTENT_TYPE, "text/csv; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, disposition)],
        body,
    ))
}

fn opt(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or_default()
}

async fn preview(State(app): State<SharedState>, session: Session, Path(id): Path<u64>) -> ApiResult<Value> {
    start_worker(app, &session, id, false).await
}

async fn start(State(app): State<SharedState>, session: Session, Path(id): Path<u64>) -> ApiResult<Value> {
    start_worker(app, &session, id, true).await
}

async fn start_worker(app: SharedState, session: &Session, id: u64, edit: bool) -> ApiResult<Value> {
    let user = require_user(session).await?;
    let run = owned_run(&app, id, &user).await?;
    if run.status == RunStatus::Loading {
        return Err(ApiError::bad_request("the run is still loading"));
    }
    let claim = app.runs.claim(id, user.id, app.config.harvest.max_active_runs_per_user)?;
    let job = Job::prepare(&app.clients, run.spec.clone()).await?;
    let mode = if edit {
        app.tokens.freshest(user.id, user.token);
        let editor = Editor::new(app.oauth.clone(), app.wikidata_api_url.clone(), user.id, app.tokens.clone());
        Mode::Edit(Box::new(editor))
    } else {
        Mode::Preview
    };
    tokio::spawn(Worker::new(app.clone(), run, job, mode, claim).run());
    Ok(Json(json!({ "started": true })))
}

async fn stop(State(app): State<SharedState>, session: Session, Path(id): Path<u64>) -> ApiResult<Value> {
    let user = require_user(&session).await?;
    owned_run(&app, id, &user).await?;
    Ok(Json(json!({ "stopping": app.runs.stop(id) })))
}

async fn owned_run(app: &SharedState, id: u64, user: &User) -> Result<RunRecord, ApiError> {
    let run = app.store.run(id).await?.ok_or(ApiError::NotFound)?;
    if run.user_id != user.id {
        return Err(ApiError::Forbidden("this is someone else's run".into()));
    }
    Ok(run)
}
