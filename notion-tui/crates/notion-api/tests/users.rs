use std::time::Duration;

use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn me_returns_bot_name() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "user", "id": "u1", "name": "My Integration", "type": "bot"})))
        .mount(&server).await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    assert_eq!(c.me().await.unwrap(), "My Integration");
}
