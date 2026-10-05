use super::*;
use crate::ids::{ItemId, PropertyId};
use crate::test_support::test_store;

fn owner() -> Owner {
    Owner { id: 7, name: "Example User".into() }
}

fn pages(n: u64) -> Vec<Page> {
    (1..=n)
        .map(|i| Page {
            id: i,
            title: format!("Page {i} – ü"),
            item: (i % 2 == 0).then_some(ItemId(i)),
            latest_revision: i,
        })
        .collect()
}

#[tokio::test]
async fn runs_rows_and_shares() {
    let (_db, store) = test_store().await;
    store.migrate().await.unwrap(); // idempotent
    let spec = JobSpec { property: Some(PropertyId(345)), template: "IMDb title".into(), ..Default::default() };

    let id = store.create_run(&owner(), &spec, None).await.unwrap();
    let run = store.run(id).await.unwrap().unwrap();
    assert_eq!((run.status, run.spec.clone(), run.editgroup.len()), (RunStatus::Loading, spec.clone(), 12));

    store.add_rows(id, &pages(1200)).await.unwrap();
    store.set_excluded(id, &serde_json::json!({"no_item": 3})).await.unwrap();
    store.set_status(id, RunStatus::Editing, None).await.unwrap();
    let run = store.run(id).await.unwrap().unwrap();
    assert!(run.started.is_some() && run.finished.is_none());
    assert_eq!(run.excluded.unwrap()["no_item"], 3);
    assert_eq!(store.counts(id).await.unwrap().pending, 1200);

    let batch = store.rows_to_process(id, &[RowStatus::Pending], None, 50).await.unwrap();
    assert_eq!((batch.len(), batch[0].seq, batch[0].title.as_str()), (50, 0, "Page 1 – ü"));
    let update =
        RowUpdate { status: RowStatus::Done, item: Some("Q99"), raw_value: Some("r"), value: Some("v"), message: None };
    store.update_row(id, 0, &update).await.unwrap();
    let skipped =
        RowUpdate { status: RowStatus::Skipped, item: None, raw_value: None, value: None, message: Some("no value") };
    store.update_row(id, 1, &skipped).await.unwrap();
    let next = store.rows_to_process(id, &[RowStatus::Pending], Some(batch[49].seq), 50).await.unwrap();
    assert_eq!(next[0].seq, 50);
    let counts = store.counts(id).await.unwrap();
    assert_eq!((counts.done, counts.skipped, counts.pending, counts.total()), (1, 1, 1198, 1200));
    let done = store.rows(id, Some(RowStatus::Done), 0, 10).await.unwrap();
    assert_eq!(done[0].item.as_deref(), Some("Q99"));
    assert_eq!(store.rows(id, None, 1, 1).await.unwrap()[0].message.as_deref(), Some("no value"));

    store.recover_after_restart().await.unwrap();
    assert_eq!(store.run(id).await.unwrap().unwrap().status, RunStatus::Paused);
    store.set_status(id, RunStatus::Done, None).await.unwrap();
    assert!(store.run(id).await.unwrap().unwrap().finished.is_some());
    assert_eq!(store.runs_of(7, 10).await.unwrap().len(), 1);
    assert!(store.runs_of(8, 10).await.unwrap().is_empty());

    let share = store.create_share(&owner(), "IMDb from enwiki", &spec).await.unwrap();
    store.set_tags(share, &["films".into(), "imdb".into()]).await.unwrap();
    store.set_tags(share, &["imdb".into(), "enwiki".into()]).await.unwrap();
    assert_eq!(store.share(share).await.unwrap().unwrap().tags, ["enwiki", "imdb"]);
    assert_eq!(store.shares().await.unwrap()[0].tags, ["enwiki", "imdb"]);
    store.record_share_run(share, id, &counts).await.unwrap();
    let s = store.share(share).await.unwrap().unwrap();
    assert_eq!((s.title.as_str(), s.last_done, s.spec), ("IMDb from enwiki", Some(1), spec));
    assert!(!store.delete_share(share, 8).await.unwrap(), "only the owner may delete");
    assert!(store.delete_share(share, 7).await.unwrap());
    assert!(store.shares().await.unwrap().is_empty());
}
