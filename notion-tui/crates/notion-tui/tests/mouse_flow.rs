use std::sync::{Arc, Mutex};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use notion_store::{DataSourceRec, NodeKind, RowRec, Store, TreeNode};
use notion_tui::app::{dispatch_mouse, App, Focus, View};
use notion_tui::ui;
use notion_tui::ui::sidebar::SidebarState;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;

fn click(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

#[test]
fn clicking_a_sidebar_entry_opens_it() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let s = store.lock().unwrap();
        s.upsert_page(&notion_store::PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Roadmap".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        })
        .unwrap();
    }
    let mut app = App::new(store);
    app.sidebar = SidebarState::new(vec![TreeNode {
        id: "p1".into(),
        title: "Roadmap".into(),
        parent_id: None,
        kind: NodeKind::Page,
    }]);

    // Render once so `app.last_layout` reflects a real frame.
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    let sidebar_rect = app.last_layout.sidebar.expect("sidebar should be visible");
    dispatch_mouse(&mut app, click(sidebar_rect.x + 2, sidebar_rect.y + 1));

    assert!(matches!(app.focus, Focus::Sidebar));
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));
}

/// board.rs render only scrolls the ACTIVE column (via `view.list_state`); every
/// other column renders unscrolled from item 0. A click in a non-active column
/// must therefore use offset 0, not the active column's scroll offset.
#[test]
fn clicking_a_card_in_a_non_active_scrolled_column_selects_the_right_card() {
    let mut row_recs: Vec<RowRec> = (0..10)
        .map(|i| RowRec {
            id: format!("todo-{i}"),
            data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": format!("Todo {i}")}]},
                "Status": {"type": "status", "status": {"name": "Todo"}}
            })
            .to_string(),
            last_edited_time: "t".into(),
            archived: false,
        })
        .collect();
    row_recs.push(RowRec {
        id: "target".into(),
        data_source_id: "ds".into(),
        properties: json!({
            "Name": {"type": "title", "title": [{"plain_text": "Target Card"}]},
            "Status": {"type": "status", "status": {"name": "Done"}}
        })
        .to_string(),
        last_edited_time: "t".into(),
        archived: false,
    });

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let mut s = store.lock().unwrap();
        s.upsert_data_source(&DataSourceRec {
            id: "ds".into(),
            database_id: "db".into(),
            title: "Tasks".into(),
            schema_json: json!({
                "Name": {"type": "title"},
                "Status": {"type": "status", "status": {"options": [
                    {"name": "Todo"}, {"name": "Done"}]}}
            })
            .to_string(),
            last_edited_time: "t".into(),
        })
        .unwrap();
        s.replace_rows("ds", &row_recs).unwrap();
    }

    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.sidebar.hidden = true;
    app.open_page("ds");
    notion_tui::app::dispatch_key(
        &mut app,
        crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('v')),
    ); // table -> board
    let View::Board(v) = &mut app.view else {
        panic!("expected a board view")
    };
    // Column 0 ("Todo") is active and scrolled to its last card; column 1
    // ("Done") holds only the single "target" card at index 0.
    v.col = 0;
    v.card = 9;

    // Small backend so column 0's 10 cards don't all fit, forcing a scroll offset.
    let backend = TestBackend::new(60, 10);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    let active_offset = {
        let View::Board(v) = &app.view else { unreachable!() };
        v.list_state.offset()
    };
    assert!(
        active_offset > 0,
        "column 0 should have scrolled (test setup sanity check)"
    );

    let done_col_rect = app.last_layout.board_columns[1];
    // Row 0 of the (unscrolled) non-active column: its single "target" card.
    dispatch_mouse(&mut app, click(done_col_rect.x + 2, done_col_rect.y + 1));

    let View::Board(v) = &app.view else {
        panic!("expected a board view")
    };
    assert_eq!(v.col, 1, "click should switch to the clicked column");
    assert_eq!(
        v.cards_in(1).get(v.card).map(|r| r.id.as_str()),
        Some("target"),
        "must select the actual clicked card, not one offset by column 0's scroll"
    );
}
