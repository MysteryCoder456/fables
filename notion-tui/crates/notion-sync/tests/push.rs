use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec, Store};
use notion_sync::{push_once, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

fn page_store_with_todo() -> SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "2026-07-05T10:00:00.000Z".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "to_do".into(),
            payload: r#"{"checked": false}"#.into(),
            plain_text: "Buy milk".into(),
            has_children: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

#[tokio::test]
async fn push_toggle_todo_updates_remote_and_clears_dirty() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();
    assert!(store.lock().unwrap().is_page_dirty("p1").unwrap());

    let server = MockServer::start().await;
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

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}

#[tokio::test]
async fn push_insert_block_rewrites_temp_id() {
    let store = page_store_with_todo();
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_insert_block_after("p1", Some("b1"), "paragraph", "New block")
        .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "real-b2",
                         "last_edited_time": "2026-07-05T11:00:00.000Z"}]
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert!(blocks.iter().any(|b| b.id == "real-b2"));
    assert!(!blocks.iter().any(|b| b.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}

#[tokio::test]
async fn push_delete_block_removes_remote_and_clears_dirty() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_delete_block("b1").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("DELETE")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "block", "id": "b1", "archived": true
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}

fn ds_store_with_row() -> SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds1",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: json!({"Name": "old"}).to_string(),
            last_edited_time: "2026-07-05T10:00:00.000Z".into(),
            archived: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

#[tokio::test]
async fn push_update_row_patches_remote_and_clears_dirty() {
    let store = ds_store_with_row();
    store.lock().unwrap().edit_update_row("r1", json!({"Name": "new"})).unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/r1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "r1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/pages/r1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "r1", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_row_dirty("r1").unwrap());
}

#[tokio::test]
async fn push_create_row_rewrites_temp_id() {
    let store = ds_store_with_row();
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_create_row("ds1", json!({"Name": "brand new"}))
        .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "real-r2", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    assert!(rows.iter().any(|r| r.id == "real-r2"));
    assert!(!rows.iter().any(|r| r.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

#[tokio::test]
async fn push_delete_row_archives_remote_and_clears_dirty() {
    let store = ds_store_with_row();
    store.lock().unwrap().edit_delete_row("r1").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/r1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "r1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/pages/r1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "r1", "archived": true,
            "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_row_dirty("r1").unwrap());
}
