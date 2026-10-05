//! OAuth 2.0 against MediaWiki: authorization code grant, Bearer tokens, refresh.

use crate::config::{OauthConfig, Secret};
use crate::storage::now;
use crate::wiki::api::Params;
use anyhow::{Context, Result, anyhow, bail};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const OAUTH2_BASE: &str = "https://meta.wikimedia.org/w/rest.php/oauth2";
/// Refresh this many seconds before expiry, so a token never lapses mid-request.
const EXPIRY_MARGIN: i64 = 300;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub access: Secret,
    pub refresh: Option<Secret>,
    /// Unix seconds.
    pub expires_at: i64,
}

impl Token {
    pub fn is_expiring(&self) -> bool {
        now() + EXPIRY_MARGIN >= self.expires_at
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("the login has expired or was revoked")]
    Unauthorized,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct OAuth {
    http: reqwest::Client,
    client_id: String,
    client_secret: Secret,
    base: String,
}

impl OAuth {
    pub fn new(http: reqwest::Client, config: &OauthConfig) -> Self {
        Self::with_base(http, config, OAUTH2_BASE)
    }

    /// Tests point this at a mock server.
    pub fn with_base(http: reqwest::Client, config: &OauthConfig, base: &str) -> Self {
        Self {
            http,
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            base: base.to_string(),
        }
    }

    /// Where to send the user to approve. `state` must come back unchanged.
    pub fn authorize_url(&self, state: &str) -> String {
        let (id, state) = (urlencoding::encode(&self.client_id), urlencoding::encode(state));
        format!(
            "{}/authorize?response_type=code&client_id={id}&state={state}",
            self.base
        )
    }

    pub async fn exchange_code(&self, code: &str) -> Result<Token> {
        self.token_request(&[("grant_type", "authorization_code"), ("code", code)])
            .await
    }

    /// A new token from the refresh token. MediaWiki rotates refresh tokens:
    /// the old one stops working once this succeeds.
    pub async fn refresh(&self, token: &Token) -> Result<Token> {
        let refresh = token.refresh.as_ref().context("no refresh token")?;
        self.token_request(&[("grant_type", "refresh_token"), ("refresh_token", refresh.expose())])
            .await
    }

    async fn token_request(&self, grant: &[(&str, &str)]) -> Result<Token> {
        let mut form = grant.to_vec();
        form.extend([
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.expose()),
        ]);
        let response = self
            .http
            .post(format!("{}/access_token", self.base))
            .form(&form)
            .send()
            .await?;
        let status = response.status();
        let json: Value = response.json().await.context("token response is not JSON")?;
        if !status.is_success() {
            let reason = ["error_description", "message", "error"]
                .iter()
                .find_map(|k| json[k].as_str());
            bail!("token request refused: {}", reason.unwrap_or("unknown reason"));
        }
        let access = json["access_token"].as_str().context("no access token")?;
        Ok(Token {
            access: Secret::from(access),
            refresh: json["refresh_token"].as_str().map(Secret::from),
            expires_at: now() + json["expires_in"].as_i64().unwrap_or(3600),
        })
    }

    /// The user's central id and name.
    pub async fn profile(&self, token: &Token) -> Result<(u64, String)> {
        let url = format!("{}/resource/profile", self.base);
        let json: Value = self
            .http
            .get(url)
            .bearer_auth(token.access.expose())
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let id = json["sub"].as_u64().or_else(|| json["sub"].as_str()?.parse().ok());
        match (id, json["username"].as_str()) {
            (Some(id), Some(name)) => Ok((id, name.to_string())),
            _ => Err(anyhow!("the OAuth profile has no user")),
        }
    }

    /// An authenticated MediaWiki API POST.
    pub async fn post(&self, api_url: &str, params: &Params, token: &Token) -> Result<Value, CallError> {
        let mut form = params.clone();
        form.extend([("format".into(), "json".into()), ("formatversion".into(), "2".into())]);
        let request = self.http.post(api_url).bearer_auth(token.access.expose()).form(&form);
        let response = request.send().await.map_err(anyhow::Error::from)?;
        if matches!(response.status(), StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(CallError::Unauthorized);
        }
        let json: Value = response
            .error_for_status()
            .map_err(anyhow::Error::from)?
            .json()
            .await
            .map_err(anyhow::Error::from)?;
        if json["error"]["code"]
            .as_str()
            .is_some_and(|c| c.starts_with("mwoauth-"))
        {
            return Err(CallError::Unauthorized);
        }
        Ok(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub fn config() -> OauthConfig {
        OauthConfig {
            client_id: "cid".into(),
            client_secret: Secret::from("cs"),
            callback_url: "x".into(),
        }
    }

    #[test]
    fn authorize_url() {
        let oauth = OAuth::new(reqwest::Client::new(), &config());
        let url = oauth.authorize_url("s t");
        assert_eq!(
            url,
            "https://meta.wikimedia.org/w/rest.php/oauth2/authorize?response_type=code&client_id=cid&state=s%20t"
        );
    }

    #[tokio::test]
    async fn code_exchange_profile_and_refusal() {
        let server = MockServer::start().await;
        Mock::given(path("/access_token"))
            .and(body_string_contains("code=good"))
            .and(body_string_contains("client_secret=cs"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "token_type": "Bearer", "expires_in": 14400, "access_token": "A", "refresh_token": "R"
            })))
            .mount(&server)
            .await;
        Mock::given(path("/access_token"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({"error": "invalid_grant", "message": "bad code"})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/resource/profile"))
            .and(header("authorization", "Bearer A"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"sub": 12345, "username": "Example"})))
            .mount(&server)
            .await;
        let oauth = OAuth::with_base(reqwest::Client::new(), &config(), &server.uri());
        let token = oauth.exchange_code("good").await.unwrap();
        assert_eq!(
            (token.access.expose(), token.refresh.as_ref().map(Secret::expose)),
            ("A", Some("R"))
        );
        assert!(!token.is_expiring());
        assert_eq!(oauth.profile(&token).await.unwrap(), (12345, "Example".to_string()));
        let err = oauth.exchange_code("bad").await.unwrap_err().to_string();
        assert_eq!(err, "token request refused: bad code");
    }
}
