use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{DataSourceRec, RowRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::table::TableView;
use serde_json::json;

fn store_with_ds_and_row() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: json!({"Name": {"type": "title"}, "Done": {"type": "checkbox"}}).to_string(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds1",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Existing"}]},
                "Done": {"type": "checkbox", "checkbox": false}
            })
            .to_string(),
            last_edited_time: "t1".into(),
            archived: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn app_on_table(store: notion_sync::SharedStore) -> App {
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let s = store.lock().unwrap();
    let ds = s.get_data_source("ds1").unwrap().unwrap();
    let rows = s.rows("ds1").unwrap();
    drop(s);
    app.view = View::Table(TableView::new(ds, rows));
    app
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        dispatch_key(app, KeyEvent::from(KeyCode::Char(c)));
    }
}

#[test]
fn o_opens_input_and_submit_creates_row_with_title() {
    let store = store_with_ds_and_row();
    let mut app = app_on_table(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('o')));
    assert_eq!(app.input.as_ref().unwrap().value(), "");

    type_str(&mut app, "New task");
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    assert!(app.input.is_none());
    let rows = store.lock().unwrap().rows("ds1").unwrap();
    assert_eq!(rows.len(), 2);
    let created = rows.iter().find(|r| r.id != "r1").unwrap();
    let props: serde_json::Value = serde_json::from_str(&created.properties).unwrap();
    assert_eq!(props["Name"]["title"][0]["plain_text"], "New task");
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 1);
}

#[test]
fn dd_deletes_selected_row() {
    let store = store_with_ds_and_row();
    let mut app = app_on_table(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    assert_eq!(store.lock().unwrap().rows("ds1").unwrap().len(), 1); // first 'd' just arms
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    assert!(rows.is_empty()); // archived rows are excluded from rows()
    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "delete_row");
    assert_eq!(ops[0].target_id, "r1");
}
