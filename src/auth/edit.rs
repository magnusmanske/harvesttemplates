//! Writing to Wikidata as the logged-in user.

use super::oauth::{CallError, OAuth, Token};
use super::tokens::TokenCache;
use crate::ids::ItemId;
use crate::wiki::api::{Params, params};
use anyhow::anyhow;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub const WIKIDATA_API: &str = "https://www.wikidata.org/w/api.php";
/// Seconds of replication lag at which Wikidata asks bots and tools to pause.
const MAXLAG: &str = "5";

/// API error codes after which more edits by this user cannot succeed.
const FATAL_CODES: [&str; 4] = ["blocked", "autoblocked", "permissiondenied", "readonly"];

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// Wait and try the same edit again.
    #[error("Wikidata is busy, waiting {0:?}")]
    Busy(Duration),
    /// This edit was refused; others may still work. The text is shown to the user.
    #[error("{0}")]
    Rejected(String),
    /// Stop the whole run (blocked user, expired login, read-only wiki).
    #[error("{0}")]
    Fatal(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// Edits Wikidata as one user, refreshing their OAuth token as needed and
/// keeping the CSRF token between edits.
#[derive(Debug)]
pub struct Editor {
    oauth: OAuth,
    api_url: String,
    user: u64,
    tokens: Arc<TokenCache>,
    csrf: Option<String>,
}

impl Editor {
    /// `user` must have a token in `tokens`.
    pub const fn new(oauth: OAuth, api_url: String, user: u64, tokens: Arc<TokenCache>) -> Self {
        Self { oauth, api_url, user, tokens, csrf: None }
    }

    /// Add new statements to an item in one edit.
    pub async fn add_statements(&mut self, item: ItemId, statements: &[Value], summary: &str) -> Result<(), EditError> {
        let data = json!({ "claims": statements }).to_string();
        let item = item.to_string();
        for _ in 0..2 {
            let csrf = self.csrf().await?;
            let p = params(&[
                ("action", "wbeditentity"),
                ("id", &item),
                ("data", &data),
                ("summary", summary),
                ("maxlag", MAXLAG),
                ("bot", "1"),
                ("token", &csrf),
            ]);
            let response = self.call(&p).await?;
            match response["error"]["code"].as_str() {
                Some("badtoken") => self.csrf = None,
                _ => return outcome(&response),
            }
        }
        Err(EditError::Fatal("Wikidata keeps rejecting the edit token".into()))
    }

    async fn csrf(&mut self) -> Result<String, EditError> {
        if let Some(token) = &self.csrf {
            return Ok(token.clone());
        }
        let p = params(&[("action", "query"), ("meta", "tokens"), ("type", "csrf")]);
        let response = self.call(&p).await?;
        let token = response["query"]["tokens"]["csrftoken"]
            .as_str()
            .filter(|t| *t != "+\\")
            .ok_or_else(|| logged_out("Wikidata does not recognise the login"))?;
        self.csrf = Some(token.to_string());
        Ok(token.to_string())
    }

    /// One API call; refreshes the OAuth token first if it is about to expire,
    /// and once more if the API rejects it.
    async fn call(&self, params: &Params) -> Result<Value, EditError> {
        let mut token = self.tokens.get(self.user).ok_or_else(|| logged_out("no login"))?;
        if token.is_expiring() {
            token = self.refresh(&token).await?;
        }
        match self.oauth.post(&self.api_url, params, &token).await {
            Err(CallError::Unauthorized) => {
                let token = self.refresh(&token).await?;
                self.oauth.post(&self.api_url, params, &token).await.map_err(|e| match e {
                    CallError::Unauthorized => logged_out("the login was rejected"),
                    CallError::Other(e) => EditError::Failed(e),
                })
            }
            result => result.map_err(|e| EditError::Failed(anyhow!(e))),
        }
    }

    async fn refresh(&self, token: &Token) -> Result<Token, EditError> {
        let fresh = self.oauth.refresh(token).await.map_err(|e| logged_out(&format!("{e:#}")))?;
        self.tokens.replace(self.user, fresh.clone());
        Ok(fresh)
    }
}

fn logged_out(reason: &str) -> EditError {
    EditError::Fatal(format!("{reason}; please log in again and resume the run"))
}

fn outcome(response: &Value) -> Result<(), EditError> {
    let Some(error) = response.get("error") else {
        return match response["success"].as_i64() {
            Some(1) => Ok(()),
            _ => Err(anyhow!("unexpected response: {response}").into()),
        };
    };
    let code = error["code"].as_str().unwrap_or_default();
    let info = error["info"].as_str().unwrap_or(code).to_string();
    Err(match code {
        "maxlag" => EditError::Busy(Duration::from_secs(error["lag"].as_f64().unwrap_or(5.0).clamp(5.0, 60.0) as u64)),
        "ratelimited" => EditError::Busy(Duration::from_secs(60)),
        _ if FATAL_CODES.contains(&code) => EditError::Fatal(info),
        _ => EditError::Rejected(info),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OauthConfig, Secret};
    use crate::storage::now;
    use wiremock::matchers::{body_string_contains, header, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn token(access: &str, expires_in: i64) -> Token {
        Token { access: Secret::from(access), refresh: Some(Secret::from("R")), expires_at: now() + expires_in }
    }

    fn editor(url: &str, token: Token) -> Editor {
        let config =
            OauthConfig { client_id: "cid".into(), client_secret: Secret::from("cs"), callback_url: "x".into() };
        let tokens = Arc::new(TokenCache::default());
        tokens.replace(1, token);
        Editor::new(OAuth::with_base(reqwest::Client::new(), &config, url), format!("{url}/api"), 1, tokens)
    }

    async fn respond(server: &MockServer, matcher: impl wiremock::Match + 'static, status: u16, body: Value) {
        Mock::given(matcher).respond_with(ResponseTemplate::new(status).set_body_json(body)).mount(server).await;
    }

    #[tokio::test]
    async fn edits_with_bearer_token() {
        let server = MockServer::start().await;
        respond(
            &server,
            body_string_contains("meta=tokens"),
            200,
            json!({"query": {"tokens": {"csrftoken": "abc+\\"}}}),
        )
        .await;
        respond(&server, body_string_contains("action=wbeditentity"), 200, json!({"success": 1})).await;
        let mut ed = editor(&server.uri(), token("A", 3600));
        ed.add_statements(ItemId(1), &[json!({})], "s").await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[1].headers.get("authorization").unwrap(), "Bearer A");
        assert!(String::from_utf8_lossy(&requests[1].body).contains("maxlag=5"));
    }

    #[tokio::test]
    async fn refreshes_expiring_and_rejected_tokens() {
        let server = MockServer::start().await;
        respond(
            &server,
            path("/access_token"),
            200,
            json!({"access_token": "B", "refresh_token": "R2", "expires_in": 14400}),
        )
        .await;
        Mock::given(header("authorization", "Bearer A")).respond_with(ResponseTemplate::new(401)).mount(&server).await;
        respond(&server, body_string_contains("meta=tokens"), 200, json!({"query": {"tokens": {"csrftoken": "t+\\"}}}))
            .await;
        respond(&server, body_string_contains("action=wbeditentity"), 200, json!({"success": 1})).await;

        let mut ed = editor(&server.uri(), token("A", 3600));
        ed.add_statements(ItemId(1), &[json!({})], "s").await.unwrap();
        assert_eq!(ed.tokens.get(1).unwrap().access.expose(), "B", "rejected token was refreshed");

        let mut ed = editor(&server.uri(), token("A", 10));
        ed.add_statements(ItemId(1), &[json!({})], "s").await.unwrap();
        let refreshed = ed.tokens.get(1).unwrap();
        assert_eq!(refreshed.refresh.unwrap().expose(), "R2", "expiring token was refreshed before use");
    }

    #[tokio::test]
    async fn failed_refresh_stops_the_run() {
        let server = MockServer::start().await;
        respond(&server, path("/access_token"), 400, json!({"error": "invalid_grant"})).await;
        respond(&server, path("/api"), 401, json!({})).await;
        let err = editor(&server.uri(), token("A", 3600)).add_statements(ItemId(1), &[], "s").await.unwrap_err();
        assert!(matches!(err, EditError::Fatal(m) if m.contains("log in again")));
    }

    #[test]
    fn error_classes() {
        let err = |code: &str| outcome(&json!({"error": {"code": code, "info": "msg", "lag": 12.3}})).unwrap_err();
        assert!(matches!(err("maxlag"), EditError::Busy(d) if d == Duration::from_secs(12)));
        assert!(matches!(err("blocked"), EditError::Fatal(_)));
        assert!(matches!(err("modification-failed"), EditError::Rejected(m) if m == "msg"));
    }
}
