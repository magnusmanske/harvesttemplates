use super::{ApiError, SharedState};
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
        .route("/logout", post(logout))
        .route("/me", get(me))
}

/// The path of the registered OAuth callback URL, where [`callback`] is mounted.
pub fn callback_path(callback_url: &str) -> String {
    reqwest::Url::parse(callback_url).map_or_else(|_| "/callback".to_string(), |u| u.path().to_string())
}

#[derive(Debug, Deserialize)]
struct LoginQuery {
    return_to: Option<String>,
}

async fn login(
    session: Session,
    State(app): State<SharedState>,
    Query(q): Query<LoginQuery>,
) -> Result<Redirect, ApiError> {
    let state: String = (0..16).map(|_| format!("{:02x}", rand::random::<u8>())).collect();
    let url = app.oauth.authorize_url(&state);
    let return_to = safe_return_path(q.return_to.as_deref());
    session::store(&session, &Login::Pending { state, return_to }).await?;
    Ok(Redirect::to(&url))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

pub async fn callback(
    State(app): State<SharedState>,
    session: Session,
    Query(q): Query<CallbackQuery>,
) -> Result<Redirect, ApiError> {
    let Login::Pending { state, return_to } = session::load(&session).await else {
        return Err(ApiError::bad_request("no login in progress; please start again"));
    };
    if let Some(error) = q.error {
        return Err(ApiError::bad_request(format!("login not completed: {error}")));
    }
    if q.state.as_deref() != Some(state.as_str()) {
        return Err(ApiError::bad_request("login state mismatch; please start again"));
    }
    let code = q.code.ok_or_else(|| ApiError::bad_request("no authorization code"))?;
    let token = app.oauth.exchange_code(&code).await?;
    let (id, name) = app.oauth.profile(&token).await?;
    tracing::info!("login: {name}");
    let token = app.tokens.freshest(id, token);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_paths() {
        assert_eq!(
            callback_path("https://harvesttemplates.toolforge.org/callback"),
            "/callback"
        );
        assert_eq!(
            callback_path("http://localhost:8000/api/auth/callback"),
            "/api/auth/callback"
        );
        assert_eq!(callback_path("nonsense"), "/callback");
    }
}
