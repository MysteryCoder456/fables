use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::{pull_once, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn pull_skips_block_fetch_for_dirty_page() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title",
                                        "title": [{"plain_text": "Remote renamed"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;
    // No mock for GET /v1/blocks/p1/children — if the puller calls it, the test fails loudly.

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let mut s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Local title".into(),
            icon: None,
            archived: false,
            last_edited_time: "2026-07-04T00:00:00.000Z".into(),
        })
        .unwrap();
        s.replace_page_blocks(
            "p1",
            &[BlockRec {
                id: "b1".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 0,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "locally edited content".into(),
                has_children: false,
            }],
        )
        .unwrap();
        s.conn()
            .execute("UPDATE pages SET dirty = 1 WHERE id = 'p1'", [])
            .unwrap();
    }

    let (status_tx, _status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    let updated = pull_once(&fast_client(server.uri()), &store, &status_tx)
        .await
        .unwrap();
    assert_eq!(updated, 0);

    // Only the search request was made — no block-children fetch.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);

    let s = store.lock().unwrap();
    // Title untouched (upsert_page's own dirty guard) and blocks untouched (puller's dirty skip).
    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "Local title");
    let blocks = s.page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].plain_text, "locally edited content");
}
