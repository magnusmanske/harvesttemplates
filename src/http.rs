//! JSON over HTTP with retries for the transient failures Wikimedia APIs produce.

use anyhow::{Context, Result, anyhow, bail};
use reqwest::{RequestBuilder, StatusCode, header::RETRY_AFTER};
use serde_json::Value;
use std::time::Duration;

const MAX_ATTEMPTS: u32 = 4;
const BASE_DELAY: Duration = Duration::from_secs(1);
const MAX_DELAY: Duration = Duration::from_secs(60);

/// Send `request` and parse the JSON body. Retries on 429, 5xx, timeouts and
/// connection errors with exponential back-off, honouring `Retry-After`.
pub async fn send_json(request: RequestBuilder) -> Result<Value> {
    send_json_with_delay(request, BASE_DELAY).await
}

async fn send_json_with_delay(request: RequestBuilder, base_delay: Duration) -> Result<Value> {
    let mut delay = base_delay;
    for attempt in 1..=MAX_ATTEMPTS {
        let req = request.try_clone().context("request body is not retryable")?;
        match attempt_once(req).await {
            Attempt::Done(result) => return result,
            Attempt::Retry(_, reason) if attempt == MAX_ATTEMPTS => bail!("{reason}"),
            Attempt::Retry(retry_after, reason) => {
                delay = retry_after.unwrap_or(delay).min(MAX_DELAY);
                tracing::warn!("{reason}, retrying in {delay:?}");
            }
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_DELAY);
    }
    unreachable!("the last attempt always returns")
}

enum Attempt {
    Done(Result<Value>),
    Retry(Option<Duration>, String),
}

async fn attempt_once(req: RequestBuilder) -> Attempt {
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            Attempt::Done(resp.json().await.context("response is not valid JSON"))
        }
        Ok(resp) if is_transient(resp.status()) => Attempt::Retry(
            retry_after(&resp),
            format!("HTTP {} from {}", resp.status(), resp.url()),
        ),
        Ok(resp) => Attempt::Done(Err(anyhow!("HTTP {} from {}", resp.status(), resp.url()))),
        Err(e) if e.is_timeout() || e.is_connect() => Attempt::Retry(None, e.to_string()),
        Err(e) => Attempt::Done(Err(e.into())),
    }
}

fn is_transient(status: StatusCode) -> bool {
    status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs = resp.headers().get(RETRY_AFTER)?.to_str().ok()?.parse().ok()?;
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn retries_transient_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&server)
            .await;
        let req = reqwest::Client::new().get(server.uri());
        let json = send_json_with_delay(req, Duration::from_millis(1)).await.unwrap();
        assert_eq!(json["ok"], true);
    }

    #[tokio::test]
    async fn gives_up_on_client_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let req = reqwest::Client::new().get(server.uri());
        assert!(send_json_with_delay(req, Duration::from_millis(1)).await.is_err());
    }
}
