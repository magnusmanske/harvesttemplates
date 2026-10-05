use crate::http::send_json;
use anyhow::{Result, anyhow};
use serde_json::Value;

/// Read-only MediaWiki Action API client for any wiki (formatversion 2).
/// Authenticated writes go through [`crate::auth`].
#[derive(Debug, Clone)]
pub struct MwApi {
    http: reqwest::Client,
    /// Tests send every host to one mock server.
    base_url_override: Option<String>,
}

pub type Params = Vec<(String, String)>;

pub fn params(pairs: &[(&str, &str)]) -> Params {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

impl MwApi {
    pub const fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            base_url_override: None,
        }
    }

    pub const fn with_base_url(http: reqwest::Client, url: String) -> Self {
        Self {
            http,
            base_url_override: Some(url),
        }
    }

    pub fn api_url(&self, host: &str) -> String {
        self.base_url_override
            .clone()
            .unwrap_or_else(|| format!("https://{host}/w/api.php"))
    }

    /// One request. Long parameter lists (e.g. 50 page ids) go as POST.
    pub async fn get(&self, host: &str, params: &Params) -> Result<Value> {
        let mut form = params.clone();
        form.extend([("format".into(), "json".into()), ("formatversion".into(), "2".into())]);
        let json = send_json(self.http.post(self.api_url(host)).form(&form)).await?;
        match json.get("error") {
            Some(err) => Err(anyhow!("API error from {host}: {err}")),
            None => Ok(json),
        }
    }

    /// Follow `continue` until done or `f` returns `false`. `f` sees each response.
    pub async fn query_continue(&self, host: &str, params: &Params, mut f: impl FnMut(&Value) -> bool) -> Result<()> {
        let mut params = params.clone();
        loop {
            let json = self.get(host, &params).await?;
            if !f(&json) {
                return Ok(());
            }
            let Some(cont) = json.get("continue").and_then(Value::as_object) else {
                return Ok(());
            };
            for (key, value) in cont {
                let value = value.as_str().map_or_else(|| value.to_string(), str::to_string);
                params.retain(|(k, _)| k != key);
                params.push((key.clone(), value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn follows_continuation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("gticontinue=next"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"query": {"n": 2}})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "query": {"n": 1}, "continue": {"gticontinue": "next", "continue": "gti||"}
            })))
            .mount(&server)
            .await;
        let api = MwApi::with_base_url(reqwest::Client::new(), server.uri());
        let mut seen = vec![];
        api.query_continue("x", &params(&[("action", "query")]), |j| {
            seen.push(j["query"]["n"].as_i64().unwrap());
            true
        })
        .await
        .unwrap();
        assert_eq!(seen, [1, 2]);
    }

    #[tokio::test]
    async fn api_errors_are_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"error": {"code": "badvalue"}})))
            .mount(&server)
            .await;
        let api = MwApi::with_base_url(reqwest::Client::new(), server.uri());
        assert!(api.get("x", &params(&[])).await.is_err());
    }
}
