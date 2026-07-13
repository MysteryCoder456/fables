use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};

fn store_with_conflicted_op() -> (notion_sync::SharedStore, i64) {
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
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    s.set_op_state(receipt.op_seq, "conflicted", None).unwrap();
    (Arc::new(Mutex::new(s)), receipt.op_seq)
}

fn shift_q() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT)
}
fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}

#[test]
fn q_opens_queue_listing_ops_and_q_returns_to_previous_view() {
    let (store, _) = store_with_conflicted_op();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("p1");

    dispatch_key(&mut app, shift_q());
    match &app.view {
        View::Queue(q) => {
            assert_eq!(q.ops.len(), 1);
            assert_eq!(q.ops[0].state, "conflicted");
        }
        _ => panic!("expected queue view"),
    }
    dispatch_key(&mut app, shift_q());
    assert!(matches!(app.view, View::Page(_)));
}

#[test]
fn keep_mine_from_queue_clears_base_and_repends_op() {
    let (store, seq) = store_with_conflicted_op();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    dispatch_key(&mut app, shift_q());

    dispatch_key(&mut app, key('p'));

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "pending");
    assert!(ops[0].base_edited_time.is_none());
    // Screen refreshed in place:
    if let View::Queue(q) = &app.view {
        assert_eq!(q.ops[0].state, "pending");
    } else {
        panic!("still on queue view");
    }
}

#[test]
fn take_theirs_from_queue_deletes_op() {
    let (store, _) = store_with_conflicted_op();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    dispatch_key(&mut app, shift_q());

    dispatch_key(&mut app, key('t'));

    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
