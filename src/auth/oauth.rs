//! OAuth 1.0a against MediaWiki, signed by hand (HMAC-SHA1), as in mix'n'match.

use crate::config::{OauthConfig, Secret};
use crate::wiki::api::Params;
use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::Sha1;

const OAUTH_BASE: &str = "https://www.mediawiki.org/wiki/Special:OAuth";

/// A request or access token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub key: String,
    pub secret: Secret,
}

#[derive(Debug, Clone)]
pub struct OAuth {
    http: reqwest::Client,
    consumer_key: String,
    consumer_secret: Secret,
    callback_url: String,
}

impl OAuth {
    pub fn new(http: reqwest::Client, config: &OauthConfig) -> Self {
        Self {
            http,
            consumer_key: config.consumer_key.clone(),
            consumer_secret: config.consumer_secret.clone(),
            callback_url: config.callback_url.clone(),
        }
    }

    /// Step 1: a one-shot request token.
    pub async fn request_token(&self) -> Result<Token> {
        let extra = [("oauth_callback", self.callback_url.as_str())];
        self.handshake("initiate", &extra, None).await
    }

    /// Step 2: where to send the user to approve.
    pub fn authorize_url(&self, request: &Token) -> String {
        format!(
            "{OAUTH_BASE}/authorize?oauth_token={}&oauth_consumer_key={}",
            encode(&request.key),
            encode(&self.consumer_key)
        )
    }

    /// Step 3: trade the request token and verifier for an access token.
    pub async fn access_token(&self, request: &Token, verifier: &str) -> Result<Token> {
        let extra = [
            ("oauth_verifier", verifier),
            ("oauth_token", request.key.as_str()),
        ];
        self.handshake("token", &extra, Some(request)).await
    }

    async fn handshake(
        &self,
        step: &str,
        extra: &[(&str, &str)],
        token: Option<&Token>,
    ) -> Result<Token> {
        let url = format!("{OAUTH_BASE}/{step}");
        let mut params = self.oauth_params();
        params.push(("format".into(), "json".into()));
        params.extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let token_secret = token.map_or("", |t| t.secret.expose());
        params.push((
            "oauth_signature".into(),
            self.sign("GET", &url, &params, token_secret),
        ));
        let json: Value = self
            .http
            .get(&url)
            .query(&params)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        parse_token(&json).with_context(|| format!("OAuth {step} failed"))
    }

    /// An API POST signed with the user's access token. Not retried: a repeated
    /// edit request could save twice.
    pub async fn post(&self, api_url: &str, params: &Params, token: &Token) -> Result<Value> {
        let mut form = params.clone();
        form.extend([
            ("format".into(), "json".into()),
            ("formatversion".into(), "2".into()),
        ]);
        let mut header = self.oauth_params();
        header.push(("oauth_token".into(), token.key.clone()));
        let signed: Params = form.iter().chain(&header).cloned().collect();
        header.push((
            "oauth_signature".into(),
            self.sign("POST", api_url, &signed, token.secret.expose()),
        ));
        let authorization = header
            .iter()
            .map(|(k, v)| format!("{}=\"{}\"", encode(k), encode(v)))
            .collect::<Vec<_>>();
        let response = self
            .http
            .post(api_url)
            .header(
                reqwest::header::AUTHORIZATION,
                format!("OAuth {}", authorization.join(", ")),
            )
            .form(&form)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    fn oauth_params(&self) -> Params {
        let nonce: String = (0..16)
            .map(|_| format!("{:02x}", rand::random::<u8>()))
            .collect();
        let timestamp = chrono::Utc::now().timestamp().to_string();
        vec![
            ("oauth_consumer_key".into(), self.consumer_key.clone()),
            ("oauth_version".into(), "1.0".into()),
            ("oauth_nonce".into(), nonce),
            ("oauth_timestamp".into(), timestamp),
            ("oauth_signature_method".into(), "HMAC-SHA1".into()),
        ]
    }

    fn sign(&self, method: &str, url: &str, params: &Params, token_secret: &str) -> String {
        sign(
            method,
            url,
            params,
            self.consumer_secret.expose(),
            token_secret,
        )
    }
}

/// RFC 5849 §3.4 HMAC-SHA1 signature. `url` must not contain a query string.
fn sign(
    method: &str,
    url: &str,
    params: &Params,
    consumer_secret: &str,
    token_secret: &str,
) -> String {
    let mut pairs: Vec<(String, String)> = params
        .iter()
        .filter(|(k, _)| k != "oauth_signature")
        .map(|(k, v)| (encode(k), encode(v)))
        .collect();
    pairs.sort();
    let normalized = pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let base = format!(
        "{}&{}&{}",
        method.to_uppercase(),
        encode(url),
        encode(&normalized)
    );
    let key = format!("{}&{}", encode(consumer_secret), encode(token_secret));
    let mut mac =
        Hmac::<Sha1>::new_from_slice(key.as_bytes()).expect("HMAC accepts any key length");
    mac.update(base.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// RFC 3986 percent-encoding (same as PHP `rawurlencode`).
fn encode(s: &str) -> String {
    urlencoding::encode(s).into_owned()
}

fn parse_token(json: &Value) -> Result<Token> {
    if let Some(err) = json.get("error").or_else(|| json.get("message")) {
        bail!("{err}");
    }
    let field = |name: &str| {
        json[name]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    match (field("key"), field("secret")) {
        (Some(key), Some(secret)) => Ok(Token {
            key,
            secret: Secret::from(secret.as_str()),
        }),
        _ => Err(anyhow!("response has no token")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 5849 §1.2 example request.
    #[test]
    fn signature_matches_rfc5849() {
        let params: Params = [
            ("oauth_consumer_key", "dpf43f3p2l4k3l03"),
            ("oauth_token", "nnch734d00sl2jdk"),
            ("oauth_nonce", "kllo9940pd9333jh"),
            ("oauth_timestamp", "1191242096"),
            ("oauth_signature_method", "HMAC-SHA1"),
            ("oauth_version", "1.0"),
            ("file", "vacation.jpg"),
            ("size", "original"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let sig = sign(
            "GET",
            "http://photos.example.net/photos",
            &params,
            "kd94hf93k423kf44",
            "pfkkdhi9sl3r4s00",
        );
        assert_eq!(sig, "tR3+Ty81lMeYAr/Fid0kMTYa/WM=");
    }

    #[test]
    fn encoding_is_rfc3986() {
        assert_eq!(encode("a b+c~._-"), "a%20b%2Bc~._-");
        assert_eq!(encode("ö"), "%C3%B6");
    }

    #[test]
    fn token_parsing() {
        let t = parse_token(&serde_json::json!({"key": "k", "secret": "s"})).unwrap();
        assert_eq!(t.key, "k");
        assert!(parse_token(&serde_json::json!({"error": "mwoauth-invalid"})).is_err());
        assert!(parse_token(&serde_json::json!({"key": ""})).is_err());
    }
}
