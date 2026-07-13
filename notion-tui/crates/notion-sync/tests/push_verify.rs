use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, CommentRec, DataSourceRec, PageRec, RowRec, Store};
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

/// Enqueues an `append_block` op via `edit_insert_block_after` and forces its state to
/// `inflight`, simulating a prior pass that sent the append but died before seeing the response.
fn append_op_left_inflight(store: &SharedStore) -> String {
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_insert_block_after("p1", Some("b1"), "paragraph", "New block")
        .unwrap();
    let seq = store
        .lock()
        .unwrap()
        .ops()
        .unwrap()
        .into_iter()
        .find(|o| o.target_id == tmp_id)
        .unwrap()
        .seq;
    store.lock().unwrap().set_op_state(seq, "inflight", None).unwrap();
    tmp_id
}

/// Forces the pending op targeting `target_id` to `inflight`, simulating a prior pass that
/// sent the create but died before seeing the response.
fn force_op_inflight(store: &SharedStore, target_id: &str) {
    let seq = store
        .lock()
        .unwrap()
        .ops()
        .unwrap()
        .into_iter()
        .find(|o| o.target_id == target_id)
        .unwrap()
        .seq;
    store.lock().unwrap().set_op_state(seq, "inflight", None).unwrap();
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
            properties: json!({"Name": {"type": "title", "title": [{"plain_text": "Existing"}]}}).to_string(),
            last_edited_time: "2026-07-05T10:00:00.000Z".into(),
            archived: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn title_props(title: &str) -> serde_json::Value {
    json!({"Name": {"type": "title", "title": [{"plain_text": title}]}})
}

// Scenario 1: the append reached Notion but the response was lost. A prior pass left the op
// `inflight`. The children listing already contains the block. push_once must adopt the
// remote id and NOT re-append (the append mock has .expect(0)).
#[tokio::test]
async fn inflight_append_is_adopted_not_resent_when_found_remotely() {
    let store = page_store_with_todo();
    let tmp_id = append_op_left_inflight(&store);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "object": "block",
                "id": "real-b2",
                "type": "paragraph",
                "paragraph": {"rich_text": [{"plain_text": "New block"}]},
                "has_children": false
            }],
            "has_more": false,
            "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert!(blocks.iter().any(|b| b.id == "real-b2"));
    assert!(!blocks.iter().any(|b| b.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}

// Scenario 2: op is `inflight` but the children listing does NOT contain the block. push_once
// must resend (append mock .expect(1)) and succeed.
#[tokio::test]
async fn inflight_append_is_resent_when_not_found_remotely() {
    let store = page_store_with_todo();
    let tmp_id = append_op_left_inflight(&store);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [],
            "has_more": false,
            "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "real-b2",
                         "last_edited_time": "2026-07-05T11:00:00.000Z"}]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert!(blocks.iter().any(|b| b.id == "real-b2"));
    assert!(!blocks.iter().any(|b| b.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}

// Scenario 3: a fresh `pending` create moves through `inflight`: mock the append endpoint to
// return 500 (fail-fast per Task 2). After push_once, the op state must be "inflight" (not
// "failed", not deleted), because a 5xx on a create is ambiguous.
#[tokio::test]
async fn ambiguous_create_failure_leaves_op_inflight() {
    let store = page_store_with_todo();
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_insert_block_after("p1", Some("b1"), "paragraph", "New block")
        .unwrap();

    let server = MockServer::start().await;
    Mock::given(method("PATCH"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 0);

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].target_id, tmp_id);
    assert_eq!(ops[0].state, "inflight");
}

// Scenario 4 (Critical 2 regression): a *definite* error (404) on the verification listing
// call itself must mark the op `failed`, not leave it `inflight` forever (which would block
// its target on every future pass with no path to resolution).
#[tokio::test]
async fn verify_call_definite_error_marks_op_failed() {
    let store = page_store_with_todo();
    append_op_left_inflight(&store);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({
            "code": "object_not_found", "message": "Could not find block"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 0);

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].state, "failed");
    assert!(ops[0].error.as_deref().unwrap_or_default().contains("404"));
}

// --- create_row verify coverage (Important 4) ---

// (a) adopt: the create landed remotely, resume finds it by matching title, no duplicate POST.
#[tokio::test]
async fn inflight_create_row_is_adopted_not_resent_when_found_remotely() {
    let store = ds_store_with_row();
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_create_row("ds1", title_props("New Row"))
        .unwrap();
    force_op_inflight(&store, &tmp_id);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "real-r2",
                "properties": title_props("New Row"),
                "last_edited_time": "2026-07-05T11:00:00.000Z",
                "archived": false
            }],
            "has_more": false,
            "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    assert!(rows.iter().any(|r| r.id == "real-r2"));
    assert!(!rows.iter().any(|r| r.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

// (b) resend: nothing matching remotely, create IS re-sent exactly once.
#[tokio::test]
async fn inflight_create_row_is_resent_when_not_found_remotely() {
    let store = ds_store_with_row();
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_create_row("ds1", title_props("New Row"))
        .unwrap();
    force_op_inflight(&store, &tmp_id);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "real-r2", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    assert!(rows.iter().any(|r| r.id == "real-r2"));
    assert!(!rows.iter().any(|r| r.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

// (c) Critical-1 collision: a remote row identical in title to the pending one, but already
// known locally under its real id ("r1", seeded by ds_store_with_row), must NOT be adopted —
// the op resends instead (creating a genuinely distinct row, since the user really did title
// two rows the same).
#[tokio::test]
async fn inflight_create_row_collision_with_locally_known_row_resends_instead_of_adopting() {
    let store = ds_store_with_row(); // seeds row "r1" titled "Existing"
    let (tmp_id, _) = store
        .lock()
        .unwrap()
        .edit_create_row("ds1", title_props("Existing"))
        .unwrap();
    force_op_inflight(&store, &tmp_id);

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "r1",
                "properties": title_props("Existing"),
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "archived": false
            }],
            "has_more": false,
            "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "real-r3", "last_edited_time": "2026-07-05T11:00:00.000Z"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    assert!(
        rows.iter().any(|r| r.id == "r1"),
        "pre-existing row must be untouched"
    );
    assert!(
        rows.iter().any(|r| r.id == "real-r3"),
        "resend must create a distinct row"
    );
    assert!(!rows.iter().any(|r| r.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

// --- create_comment verify coverage (Important 4) ---

// (a) adopt: the comment landed remotely, resume finds it by matching body, no duplicate POST.
#[tokio::test]
async fn inflight_create_comment_is_adopted_not_resent_when_found_remotely() {
    let mut s = Store::open_in_memory().unwrap();
    let tmp_id = s.edit_add_comment("p1", "page", None, "hello world").unwrap();
    let store = Arc::new(Mutex::new(s));
    force_op_inflight(&store, &tmp_id);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "real-c2", "discussion_id": "d1",
                "created_time": "2026-07-05T11:00:00.000Z",
                "created_by": {"id": "u1"},
                "rich_text": [{"plain_text": "hello world"}]
            }],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let comments = store.lock().unwrap().comments_for("p1").unwrap();
    assert!(comments.iter().any(|c| c.id == "real-c2"));
    assert!(!comments.iter().any(|c| c.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

// (b) resend: nothing matching remotely, create IS re-sent exactly once.
#[tokio::test]
async fn inflight_create_comment_is_resent_when_not_found_remotely() {
    let mut s = Store::open_in_memory().unwrap();
    let tmp_id = s.edit_add_comment("p1", "page", None, "hello world").unwrap();
    let store = Arc::new(Mutex::new(s));
    force_op_inflight(&store, &tmp_id);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "real-c2", "discussion_id": "d1"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let comments = store.lock().unwrap().comments_for("p1").unwrap();
    assert!(comments.iter().any(|c| c.id == "real-c2"));
    assert!(!comments.iter().any(|c| c.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

// (c) Critical-1 collision: a remote comment identical in body to the pending one, but already
// known locally under its real id ("c-existing"), must NOT be adopted — the op resends instead.
#[tokio::test]
async fn inflight_create_comment_collision_with_locally_known_comment_resends_instead_of_adopting() {
    let mut s = Store::open_in_memory().unwrap();
    s.replace_comments(
        "p1",
        &[CommentRec {
            id: "c-existing".into(),
            parent_id: "p1".into(),
            parent_kind: "page".into(),
            thread_id: Some("d1".into()),
            author: "u1".into(),
            body: "duplicate text".into(),
            created_time: "2026-07-05T09:00:00.000Z".into(),
        }],
    )
    .unwrap();
    let tmp_id = s.edit_add_comment("p1", "page", None, "duplicate text").unwrap();
    let store = Arc::new(Mutex::new(s));
    force_op_inflight(&store, &tmp_id);

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "c-existing", "discussion_id": "d1",
                "created_time": "2026-07-05T09:00:00.000Z",
                "created_by": {"id": "u1"},
                "rich_text": [{"plain_text": "duplicate text"}]
            }],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "real-c3", "discussion_id": "d1"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);

    let comments = store.lock().unwrap().comments_for("p1").unwrap();
    assert!(
        comments.iter().any(|c| c.id == "c-existing"),
        "pre-existing comment must be untouched"
    );
    assert!(
        comments.iter().any(|c| c.id == "real-c3"),
        "resend must create a distinct comment"
    );
    assert!(!comments.iter().any(|c| c.id == tmp_id));
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}
