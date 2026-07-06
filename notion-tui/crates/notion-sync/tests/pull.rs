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

#[tokio::test]
async fn first_crawl_stores_pages_blocks_and_hwm() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            page_json("p1", "Newest", "2026-07-05T10:00:00.000Z"),
            page_json("p2", "Older", "2026-07-04T10:00:00.000Z"),
        ]))))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph",
                         "has_children": false,
                         "paragraph": {"rich_text": [{"plain_text": "hello world"}]}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());

    let updated = pull_once(&client, &store).await.unwrap();
    assert_eq!(updated, 2);

    let s = store.lock().unwrap();
    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "Newest");
    assert_eq!(s.page_blocks("p1").unwrap().len(), 1);
    assert_eq!(s.meta_get("hwm").unwrap().as_deref(), Some("2026-07-05T10:00:00.000Z"));
    assert_eq!(s.search("hello").unwrap()[0].page_id, "p1");
}

#[tokio::test]
async fn incremental_pull_skips_unchanged() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            page_json("p1", "Same", "2026-07-05T10:00:00.000Z"),
        ]))))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().meta_set("hwm", "2026-07-05T10:00:00.000Z").unwrap();

    let updated = pull_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(updated, 0);
    // Only the search request went out — no block fetches.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn data_source_pull_stores_schema_and_rows() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            {"object": "data_source", "id": "ds1",
             "last_edited_time": "2026-07-05T10:00:00.000Z",
             "parent": {"type": "database_id", "database_id": "db1"},
             "title": [{"plain_text": "Tasks"}]}
        ]))))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/data_sources/ds1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "data_source", "id": "ds1",
            "parent": {"type": "database_id", "database_id": "db1"},
            "last_edited_time": "2026-07-05T10:00:00.000Z",
            "title": [{"plain_text": "Tasks"}],
            "properties": {"Name": {"type": "title"}}
        })))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r1", "archived": false,
                         "last_edited_time": "2026-07-05T09:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {"Name": {"type": "title",
                                                 "title": [{"plain_text": "Buy milk"}]}}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    pull_once(&fast_client(server.uri()), &store).await.unwrap();

    let s = store.lock().unwrap();
    assert_eq!(s.get_data_source("ds1").unwrap().unwrap().title, "Tasks");
    assert_eq!(s.rows("ds1").unwrap().len(), 1);
}
