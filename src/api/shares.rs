//! Publicly saved queries ("Harvest Templates Share").

use super::{ApiError, ApiResult, SharedState};
use crate::auth::require_user;
use crate::harvest::{Job, JobSpec};
use crate::storage::{Owner, ShareRecord};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_sessions::Session;

const MAX_TITLE: usize = 255;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(show).delete(delete))
}

async fn list(State(app): State<SharedState>) -> ApiResult<Vec<ShareRecord>> {
    Ok(Json(app.store.shares().await?))
}

async fn show(State(app): State<SharedState>, Path(id): Path<u64>) -> ApiResult<ShareRecord> {
    Ok(Json(app.store.share(id).await?.ok_or(ApiError::NotFound)?))
}

#[derive(Debug, Deserialize)]
struct CreateShare {
    title: String,
    spec: JobSpec,
}

/// Only specs that actually work can be shared.
async fn create(State(app): State<SharedState>, session: Session, Json(body): Json<CreateShare>) -> ApiResult<Value> {
    let user = require_user(&session).await?;
    let title = body.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return Err(ApiError::bad_request(format!(
            "the title must have 1 to {MAX_TITLE} characters"
        )));
    }
    Job::prepare(&app.clients, body.spec.clone()).await?;
    let owner = Owner {
        id: user.id,
        name: user.name,
    };
    let id = app.store.create_share(&owner, title, &body.spec).await?;
    Ok(Json(json!({ "id": id })))
}

async fn delete(State(app): State<SharedState>, session: Session, Path(id): Path<u64>) -> Result<StatusCode, ApiError> {
    let user = require_user(&session).await?;
    let share = app.store.share(id).await?.ok_or(ApiError::NotFound)?;
    if share.user_id != user.id || !app.store.delete_share(id, user.id).await? {
        return Err(ApiError::Forbidden("only the creator can delete a shared query".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}
