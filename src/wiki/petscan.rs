//! Saved PetScan queries as a candidate filter (#71).

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

pub const BASE_URL: &str = "https://petscan.wmcloud.org/";
/// PetScan can take minutes; one patient attempt beats retrying a slow query.
const TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub struct PetScan {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Default)]
pub struct PetScanResult {
    /// Database name of the wiki the pages are on, e.g. `enwiki` or `wikidatawiki`.
    pub wiki: String,
    pub page_ids: HashSet<u64>,
    /// Titles in the main namespace (on Wikidata: item ids).
    pub titles: HashSet<String>,
}

impl PetScan {
    pub fn new(http: reqwest::Client, base_url: &str) -> Self {
        Self { http, base_url: base_url.to_string() }
    }

    /// Run the saved query `psid`.
    pub async fn query(&self, psid: u64) -> Result<PetScanResult> {
        let params = [
            ("psid", psid.to_string()),
            ("format", "json".into()),
            ("output_compatability", "quick-intersection".into()),
            ("doit", "1".into()),
        ];
        let request = self.http.get(&self.base_url).query(&params).timeout(TIMEOUT);
        let json: Value =
            request.send().await?.error_for_status()?.json().await.context("PetScan did not return JSON")?;
        let mut result =
            PetScanResult { wiki: json["wiki"].as_str().unwrap_or_default().to_string(), ..Default::default() };
        for page in json["pages"].as_array().context("PetScan returned no page list")? {
            result.page_ids.extend(page["page_id"].as_u64().filter(|id| *id > 0));
            if page["page_namespace"] == 0 {
                result.titles.extend(page["page_title"].as_str().map(|t| t.replace('_', " ")));
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn reads_quick_intersection_output() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("psid", "123"))
            .and(query_param("output_compatability", "quick-intersection"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "wiki": "enwiki", "status": "OK",
                "pages": [
                    {"page_id": 10, "page_namespace": 0, "page_title": "Paris_Hilton"},
                    {"page_id": 11, "page_namespace": 14, "page_title": "Hiltons"}
                ]
            })))
            .mount(&server)
            .await;
        let result = PetScan::new(reqwest::Client::new(), &server.uri()).query(123).await.unwrap();
        assert_eq!(result.wiki, "enwiki");
        assert_eq!(result.page_ids, HashSet::from([10, 11]));
        assert_eq!(result.titles, HashSet::from(["Paris Hilton".to_string()]));
    }
}
