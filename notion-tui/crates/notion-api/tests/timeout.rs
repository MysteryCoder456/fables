use notion_api::NotionClient;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn stalled_response_times_out_instead_of_hanging() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(120)))
        .mount(&server)
        .await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c.set_request_timeout(Duration::from_millis(200)); // test hook, mirrors set_timing
    let started = std::time::Instant::now();
    let res = c.get_json("/v1/users/me").await;
    assert!(res.is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
}
