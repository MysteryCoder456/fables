use notion_api::{NotionClient, SearchItem};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn page_fixture(id: &str, title: &str, edited: &str) -> serde_json::Value {
    json!({
        "object": "page",
        "id": id,
        "archived": false,
        "last_edited_time": edited,
        "parent": {"type": "workspace", "workspace": true},
        "icon": {"type": "emoji", "emoji": "📄"},
        "properties": {
            "Name": {"id": "title", "type": "title",
                     "title": [{"type": "text", "plain_text": title}]}
        }
    })
}

#[tokio::test]
async fn search_paginates_and_parses() {
    let server = MockServer::start().await;
    // First page: sorted request, no cursor.
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(body_partial_json(json!({
            "sort": {"timestamp": "last_edited_time", "direction": "descending"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_fixture("p1", "First", "2026-07-05T10:00:00.000Z")],
            "has_more": true,
            "next_cursor": "c2"
        })))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Second page: with cursor c2, contains a data source.
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .and(body_partial_json(json!({"start_cursor": "c2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "object": "data_source",
                "id": "ds1",
                "last_edited_time": "2026-07-04T09:00:00.000Z",
                "parent": {"type": "database_id", "database_id": "db1"},
                "title": [{"type": "text", "plain_text": "Tasks"}]
            }],
            "has_more": false,
            "next_cursor": null
        })))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());

    let page1 = client.search_page(None).await.unwrap();
    assert_eq!(page1.next_cursor.as_deref(), Some("c2"));
    match &page1.items[0] {
        SearchItem::Page(p) => {
            assert_eq!(p.id, "p1");
            assert_eq!(p.title, "First");
            assert_eq!(p.icon.as_deref(), Some("📄"));
            assert_eq!(p.last_edited_time, "2026-07-05T10:00:00.000Z");
        }
        other => panic!("expected page, got {other:?}"),
    }

    let page2 = client.search_page(Some("c2")).await.unwrap();
    assert!(page2.next_cursor.is_none());
    match &page2.items[0] {
        SearchItem::DataSource(ds) => {
            assert_eq!(ds.id, "ds1");
            assert_eq!(ds.database_id, "db1");
            assert_eq!(ds.title, "Tasks");
        }
        other => panic!("expected data source, got {other:?}"),
    }
}
