use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::push_once;
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn conflicted_push_surfaces_in_queue_and_keep_mine_pushes_through() {
    let server = MockServer::start().await;
    // Remote page has moved past our base time -> conflict on first push.
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "p1", "last_edited_time": "2026-07-06T12:00:00.000Z"})))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "b1"})))
        .mount(&server)
        .await;

    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "2026-07-06T10:00:00.000Z".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
    s.edit_update_block_text("b1", "local edit").unwrap();
    let store = Arc::new(Mutex::new(s));

    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));

    // First push: conflict detected, op marked conflicted, nothing pushed.
    assert_eq!(push_once(&client, &store).await.unwrap(), 0);
    assert_eq!(store.lock().unwrap().ops().unwrap()[0].state, "conflicted");

    // Queue screen shows it; keep-mine re-pends it without a base.
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    app.refresh_conflicted();
    assert_eq!(app.conflicted, 1);
    dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT));
    assert!(matches!(app.view, View::Queue(_)));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));

    // Second push: base is gone, PATCH succeeds, queue drains.
    assert_eq!(push_once(&client, &store).await.unwrap(), 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
