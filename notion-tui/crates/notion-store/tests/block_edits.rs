use notion_store::{BlockRec, PageRec, Store};
use serde_json::Value;

fn page(id: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "2026-01-01T00:00:00.000Z".into(),
    }
}

fn setup() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.replace_page_blocks(
        "p1",
        &[
            BlockRec {
                id: "b1".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 0,
                block_type: "to_do".into(),
                payload: r#"{"checked": false}"#.into(),
                plain_text: "Buy milk".into(),
                has_children: false,
            },
            BlockRec {
                id: "b2".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 1,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "Second block".into(),
                has_children: false,
            },
        ],
    )
    .unwrap();
    s
}

#[test]
fn toggle_todo_flips_checked_and_queues_op() {
    let mut s = setup();
    let receipt = s.edit_toggle_todo("b1").unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    let payload: Value = serde_json::from_str(&b1.payload).unwrap();
    assert_eq!(payload["checked"], true);
    assert!(s.is_page_dirty("p1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].seq, receipt.op_seq);
    assert_eq!(ops[0].op_type, "update_block");
    assert_eq!(ops[0].target_id, "b1");
    assert_eq!(
        ops[0].base_edited_time.as_deref(),
        Some("2026-01-01T00:00:00.000Z")
    );
}

#[test]
fn update_block_text_changes_payload_and_plain_text() {
    let mut s = setup();
    s.edit_update_block_text("b2", "Updated text").unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let b2 = blocks.iter().find(|b| b.id == "b2").unwrap();
    assert_eq!(b2.plain_text, "Updated text");
    let payload: Value = serde_json::from_str(&b2.payload).unwrap();
    assert_eq!(payload["rich_text"][0]["plain_text"], "Updated text");

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "update_block");
    assert_eq!(ops[0].target_id, "b2");
}

#[test]
fn insert_block_after_shifts_siblings_and_queues_append() {
    let mut s = setup();
    let (new_id, receipt) = s
        .edit_insert_block_after("p1", Some("b1"), "paragraph", "New block")
        .unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let mut by_id: std::collections::HashMap<&str, &BlockRec> =
        blocks.iter().map(|b| (b.id.as_str(), b)).collect();
    assert_eq!(by_id.remove("b1").unwrap().ordinal, 0);
    assert_eq!(by_id.remove(new_id.as_str()).unwrap().ordinal, 1);
    assert_eq!(by_id.remove("b2").unwrap().ordinal, 2);
    assert!(new_id.starts_with("tmp-"));
    assert!(s.is_page_dirty("p1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "append_block");
    assert_eq!(ops[0].target_id, new_id);
    assert_eq!(ops[0].seq, receipt.op_seq);
}

#[test]
fn delete_block_removes_row_and_queues_op() {
    let mut s = setup();
    s.edit_delete_block("b2").unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].id, "b1");
    assert!(s.is_page_dirty("p1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "delete_block");
    assert_eq!(ops[0].target_id, "b2");
}

#[test]
fn undo_still_pending_removes_op_and_reverts_locally() {
    let mut s = setup();
    let receipt = s.edit_toggle_todo("b1").unwrap();
    s.undo(receipt).unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    let payload: Value = serde_json::from_str(&b1.payload).unwrap();
    assert_eq!(payload["checked"], false);
    assert!(s.ops().unwrap().is_empty());
}

#[test]
fn undo_already_pushed_enqueues_inverse_edit() {
    let mut s = setup();
    let receipt = s.edit_toggle_todo("b1").unwrap();
    // Simulate the pusher having already drained the op successfully.
    s.delete_op(receipt.op_seq).unwrap();

    s.undo(receipt).unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    let payload: Value = serde_json::from_str(&b1.payload).unwrap();
    assert_eq!(payload["checked"], false);

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "update_block");
    assert_eq!(ops[0].target_id, "b1");
}
