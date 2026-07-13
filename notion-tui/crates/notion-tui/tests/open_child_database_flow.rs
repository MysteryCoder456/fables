use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, DataSourceRec, NodeKind, PageRec, RowRec, Store, TreeNode};
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    }
}

fn child_database_block(id: &str, page_id: &str, title: &str) -> BlockRec {
    BlockRec {
        id: id.into(),
        page_id: page_id.into(),
        parent_block_id: None,
        ordinal: 0,
        block_type: "child_database".into(),
        payload: "{}".into(),
        plain_text: title.into(),
        has_children: false,
    }
}

/// A page ("Marc") containing a child_database block. Notion gives child_database
/// blocks the *database* id, which is stored separately from the data source id
/// that owns the schema/rows.
fn store_with_page_and_linked_database() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Marc")).unwrap();
    s.replace_page_blocks("p1", &[child_database_block("db1", "p1", "Tracker")])
        .unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tracker".into(),
        schema_json: json!({"Name": {"type": "title"}, "Status": {"type": "status"}}).to_string(),
        last_edited_time: "t1".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds1",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Ship it"}]},
                "Status": {"type": "status", "status": {"name": "Doing"}}
            })
            .to_string(),
            last_edited_time: "t1".into(),
            archived: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn app_on_page(store: notion_sync::SharedStore, page_id: &str) -> App {
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page(page_id);
    app
}

#[test]
fn enter_on_child_database_block_opens_its_table() {
    let store = store_with_page_and_linked_database();
    let mut app = app_on_page(store, "p1");
    assert!(matches!(app.view, View::Page(_)), "should start on the page view");

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    match &app.view {
        View::Table(t) => assert_eq!(t.ds.id, "ds1"),
        _ => panic!("expected Table view after opening child_database"),
    }
}

#[test]
fn v_on_child_database_block_opens_it_as_a_board() {
    let store = store_with_page_and_linked_database();
    let mut app = app_on_page(store, "p1");

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('v')));

    match &app.view {
        View::Board(b) => assert_eq!(b.ds.id, "ds1"),
        _ => panic!("expected Board view after pressing v on a child_database block"),
    }
}

#[test]
fn opening_an_unknown_id_sets_a_notice_instead_of_silently_failing() {
    let store = store_with_page_and_linked_database();
    let mut app = App::new(store);
    app.focus = Focus::Main;

    app.open_page("does-not-exist");

    assert!(matches!(app.view, View::Empty), "view should be untouched");
    assert!(app.notice.is_some(), "a notice should explain why nothing opened");
}

#[test]
fn toggling_board_with_no_table_open_sets_a_notice() {
    let store = store_with_page_and_linked_database();
    let mut app = App::new(store);
    app.focus = Focus::Main;

    app.toggle_board();

    assert!(
        app.notice.is_some(),
        "toggling board with nothing open should explain why"
    );
}

#[test]
fn toggling_board_without_a_groupable_property_sets_a_notice() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds2".into(),
        database_id: "db2".into(),
        title: "No group prop".into(),
        schema_json: json!({"Name": {"type": "title"}}).to_string(),
        last_edited_time: "t1".into(),
    })
    .unwrap();
    let store: notion_sync::SharedStore = Arc::new(Mutex::new(s));
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds2");

    app.toggle_board();

    match &app.view {
        View::Table(_) => {}
        _ => panic!("should stay on the table"),
    }
    assert!(
        app.notice.is_some(),
        "should explain why board mode isn't available"
    );
}

#[test]
fn v_on_an_unsynced_sidebar_database_keeps_the_specific_not_found_notice() {
    let store = store_with_page_and_linked_database();
    let mut app = App::new(store);
    app.refresh_sidebar();
    // Simulate a sidebar entry the local cache knows about but whose row hasn't
    // synced into the data_sources table yet.
    app.sidebar.nodes.push(TreeNode {
        id: "unsynced-ds".into(),
        title: "Unsynced".into(),
        parent_id: None,
        kind: NodeKind::DataSource,
    });
    let idx = app
        .sidebar
        .visible()
        .iter()
        .position(|v| v.node.id == "unsynced-ds")
        .unwrap();
    app.sidebar.cursor = idx;

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('v')));

    let notice = app.notice.as_deref().unwrap_or("");
    assert!(
        notice.contains("unsynced-ds"),
        "expected the specific not-found notice naming the id, got: {notice:?}"
    );
}

#[test]
fn v_on_an_unsynced_child_database_block_keeps_the_specific_not_found_notice() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Marc")).unwrap();
    s.replace_page_blocks("p1", &[child_database_block("missing-db", "p1", "Tracker")])
        .unwrap();
    let store: notion_sync::SharedStore = Arc::new(Mutex::new(s));
    let mut app = app_on_page(store, "p1");

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('v')));

    let notice = app.notice.as_deref().unwrap_or("");
    assert!(
        notice.contains("missing-db"),
        "expected the specific not-found notice naming the id, got: {notice:?}"
    );
}

#[test]
fn v_on_a_hovered_sidebar_database_opens_it_as_a_board() {
    let store = store_with_page_and_linked_database();
    let mut app = App::new(store);
    app.refresh_sidebar();
    let ds_idx = app
        .sidebar
        .visible()
        .iter()
        .position(|v| v.node.id == "ds1")
        .expect("ds1 should be in the sidebar");
    app.sidebar.cursor = ds_idx;
    assert!(matches!(app.focus, Focus::Sidebar));

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('v')));

    assert!(matches!(app.focus, Focus::Main), "v should focus the opened view");
    match &app.view {
        View::Board(b) => assert_eq!(b.ds.id, "ds1"),
        _ => panic!("expected Board view after pressing v on a sidebar database"),
    }
}
