use std::time::{Duration, Instant};

use notion_api::NotionClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(50), Duration::from_millis(10));
    c
}

#[tokio::test]
async fn retries_429_honoring_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let v = fast_client(server.uri()).get_json("/v1/x").await.unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn retries_5xx_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server)
        .await;

    let v = fast_client(server.uri()).get_json("/v1/x").await.unwrap();
    assert_eq!(v["ok"], true);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = fast_client(server.uri()).get_json("/v1/x").await.unwrap_err();
    assert!(matches!(err, notion_api::ApiError::RetriesExhausted(_)));
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test]
async fn paces_consecutive_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let c = fast_client(server.uri()); // min_interval = 50ms
    let start = Instant::now();
    c.get_json("/v1/x").await.unwrap();
    c.get_json("/v1/x").await.unwrap();
    c.get_json("/v1/x").await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_millis(100),
        "3 calls must span >= 2 intervals"
    );
}

#[tokio::test]
async fn non_idempotent_post_is_not_retried_on_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1) // exactly one attempt — no blind retry
        .mount(&server)
        .await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let err = c
        .post_json("/v1/pages", &serde_json::json!({}))
        .await
        .unwrap_err();
    assert!(matches!(err, notion_api::ApiError::Api { status: 500, .. }));
}

#[tokio::test]
async fn non_idempotent_post_still_retries_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "p1"})))
        .mount(&server)
        .await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let v = c.post_json("/v1/pages", &serde_json::json!({})).await.unwrap();
    assert_eq!(v["id"], "p1");
}

#[tokio::test]
async fn idempotent_post_wrapper_retries_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": []})))
        .mount(&server)
        .await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    assert!(c
        .post_json_idempotent("/v1/search", &serde_json::json!({}))
        .await
        .is_ok());
}
