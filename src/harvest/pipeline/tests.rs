use super::*;
use crate::harvest::spec::JobSpec;
use crate::ids::PropertyId;
use crate::value::TransformSpec;
use serde_json::json;
use wiremock::matchers::body_string_contains;
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn mock(server: &MockServer, needle: &str, body: Json) {
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

/// A small enwiki + Wikidata world.
async fn world() -> (MockServer, Clients) {
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
    mock(
        &server,
        "ids=P19&",
        json!({"entities": {"P19": {"id": "P19", "datatype": "wikibase-item", "claims": {}}}}),
    )
    .await;
    mock(
        &server,
        "prop=redirects",
        json!({"query": {"pages": [
            {"title": "Template:X", "redirects": [{"title": "Template:IMDb"}]}
        ]}}),
    )
    .await;
    mock(
        &server,
        "ids=Q1&",
        json!({"entities": {"Q1": {"id": "Q1", "claims": {}}}}),
    )
    .await;
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
    mock(
        &server,
        "titles=Nowhere&",
        json!({"query": {"pages": [{"title": "Nowhere", "missing": true}]}}),
    )
    .await;
    let clients = Clients::mocked(&server.uri());
    (server, clients)
}

fn page(item: Option<u64>) -> Page {
    Page {
        id: 1,
        title: "The Shawshank Redemption".into(),
        item: item.map(ItemId),
        latest_revision: 7,
    }
}

fn revision(text: &str) -> Revision {
    Revision {
        id: 7,
        text: text.into(),
    }
}

async fn imdb_job(clients: &Clients) -> Job {
    let spec = JobSpec {
        property: Some(PropertyId(345)),
        template: "IMDb title".into(),
        parameters: vec!["id".into(), "1".into()],
        transform: TransformSpec {
            add_prefix: "tt".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    Job::prepare(clients, spec).await.unwrap()
}

#[tokio::test]
async fn external_id_is_harvested_with_reference() {
    let (_server, clients) = world().await;
    let job = imdb_job(&clients).await;
    assert_eq!(job.site.edition, Some(ItemId(328)));
    let out = evaluate(
        &job,
        &clients,
        &page(Some(1)),
        &revision("{{IMDb title|0111161|Shawshank}}"),
    )
    .await;
    assert_eq!(out.raw.as_deref(), Some("0111161"));
    let edit = out.result.unwrap();
    assert_eq!(edit.value, Value::String("tt0111161".into()));
    let reference = &edit.statement["references"][0]["snaks"];
    assert_eq!(reference["P143"][0]["datavalue"]["value"]["id"], "Q328");
    assert!(
        reference["P4656"][0]["datavalue"]["value"]
            .as_str()
            .unwrap()
            .ends_with("&oldid=7")
    );
    assert!(summary(&job, &edit.value, "abc").contains("[[Property:P345]]: tt0111161 from enwiki"));
}

#[tokio::test]
async fn redirect_names_and_spaces() {
    let (_server, clients) = world().await;
    let job = imdb_job(&clients).await;
    let out = evaluate(&job, &clients, &page(Some(1)), &revision("{{IMDb | id = 0111161 }}")).await;
    assert_eq!(out.result.unwrap().value, Value::String("tt0111161".into()));
}

#[tokio::test]
async fn rejections() {
    let (_server, clients) = world().await;
    let job = imdb_job(&clients).await;
    let check = |item, text: &'static str| {
        let (job, clients) = (&job, &clients);
        async move {
            evaluate(job, clients, &page(item), &revision(text))
                .await
                .result
                .unwrap_err()
        }
    };
    assert_eq!(
        check(None, "{{IMDb title|0111161}}").await,
        skip("the page has no Wikidata item")
    );
    assert_eq!(check(Some(1), "{{Other|0111161}}").await, skip("template not found"));
    assert_eq!(check(Some(1), "{{IMDb title|id=}}").await, skip("no value"));
    assert_eq!(
        check(Some(1), "{{IMDb title|abc}}").await,
        error("constraint violation: format")
    );
    assert_eq!(
        check(Some(2), "{{IMDb title|0111161}}").await,
        skip("the item already has the property")
    );
    assert_eq!(
        check(Some(2), "{{IMDb title|0000002}}").await,
        skip("the item already has this value")
    );
}

#[tokio::test]
async fn items_from_links() {
    let (_server, clients) = world().await;
    let spec = JobSpec {
        property: Some(PropertyId(19)),
        template: "X".into(),
        parameters: vec!["place".into()],
        plain_links: false,
        ..Default::default()
    };
    let job = Job::prepare(&clients, spec).await.unwrap();
    let eval = |text: &'static str| {
        let (job, clients) = (&job, &clients);
        async move { evaluate(job, clients, &page(Some(1)), &revision(text)).await.result }
    };
    assert_eq!(
        eval("{{X|place=[[Paris|the city]]}}").await.unwrap().value,
        Value::Item(ItemId(90))
    );
    assert_eq!(
        eval("{{X|place=Paris}}").await.unwrap_err(),
        error("no link to a target page")
    );
    assert_eq!(
        eval("{{X|place=[[Self]]}}").await.unwrap_err(),
        error("the link points to the page itself")
    );
    assert_eq!(
        eval("{{X|place=[[Nowhere]]}}").await.unwrap_err(),
        error("[[Nowhere]] does not exist")
    );
}

#[tokio::test]
async fn invalid_specs_are_explained() {
    let (_server, clients) = world().await;
    let base = JobSpec {
        property: Some(PropertyId(345)),
        template: "X".into(),
        ..Default::default()
    };
    let err = Job::prepare(&clients, base.clone()).await.unwrap_err();
    assert_eq!(err.to_string(), "choose a template parameter");
    let bad_regex = JobSpec {
        parameters: vec!["1".into()],
        transform: TransformSpec {
            search: "(".into(),
            ..Default::default()
        },
        ..base
    };
    assert!(
        Job::prepare(&clients, bad_regex)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("invalid regex")
    );
}
