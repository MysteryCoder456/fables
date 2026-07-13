use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::{DataSourceRec, RowRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;

fn store_with_board_ds() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds".into(),
        database_id: "db".into(),
        title: "Tasks".into(),
        schema_json: json!({
            "Name": {"type": "title"},
            "Status": {"type": "status", "status": {"options": [{"name": "Todo"}, {"name": "Done"}]}}
        })
        .to_string(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Task A"}]},
                "Status": {"type": "status", "status": {"name": "Todo"}}
            })
            .to_string(),
            last_edited_time: "t0".into(),
            archived: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}
fn shift(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
}

#[test]
fn v_toggles_table_to_board_and_back() {
    let mut app = App::new(store_with_board_ds());
    app.focus = Focus::Main;
    app.open_page("ds");
    assert!(matches!(app.view, View::Table(_)));
    dispatch_key(&mut app, key('v'));
    assert!(matches!(app.view, View::Board(_)));
    dispatch_key(&mut app, key('v'));
    assert!(matches!(app.view, View::Table(_)));
}

#[test]
fn shift_j_moves_card_and_queues_an_update_row_op() {
    let store = store_with_board_ds();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    app.open_page("ds");
    dispatch_key(&mut app, key('v'));

    dispatch_key(&mut app, shift('J')); // Todo -> Done

    let s = store.lock().unwrap();
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "update_row");
    let rows = s.rows("ds").unwrap();
    let props: serde_json::Value = serde_json::from_str(&rows[0].properties).unwrap();
    assert_eq!(props["Status"]["status"]["name"], "Done");
}
