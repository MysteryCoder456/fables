use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_two_blocks() -> notion_sync::SharedStore {
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
        &[
            BlockRec {
                id: "b1".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 0,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "First".into(),
                has_children: false,
            },
            BlockRec {
                id: "b2".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 1,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "Second".into(),
                has_children: false,
            },
        ],
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

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        dispatch_key(app, KeyEvent::from(KeyCode::Char(c)));
    }
}

#[test]
fn i_prefills_input_with_block_text_and_submit_updates_it() {
    let store = store_with_two_blocks();
    let mut app = app_on_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('i')));
    assert_eq!(app.input.as_ref().unwrap().value, "First");

    // Clear the prefilled text and type something new.
    for _ in 0.."First".len() {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    }
    type_str(&mut app, "Edited text");
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    assert!(app.input.is_none());
    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "Edited text");
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 1);
}

#[test]
fn a_opens_empty_input_and_submit_inserts_block_after_cursor() {
    let store = store_with_two_blocks();
    let mut app = app_on_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('a')));
    assert_eq!(app.input.as_ref().unwrap().value, "");

    type_str(&mut app, "Brand new");
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    assert!(app.input.is_none());
    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 3);
    let inserted = blocks.iter().find(|b| b.plain_text == "Brand new").unwrap();
    assert_eq!(inserted.ordinal, 1); // right after b1 (cursor started at 0)
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 1);
}

#[test]
fn esc_cancels_input_without_changes() {
    let store = store_with_two_blocks();
    let mut app = app_on_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('i')));
    type_str(&mut app, "should not be saved");
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Esc));

    assert!(app.input.is_none());
    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "First");
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}
