use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::board::BoardView;
use notion_tui::ui::table::TableView;
use serde_json::json;

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn ds(schema: serde_json::Value) -> notion_store::DataSourceRec {
    notion_store::DataSourceRec {
        id: "ds".into(),
        database_id: "db".into(),
        title: "Tasks".into(),
        schema_json: schema.to_string(),
        last_edited_time: "t".into(),
    }
}

fn row(i: usize, status: &str) -> notion_store::RowRec {
    notion_store::RowRec {
        id: format!("r{i}"),
        data_source_id: "ds".into(),
        properties: json!({
            "Name": {"type": "title", "title": [{"plain_text": format!("task {i}")}]},
            "Status": {"type": "status", "status": {"name": status}}
        })
        .to_string(),
        last_edited_time: "t".into(),
        archived: false,
    }
}

fn table_app(n: usize) -> App {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    let rows = (0..n).map(|i| row(i, "Todo")).collect();
    app.view = View::Table(TableView::new(ds(json!({"Name": {"type": "title"}})), rows));
    app
}

fn board_app(n: usize) -> App {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    let schema = json!({
        "Name": {"type": "title"},
        "Status": {"type": "status", "status": {"options": [{"name": "Todo"}, {"name": "Done"}]}}
    });
    let rows = (0..n).map(|i| row(i, "Todo")).collect();
    app.view = View::Board(BoardView::new(ds(schema), rows));
    app
}

#[test]
fn ctrl_d_u_page_through_table() {
    let mut app = table_app(30);
    dispatch_key(&mut app, ctrl('d'));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 10);
    dispatch_key(&mut app, ctrl('u'));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 0);
}

#[test]
fn ctrl_d_u_page_through_board_column() {
    let mut app = board_app(30);
    dispatch_key(&mut app, ctrl('d'));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 10);
    dispatch_key(&mut app, ctrl('u'));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 0);
}

#[test]
fn g_and_shift_g_jump_in_board() {
    let mut app = board_app(25);
    dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 24);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('g')));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 0);
}

#[test]
fn g_and_shift_g_still_jump_in_table() {
    let mut app = table_app(25); // regression pin — table already had top/bottom
    dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 24);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('g')));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 0);
}
