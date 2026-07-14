use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_page() -> notion_sync::SharedStore {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Old Title".into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    })
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}

#[test]
fn palette_rename_command_renames_the_open_page() {
    let store = store_with_page();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let page = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    app.view = View::Page(PageView::new(page, Vec::new()));

    dispatch_key(&mut app, key(':')); // open palette
    for c in "rename".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // run "rename"
    assert!(app.input.is_some(), "rename should open a text prompt");

    // Replace the prefilled title.
    for _ in 0.."Old Title".len() {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    }
    for c in "New Title".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    assert_eq!(
        store.lock().unwrap().get_page("p1").unwrap().unwrap().title,
        "New Title"
    );
}

#[test]
fn sync_now_without_a_running_sync_reports_unavailable() {
    let mut app = App::new(store_with_page());
    app.request_sync_now();
    assert_eq!(app.notice.as_deref(), Some("sync not available"));
}

#[test]
fn move_page_command_reparents_via_the_picker() {
    let store = store_with_page();
    {
        let s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "dest".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Destination".into(),
            icon: None,
            archived: false,
            last_edited_time: "t1".into(),
        })
        .unwrap();
    }
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let page = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    app.view = View::Page(PageView::new(page, Vec::new()));

    app.start_move_page();
    assert!(app.picker.is_some());
    for c in "Dest".chars() {
        notion_tui::app::dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let moved = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    assert_eq!(moved.parent_id.as_deref(), Some("dest"));
}

#[test]
fn move_page_picker_excludes_the_moved_pages_own_descendants() {
    // p1 (moved page) -> child (parent_id = p1) -> grandchild (parent_id = child).
    // Moving p1 onto child or grandchild would create a parent cycle, so both
    // (plus p1 itself) must be excluded from the picker's candidate list.
    let store = store_with_page();
    {
        let s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "child".into(),
            parent_type: "page_id".into(),
            parent_id: Some("p1".into()),
            title: "Child".into(),
            icon: None,
            archived: false,
            last_edited_time: "t1".into(),
        })
        .unwrap();
        s.upsert_page(&PageRec {
            id: "grandchild".into(),
            parent_type: "page_id".into(),
            parent_id: Some("child".into()),
            title: "Grandchild".into(),
            icon: None,
            archived: false,
            last_edited_time: "t1".into(),
        })
        .unwrap();
        s.upsert_page(&PageRec {
            id: "unrelated".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Unrelated".into(),
            icon: None,
            archived: false,
            last_edited_time: "t1".into(),
        })
        .unwrap();
    }
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let page = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    app.view = View::Page(PageView::new(page, Vec::new()));

    app.start_move_page();
    let picker = app.picker.as_ref().expect("picker should open");
    let ids: Vec<&str> = picker.items.iter().map(|(id, _)| id.as_str()).collect();

    assert!(!ids.contains(&"p1"), "must not offer the moved page itself");
    assert!(
        !ids.contains(&"child"),
        "must not offer a direct child (would cycle)"
    );
    assert!(
        !ids.contains(&"grandchild"),
        "must not offer a transitive descendant (would cycle)"
    );
    assert!(ids.contains(&"unrelated"), "unrelated pages remain valid targets");
}
