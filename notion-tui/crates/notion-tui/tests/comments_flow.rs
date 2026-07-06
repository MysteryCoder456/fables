use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, CommentRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_commented_page() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false,
    }]).unwrap();
    s.replace_comments("p1", &[CommentRec {
        id: "c1".into(), parent_id: "p1".into(), parent_kind: "page".into(),
        thread_id: Some("d1".into()), author: "u1".into(), body: "existing".into(),
        created_time: "t".into(),
    }]).unwrap();
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

fn key(c: char) -> KeyEvent { KeyEvent::from(KeyCode::Char(c)) }

#[test]
fn c_opens_panel_with_page_comments_and_c_closes() {
    let mut app = app_on_page(store_with_commented_page());
    dispatch_key(&mut app, key('c'));
    let panel = app.comments.as_ref().expect("panel open");
    assert_eq!(panel.items.len(), 1);
    assert_eq!(panel.items[0].body, "existing");
    dispatch_key(&mut app, key('c'));
    assert!(app.comments.is_none());
}

#[test]
fn n_composes_a_new_thread_through_the_input_modal() {
    let store = store_with_commented_page();
    let mut app = app_on_page(store.clone());
    dispatch_key(&mut app, key('c'));
    dispatch_key(&mut app, key('n'));
    assert!(app.input.is_some());
    for ch in "my reply".chars() {
        dispatch_key(&mut app, key(ch));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let s = store.lock().unwrap();
    let comments = s.comments_for("p1").unwrap();
    assert_eq!(comments.len(), 2);
    assert!(comments.iter().any(|c| c.body == "my reply"));
    assert_eq!(s.ops().unwrap()[0].op_type, "create_comment");
    drop(s);
    // Panel refreshed with the new comment:
    assert_eq!(app.comments.as_ref().unwrap().items.len(), 2);
}
