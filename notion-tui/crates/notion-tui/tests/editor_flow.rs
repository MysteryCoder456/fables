use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_page() -> notion_sync::SharedStore {
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
            plain_text: "First".into(),
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
        s.replace_page_blocks(
            "p1",
            &[BlockRec {
                id: "tg1".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 0,
                block_type: "toggle".into(),
                payload: "{}".into(),
                plain_text: "More".into(),
                has_children: false,
            }],
        )
        .unwrap();
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

#[test]
fn editor_failure_is_surfaced_in_notice() {
    let store = store_with_page();
    let mut app = app_on_page(store);

    app.edit_in_editor(|_initial| anyhow::bail!("editor exited with signal 9"));

    assert!(app.notice.as_deref().unwrap_or("").contains("editor failed"));
    assert!(app.notice.as_deref().unwrap().contains("signal 9"));
}

fn store_with_two_paragraphs() -> notion_sync::SharedStore {
    let store = store_with_page();
    let mut s = store.lock().unwrap();
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
    drop(s);
    store
}

#[test]
fn successful_edit_reports_a_change_summary() {
    let store = store_with_two_paragraphs();
    let mut app = app_on_page(store);

    app.edit_in_editor(|initial| Ok(format!("{initial}\nbrand new line")));

    let n = app.notice.as_deref().unwrap_or("");
    assert!(n.contains("1 added"), "notice was: {n}");
}

#[test]
fn unclosed_fence_asks_for_confirmation_and_discard_keeps_page_intact() {
    let store = store_with_two_paragraphs();
    let mut app = app_on_page(store.clone());

    app.edit_in_editor(|initial| Ok(format!("{initial}\n```\ntrailing")));

    assert!(
        app.confirm.is_some(),
        "must ask before applying a fence-swallowing edit"
    );
    // Say no:
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('n')));
    assert!(app.confirm.is_none());
    assert_eq!(app.notice.as_deref(), Some("edit discarded"));
    // Store unchanged: no pending ops were enqueued.
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 0);
}

#[test]
fn unclosed_fence_confirm_yes_applies_the_edit() {
    let store = store_with_two_paragraphs();
    let mut app = app_on_page(store.clone());

    app.edit_in_editor(|initial| Ok(format!("{initial}\n```\ntrailing")));

    assert!(app.confirm.is_some());
    let msg = app.confirm.as_ref().unwrap().message.clone();
    assert!(
        msg.contains("unclosed code fence at line 3"),
        "message was: {msg}"
    );

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));
    assert!(app.confirm.is_none());
    let n = app.notice.as_deref().unwrap_or("");
    assert!(n.contains("1 added"), "notice was: {n}");
    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[2].block_type, "code");
}

#[test]
fn no_op_edit_reports_no_changes() {
    let store = store_with_two_paragraphs();
    let mut app = app_on_page(store);

    app.edit_in_editor(|initial| Ok(initial.to_string()));

    assert_eq!(app.notice.as_deref(), Some("no changes"));
}
