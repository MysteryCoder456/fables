use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::{pull_once, SyncStatus};
use notion_tui::app::App;
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use tokio::sync::watch;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn crawl_then_browse_smoke() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title",
                                        "title": [{"plain_text": "Hello Page"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph",
                "has_children": false,
                "paragraph": {"rich_text": [{"plain_text": "hello world"}]}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let (status_tx, _status_rx) = watch::channel(SyncStatus::Starting);
    pull_once(&client, &store, &status_tx).await.unwrap();

    let mut app = App::new(store);
    app.refresh_sidebar();
    let node = app.sidebar.selected().cloned().unwrap();
    app.open_node(&node);

    let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let rendered = format!("{:?}", term.backend().buffer());
    assert!(rendered.contains("Hello Page"));
    assert!(rendered.contains("hello world"));
}
