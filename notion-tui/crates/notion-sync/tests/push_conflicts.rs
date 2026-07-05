use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::{push_once, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

fn page_store_with_todo() -> SharedStore {
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
    Arc::new(Mutex::new(s))
}

#[tokio::test]
async fn remote_change_since_base_marks_conflicted_and_blocks_same_target() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();
    // A second edit queued against the same block before the first has pushed.
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T12:00:00.000Z"
        })))
        .mount(&server).await;
    // No PATCH mock: if the pusher tries to push past the conflict check, this test fails loudly.

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 0);

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].state, "conflicted");
    assert_eq!(ops[1].state, "pending"); // never attempted: target was already blocked

    // Only one GET for the conflict check on the first op; the second op was skipped entirely.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(store.lock().unwrap().is_page_dirty("p1").unwrap());
}

#[tokio::test]
async fn api_rejection_marks_op_failed_with_reason() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "validation_error", "message": "bad request"
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 0);

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].state, "failed");
    assert!(ops[0].error.as_deref().unwrap().contains("bad request"));
    // Failed ops aren't silently dropped from the queue.
    assert!(store.lock().unwrap().is_page_dirty("p1").unwrap());
}

#[tokio::test]
async fn network_failure_aborts_pass_leaving_ops_pending() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();

    // Nothing listens on port 1 (privileged, refused for a non-root client): the request
    // errors at the transport layer.
    let result = push_once(&fast_client("http://127.0.0.1:1".to_string()), &store).await;
    assert!(result.is_err());

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].state, "pending");
}

#[tokio::test]
async fn conflict_on_one_target_does_not_block_another() {
    let store = page_store_with_todo();
    store.lock().unwrap().replace_page_blocks(
        "p1",
        &[
            BlockRec {
                id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
                block_type: "to_do".into(), payload: r#"{"checked": false}"#.into(),
                plain_text: "Buy milk".into(), has_children: false,
            },
            BlockRec {
                id: "b2".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 1,
                block_type: "to_do".into(), payload: r#"{"checked": false}"#.into(),
                plain_text: "Walk dog".into(), has_children: false,
            },
        ],
    ).unwrap();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();
    store.lock().unwrap().edit_toggle_todo("b2").unwrap();

    let server = MockServer::start().await;
    // Both ops share the same page, so both conflict-check against p1's edited time.
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({
            "code": "validation_error", "message": "bad"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b2", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].target_id, "b1");
    assert_eq!(ops[0].state, "failed");
}
