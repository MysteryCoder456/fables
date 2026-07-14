use notion_api::NotionClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_page_returns_metadata_including_archived() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "page", "id": "p1", "archived": true,
            "parent": {"type": "workspace", "workspace": true},
            "last_edited_time": "2026-07-05T10:00:00.000Z",
            "properties": {"title": {"type": "title", "title": [{"plain_text": "Gone"}]}}
        })))
        .mount(&server)
        .await;
    let client = NotionClient::with_base_url("t", server.uri());

    let meta = client.get_page("p1").await.unwrap();
    assert!(meta.archived);
    assert_eq!(meta.title, "Gone");
}

#[tokio::test]
async fn get_page_surfaces_404_as_an_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/gone"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "code": "object_not_found", "message": "not found"
        })))
        .mount(&server)
        .await;
    let client = NotionClient::with_base_url("t", server.uri());

    let err = client.get_page("gone").await.unwrap_err();
    assert!(matches!(err, notion_api::ApiError::Api { status: 404, .. }));
}
