use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_page() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[
        BlockRec { id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
            block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false },
    ]).unwrap();
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
fn e_key_edits_via_injected_editor_and_applies_the_result() {
    let store = store_with_page();
    let mut app = app_on_page(store.clone());

    app.edit_in_editor(|_initial| Ok("First\nSecond".to_string()));

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 2);
}

#[test]
fn missing_protected_block_asks_for_confirmation_before_deleting() {
    let store = store_with_page();
    {
        let mut s = store.lock().unwrap();
        s.replace_page_blocks("p1", &[
            BlockRec { id: "tg1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
                block_type: "toggle".into(), payload: "{}".into(), plain_text: "More".into(), has_children: false },
        ]).unwrap();
    }
    let mut app = app_on_page(store.clone());
    if let View::Page(v) = &mut app.view {
        *v = PageView::new(v.page.clone(), store.lock().unwrap().page_blocks("p1").unwrap());
    }

    app.edit_in_editor(|_initial| Ok(String::new())); // user deleted the marker line

    assert!(app.confirm.is_some());
    assert_eq!(store.lock().unwrap().page_blocks("p1").unwrap().len(), 1); // not deleted yet

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));
    assert!(app.confirm.is_none());
    assert!(store.lock().unwrap().page_blocks("p1").unwrap().is_empty());
}
