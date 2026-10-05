use super::{ApiError, SharedState};
use crate::auth::edit::identify;
use crate::auth::session::{self, Login, User, safe_return_path};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tower_sessions::Session;

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/login", get(login))
        .route("/callback", get(callback))
        .route("/logout", post(logout))
        .route("/me", get(me))
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    return_to: Option<String>,
}

async fn login(
    State(app): State<SharedState>,
    session: Session,
    Query(q): Query<LoginQuery>,
) -> Result<Redirect, ApiError> {
    let request = app.oauth.request_token().await?;
    let url = app.oauth.authorize_url(&request);
    let return_to = safe_return_path(q.return_to.as_deref());
    session::store(&session, &Login::Pending { request, return_to }).await?;
    Ok(Redirect::to(&url))
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    oauth_verifier: String,
    oauth_token: String,
}

async fn callback(
    State(app): State<SharedState>,
    session: Session,
    Query(q): Query<CallbackQuery>,
) -> Result<Redirect, ApiError> {
    let Login::Pending { request, return_to } = session::load(&session).await else {
        return Err(ApiError::bad_request("no login in progress; please start again"));
    };
    if request.key != q.oauth_token {
        return Err(ApiError::bad_request("login token mismatch; please start again"));
    }
    let token = app.oauth.access_token(&request, &q.oauth_verifier).await?;
    let (id, name) = identify(&app.oauth, &app.wikidata_api_url, &token).await?;
    tracing::info!("login: {name}");
    session::store(&session, &Login::LoggedIn(User { id, name, token })).await?;
    Ok(Redirect::to(&return_to))
}

async fn logout(session: Session) -> Result<StatusCode, ApiError> {
    session::clear(&session).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(session: Session) -> Json<Value> {
    Json(json!({ "user": session::current_user(&session).await.map(|u| u.name) }))
}
