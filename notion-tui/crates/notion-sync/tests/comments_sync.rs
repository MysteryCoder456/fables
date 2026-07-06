use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::{pull_once, push_once};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn pull_stores_page_comments_for_changed_pages() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-06T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title", "title": [{"plain_text": "Notes"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"id": "c1", "discussion_id": "d1",
                "created_time": "2026-07-06T09:00:00.000Z",
                "created_by": {"id": "u1"},
                "rich_text": [{"plain_text": "remote comment"}]}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    pull_once(&client(&server), &store).await.unwrap();

    let comments = store.lock().unwrap().comments_for("p1").unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "remote comment");
    assert_eq!(comments[0].parent_kind, "page");
}

#[tokio::test]
async fn push_create_comment_calls_api_and_rewrites_temp_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/comments"))
        .and(body_partial_json(json!({"parent": {"page_id": "p1"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "real-c1", "discussion_id": "d1"})))
        .mount(&server).await;

    let mut s = Store::open_in_memory().unwrap();
    let tmp = s.edit_add_comment("p1", "page", None, "hello").unwrap();
    let store = Arc::new(Mutex::new(s));

    let pushed = push_once(&client(&server), &store).await.unwrap();
    assert_eq!(pushed, 1);
    let s = store.lock().unwrap();
    assert!(s.ops().unwrap().is_empty());
    assert_eq!(s.comments_for("p1").unwrap()[0].id, "real-c1");
    assert_ne!(tmp, "real-c1");
}
