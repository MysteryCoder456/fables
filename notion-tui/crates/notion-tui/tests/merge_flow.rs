use std::sync::{Arc, Mutex};

use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{App, AppMsg};

fn store_with_conflicted_op() -> (notion_sync::SharedStore, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "orig".into(), has_children: false,
    }]).unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    s.set_op_state(receipt.op_seq, "conflicted", None).unwrap();
    (Arc::new(Mutex::new(s)), receipt.op_seq)
}

#[test]
fn merge_editor_shows_both_versions_and_applies_the_result() {
    let (store, seq) = store_with_conflicted_op();
    let mut app = App::new(store.clone());

    let msg = AppMsg::MergeReady {
        op_seq: seq,
        block_id: "b1".into(),
        local_text: "local edit".into(),
        remote_text: "remote edit".into(),
        remote_edited_time: "t9".into(),
    };
    app.open_merge_editor(msg, |initial| {
        assert!(initial.contains("<<<<<<< local"));
        assert!(initial.contains("local edit"));
        assert!(initial.contains("remote edit"));
        assert!(initial.contains(">>>>>>> remote"));
        Ok("merged result".to_string())
    });

    let s = store.lock().unwrap();
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_ne!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "pending");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("t9"));
    let b1 = s.page_blocks("p1").unwrap().into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "merged result");
}

#[test]
fn merge_editor_abort_leaves_op_conflicted() {
    let (store, seq) = store_with_conflicted_op();
    let mut app = App::new(store.clone());
    let msg = AppMsg::MergeReady {
        op_seq: seq, block_id: "b1".into(), local_text: "l".into(),
        remote_text: "r".into(), remote_edited_time: "t9".into(),
    };
    app.open_merge_editor(msg, |_| anyhow::bail!("editor aborted"));
    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "conflicted");
}
