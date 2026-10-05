//! Writing to Wikidata as the logged-in user.

use super::oauth::{OAuth, Token};
use crate::ids::ItemId;
use crate::wiki::api::params;
use anyhow::anyhow;
use serde_json::{Value, json};
use std::time::Duration;

pub const WIKIDATA_API: &str = "https://www.wikidata.org/w/api.php";
/// Seconds of replication lag at which Wikidata asks bots and tools to pause.
const MAXLAG: &str = "5";

/// API error codes after which more edits by this user cannot succeed.
const FATAL_CODES: [&str; 6] = [
    "blocked",
    "autoblocked",
    "permissiondenied",
    "readonly",
    "mwoauth-invalid-authorization",
    "mwoauth-invalid-authorization-invalid-user",
];

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    /// Wait and try the same edit again.
    #[error("Wikidata is busy, waiting {0:?}")]
    Busy(Duration),
    /// This edit was refused; others may still work. The text is shown to the user.
    #[error("{0}")]
    Rejected(String),
    /// Stop the whole run (blocked user, revoked authorisation, read-only wiki).
    #[error("{0}")]
    Fatal(String),
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

/// Edits Wikidata as one user, keeping the CSRF token between edits.
#[derive(Debug)]
pub struct Editor {
    oauth: OAuth,
    api_url: String,
    user: Token,
    csrf: Option<String>,
}

impl Editor {
    pub const fn new(oauth: OAuth, api_url: String, user: Token) -> Self {
        Self {
            oauth,
            api_url,
            user,
            csrf: None,
        }
    }

    /// Add new statements to an item in one edit.
    pub async fn add_statements(
        &mut self,
        item: ItemId,
        statements: &[Value],
        summary: &str,
    ) -> Result<(), EditError> {
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
            let response = self.oauth.post(&self.api_url, &p, &self.user).await?;
            match response["error"]["code"].as_str() {
                Some("badtoken") => self.csrf = None,
                _ => return outcome(&response),
            }
        }
        Err(EditError::Fatal(
            "Wikidata keeps rejecting the edit token".into(),
        ))
    }

    async fn csrf(&mut self) -> Result<String, EditError> {
        if let Some(token) = &self.csrf {
            return Ok(token.clone());
        }
        let p = params(&[("action", "query"), ("meta", "tokens"), ("type", "csrf")]);
        let response = self.oauth.post(&self.api_url, &p, &self.user).await?;
        let token = response["query"]["tokens"]["csrftoken"]
            .as_str()
            .filter(|t| *t != "+\\")
            .ok_or_else(|| {
                EditError::Fatal("not logged in to Wikidata; please log in again".into())
            })?;
        self.csrf = Some(token.to_string());
        Ok(token.to_string())
    }
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
        "maxlag" => EditError::Busy(Duration::from_secs(
            error["lag"].as_f64().unwrap_or(5.0).clamp(5.0, 60.0) as u64,
        )),
        "ratelimited" => EditError::Busy(Duration::from_secs(60)),
        _ if FATAL_CODES.contains(&code) => EditError::Fatal(info),
        _ => EditError::Rejected(info),
    })
}

/// The Wikidata account behind an access token.
pub async fn identify(
    oauth: &OAuth,
    api_url: &str,
    token: &Token,
) -> anyhow::Result<(u64, String)> {
    let p = params(&[("action", "query"), ("meta", "userinfo")]);
    let info = oauth.post(api_url, &p, token).await?["query"]["userinfo"].clone();
    match (info["id"].as_u64(), info["name"].as_str(), info.get("anon")) {
        (Some(id), Some(name), None) if id > 0 => Ok((id, name.to_string())),
        _ => Err(anyhow!("OAuth login did not identify a user")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OauthConfig, Secret};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn editor(url: String) -> Editor {
        let config = OauthConfig {
            consumer_key: "ck".into(),
            consumer_secret: Secret::from("cs"),
            callback_url: "http://localhost/cb".into(),
        };
        let oauth = OAuth::new(reqwest::Client::new(), &config);
        Editor::new(
            oauth,
            url,
            Token {
                key: "k".into(),
                secret: Secret::from("s"),
            },
        )
    }

    async fn respond(server: &MockServer, body_contains: &str, response: Value) {
        Mock::given(method("POST"))
            .and(body_string_contains(body_contains))
            .respond_with(ResponseTemplate::new(200).set_body_json(response))
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn edit_success_and_signed_header() {
        let server = MockServer::start().await;
        respond(
            &server,
            "meta=tokens",
            json!({"query": {"tokens": {"csrftoken": "abc+\\"}}}),
        )
        .await;
        respond(&server, "action=wbeditentity", json!({"success": 1})).await;
        let mut ed = editor(server.uri());
        ed.add_statements(ItemId(1), &[json!({})], "s")
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let auth = requests[1]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(auth.starts_with("OAuth ") && auth.contains("oauth_signature="));
        assert!(String::from_utf8_lossy(&requests[1].body).contains("maxlag=5"));
    }

    #[test]
    fn error_classes() {
        let err = |code: &str| {
            outcome(&json!({"error": {"code": code, "info": "msg", "lag": 12.3}})).unwrap_err()
        };
        assert!(matches!(err("maxlag"), EditError::Busy(d) if d == Duration::from_secs(12)));
        assert!(matches!(err("blocked"), EditError::Fatal(_)));
        assert!(matches!(err("modification-failed"), EditError::Rejected(m) if m == "msg"));
    }

    #[tokio::test]
    async fn anonymous_csrf_token_is_fatal() {
        let server = MockServer::start().await;
        respond(
            &server,
            "meta=tokens",
            json!({"query": {"tokens": {"csrftoken": "+\\"}}}),
        )
        .await;
        let err = editor(server.uri())
            .add_statements(ItemId(1), &[], "s")
            .await
            .unwrap_err();
        assert!(matches!(err, EditError::Fatal(_)));
    }
}
