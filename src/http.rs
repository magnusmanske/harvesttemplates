//! JSON over HTTP with retries for the transient failures Wikimedia APIs produce.

use anyhow::{Context, Result};
use reqwest::RequestBuilder;
use serde_json::Value;
use wikimisc::http::{HttpError, HttpRetryPolicy};
use wikimisc::retry::Retry;

/// Retries after the first attempt; the waits are 1 s, 2 s, 4 s.
const RETRIES: u32 = 3;

/// Send `request` and parse the JSON body. Retries on 408, 429, 5xx, timeouts and
/// connection errors with exponential back-off, honouring `Retry-After` (at most 60 s).
pub async fn send_json(request: RequestBuilder) -> Result<Value> {
    send_json_with(request, policy()).await
}

fn policy() -> HttpRetryPolicy {
    HttpRetryPolicy::new(RETRIES).clamp_retry_after().on_retry(log_retry)
}

async fn send_json_with(request: RequestBuilder, policy: HttpRetryPolicy) -> Result<Value> {
    let response = policy.send(request).await?;
    response.json().await.context("response is not valid JSON")
}

fn log_retry(retry: &Retry<'_, HttpError>) {
    tracing::warn!("{}, retrying in {:?}", retry.error, retry.wait);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn quick() -> HttpRetryPolicy {
        policy().with_initial_backoff(Duration::from_millis(1))
    }

    #[tokio::test]
    async fn retries_transient_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(503)).up_to_n_times(2).mount(&server).await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&server)
            .await;
        let req = reqwest::Client::new().get(server.uri());
        let json = send_json_with(req, quick()).await.unwrap();
        assert_eq!(json["ok"], true);
    }

    #[tokio::test]
    async fn gives_up_on_client_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).respond_with(ResponseTemplate::new(404)).expect(1).mount(&server).await;
        let req = reqwest::Client::new().get(server.uri());
        assert!(send_json_with(req, quick()).await.is_err());
    }
}
