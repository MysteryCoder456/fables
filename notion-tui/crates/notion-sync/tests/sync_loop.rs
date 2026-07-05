use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::{spawn_sync, SyncStatus};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn spawn_sync_pushes_then_pulls_and_reports_pending() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "2026-07-05T10:00:00.000Z".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "to_do".into(),
            payload: r#"{"checked": false}"#.into(),
            plain_text: "Buy milk".into(),
            has_children: false,
        }],
    )
    .unwrap();
    s.edit_toggle_todo("b1").unwrap();
    let store = Arc::new(Mutex::new(s));

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;

    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let mut handle = spawn_sync(client, store.clone(), Duration::from_millis(50));

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(*handle.status.borrow(), SyncStatus::Idle { .. }) {
                break;
            }
            handle.status.changed().await.unwrap();
        }
    })
    .await
    .expect("sync loop never reached Idle");

    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
    assert_eq!(*handle.pending.borrow(), 0);
}
