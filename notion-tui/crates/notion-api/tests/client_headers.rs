use notion_api::NotionClient;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn sends_auth_and_version_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .and(header("Authorization", "Bearer test-token"))
        .and(header("Notion-Version", "2025-09-03"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"object": "user"})))
        .expect(1)
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("test-token", server.uri());
    let v = client.get_json("/v1/users/me").await.unwrap();
    assert_eq!(v["object"], "user");
}

#[tokio::test]
async fn api_error_is_parsed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            serde_json::json!({"object": "error", "code": "object_not_found", "message": "Not found"}),
        ))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());
    let err = client.get_json("/v1/users/me").await.unwrap_err();
    match err {
        notion_api::ApiError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(status, 404);
            assert_eq!(code, "object_not_found");
            assert_eq!(message, "Not found");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}
