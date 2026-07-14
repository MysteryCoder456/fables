use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, NodeKind, View};
use notion_tui::ui::sidebar::SidebarState;

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    }
}

fn store_with_two_pages() -> notion_sync::SharedStore {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "First")).unwrap();
    s.upsert_page(&page("p2", "Second")).unwrap();
    Arc::new(Mutex::new(s))
}

#[test]
fn sidebar_open_pushes_history_so_backspace_returns() {
    let store = store_with_two_pages();
    let mut app = App::new(store);
    app.focus = Focus::Sidebar;
    app.sidebar = SidebarState::new(vec![
        notion_store::TreeNode {
            id: "p1".into(),
            title: "First".into(),
            parent_id: None,
            kind: NodeKind::Page,
        },
        notion_store::TreeNode {
            id: "p2".into(),
            title: "Second".into(),
            parent_id: None,
            kind: NodeKind::Page,
        },
    ]);

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // open "First"
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));

    app.focus = Focus::Sidebar;
    app.sidebar.cursor = 1;
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // open "Second" (sidebar-driven, no link follow)
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p2"));

    app.focus = Focus::Main;
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    assert!(
        matches!(&app.view, View::Page(v) if v.page.id == "p1"),
        "backspace should return to First"
    );
}

#[test]
fn back_navigation_itself_does_not_grow_history() {
    let store = store_with_two_pages();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("p1");
    app.push_history_for_test(); // p1 pushed
    app.open_page("p2");

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace)); // back to p1
    assert!(
        app.history.is_empty(),
        "going back must not re-push where we came from"
    );
}

#[test]
fn breadcrumb_does_not_hang_on_a_cyclic_parent_chain() {
    // A parent cycle should never exist (guarded at move-time), but a corrupt
    // store or a bad crawl could still produce one; breadcrumb()'s upward walk
    // must degrade to a truncated chain instead of looping forever.
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "a".into(),
        parent_type: "page_id".into(),
        parent_id: Some("b".into()),
        title: "A".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.upsert_page(&PageRec {
        id: "b".into(),
        parent_type: "page_id".into(),
        parent_id: Some("a".into()), // a <-> b cycle
        title: "B".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    })
    .unwrap();
    let store = Arc::new(Mutex::new(s));
    let mut app = App::new(store);
    app.open_page("a");

    let crumb = app.breadcrumb(); // must return promptly, not hang

    assert!(crumb.is_some());
}
