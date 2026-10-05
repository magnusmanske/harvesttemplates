//! Publicly saved queries ("Harvest Templates Share").

use super::{ApiError, ApiResult, SharedState};
use crate::auth::require_user;
use crate::harvest::{Job, JobSpec};
use crate::storage::{Owner, ShareRecord};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_sessions::Session;

const MAX_TITLE: usize = 255;
const MAX_TAG: usize = 32;
const MAX_TAGS: usize = 10;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/", get(list).post(create))
        .route("/{id}", get(show).delete(delete))
        .route("/{id}/tags", put(set_tags))
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
    #[serde(default)]
    tags: Vec<String>,
}

/// Only specs that actually work can be shared.
async fn create(State(app): State<SharedState>, session: Session, Json(body): Json<CreateShare>) -> ApiResult<Value> {
    let user = require_user(&session).await?;
    let title = body.title.trim();
    if title.is_empty() || title.chars().count() > MAX_TITLE {
        return Err(ApiError::bad_request(format!("the title must have 1 to {MAX_TITLE} characters")));
    }
    let tags = normalize_tags(&body.tags)?;
    Job::prepare(&app.clients, body.spec.clone()).await?;
    let owner = Owner { id: user.id, name: user.name };
    let id = app.store.create_share(&owner, title, &body.spec).await?;
    app.store.set_tags(id, &tags).await?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Debug, Deserialize)]
struct Tags {
    tags: Vec<String>,
}

async fn set_tags(
    State(app): State<SharedState>,
    session: Session,
    Path(id): Path<u64>,
    Json(body): Json<Tags>,
) -> ApiResult<Vec<String>> {
    let user = require_user(&session).await?;
    let share = app.store.share(id).await?.ok_or(ApiError::NotFound)?;
    if share.user_id != user.id {
        return Err(ApiError::Forbidden("only the creator can change the tags".into()));
    }
    let tags = normalize_tags(&body.tags)?;
    app.store.set_tags(id, &tags).await?;
    Ok(Json(tags))
}

async fn delete(State(app): State<SharedState>, session: Session, Path(id): Path<u64>) -> Result<StatusCode, ApiError> {
    let user = require_user(&session).await?;
    let share = app.store.share(id).await?.ok_or(ApiError::NotFound)?;
    if share.user_id != user.id || !app.store.delete_share(id, user.id).await? {
        return Err(ApiError::Forbidden("only the creator can delete a shared query".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `Czech Wikipedia` → `czech-wikipedia`; letters and digits of any script, and hyphens.
fn normalize_tags(tags: &[String]) -> Result<Vec<String>, ApiError> {
    let mut out: Vec<String> = tags
        .iter()
        .map(|t| t.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("-"))
        .filter(|t| !t.is_empty())
        .collect();
    out.sort();
    out.dedup();
    if let Some(bad) =
        out.iter().find(|t| t.chars().count() > MAX_TAG || !t.chars().all(|c| c.is_alphanumeric() || c == '-'))
    {
        return Err(ApiError::bad_request(format!("invalid tag '{bad}': up to {MAX_TAG} letters, digits and hyphens")));
    }
    if out.len() > MAX_TAGS {
        return Err(ApiError::bad_request(format!("at most {MAX_TAGS} tags")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        let tags = |t: &[&str]| normalize_tags(&t.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(tags(&[" Czech  Wikipedia ", "films", "films", ""]).unwrap(), ["czech-wikipedia", "films"]);
        assert_eq!(tags(&["čeština", "日本"]).unwrap(), ["čeština", "日本"]);
        assert!(tags(&["<script>"]).is_err());
        assert!(tags(&[&"x".repeat(33)]).is_err());
        assert!(tags(&["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11"]).is_err());
    }
}
