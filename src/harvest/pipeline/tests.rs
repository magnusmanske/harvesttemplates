use super::*;
use crate::harvest::spec::JobSpec;
use crate::ids::PropertyId;
use crate::test_support::world;
use crate::value::TransformSpec;

fn page(item: Option<u64>) -> Page {
    Page { id: 1, title: "The Shawshank Redemption".into(), item: item.map(ItemId), latest_revision: 7 }
}

fn revision(text: &str) -> Revision {
    Revision { id: 7, text: text.into() }
}

async fn imdb_job(clients: &Clients) -> Job {
    let spec = JobSpec {
        property: Some(PropertyId(345)),
        template: "IMDb title".into(),
        parameters: vec!["id".into(), "1".into()],
        transform: TransformSpec { add_prefix: "tt".into(), ..Default::default() },
        ..Default::default()
    };
    Job::prepare(clients, spec).await.unwrap()
}

#[tokio::test]
async fn external_id_is_harvested_with_reference() {
    let (_server, clients) = world().await;
    let job = imdb_job(&clients).await;
    assert_eq!(job.site.edition, Some(ItemId(328)));
    let out = evaluate(&job, &clients, &page(Some(1)), &revision("{{IMDb title|0111161|Shawshank}}")).await;
    assert_eq!(out.raw.as_deref(), Some("0111161"));
    let edit = out.result.unwrap();
    assert_eq!(edit.value, Value::String("tt0111161".into()));
    let reference = &edit.statement["references"][0]["snaks"];
    assert_eq!(reference["P143"][0]["datavalue"]["value"]["id"], "Q328");
    assert!(reference["P4656"][0]["datavalue"]["value"].as_str().unwrap().ends_with("&oldid=7"));
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
        async move { evaluate(job, clients, &page(item), &revision(text)).await.result.unwrap_err() }
    };
    assert_eq!(check(None, "{{IMDb title|0111161}}").await, skip("the page has no Wikidata item"));
    assert_eq!(check(Some(1), "{{Other|0111161}}").await, skip("template not found"));
    assert_eq!(check(Some(1), "{{IMDb title|id=}}").await, skip("no value"));
    assert_eq!(check(Some(1), "{{IMDb title|abc}}").await, error("constraint violation: format"));
    assert_eq!(check(Some(2), "{{IMDb title|0111161}}").await, skip("the item already has the property"));
    assert_eq!(check(Some(2), "{{IMDb title|0000002}}").await, skip("the item already has this value"));
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
    assert_eq!(eval("{{X|place=[[Paris|the city]]}}").await.unwrap().value, Value::Item(ItemId(90)));
    assert_eq!(eval("{{X|place=Paris}}").await.unwrap_err(), error("no link to a target page"));
    assert_eq!(eval("{{X|place=[[Self]]}}").await.unwrap_err(), error("the link points to the page itself"));
    assert_eq!(eval("{{X|place=[[Nowhere]]}}").await.unwrap_err(), error("[[Nowhere]] does not exist"));
}

#[tokio::test]
async fn invalid_specs_are_explained() {
    let (_server, clients) = world().await;
    let base = JobSpec { property: Some(PropertyId(345)), template: "X".into(), ..Default::default() };
    let err = Job::prepare(&clients, base.clone()).await.unwrap_err();
    assert_eq!(err.to_string(), "choose a template parameter");
    let bad_regex = JobSpec {
        parameters: vec!["1".into()],
        transform: TransformSpec { search: "(".into(), ..Default::default() },
        ..base
    };
    assert!(Job::prepare(&clients, bad_regex).await.unwrap_err().to_string().starts_with("invalid regex"));
}

/// Read-only: loads and evaluates real pages, never edits. Needs `config.json` (replica tunnel optional).
#[tokio::test]
#[ignore = "requires database / external services — run with cargo test -- --ignored"]
#[allow(clippy::print_stdout)]
async fn live_preview_on_enwiki() {
    use crate::harvest::load::candidates;
    use crate::wiki::{ApiSource, Replicas, WithFallback};
    let config = crate::config::Config::load("config.json".as_ref()).unwrap();
    let clients = Clients::new(&crate::app_state::http_client(&config.user_agent).unwrap());
    let source = WithFallback {
        primary: Replicas::new(config.replicas, config.db_user),
        fallback: ApiSource { api: clients.mw.clone() },
    };
    let spec = JobSpec {
        property: Some(PropertyId(345)),
        template: "IMDb title".into(),
        parameters: vec!["id".into(), "1".into()],
        transform: TransformSpec { add_prefix: "tt".into(), ..Default::default() },
        category: "2024 films".into(),
        depth: 1,
        ..Default::default()
    };
    let job = Job::prepare(&clients, spec).await.unwrap();
    let limits = crate::wiki::Limits { max_pages: 500_000, max_depth: 30, max_categories: 20_000 };
    let started = std::time::Instant::now();
    let (pages, excluded) = candidates(&job, &clients, &source, limits).await.unwrap();
    println!("{} candidates in {:?}, excluded {excluded:?}", pages.len(), started.elapsed());
    let ids: Vec<u64> = pages.iter().take(10).map(|p| p.id).collect();
    let revisions = crate::wiki::content::revisions(&clients.mw, &job.site, &ids).await.unwrap();
    for page in pages.iter().take(10) {
        let out = evaluate(&job, &clients, page, &revisions[&page.id]).await;
        let result = out.result.as_ref().map(|e| e.item.to_string());
        println!("{:40} raw={:?} value={:?} -> {result:?}", page.title, out.raw, out.value);
    }
}

#[tokio::test]
async fn qualifiers_fixed_and_from_parameters() {
    use crate::harvest::spec::{QualifierSource, QualifierSpec};
    let (_server, clients) = world().await;
    let fixed = |value: &str| QualifierSpec {
        property: PropertyId(407),
        source: QualifierSource::Fixed { value: value.into() },
    };
    let from_date =
        QualifierSpec { property: PropertyId(585), source: QualifierSource::Parameter { names: vec!["date".into()] } };
    let spec = JobSpec {
        property: Some(PropertyId(345)),
        template: "IMDb title".into(),
        parameters: vec!["1".into()],
        transform: TransformSpec { add_prefix: "tt".into(), ..Default::default() },
        qualifiers: vec![fixed("Q1860"), from_date],
        date_limit: None,
        ..Default::default()
    };
    let job = Job::prepare(&clients, spec.clone()).await.unwrap();
    let eval = |text: &'static str| {
        let (job, clients) = (&job, &clients);
        async move { evaluate(job, clients, &page(Some(1)), &revision(text)).await }
    };

    let out = eval("{{IMDb title|0111161|date=12 May 1950}}").await;
    assert_eq!(out.value.as_deref(), Some("tt0111161; P407: Q1860; P585: 1950-05-12"));
    let statement = out.result.unwrap().statement;
    assert_eq!(statement["qualifiers"]["P407"][0]["datavalue"]["value"]["id"], "Q1860");
    assert_eq!(statement["qualifiers"]["P585"][0]["datavalue"]["value"]["precision"], 11);
    assert_eq!(statement["qualifiers-order"], serde_json::json!(["P407", "P585"]));

    let without_date = eval("{{IMDb title|0111161}}").await.result.unwrap().statement;
    assert_eq!(without_date["qualifiers-order"], serde_json::json!(["P407"]), "missing parameter: qualifier left out");

    let bad = eval("{{IMDb title|0111161|date=sometime}}").await.result.unwrap_err();
    assert_eq!(bad, error("qualifier P585: could not find a date"));

    let typo = JobSpec { qualifiers: vec![fixed("Q18x60")], ..spec };
    let err = Job::prepare(&clients, typo).await.unwrap_err().to_string();
    assert!(err.starts_with("qualifier P407: 'Q18x60'"), "{err}");
}

#[tokio::test]
async fn quantities_with_unit_suffixes() {
    use crate::test_support::mock;
    use crate::value::DecimalMark;
    let (server, clients) = world().await;
    let unit = |id: &str| json_value(id);
    mock(&server, "ids=P2067&", serde_json::json!({"entities": {"P2067": {
        "id": "P2067", "datatype": "quantity",
        "claims": {"P2302": [{"mainsnak": unit("Q21514353"), "qualifiers": {"P2305": [unit("Q11570"), unit("Q41803"), unit("Q191118")]}}]}
    }}}))
    .await;
    let names = |id: &str, label: &str, symbol: &str, alias: &str| {
        serde_json::json!({"id": id, "labels": {"en": {"value": label}}, "aliases": {"en": [{"value": alias}]},
            "claims": {"P5061": [{"mainsnak": {"datavalue": {"value": {"text": symbol, "language": "en"}}}}]}})
    };
    mock(
        &server,
        "props=labels%7Caliases%7Cclaims",
        serde_json::json!({"entities": {
            "Q11570": names("Q11570", "kilogram", "kg", "kilo"),
            "Q41803": names("Q41803", "gram", "g", "gramme"),
            "Q191118": names("Q191118", "tonne", "t", "metric ton"),
            "Q999": names("Q999", "other", "x", "t"),
        }}),
    )
    .await;
    let spec = JobSpec {
        property: Some(PropertyId(2067)),
        template: "X".into(),
        parameters: vec!["mass".into()],
        unit: Some(ItemId(11_570)),
        decimal_mark: DecimalMark::Comma,
        ..Default::default()
    };
    let job = Job::prepare(&clients, spec).await.unwrap();
    let eval = |text: &'static str| {
        let (job, clients) = (&job, &clients);
        async move { evaluate(job, clients, &page(Some(1)), &revision(text)).await.result.map(|e| e.value) }
    };
    let qty = |amount: &str, unit: u64| Value::Quantity { amount: amount.into(), unit: Some(ItemId(unit)) };
    assert_eq!(eval("{{X|mass=82 g}}").await, Ok(qty("+82", 41_803)));
    assert_eq!(eval("{{X|mass=2,4 [[Kilogram|kg]]}}").await, Ok(qty("+2.4", 11_570)));
    assert_eq!(eval("{{X|mass=62}}").await, Ok(qty("+62", 11_570)), "no suffix: the chosen unit");
    assert_eq!(eval("{{X|mass=5 lb}}").await, Err(error("unknown unit 'lb'")));
    assert_eq!(eval("{{X|mass=5 t}}").await, Err(error("unknown unit 't'")), "ambiguous names are not guessed");
}

fn json_value(id: &str) -> Json {
    serde_json::json!({"snaktype": "value", "datavalue": {"type": "wikibase-entityid", "value": {"id": id}}})
}
