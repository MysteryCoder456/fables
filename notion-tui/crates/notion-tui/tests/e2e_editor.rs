use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::pull_once;
use notion_tui::app::App;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn crawl_then_editor_round_trip_updates_and_inserts_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-06T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title", "title": [{"plain_text": "Notes"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph", "has_children": false,
                "paragraph": {"rich_text": [{"plain_text": "First"}]}}],
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
    pull_once(&client, &store).await.unwrap();

    let mut app = App::new(store.clone());
    app.refresh_sidebar();
    let node = app.sidebar.selected().cloned().unwrap();
    app.open_node(&node);

    assert!(!app.force_redraw);
    app.edit_in_editor(|initial| {
        assert_eq!(initial, "First");
        Ok(format!("{initial}, edited\nSecond paragraph"))
    });
    // The editor took over the screen, so the next frame must be a full repaint.
    assert!(app.force_redraw);

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 2);
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "First, edited");
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 2); // update_block + append_block
}
