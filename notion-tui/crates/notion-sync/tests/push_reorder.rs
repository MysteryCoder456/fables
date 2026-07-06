use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::push_once;
use wiremock::MockServer;

#[tokio::test]
async fn reorder_block_op_clears_without_any_http_call() {
    // No mocks registered at all — if the pusher made an HTTP request for this
    // op type, wiremock would return a 404 and the test would fail.
    let server = MockServer::start().await;
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));

    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[
        BlockRec { id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
            block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false },
    ]).unwrap();
    s.edit_reorder_block("b1", None, None, 0).unwrap(); // no-op position change forced below
    // Force a real position change so an op is actually enqueued:
    s.conn().execute("UPDATE blocks SET ordinal = 5 WHERE id = 'b1'", []).unwrap();
    s.conn().execute("UPDATE pages SET dirty = 1 WHERE id = 'p1'", []).unwrap();
    s.enqueue_op("reorder_block", "b1", &serde_json::json!({"page_id": "p1"}).to_string(), None).unwrap();

    let store = Arc::new(Mutex::new(s));
    let pushed = push_once(&client, &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
