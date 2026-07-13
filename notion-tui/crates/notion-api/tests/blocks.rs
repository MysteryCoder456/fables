use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn fetches_nested_block_tree() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/page1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"object": "block", "id": "b1", "type": "heading_1", "has_children": false,
                 "heading_1": {"rich_text": [{"plain_text": "Title"}]}},
                {"object": "block", "id": "b2", "type": "toggle", "has_children": true,
                 "toggle": {"rich_text": [{"plain_text": "More"}]}},
                {"object": "block", "id": "b3", "type": "child_page", "has_children": true,
                 "child_page": {"title": "Sub page"}}
            ],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/b2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"object": "block", "id": "b21", "type": "paragraph", "has_children": false,
                 "paragraph": {"rich_text": [{"plain_text": "hidden text"}]}}
            ],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());
    let flat = client.fetch_block_tree("page1").await.unwrap();

    assert_eq!(flat.len(), 4);
    assert_eq!(flat[0].block.id, "b1");
    assert_eq!(flat[0].block.plain_text, "Title");
    assert_eq!(flat[0].parent_block_id, None);
    assert_eq!(flat[0].ordinal, 0);

    let b21 = flat.iter().find(|f| f.block.id == "b21").unwrap();
    assert_eq!(b21.parent_block_id.as_deref(), Some("b2"));
    assert_eq!(b21.block.plain_text, "hidden text");

    // child_page has_children but must NOT be recursed into
    assert!(flat.iter().all(|f| f.parent_block_id.as_deref() != Some("b3")));
    let b3 = flat.iter().find(|f| f.block.id == "b3").unwrap();
    assert_eq!(b3.block.plain_text, "Sub page");
}

#[tokio::test]
async fn queries_data_source_with_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/data_sources/ds1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "data_source", "id": "ds1",
            "parent": {"type": "database_id", "database_id": "db1"},
            "last_edited_time": "2026-07-01T00:00:00.000Z",
            "title": [{"plain_text": "Tasks"}],
            "properties": {"Name": {"id": "title", "type": "title"},
                           "Done": {"id": "d1", "type": "checkbox"}}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/data_sources/ds1/query"))
        .and(body_partial_json(json!({"start_cursor": "c2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r2", "archived": false,
                         "last_edited_time": "2026-07-01T00:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r1", "archived": false,
                         "last_edited_time": "2026-07-02T00:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {}}],
            "has_more": true, "next_cursor": "c2"
        })))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());

    let ds = client.get_data_source("ds1").await.unwrap();
    assert_eq!(ds.meta.title, "Tasks");
    assert_eq!(ds.schema["Done"]["type"], "checkbox");

    let rows = client.query_data_source_all("ds1").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "r1");
    assert_eq!(rows[1].id, "r2");
}
