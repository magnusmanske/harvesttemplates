//! Shared test helpers. DB tests need Docker (as on GitHub's runners).

use crate::app_state::{AppState, Clients};
use crate::auth::{FileSessionStore, OAuth};
use crate::config::Config;
use crate::harvest::ActiveRuns;
use crate::storage::Store;
use crate::wiki::ApiSource;
use serde_json::{Value as Json, json};
use std::sync::Arc;
use testcontainers_modules::mariadb::Mariadb;
use testcontainers_modules::testcontainers::ContainerAsync;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use wiremock::matchers::body_string_contains;
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A migrated store in a fresh MariaDB container. Keep the container alive for the test.
pub async fn test_store() -> (ContainerAsync<Mariadb>, Store) {
    let container = Mariadb::default().start().await.expect("Docker must be running for DB tests");
    let port = container.get_host_port_ipv4(3306).await.unwrap();
    let store = Store::from_url(&format!("mysql://root@127.0.0.1:{port}/test"), 4).unwrap();
    store.migrate().await.unwrap();
    (container, store)
}

/// An app whose every outbound request goes to `mock_url`, with a real (container) database.
pub fn test_app(store: Store, mock_url: &str, session_dir: &std::path::Path) -> Arc<AppState> {
    let mut config: Config = serde_json::from_str(include_str!("../config.json.template")).unwrap();
    config.harvest.edit_interval_ms = 0;
    let clients = Clients::mocked(mock_url);
    Arc::new(AppState {
        pages: Arc::new(ApiSource { api: clients.mw.clone() }),
        oauth: OAuth::with_base(reqwest::Client::new(), &config.oauth, mock_url),
        sessions: FileSessionStore::new(session_dir.to_path_buf()).unwrap(),
        wikidata_api_url: mock_url.to_string(),
        runs: Arc::new(ActiveRuns::default()),
        tokens: Arc::default(),
        clients,
        store,
        config,
    })
}

pub async fn mock(server: &MockServer, needle: &str, body: Json) {
    Mock::given(body_string_contains(needle))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

fn item_snak(id: &str) -> Json {
    json!({"snaktype": "value", "datavalue": {"type": "wikibase-entityid", "value": {"id": id}}})
}

fn string_snak(s: &str) -> Json {
    json!({"snaktype": "value", "datavalue": {"type": "string", "value": s}})
}

/// A small enwiki + Wikidata world on one mock server.
pub async fn world() -> (MockServer, Clients) {
    let server = MockServer::start().await;
    let ns = |id: i32, name: &str| json!({"id": id, "name": name, "canonical": name, "case": "first-letter"});
    mock(
        &server,
        "meta=siteinfo",
        json!({"query": {
            "general": {"wikiid": "enwiki", "lang": "en"},
            "namespaces": {"0": ns(0, ""), "6": ns(6, "File"), "10": ns(10, "Template"), "14": ns(14, "Category")},
            "namespacealiases": [{"id": 6, "alias": "Image"}]
        }}),
    )
    .await;
    mock(
        &server,
        "P1800",
        json!({"results": {"bindings": [
            {"wiki": {"type": "uri", "value": "http://www.wikidata.org/entity/Q328"}}
        ]}}),
    )
    .await;
    mock(
        &server,
        "ids=P345&",
        json!({"entities": {"P345": {
            "id": "P345", "datatype": "external-id", "labels": {"en": {"value": "IMDb ID"}},
            "claims": {"P2302": [
                {"mainsnak": item_snak("Q21502404"), "qualifiers": {"P1793": [string_snak(r"tt\d{7,8}")]}},
                {"mainsnak": item_snak("Q19474404")}
            ]}
        }}}),
    )
    .await;
    for (id, datatype) in [("P19", "wikibase-item"), ("P407", "wikibase-item"), ("P585", "time")] {
        mock(&server, &format!("ids={id}&"), json!({"entities": {id: {"id": id, "datatype": datatype, "claims": {}}}}))
            .await;
    }
    mock(
        &server,
        "prop=redirects",
        json!({"query": {"pages": [
            {"title": "Template:X", "redirects": [{"title": "Template:IMDb"}]}
        ]}}),
    )
    .await;
    mock(&server, "ids=Q1&", json!({"entities": {"Q1": {"id": "Q1", "claims": {}}}})).await;
    mock(
        &server,
        "ids=Q2&",
        json!({"entities": {"Q2": {"id": "Q2", "claims": {"P345": [
            {"rank": "normal", "mainsnak": {"datavalue": {"type": "string", "value": "tt0000002"}}}
        ]}}}}),
    )
    .await;
    mock(
        &server,
        "titles=Paris&",
        json!({"query": {"pages": [{"title": "Paris", "pageprops": {"wikibase_item": "Q90"}}]}}),
    )
    .await;
    mock(
        &server,
        "titles=Self&",
        json!({"query": {"pages": [{"title": "Self", "pageprops": {"wikibase_item": "Q1"}}]}}),
    )
    .await;
    mock(&server, "titles=Nowhere&", json!({"query": {"pages": [{"title": "Nowhere", "missing": true}]}})).await;
    // Like the real API for text it cannot parse; tests mount specific answers on top.
    Mock::given(body_string_contains("action=wbparsevalue"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"results": [{"error": "ValueParsers\\ParseException"}]})),
        )
        .with_priority(10)
        .mount(&server)
        .await;
    let clients = Clients::mocked(&server.uri());
    (server, clients)
}
