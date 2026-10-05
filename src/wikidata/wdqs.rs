use crate::http::send_json;
use crate::ids::{ItemId, PropertyId};
use anyhow::Result;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub const ENDPOINT: &str = "https://query.wikidata.org/sparql";
/// WDQS allows a handful of parallel queries per client; stay below that.
const MAX_PARALLEL: usize = 4;
const VALUES_CHUNK: usize = 250;

/// Wikidata Query Service client.
#[derive(Debug, Clone)]
pub struct Wdqs {
    http: reqwest::Client,
    endpoint: String,
    permits: Arc<Semaphore>,
}

impl Wdqs {
    pub fn new(http: reqwest::Client, endpoint: &str) -> Self {
        Self {
            http,
            endpoint: endpoint.to_string(),
            permits: Arc::new(Semaphore::new(MAX_PARALLEL)),
        }
    }

    async fn query(&self, sparql: &str) -> Result<Value> {
        let _permit = self.permits.acquire().await?;
        let request = self
            .http
            .post(&self.endpoint)
            .header(reqwest::header::ACCEPT, "application/sparql-results+json")
            .form(&[("query", sparql)]);
        send_json(request).await
    }

    pub async fn ask(&self, sparql: &str) -> Result<bool> {
        Ok(self.query(sparql).await?["boolean"]
            .as_bool()
            .unwrap_or(false))
    }

    /// Item ids bound to `var` in the results of a SELECT query.
    pub async fn select_items(&self, sparql: &str, var: &str) -> Result<Vec<ItemId>> {
        let json = self.query(sparql).await?;
        let bindings = json["results"]["bindings"].as_array().into_iter().flatten();
        Ok(bindings
            .filter_map(|b| item_from_uri(b[var]["value"].as_str()?))
            .collect())
    }

    /// The Wikidata item of a wiki, via "Wikimedia database name" (P1800).
    pub async fn edition_for(&self, dbname: &str) -> Result<Option<ItemId>> {
        let sparql = format!(
            "SELECT ?wiki {{ ?wiki wdt:P1800 {} }}",
            string_literal(dbname)
        );
        Ok(self.select_items(&sparql, "wiki").await?.into_iter().next())
    }

    /// Which of `items` already have a statement for `property` (any rank, any value).
    /// Bounded by the number of candidates, unlike asking for every item with the property.
    pub async fn items_with_property(
        &self,
        property: PropertyId,
        items: &[ItemId],
    ) -> Result<HashSet<ItemId>> {
        let mut found = HashSet::new();
        for chunk in items.chunks(VALUES_CHUNK) {
            let values: Vec<String> = chunk.iter().map(|q| format!("wd:{q}")).collect();
            let sparql = format!(
                "SELECT ?item {{ VALUES ?item {{ {} }} ?item p:{property} [] }}",
                values.join(" ")
            );
            found.extend(self.select_items(&sparql, "item").await?);
        }
        Ok(found)
    }
}

pub fn item_from_uri(uri: &str) -> Option<ItemId> {
    uri.strip_prefix("http://www.wikidata.org/entity/")?
        .parse()
        .ok()
}

/// A SPARQL string literal; JSON escaping is a valid subset of SPARQL escaping.
pub fn string_literal(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn literals_are_escaped() {
        assert_eq!(string_literal(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(
            item_from_uri("http://www.wikidata.org/entity/Q42"),
            Some(ItemId(42))
        );
        assert_eq!(item_from_uri("http://www.wikidata.org/entity/P42"), None);
    }

    #[tokio::test]
    async fn items_with_property_batches() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("p%3AP345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"results": {"bindings": [
                    {"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q1"}}
                ]}}),
            ))
            .expect(2)
            .mount(&server)
            .await;
        let wdqs = Wdqs::new(reqwest::Client::new(), &server.uri());
        let items: Vec<ItemId> = (1..=300).map(ItemId).collect();
        let found = wdqs
            .items_with_property(PropertyId(345), &items)
            .await
            .unwrap();
        assert_eq!(found, HashSet::from([ItemId(1)]));
    }
}
