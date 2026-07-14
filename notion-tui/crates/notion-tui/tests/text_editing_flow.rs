use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui;
use notion_tui::ui::page::PageView;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn store_with_block(text: &str) -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: text.into(),
            has_children: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn app_on_page(store: notion_sync::SharedStore) -> App {
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let s = store.lock().unwrap();
    let page = s.get_page("p1").unwrap().unwrap();
    let blocks = s.page_blocks("p1").unwrap();
    drop(s);
    app.view = View::Page(PageView::new(page, blocks));
    app
}

#[test]
fn home_delete_right_insert_edits_mid_string() {
    let store = store_with_block("First");
    let mut app = app_on_page(store.clone());
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('i'))); // prefill "First", cursor at end
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Home));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Delete)); // "irst"
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Right)); // after 'i'
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('X'))); // "iXrst"
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));
    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks[0].plain_text, "iXrst");
}

#[test]
fn backspace_removes_a_whole_flag_emoji_in_the_input_modal() {
    let store = store_with_block("x");
    let mut app = app_on_page(store.clone());
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('a'))); // empty "new block" input
    for c in "hi".chars() {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Char(c)));
    }
    // Terminals deliver 🇨🇦 as two scalar-value key events:
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('🇨')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('🇦')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace)); // must remove the whole flag
    assert_eq!(app.input.as_ref().unwrap().value(), "hi");
}

#[test]
fn input_modal_shows_the_terminal_cursor_at_the_edit_point() {
    let store = store_with_block("First");
    let mut app = app_on_page(store);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('i')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Home));
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    // input popup: width (60*2/3).clamp(20,70)=40 → x=10; height 3 → y=(12-3)/2=4.
    // Cursor at Home ⇒ one cell inside the border: (11, 5).
    let pos = term.get_cursor_position().unwrap();
    assert_eq!((pos.x, pos.y), (11, 5));
}

#[test]
fn search_left_arrow_does_not_refire_the_query() {
    let store = store_with_block("hello world");
    let mut app = app_on_page(store);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('/')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('h')));
    let hits = app.search.as_ref().unwrap().results.len();
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Left)); // movement only
    assert_eq!(app.search.as_ref().unwrap().results.len(), hits);
    assert_eq!(app.search.as_ref().unwrap().input.text(), "h");
}
