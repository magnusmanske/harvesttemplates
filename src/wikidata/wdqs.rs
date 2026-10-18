//! The Wikidata Query Service.
//!
//! Wikidata keeps scholarly articles in a graph of their own, apart from
//! everything else — classes, properties, and most items templates are about.
//! Statements of an item that may be an article are asked of both graphs; class
//! hierarchies exist only in the main one. This holds on WDQS v1 (Blazegraph)
//! and v2 (QLever) alike, and every query here is plain SPARQL for both.

use crate::ids::{ItemId, PropertyId};
use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Semaphore;
use wikimisc::sparql_client::SparqlClient;
use wikimisc::sparql_endpoint::SparqlEndpoint;
use wikimisc::sparql_retry::{Retry, RetryPolicy};

/// The path from an item to its classes.
pub const INSTANCE_OF: &str = "wdt:P31";
/// WDQS allows a handful of parallel queries per client; stay below that.
const MAX_PARALLEL: usize = 4;
const VALUES_CHUNK: usize = 250;
/// Retries of a query that failed in a way that may go away on its own (rate
/// limiting, 5xx); a timeout or a malformed query fails the same way again.
const RETRIES: u32 = 3;

/// Wikidata Query Service client for both of Wikidata's graphs.
#[derive(Debug, Clone)]
pub struct Wdqs {
    main: SparqlClient,
    scholarly: SparqlClient,
    permits: Arc<Semaphore>,
}

impl Wdqs {
    /// WDQS v1. v2 (`SparqlEndpoint::wikidata_main` / `wikidata_scholarly`)
    /// refuses user agents it has not allow-listed until its public launch.
    pub fn wikidata(http: reqwest::Client) -> Self {
        Self::new(http, SparqlEndpoint::wdqs(), SparqlEndpoint::wdqs_scholarly())
    }

    pub fn new(http: reqwest::Client, main: SparqlEndpoint, scholarly: SparqlEndpoint) -> Self {
        let client = |endpoint| {
            SparqlClient::with_client(endpoint, http.clone())
                .with_retry_policy(RetryPolicy::new(RETRIES).on_retry(log_retry))
        };
        Self { main: client(main), scholarly: client(scholarly), permits: Arc::new(Semaphore::new(MAX_PARALLEL)) }
    }

    async fn query(&self, graph: &SparqlClient, sparql: &str) -> Result<Value> {
        let _permit = self.permits.acquire().await?;
        Ok(graph.query_json(sparql).await?)
    }

    async fn ask_in(&self, graph: &SparqlClient, sparql: &str) -> Result<bool> {
        let _permit = self.permits.acquire().await?;
        Ok(graph.ask(sparql).await?)
    }

    /// An ASK query on the main graph. A reply without an answer is an error,
    /// never a "no".
    pub async fn ask(&self, sparql: &str) -> Result<bool> {
        self.ask_in(&self.main, sparql).await
    }

    /// An ASK query about the statements of an item that may be a scholarly
    /// article: "yes" if either graph says so.
    pub async fn ask_either(&self, sparql: &str) -> Result<bool> {
        Ok(self.ask(sparql).await? || self.ask_in(&self.scholarly, sparql).await?)
    }

    /// Item ids bound to `var` in the results of a SELECT query on the main graph.
    pub async fn select_items(&self, sparql: &str, var: &str) -> Result<Vec<ItemId>> {
        self.select_items_in(&self.main, sparql, var).await
    }

    async fn select_items_in(&self, graph: &SparqlClient, sparql: &str, var: &str) -> Result<Vec<ItemId>> {
        let json = self.query(graph, sparql).await?;
        Ok(bindings(&json).filter_map(|b| item_from_uri(b[var]["value"].as_str()?)).collect())
    }

    /// The Wikidata item of a wiki, via "Wikimedia database name" (P1800).
    pub async fn edition_for(&self, dbname: &str) -> Result<Option<ItemId>> {
        let sparql = format!("SELECT ?wiki {{ ?wiki wdt:P1800 {} }}", string_literal(dbname));
        Ok(self.select_items(&sparql, "wiki").await?.into_iter().next())
    }

    /// Which of `items` already have a statement for `property` (any rank, any value).
    /// Bounded by the number of candidates, unlike asking for every item with the property.
    /// Main graph only: this is a pre-filter, and the live check sees articles too.
    pub async fn items_with_property(&self, property: PropertyId, items: &[ItemId]) -> Result<HashSet<ItemId>> {
        let mut found = HashSet::new();
        for chunk in items.chunks(VALUES_CHUNK) {
            let sparql = format!("SELECT ?item {{ VALUES ?item {{ {} }} ?item p:{property} [] }}", values(chunk));
            found.extend(self.select_items(&sparql, "item").await?);
        }
        Ok(found)
    }

    /// Which of `items` are linked by `relation` (e.g. [`INSTANCE_OF`]) to one
    /// of `classes`, or to one of their subclasses.
    pub async fn items_in_classes(
        &self,
        items: &[ItemId],
        relation: &str,
        classes: &[ItemId],
    ) -> Result<HashSet<ItemId>> {
        let classes = values(classes);
        let mut found = HashSet::new();
        for chunk in items.chunks(VALUES_CHUNK) {
            let sparql = format!(
                "SELECT DISTINCT ?item {{ VALUES ?item {{ {} }} VALUES ?class {{ {classes} }} ?item {relation}/wdt:P279* ?class }}",
                values(chunk)
            );
            found.extend(self.select_items(&sparql, "item").await?);
        }
        let rest: Vec<ItemId> = items.iter().filter(|q| !found.contains(q)).copied().collect();
        found.extend(self.articles_in_classes(&rest, relation, &classes).await?);
        Ok(found)
    }

    /// Which of `items` are articles whose `relation` values — read in the
    /// scholarly graph — are `classes` or below them in the main graph.
    async fn articles_in_classes(&self, items: &[ItemId], relation: &str, classes: &str) -> Result<HashSet<ItemId>> {
        let mut items_by_type: HashMap<ItemId, Vec<ItemId>> = HashMap::new();
        for chunk in items.chunks(VALUES_CHUNK) {
            let sparql =
                format!("SELECT ?item ?type {{ VALUES ?item {{ {} }} ?item {relation} ?type }}", values(chunk));
            let json = self.query(&self.scholarly, &sparql).await?;
            for b in bindings(&json) {
                let uri = |var: &str| item_from_uri(b[var]["value"].as_str()?);
                if let (Some(item), Some(class)) = (uri("item"), uri("type")) {
                    items_by_type.entry(class).or_default().push(item);
                }
            }
        }
        let types: Vec<ItemId> = items_by_type.keys().copied().collect();
        let mut found = HashSet::new();
        for chunk in types.chunks(VALUES_CHUNK) {
            let sparql = format!(
                "SELECT DISTINCT ?type {{ VALUES ?type {{ {} }} VALUES ?class {{ {classes} }} ?type wdt:P279* ?class }}",
                values(chunk)
            );
            for class in self.select_items(&sparql, "type").await? {
                found.extend(items_by_type.get(&class).into_iter().flatten());
            }
        }
        Ok(found)
    }
}

/// Logs a retry, as `crate::http` logs its own.
fn log_retry(retry: &Retry<'_>) {
    tracing::warn!("{}, retrying in {:?}", retry.error, retry.wait);
}

fn bindings(json: &Value) -> impl Iterator<Item = &Value> {
    json["results"]["bindings"].as_array().into_iter().flatten()
}

/// Items as a `VALUES` list.
fn values(items: &[ItemId]) -> String {
    items.iter().map(|q| format!("wd:{q}")).collect::<Vec<_>>().join(" ")
}

pub fn item_from_uri(uri: &str) -> Option<ItemId> {
    uri.strip_prefix("http://www.wikidata.org/entity/")?.parse().ok()
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

    fn endpoint(server: &MockServer) -> SparqlEndpoint {
        SparqlEndpoint::new(server.uri())
    }

    /// Both graphs on one mock server.
    fn mocked(server: &MockServer) -> Wdqs {
        Wdqs::new(reqwest::Client::new(), endpoint(server), endpoint(server))
    }

    fn rows(rows: serde_json::Value) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({"results": {"bindings": rows}}))
    }

    fn uri(q: &str) -> serde_json::Value {
        json!({"type": "uri", "value": format!("http://www.wikidata.org/entity/{q}")})
    }

    #[test]
    fn literals_are_escaped() {
        assert_eq!(string_literal(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(item_from_uri("http://www.wikidata.org/entity/Q42"), Some(ItemId(42)));
        assert_eq!(item_from_uri("http://www.wikidata.org/entity/P42"), None);
    }

    #[tokio::test]
    async fn items_in_classes_follows_subclasses() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("wdt%3AP31%2Fwdt%3AP279*"))
            .and(body_string_contains("wd%3AQ571"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": {"bindings": [
                {"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q2"}}
            ]}})))
            .mount(&server)
            .await;
        // Q1 is looked up as an article too, and is not one.
        Mock::given(method("POST")).respond_with(rows(json!([]))).mount(&server).await;
        let wdqs = mocked(&server);
        let found = wdqs.items_in_classes(&[ItemId(1), ItemId(2)], INSTANCE_OF, &[ItemId(571)]).await.unwrap();
        assert_eq!(found, HashSet::from([ItemId(2)]));
    }

    #[tokio::test]
    async fn items_with_property_batches() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("p%3AP345"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": {"bindings": [
                {"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q1"}}
            ]}})))
            .expect(2)
            .mount(&server)
            .await;
        let wdqs = mocked(&server);
        let items: Vec<ItemId> = (1..=300).map(ItemId).collect();
        let found = wdqs.items_with_property(PropertyId(345), &items).await.unwrap();
        assert_eq!(found, HashSet::from([ItemId(1)]));
    }

    #[tokio::test]
    async fn articles_are_classified_through_the_main_graph() {
        let (main, scholarly) = (MockServer::start().await, MockServer::start().await);
        // Q3 is an article: unknown to the main graph, typed in the scholarly
        // one, and its type is a subclass of the wanted class in the main one.
        Mock::given(method("POST"))
            .and(body_string_contains("VALUES+%3Ftype"))
            .respond_with(rows(json!([{"type": uri("Q13442814")}])))
            .expect(1)
            .mount(&main)
            .await;
        Mock::given(method("POST")).respond_with(rows(json!([]))).expect(1).mount(&main).await;
        Mock::given(method("POST"))
            .and(body_string_contains("%3Fitem+wdt%3AP31+%3Ftype"))
            .respond_with(rows(json!([{"item": uri("Q3"), "type": uri("Q13442814")}])))
            .expect(1)
            .mount(&scholarly)
            .await;
        let wdqs = Wdqs::new(reqwest::Client::new(), endpoint(&main), endpoint(&scholarly));
        let found = wdqs.items_in_classes(&[ItemId(3)], INSTANCE_OF, &[ItemId(732_577)]).await.unwrap();
        assert_eq!(found, HashSet::from([ItemId(3)]));
    }

    #[tokio::test]
    async fn ask_either_asks_the_scholarly_graph_only_after_a_no() {
        let (main, scholarly) = (MockServer::start().await, MockServer::start().await);
        let answer = |b: bool| ResponseTemplate::new(200).set_body_json(json!({"head": {}, "boolean": b}));
        Mock::given(method("POST")).respond_with(answer(false)).expect(1).mount(&main).await;
        Mock::given(method("POST")).respond_with(answer(true)).expect(1).mount(&scholarly).await;
        let wdqs = Wdqs::new(reqwest::Client::new(), endpoint(&main), endpoint(&scholarly));
        assert!(wdqs.ask_either("ASK {}").await.unwrap());
    }

    #[tokio::test]
    async fn an_ask_without_an_answer_is_an_error_not_a_no() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).respond_with(rows(json!([]))).mount(&server).await;
        assert!(mocked(&server).ask("ASK {}").await.is_err());
    }
}
