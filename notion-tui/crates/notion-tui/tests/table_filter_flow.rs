use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{DataSourceRec, RowRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;

fn store_with_rows() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds".into(),
        database_id: "db".into(),
        title: "Tasks".into(),
        schema_json: json!({"Name": {"type": "title"}}).to_string(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds",
        &[
            RowRec {
                id: "r1".into(),
                data_source_id: "ds".into(),
                properties: json!({"Name": {"type": "title", "title": [{"plain_text": "Buy milk"}]}})
                    .to_string(),
                last_edited_time: "t0".into(),
                archived: false,
            },
            RowRec {
                id: "r2".into(),
                data_source_id: "ds".into(),
                properties: json!({"Name": {"type": "title", "title": [{"plain_text": "Buy eggs"}]}})
                    .to_string(),
                last_edited_time: "t0".into(),
                archived: false,
            },
        ],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}

#[test]
fn filter_key_narrows_the_table_and_survives_a_refresh() {
    let store = store_with_rows();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds");

    dispatch_key(&mut app, key('f')); // filter current (title) column
    for c in "eggs".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let View::Table(v) = &app.view else {
        panic!("expected table")
    };
    assert_eq!(v.visible().len(), 1);
    assert_eq!(v.visible()[0].id, "r2");

    app.refresh_current_view(); // simulate a sync tick
    let View::Table(v) = &app.view else {
        panic!("expected table")
    };
    assert_eq!(v.visible().len(), 1, "filter must survive a refresh");
    assert_eq!(v.visible()[0].id, "r2");
}

#[test]
fn props_under_filter_targets_visible_row_not_hidden_one() {
    let store = store_with_rows();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds");

    dispatch_key(&mut app, key('f')); // filter current (title) column
    for c in "eggs".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    {
        let View::Table(v) = &app.view else {
            panic!("expected table")
        };
        assert_eq!(v.visible().len(), 1);
        assert_eq!(v.visible()[0].id, "r2");
        assert_eq!(v.cursor, 0);
    }

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p'))); // props

    let props = app.props.as_ref().expect("props should open");
    assert_eq!(
        props.row_id, "r2",
        "props must target the visible row, not a hidden one"
    );
}

#[test]
fn rename_under_filter_targets_visible_row_not_hidden_one() {
    let store = store_with_rows();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds");

    dispatch_key(&mut app, key('f')); // filter current (title) column
    for c in "eggs".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    app.start_rename();

    match app.input_purpose {
        Some(notion_tui::app::InputPurpose::RenameRow { ref row_id, .. }) => {
            assert_eq!(
                row_id, "r2",
                "rename must target the visible row, not a hidden one"
            );
        }
        _ => panic!("expected RenameRow input purpose"),
    }
}

#[test]
fn bottom_lands_on_last_visible_row_under_filter() {
    let store = store_with_rows();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds");

    dispatch_key(&mut app, key('f')); // filter current (title) column
    for c in "eggs".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('G'))); // bottom

    let View::Table(v) = &app.view else {
        panic!("expected table")
    };
    assert_eq!(v.cursor, v.row_count().saturating_sub(1));
    assert_eq!(v.visible()[v.cursor].id, "r2");
}
