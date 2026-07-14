use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{PageRec, Store};
use notion_sync::{reconcile_deletions, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn reconcile_deletions_removes_a_page_absent_from_a_full_crawl() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title", "title": [{"plain_text": "Still here"}]}}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Still here".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        })
        .unwrap();
        s.upsert_page(&PageRec {
            id: "trashed".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Trashed elsewhere".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        })
        .unwrap();
    }

    let (removed_pages, _) = reconcile_deletions(&fast_client(server.uri()), &store)
        .await
        .unwrap();
    assert_eq!(removed_pages, 1);
    assert!(store.lock().unwrap().get_page("p1").unwrap().is_some());
    assert!(store.lock().unwrap().get_page("trashed").unwrap().is_none());
}

/// A successful crawl that comes back with zero pages and zero data sources (an
/// index-lag blip, a permission hiccup that still 200s, or any other anomaly short
/// of an outright transport error) must not be treated as "the workspace is empty" —
/// it must not wipe out a non-empty local store.
#[tokio::test]
async fn reconcile_deletions_skips_pruning_on_an_anomalous_empty_crawl() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Still here".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        })
        .unwrap();
    }

    let (removed_pages, removed_ds) = reconcile_deletions(&fast_client(server.uri()), &store)
        .await
        .unwrap();
    assert_eq!(removed_pages, 0);
    assert_eq!(removed_ds, 0);
    assert!(
        store.lock().unwrap().get_page("p1").unwrap().is_some(),
        "an anomalous empty crawl must not delete the local cache"
    );
}
