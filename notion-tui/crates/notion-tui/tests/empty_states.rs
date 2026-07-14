use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_tui::app::{App, Focus, View};
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

fn draw(app: &mut App) -> String {
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(70, 16);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    term.backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}

#[test]
fn empty_sidebar_hints_first_sync() {
    let mut app = App::new(test_store());
    assert!(draw(&mut app).contains("first sync in progress…"));
}

#[test]
fn empty_page_hints_add_block() {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.view = View::Page(notion_tui::ui::page::PageView::new(
        notion_store::PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Empty".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        },
        vec![],
    ));
    assert!(draw(&mut app).contains("empty page — press a to add a block"));
}

#[test]
fn empty_table_hints_new_row() {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.view = View::Table(notion_tui::ui::table::TableView::new(
        notion_store::DataSourceRec {
            id: "ds".into(),
            database_id: "db".into(),
            title: "Tasks".into(),
            schema_json: r#"{"Name": {"type": "title"}}"#.into(),
            last_edited_time: "t".into(),
        },
        vec![],
    ));
    assert!(draw(&mut app).contains("no rows — press o to add one"));
}

#[test]
fn search_with_query_and_no_hits_says_no_results() {
    let mut app = App::new(test_store());
    let mut search = notion_tui::ui::search::SearchState::new();
    search.on_key(KeyEvent::from(KeyCode::Char('z'))); // non-empty query, no hits
    app.search = Some(search);
    assert!(draw(&mut app).contains("no results"));
}

#[test]
fn search_with_empty_query_stays_quiet() {
    let mut app = App::new(test_store());
    app.search = Some(notion_tui::ui::search::SearchState::new());
    assert!(!draw(&mut app).contains("no results"));
}

#[test]
fn empty_comments_hint_pressing_n() {
    let mut app = App::new(test_store());
    app.comments = Some(notion_tui::ui::comments::CommentsState {
        parent_id: "p1".into(),
        parent_kind: "page".into(),
        items: vec![],
        cursor: 0,
        list_state: ratatui::widgets::ListState::default(),
    });
    assert!(draw(&mut app).contains("no comments yet — press n"));
}

#[test]
fn empty_queue_explains_itself() {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.view = View::Queue(notion_tui::ui::queue::QueueView::new(vec![], vec![]));
    assert!(draw(&mut app).contains("queue is empty"));
}
