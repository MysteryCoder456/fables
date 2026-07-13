use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{DataSourceRec, RowRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;

fn ds() -> DataSourceRec {
    DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tracker".into(),
        schema_json: json!({
            "Name": {"type": "title"},
            "Status": {"type": "status", "status": {"options": [
                {"name": "Todo"}, {"name": "Doing"}, {"name": "Done"}]}}
        })
        .to_string(),
        last_edited_time: "t1".into(),
    }
}

fn row(id: &str, name: &str, status: &str) -> RowRec {
    RowRec {
        id: id.into(),
        data_source_id: "ds1".into(),
        properties: json!({
            "Name": {"type": "title", "title": [{"plain_text": name}]},
            "Status": {"type": "status", "status": {"name": status}}
        })
        .to_string(),
        last_edited_time: "t1".into(),
        archived: false,
    }
}

fn app_on_board() -> App {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&ds()).unwrap();
    s.replace_rows("ds1", &[row("r1", "Ship it", "Doing")]).unwrap();
    let store: notion_sync::SharedStore = Arc::new(Mutex::new(s));
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds1");
    app.toggle_board();
    match &mut app.view {
        View::Board(b) => b.col = 1, // r1's status is "Doing", the second column
        _ => panic!("test setup should land on a board"),
    }
    app
}

#[test]
fn p_on_a_board_opens_the_props_modal_for_the_selected_card() {
    let mut app = app_on_board();

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));

    let props = app.props.as_ref().expect("props modal should be open");
    assert_eq!(props.row_id, "r1");
    assert!(props
        .fields
        .iter()
        .any(|f| f.name == "Name" && f.value_text == "Ship it"));
}

#[test]
fn committing_a_board_props_edit_updates_the_row_and_refreshes_the_modal() {
    let mut app = app_on_board();
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));
    // Move onto the Status field and open its option picker.
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));
    // Picker is preselected on "Doing"; move down to "Done" and commit.
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let props = app.props.as_ref().expect("modal should stay open after commit");
    let status = props.fields.iter().find(|f| f.name == "Status").unwrap();
    assert_eq!(status.value_text, "Done");
    match &app.view {
        View::Board(b) => assert!(b.rows[0].properties.contains("Done")),
        _ => panic!("expected to remain on the board view"),
    }
}

#[test]
fn committing_a_group_property_edit_keeps_the_card_selected_in_its_new_column() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&ds()).unwrap();
    s.replace_rows(
        "ds1",
        &[row("r1", "Ship it", "Doing"), row("r2", "Buy milk", "Todo")],
    )
    .unwrap();
    let store: notion_sync::SharedStore = Arc::new(Mutex::new(s));
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds1");
    app.toggle_board();
    match &mut app.view {
        View::Board(b) => {
            let idx = b.cards_in(1).iter().position(|r| r.id == "r1").unwrap();
            b.col = 1; // "Doing"
            b.card = idx;
        }
        _ => panic!("test setup should land on a board"),
    }

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j'))); // to Status field
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // open picker, preselected on "Doing"
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j'))); // move to "Done"
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // commit

    match &app.view {
        View::Board(b) => assert_eq!(
            b.selected_row_id().as_deref(),
            Some("r1"),
            "the edited card should still be the selected card, now in the Done column"
        ),
        _ => panic!("expected to remain on the board view"),
    }
}

#[test]
fn moving_a_card_with_the_modal_closed_does_not_open_the_props_modal() {
    let mut app = app_on_board();
    assert!(app.props.is_none());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('J')));

    assert!(
        app.props.is_none(),
        "moving a card should not pop open the properties modal"
    );
    match &app.view {
        View::Board(b) => assert_eq!(b.col, 2, "card should have moved to the Doing column"),
        _ => panic!("expected to remain on the board view"),
    }
}
