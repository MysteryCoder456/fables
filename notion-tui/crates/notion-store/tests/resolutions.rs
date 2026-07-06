use notion_store::{BlockRec, PageRec, ResolvedTarget, Store};

fn store_with_todo() -> (Store, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "local text".into(), has_children: false,
    }]).unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    (s, receipt.op_seq)
}

#[test]
fn retry_resets_failed_op_to_pending() {
    let (s, seq) = store_with_todo();
    s.set_op_state(seq, "failed", Some("boom")).unwrap();
    s.retry_op(seq).unwrap();
    let op = &s.ops().unwrap()[0];
    assert_eq!(op.state, "pending");
    assert!(op.error.is_none());
}

#[test]
fn keep_mine_clears_base_so_push_skips_conflict_check() {
    let (s, seq) = store_with_todo();
    s.set_op_state(seq, "conflicted", None).unwrap();
    s.resolve_keep_mine(seq).unwrap();
    let op = &s.ops().unwrap()[0];
    assert_eq!(op.state, "pending");
    assert!(op.base_edited_time.is_none());
}

#[test]
fn take_theirs_drops_op_clears_dirty_and_names_the_page_to_refetch() {
    let (mut s, seq) = store_with_todo();
    s.set_op_state(seq, "conflicted", None).unwrap();
    let target = s.resolve_take_theirs(seq).unwrap();
    assert!(matches!(target, Some(ResolvedTarget::Page(ref p)) if p == "p1"));
    assert!(s.ops().unwrap().is_empty());
    assert!(!s.is_page_dirty("p1").unwrap());
}

#[test]
fn merge_replaces_conflicted_op_with_fresh_edit_based_on_remote_time() {
    let (mut s, seq) = store_with_todo();
    s.set_op_state(seq, "conflicted", None).unwrap();
    s.resolve_conflict_merge(seq, "b1", "merged text", "t9-remote").unwrap();

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_ne!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "pending");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("t9-remote"));

    let b1 = s.page_blocks("p1").unwrap().into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "merged text");
    assert!(s.is_page_dirty("p1").unwrap());
}
