use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::{pull_once, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn search_body(results: serde_json::Value) -> serde_json::Value {
    json!({"results": results, "has_more": false, "next_cursor": null})
}

fn page_json(id: &str, title: &str, edited: &str) -> serde_json::Value {
    json!({"object": "page", "id": id, "archived": false, "last_edited_time": edited,
           "parent": {"type": "workspace", "workspace": true},
           "properties": {"Name": {"type": "title", "title": [{"plain_text": title}]}}})
}

fn empty_children() -> serde_json::Value {
    json!({"results": [], "has_more": false, "next_cursor": null})
}

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

/// Mounts a single-page search result plus its (empty) block tree and comments,
/// and returns a ready-to-use mock server, client, and in-memory store.
async fn pull_fixture_one_page(edited: &str) -> (MockServer, NotionClient, SharedStore) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(search_body(json!([page_json("p1", "Newest", edited),]))),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());
    (server, client, store)
}

#[tokio::test]
async fn first_crawl_stores_pages_blocks_and_hwm() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            page_json("p1", "Newest", "2026-07-05T10:00:00.000Z"),
            page_json("p2", "Older", "2026-07-04T10:00:00.000Z"),
        ]))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph",
                         "has_children": false,
                         "paragraph": {"rich_text": [{"plain_text": "hello world"}]}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());

    let (status_tx, _status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    let updated = pull_once(&client, &store, &status_tx).await.unwrap();
    assert_eq!(updated, 2);

    let s = store.lock().unwrap();
    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "Newest");
    assert_eq!(s.page_blocks("p1").unwrap().len(), 1);
    assert_eq!(
        s.meta_get("hwm").unwrap().as_deref(),
        Some("2026-07-05T10:00:00.000Z")
    );
    assert_eq!(s.search("hello").unwrap()[0].page_id, "p1");
}

#[tokio::test]
async fn incremental_pull_skips_unchanged() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(search_body(json!([page_json(
                "p1",
                "Same",
                "2026-07-05T10:00:00.000Z"
            ),]))),
        )
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .meta_set("hwm", "2026-07-05T10:00:00.000Z")
        .unwrap();

    let (status_tx, _status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    let updated = pull_once(&fast_client(server.uri()), &store, &status_tx)
        .await
        .unwrap();
    assert_eq!(updated, 0);
    // Only the search request went out — no block fetches.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn data_source_pull_stores_schema_and_rows() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            {"object": "data_source", "id": "ds1",
             "last_edited_time": "2026-07-05T10:00:00.000Z",
             "parent": {"type": "database_id", "database_id": "db1"},
             "title": [{"plain_text": "Tasks"}]}
        ]))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/data_sources/ds1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "data_source", "id": "ds1",
            "parent": {"type": "database_id", "database_id": "db1"},
            "last_edited_time": "2026-07-05T10:00:00.000Z",
            "title": [{"plain_text": "Tasks"}],
            "properties": {"Name": {"type": "title"}}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r1", "archived": false,
                         "last_edited_time": "2026-07-05T09:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {"Name": {"type": "title",
                                                 "title": [{"plain_text": "Buy milk"}]}}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let (status_tx, _status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    pull_once(&fast_client(server.uri()), &store, &status_tx)
        .await
        .unwrap();

    let s = store.lock().unwrap();
    assert_eq!(s.get_data_source("ds1").unwrap().unwrap().title, "Tasks");
    assert_eq!(s.rows("ds1").unwrap().len(), 1);
}

#[tokio::test]
async fn store_write_failure_fails_the_cycle_and_preserves_hwm() {
    let (server, client, store) = pull_fixture_one_page("2026-01-02T00:00:00.000Z").await;
    store
        .lock()
        .unwrap()
        .conn()
        .execute_batch("DROP TABLE pages;")
        .unwrap();

    let (status_tx, _status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    let res = notion_sync::pull_once(&client, &store, &status_tx).await;
    assert!(res.is_err(), "store failure must fail the pull cycle");
    // hwm must NOT have advanced past the failed item.
    let hwm = store.lock().unwrap().meta_get("hwm").unwrap().unwrap_or_default();
    assert_eq!(hwm, "");
    let _ = server;
}

/// Matches a request body that does NOT contain `start_cursor` — i.e. the very first
/// search request of a crawl. `serde_json`'s default (non-`preserve_order`) map serializes
/// object keys alphabetically, so a literal `"page_size":100}` substring match isn't reliable
/// across serde_json versions; matching on the *absence* of `start_cursor` is.
struct NoStartCursor;
impl wiremock::Match for NoStartCursor {
    fn matches(&self, request: &wiremock::Request) -> bool {
        !String::from_utf8_lossy(&request.body).contains("start_cursor")
    }
}

#[tokio::test]
async fn interrupted_first_crawl_resumes_from_the_checkpoint_instead_of_restarting() {
    let server = MockServer::start().await;
    // Page 1: p1, followed by cursor "c2".
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(NoStartCursor)
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_json("p1", "First", "2026-07-05T10:00:00.000Z")],
            "has_more": true, "next_cursor": "c2"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results": [], "has_more": false, "next_cursor": null})),
        )
        .mount(&server)
        .await;
    // Page 2 (cursor "c2") fails the first time.
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(wiremock::matchers::body_string_contains("c2"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());
    let (status_tx, status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);

    let err = notion_sync::pull_once(&client, &store, &status_tx).await;
    assert!(err.is_err(), "the second page's 500 must fail this pass");
    assert_eq!(
        store.lock().unwrap().meta_get("pull_cursor").unwrap().as_deref(),
        Some("c2")
    );
    assert_eq!(
        store.lock().unwrap().get_page("p1").unwrap().unwrap().title,
        "First"
    );
    match &*status_rx.borrow() {
        notion_sync::SyncStatus::Syncing { done: 1, total } => assert!(*total >= 1),
        other => panic!("expected a Syncing progress update, got {other:?}"),
    }

    // Second attempt: page 2 now succeeds and is the last page.
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(wiremock::matchers::body_string_contains("c2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_json("p2", "Second", "2026-07-04T10:00:00.000Z")],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server)
        .await;

    let updated = notion_sync::pull_once(&client, &store, &status_tx).await.unwrap();
    assert_eq!(
        updated, 1,
        "only p2 should be (re-)processed — p1 must not be re-fetched"
    );
    assert_eq!(
        store.lock().unwrap().meta_get("hwm").unwrap().as_deref(),
        Some("2026-07-05T10:00:00.000Z")
    );
    assert_eq!(
        store.lock().unwrap().meta_get("pull_cursor").unwrap(),
        None,
        "checkpoint clears on success"
    );
    // The page-1 search mock's `.expect(1)` (asserted on drop) proves it wasn't re-hit.
}

#[tokio::test]
async fn mid_batch_failure_then_resume_does_not_double_count_progress() {
    let server = MockServer::start().await;
    // A single batch of two pages: p1 succeeds, p2's block fetch fails the first time.
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            page_json("p1", "First", "2026-07-05T10:00:00.000Z"),
            page_json("p2", "Second", "2026-07-04T10:00:00.000Z"),
        ]))))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results": [], "has_more": false, "next_cursor": null})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());
    let (status_tx, status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);

    let err = notion_sync::pull_once(&client, &store, &status_tx).await;
    assert!(err.is_err(), "p2's block-fetch 500 must fail this pass");
    // The batch never completed, so no done checkpoint may be persisted:
    // resuming restarts the batch and would otherwise re-count p1.
    assert_eq!(store.lock().unwrap().meta_get("pull_done").unwrap(), None);

    // Second attempt: p2's block fetch now succeeds.
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server)
        .await;

    let updated = notion_sync::pull_once(&client, &store, &status_tx).await.unwrap();
    assert_eq!(updated, 2);
    match &*status_rx.borrow() {
        notion_sync::SyncStatus::Syncing { done, .. } => {
            assert_eq!(*done, 2, "progress must equal real item count, not inflated")
        }
        other => panic!("expected a Syncing progress update, got {other:?}"),
    }
    assert_eq!(
        store.lock().unwrap().meta_get("pull_done").unwrap(),
        None,
        "done checkpoint clears on success"
    );
}
