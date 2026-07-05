use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn update_block_patches_typed_body() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b1"))
        .and(body_json(json!({"to_do": {"checked": true}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "type": "to_do",
            "last_edited_time": "2026-07-05T11:00:00.000Z",
            "to_do": {"checked": true}
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let v = client.update_block("b1", "to_do", &json!({"checked": true})).await.unwrap();
    assert_eq!(v["last_edited_time"], "2026-07-05T11:00:00.000Z");
}

#[tokio::test]
async fn delete_block_sends_delete() {
    let server = MockServer::start().await;
    Mock::given(method("DELETE")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "archived": true
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let v = client.delete_block("b1").await.unwrap();
    assert_eq!(v["archived"], true);
}

#[tokio::test]
async fn append_children_posts_block_and_after() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/page1/children"))
        .and(body_json(json!({
            "children": [{"paragraph": {"rich_text": [{"type": "text", "text": {"content": "hi"}}]}}],
            "after": "b1"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "new-block-id", "type": "paragraph",
                         "last_edited_time": "2026-07-05T11:00:00.000Z",
                         "paragraph": {"rich_text": [{"plain_text": "hi"}]}}]
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let block = json!({"paragraph": {"rich_text": [{"type": "text", "text": {"content": "hi"}}]}});
    let v = client.append_children("page1", Some("b1"), block).await.unwrap();
    assert_eq!(v["results"][0]["id"], "new-block-id");
}

#[tokio::test]
async fn create_page_posts_parent_and_properties() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/pages"))
        .and(body_json(json!({
            "parent": {"data_source_id": "ds1"},
            "properties": {"Name": {"title": [{"text": {"content": "New task"}}]}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "real-page-id",
            "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let parent = json!({"data_source_id": "ds1"});
    let props = json!({"Name": {"title": [{"text": {"content": "New task"}}]}});
    let v = client.create_page(parent, props).await.unwrap();
    assert_eq!(v["id"], "real-page-id");
}

#[tokio::test]
async fn update_page_patches_body() {
    let server = MockServer::start().await;
    Mock::given(method("PATCH")).and(path("/v1/pages/p1"))
        .and(body_json(json!({"archived": true})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "archived": true,
            "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let v = client.update_page("p1", json!({"archived": true})).await.unwrap();
    assert_eq!(v["archived"], true);
}

#[tokio::test]
async fn get_block_edited_time_reads_timestamp() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "last_edited_time": "2026-07-05T12:00:00.000Z"
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let t = client.get_block_edited_time("b1").await.unwrap();
    assert_eq!(t, "2026-07-05T12:00:00.000Z");
}

#[tokio::test]
async fn get_page_edited_time_reads_timestamp() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T13:00:00.000Z"
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let t = client.get_page_edited_time("p1").await.unwrap();
    assert_eq!(t, "2026-07-05T13:00:00.000Z");
}
