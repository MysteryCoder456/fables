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
        schema_json: json!({
            "Name": {"type": "title"},
            "Done": {"type": "checkbox"},
            "Prio": {"type": "select"}
        })
        .to_string(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds1",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Buy milk"}]},
                "Done": {"type": "checkbox", "checkbox": false},
                "Prio": {"type": "select", "select": {"name": "Low"}}
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
fn p_opens_props_modal_listing_fields() {
    let store = store_with_ds_and_row();
    let mut app = app_on_table(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));

    let props = app.props.as_ref().unwrap();
    assert_eq!(props.row_id, "r1");
    assert!(props.fields.iter().any(|f| f.name == "Name" && f.value_text == "Buy milk"));
    assert!(props.fields.iter().any(|f| f.name == "Prio" && f.value_text == "Low"));
}

#[test]
fn editing_a_text_field_commits_new_value() {
    let store = store_with_ds_and_row();
    let mut app = app_on_table(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));
    // Move cursor to the "Prio" field (Name, Done, Prio — column order puts title first).
    let prio_idx = app.props.as_ref().unwrap().fields.iter().position(|f| f.name == "Prio").unwrap();
    for _ in 0..prio_idx {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // begin editing
    for _ in 0.."Low".len() {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    }
    type_str(&mut app, "High");
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // commit

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    let props: serde_json::Value = serde_json::from_str(&rows[0].properties).unwrap();
    assert_eq!(props["Prio"]["select"]["name"], "High");
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 1);
    // Modal stays open, reflecting the committed value.
    assert!(app.props.as_ref().unwrap().fields.iter().any(|f| f.name == "Prio" && f.value_text == "High"));
}

#[test]
fn enter_on_checkbox_field_toggles_immediately() {
    let store = store_with_ds_and_row();
    let mut app = app_on_table(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));
    let done_idx = app.props.as_ref().unwrap().fields.iter().position(|f| f.name == "Done").unwrap();
    for _ in 0..done_idx {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let rows = store.lock().unwrap().rows("ds1").unwrap();
    let props: serde_json::Value = serde_json::from_str(&rows[0].properties).unwrap();
    assert_eq!(props["Done"]["checkbox"], true);
}

#[test]
fn esc_closes_modal_without_changes() {
    let store = store_with_ds_and_row();
    let mut app = app_on_table(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Esc));

    assert!(app.props.is_none());
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}
