use std::sync::{Arc, Mutex};

use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec, Store};
use notion_tui::app::{App, Focus, View};
use serde_json::json;

fn app_with_page_of_blocks(n: usize) -> App {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Roadmap".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    })
    .unwrap();
    let blocks: Vec<BlockRec> = (0..n)
        .map(|i| BlockRec {
            id: format!("b{i}"),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: i as i64,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: format!("Line {i}"),
            has_children: false,
        })
        .collect();
    s.replace_page_blocks("p1", &blocks).unwrap();
    let store = Arc::new(Mutex::new(s));
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("p1");
    app
}

fn store_with_ds_and_rows() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: json!({
            "Name": {"type": "title"},
            "Priority": {"type": "number"}
        })
        .to_string(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    let rows: Vec<RowRec> = (1..=5)
        .map(|i| RowRec {
            id: format!("r{i}"),
            data_source_id: "ds1".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": format!("Row {i}")}]},
                "Priority": {"type": "number", "number": i}
            })
            .to_string(),
            last_edited_time: "t1".into(),
            archived: false,
        })
        .collect();
    s.replace_rows("ds1", &rows).unwrap();
    Arc::new(Mutex::new(s))
}

#[test]
fn page_refresh_preserves_cursor_and_collapsed_toggles() {
    let mut app = app_with_page_of_blocks(10);
    if let View::Page(v) = &mut app.view {
        v.cursor = 7;
        v.collapsed_toggles.insert("b3".into());
    }
    app.refresh_current_view();
    let View::Page(v) = &app.view else {
        panic!("still a page view")
    };
    assert_eq!(v.cursor, 7);
    assert!(v.collapsed_toggles.contains("b3"));
}

fn seed_page_blocks(s: &mut Store, n: usize) {
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Roadmap".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    })
    .unwrap();
    let blocks: Vec<BlockRec> = (0..n)
        .map(|i| BlockRec {
            id: format!("b{i}"),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: i as i64,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: format!("Line {i}"),
            has_children: false,
        })
        .collect();
    s.replace_page_blocks("p1", &blocks).unwrap();
}

#[test]
fn page_refresh_clamps_cursor_when_blocks_shrink() {
    let mut s = Store::open_in_memory().unwrap();
    seed_page_blocks(&mut s, 10);
    let store = Arc::new(Mutex::new(s));
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    app.open_page("p1");
    if let View::Page(v) = &mut app.view {
        v.cursor = 9;
    }
    // delete blocks b5..b9 in the store, then refresh
    {
        let mut s = store.lock().unwrap();
        seed_page_blocks(&mut s, 5);
    }
    app.refresh_current_view();
    let View::Page(v) = &app.view else { panic!() };
    assert!(v.cursor < v.lines().len().max(1));
}

#[test]
fn table_refresh_follows_selected_row_and_keeps_sort() {
    let store = store_with_ds_and_rows();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds1");
    let View::Table(v) = &mut app.view else {
        panic!("expected table view")
    };
    v.toggle_sort(1); // asc
    v.toggle_sort(1); // desc
    let idx = v.rows.iter().position(|r| r.id == "r4").unwrap();
    v.cursor = idx;
    let prev_sort = v.sort;

    app.refresh_current_view();

    let View::Table(v) = &app.view else {
        panic!("expected table view")
    };
    assert_eq!(v.selected_row_id(), Some("r4".to_string()));
    assert_eq!(v.sort, prev_sort);
}

#[test]
fn table_refresh_drops_sort_when_sorted_column_removed() {
    let store = store_with_ds_and_rows();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    app.open_page("ds1");
    let View::Table(v) = &mut app.view else {
        panic!("expected table view")
    };
    // Sort on "Priority", the second column (index 1).
    v.toggle_sort(1);
    assert!(v.sort.is_some());

    // Background sync shrinks the schema: "Priority" is gone, only "Name" remains.
    {
        let mut s = store.lock().unwrap();
        s.upsert_data_source(&DataSourceRec {
            id: "ds1".into(),
            database_id: "db1".into(),
            title: "Tasks".into(),
            schema_json: json!({
                "Name": {"type": "title"}
            })
            .to_string(),
            last_edited_time: "t2".into(),
        })
        .unwrap();
        let rows: Vec<RowRec> = (1..=5)
            .map(|i| RowRec {
                id: format!("r{i}"),
                data_source_id: "ds1".into(),
                properties: json!({
                    "Name": {"type": "title", "title": [{"plain_text": format!("Row {i}")}]}
                })
                .to_string(),
                last_edited_time: "t2".into(),
                archived: false,
            })
            .collect();
        s.replace_rows("ds1", &rows).unwrap();
    }

    // Must not panic, and the stale sort must be dropped rather than carried
    // over as an out-of-bounds column index.
    app.refresh_current_view();

    let View::Table(v) = &app.view else {
        panic!("expected table view")
    };
    assert_eq!(v.sort, None);
    assert_eq!(v.columns.len(), 1);
    assert!(v.cursor < v.row_count().max(1));
}

#[test]
fn fresh_navigation_still_resets_state() {
    let mut app = app_with_page_of_blocks(10);
    if let View::Page(v) = &mut app.view {
        v.cursor = 7;
    }
    app.open_page("p1"); // explicit re-open, not a refresh
    let View::Page(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 0);
}
