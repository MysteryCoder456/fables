use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;
use serde_json::Value;

fn store_with_todo() -> notion_sync::SharedStore {
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
            block_type: "to_do".into(),
            payload: r#"{"checked": false}"#.into(),
            plain_text: "Buy milk".into(),
            has_children: false,
        }],
    )
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn app_on_todo_page(store: notion_sync::SharedStore) -> App {
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
fn space_toggles_todo_and_queues_op() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char(' ')));

    let s = store.lock().unwrap();
    let blocks = s.page_blocks("p1").unwrap();
    let payload: Value = serde_json::from_str(&blocks[0].payload).unwrap();
    assert_eq!(payload["checked"], true);
    assert_eq!(s.ops().unwrap().len(), 1);
}

#[test]
fn dd_asks_for_confirmation_then_y_deletes() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    assert!(app.confirm.is_some(), "dd must confirm before deleting");
    assert_eq!(
        store.lock().unwrap().page_blocks("p1").unwrap().len(),
        1,
        "not deleted yet"
    );

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));
    let s = store.lock().unwrap();
    assert!(s.page_blocks("p1").unwrap().is_empty());
    assert_eq!(s.ops().unwrap()[0].op_type, "delete_block");
    drop(s);
    assert!(app.notice.as_deref().unwrap_or("").contains("undo"));
}

#[test]
fn dd_then_n_keeps_the_block() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('n')));

    assert!(app.confirm.is_none());
    assert_eq!(store.lock().unwrap().page_blocks("p1").unwrap().len(), 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}

#[test]
fn single_d_does_not_delete() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    // A different key in between cancels the pending delete.
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('j')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));

    // Still armed only once (not deleted) since the 'j' reset the pending state.
    assert_eq!(store.lock().unwrap().page_blocks("p1").unwrap().len(), 1);
}

#[test]
fn undo_after_toggle_reverts_and_clears_queue() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char(' ')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('u')));

    let s = store.lock().unwrap();
    let blocks = s.page_blocks("p1").unwrap();
    let payload: Value = serde_json::from_str(&blocks[0].payload).unwrap();
    assert_eq!(payload["checked"], false);
    assert!(s.ops().unwrap().is_empty());
}
