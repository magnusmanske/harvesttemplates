use super::oauth::Token;
use crate::api::ApiError;
use crate::config::Secret;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use tower_sessions::Session;

const KEY: &str = "login";

/// A logged-in Wikidata user. Only ever built from a completed OAuth handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// MediaWiki central user id: the same on every wiki, survives renames.
    pub id: u64,
    pub name: String,
    pub token: Token,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum Login {
    #[default]
    Anonymous,
    /// Waiting for the user to come back from `Special:OAuth/authorize`.
    Pending {
        state: String,
        return_to: String,
    },
    LoggedIn(User),
}

pub async fn load(session: &Session) -> Login {
    session.get(KEY).await.ok().flatten().unwrap_or_default()
}

pub async fn store(session: &Session, login: &Login) -> Result<(), ApiError> {
    session.insert(KEY, login).await.map_err(|e| ApiError::Internal(e.into()))?;
    // A new id on every privilege change prevents session fixation.
    session.cycle_id().await.map_err(|e| ApiError::Internal(e.into()))
}

pub async fn clear(session: &Session) -> Result<(), ApiError> {
    session.flush().await.map_err(|e| ApiError::Internal(e.into()))
}

static DEV_USER: OnceLock<User> = OnceLock::new();

/// Development only: every request counts as logged in as `name`. Its token is
/// fake, so editing fails; previews work. Refused on Toolforge.
pub fn enable_dev_user(name: &str) -> anyhow::Result<()> {
    if std::path::Path::new("/etc/wmcs-project").exists() {
        anyhow::bail!("--dev-user is not allowed on Toolforge");
    }
    let token = Token { access: Secret::from("dev"), refresh: None, expires_at: 0 };
    let _ = DEV_USER.set(User { id: 0, name: name.to_string(), token });
    Ok(())
}

pub async fn current_user(session: &Session) -> Option<User> {
    if let Some(user) = DEV_USER.get() {
        return Some(user.clone());
    }
    match load(session).await {
        Login::LoggedIn(user) => Some(user),
        _ => None,
    }
}

pub async fn require_user(session: &Session) -> Result<User, ApiError> {
    current_user(session).await.ok_or(ApiError::Unauthorized)
}

/// Only same-site paths, so the login cannot be abused as an open redirect.
pub fn safe_return_path(path: Option<&str>) -> String {
    match path {
        Some(p) if p.starts_with('/') && !p.starts_with("//") && !p.contains('\\') => p.to_string(),
        _ => "/".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_paths() {
        assert_eq!(safe_return_path(Some("/?p=345&run=1")), "/?p=345&run=1");
        assert_eq!(safe_return_path(Some("//evil.example")), "/");
        assert_eq!(safe_return_path(Some("https://evil.example")), "/");
        assert_eq!(safe_return_path(Some("/\\evil.example")), "/");
        assert_eq!(safe_return_path(None), "/");
    }
}
