use std::sync::{Arc, Mutex};

use notion_store::{PageRec, Store};
use notion_tui::app::{App, View};

fn store_with_page() -> notion_sync::SharedStore {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "T".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    })
    .unwrap();
    Arc::new(Mutex::new(s))
}

#[test]
fn page_gone_forgets_the_page_and_shows_a_notice() {
    let store = store_with_page();
    let mut app = App::new(store.clone());
    app.open_page("p1");
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));

    app.handle_page_gone("p1");

    assert!(store.lock().unwrap().get_page("p1").unwrap().is_none());
    assert!(app
        .notice
        .as_deref()
        .unwrap_or("")
        .contains("deleted or unshared"));
    assert!(
        matches!(app.view, View::Empty),
        "no history to fall back to, so the view clears"
    );
}

#[test]
fn page_gone_falls_back_to_history_when_available() {
    let store = store_with_page();
    {
        let s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "p2".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Second".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        })
        .unwrap();
    }
    let mut app = App::new(store.clone());
    app.open_page("p1");
    app.history.push("p1".to_string());
    app.open_page("p2");

    app.handle_page_gone("p2");

    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));
}

#[test]
fn page_gone_does_not_drop_a_page_with_a_pending_local_edit() {
    let store = store_with_page();
    {
        let mut s = store.lock().unwrap();
        s.edit_rename_page("p1", "Renamed locally").unwrap(); // unpushed edit
    }
    let mut app = App::new(store.clone());
    app.open_page("p1");

    app.handle_page_gone("p1"); // racy false-positive 404/archived

    assert!(
        store.lock().unwrap().get_page("p1").unwrap().is_some(),
        "a page with a pending local edit must survive a racy liveness-check 404"
    );
    assert!(
        matches!(&app.view, View::Page(v) if v.page.id == "p1"),
        "must not navigate away from a page that was kept"
    );
}
