//! Read-only MediaWiki Action API client for any wiki (formatversion 2).
//! Authenticated writes go through [`crate::auth`].

pub use wikimisc::wiki_api::{Client as MwApi, Params, params};

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
