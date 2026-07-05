use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::{pull_once, push_once};
use notion_tui::app::{dispatch_key, App};
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn crawl_edit_queue_push_smoke() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title",
                                        "title": [{"plain_text": "Groceries"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "to_do",
                "has_children": false,
                "to_do": {"rich_text": [{"plain_text": "Buy milk"}], "checked": false}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));

    // 1. Crawl (read-only sync, as in M1).
    pull_once(&client, &store).await.unwrap();

    // 2. Open the page in the TUI, exactly as a user browsing would.
    let mut app = App::new(store.clone());
    app.refresh_sidebar();
    let node = app.sidebar.selected().cloned().unwrap();
    app.open_node(&node);

    // 3. Toggle the to-do — an optimistic local write that queues a pending_ops row.
    app.focus = notion_tui::app::Focus::Main;
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char(' ')));

    {
        let s = store.lock().unwrap();
        let blocks = s.page_blocks("p1").unwrap();
        let payload: serde_json::Value = serde_json::from_str(&blocks[0].payload).unwrap();
        assert_eq!(payload["checked"], true);
        assert_eq!(s.ops().unwrap().len(), 1);
        assert!(s.is_page_dirty("p1").unwrap());
    }

    // The UI must already reflect the optimistic edit before anything is pushed. In the real
    // app main.rs updates this from the sync loop's `pending` watch channel; simulate that here.
    app.pending = store.lock().unwrap().pending_count().unwrap();
    let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
    term.draw(|f| ui::draw(f, &app)).unwrap();
    let rendered = format!("{:?}", term.backend().buffer());
    assert!(rendered.contains("[x] Buy milk"));
    assert!(rendered.contains("1 pending"));

    // 4. The pusher drains the queue against the real API surface.
    let pushed = push_once(&client, &store).await.unwrap();
    assert_eq!(pushed, 1);

    let s = store.lock().unwrap();
    assert!(s.ops().unwrap().is_empty());
    assert!(!s.is_page_dirty("p1").unwrap());
}
