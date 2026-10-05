use super::*;
use crate::auth::{OAuth, Token};
use crate::config::{OauthConfig, Secret};
use crate::harvest::JobSpec;
use crate::ids::PropertyId;
use crate::storage::Owner;
use crate::test_support::{mock, test_app, test_store, world};
use crate::value::TransformSpec;
use serde_json::json;

const OWNER: u64 = 7;

fn spec() -> JobSpec {
    JobSpec {
        property: Some(PropertyId(345)),
        template: "IMDb title".into(),
        parameters: vec!["1".into()],
        transform: TransformSpec {
            add_prefix: "tt".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn editor(url: &str) -> Editor {
    let config = OauthConfig {
        consumer_key: "ck".into(),
        consumer_secret: Secret::from("cs"),
        callback_url: "x".into(),
    };
    let token = Token {
        key: "k".into(),
        secret: Secret::from("s"),
    };
    Editor::new(OAuth::new(reqwest::Client::new(), &config), url.to_string(), token)
}

#[tokio::test]
async fn load_preview_and_edit() {
    let (server, _) = world().await;
    let page = |id: u64, title: &str, item: Option<&str>| json!({"pageid": id, "ns": 0, "title": title, "lastrevid": id * 10, "pageprops": item.map(|q| json!({"wikibase_item": q}))});
    mock(
        &server,
        "generator=transcludedin",
        json!({"query": {"pages": [
            page(1, "Shawshank", Some("Q1")), page(2, "Already set", Some("Q2")), page(3, "No item", None)
        ]}}),
    )
    .await;
    mock(
        &server,
        "p%3AP345",
        json!({"results": {"bindings": [
            {"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q2"}}
        ]}}),
    )
    .await;
    mock(
        &server,
        "prop=revisions",
        json!({"query": {"pages": [
            {"pageid": 1, "revisions": [{"revid": 11, "slots": {"main": {"content": "{{IMDb title|0111161}}"}}}]}
        ]}}),
    )
    .await;
    mock(
        &server,
        "meta=tokens",
        json!({"query": {"tokens": {"csrftoken": "t+\\"}}}),
    )
    .await;
    mock(&server, "action=wbeditentity", json!({"success": 1})).await;

    let (_db, store) = test_store().await;
    let sessions = tempfile::tempdir().unwrap();
    let app = test_app(store, &server.uri(), sessions.path());
    let owner = Owner {
        id: OWNER,
        name: "Tester".into(),
    };
    let run_id = app.store.create_run(&owner, &spec(), None).await.unwrap();
    let prepare = || Job::prepare(&app.clients, spec());

    let claim = app.runs.claim(run_id, OWNER, 2).unwrap();
    load(app.clone(), run_id, prepare().await.unwrap(), claim).await;
    let run = app.store.run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Ready, "{:?}", run.message);
    assert_eq!(
        run.excluded.clone().unwrap(),
        json!({"not_in_category": 0, "not_in_list": 0, "no_item": 1, "already_set": 1})
    );
    assert_eq!(app.store.counts(run_id).await.unwrap().pending, 1);

    let claim = app.runs.claim(run_id, OWNER, 2).unwrap();
    Worker::new(app.clone(), run.clone(), prepare().await.unwrap(), Mode::Preview, claim)
        .run()
        .await;
    let row = &app.store.rows(run_id, None, 0, 10).await.unwrap()[0];
    assert_eq!(
        (row.status, row.value.as_deref()),
        (RowStatus::Ready, Some("tt0111161"))
    );
    assert_eq!(app.store.run(run_id).await.unwrap().unwrap().status, RunStatus::Ready);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| !String::from_utf8_lossy(&r.body).contains("wbeditentity"))
    );

    let claim = app.runs.claim(run_id, OWNER, 2).unwrap();
    let mode = Mode::Edit(Box::new(editor(&server.uri())));
    Worker::new(app.clone(), run, prepare().await.unwrap(), mode, claim)
        .run()
        .await;
    let row = &app.store.rows(run_id, None, 0, 10).await.unwrap()[0];
    assert_eq!((row.status, row.item.as_deref()), (RowStatus::Done, Some("Q1")));
    let run = app.store.run(run_id).await.unwrap().unwrap();
    assert_eq!(run.status, RunStatus::Done);
    assert!(!app.runs.is_active(run_id), "the claim is released");

    let edits: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .filter(|b| b.contains("wbeditentity"))
        .collect();
    assert_eq!(edits.len(), 1);
    let summary = urlencoding::decode(&edits[0].replace('+', " ")).unwrap().into_owned();
    assert!(
        summary.contains(&format!("editgroups/b/harvesttemplates/{}", run.editgroup)),
        "{summary}"
    );
}
