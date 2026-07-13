use std::time::Duration;

use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn list_comments_parses_and_paginates() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .and(query_param("block_id", "p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "c1", "discussion_id": "d1",
                "created_time": "2026-07-06T10:00:00.000Z",
                "created_by": {"object": "user", "id": "u1"},
                "rich_text": [{"plain_text": "Nice page"}]
            }],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let comments = client(&server).list_comments("p1").await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, "c1");
    assert_eq!(comments[0].discussion_id, "d1");
    assert_eq!(comments[0].body, "Nice page");
    assert_eq!(comments[0].author, "u1");
}

#[tokio::test]
async fn create_comment_posts_parent_and_rich_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/comments"))
        .and(body_partial_json(json!({
            "parent": {"page_id": "p1"},
            "rich_text": [{"type": "text", "text": {"content": "hello"}}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "c9", "discussion_id": "d9"})))
        .mount(&server)
        .await;

    let v = client(&server)
        .create_comment(json!({"page_id": "p1"}), "hello")
        .await
        .unwrap();
    assert_eq!(v["id"], "c9");
}
