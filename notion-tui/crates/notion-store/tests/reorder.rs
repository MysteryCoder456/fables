use notion_store::{BlockRec, PageRec, Store};

fn store_with_page_and_blocks() -> Store {
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
    s
}

#[test]
fn reordering_an_already_remote_block_enqueues_local_only_op_and_marks_dirty() {
    let mut s = store_with_page_and_blocks();
    s.edit_reorder_block("b2", None, None, 0).unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let b2 = blocks.iter().find(|b| b.id == "b2").unwrap();
    assert_eq!(b2.ordinal, 0);
    assert!(s.is_page_dirty("p1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "reorder_block");
}

#[test]
fn reordering_a_still_pending_insert_patches_its_append_op_instead_of_enqueuing_a_new_one() {
    let mut s = store_with_page_and_blocks();
    let (new_id, _receipt) = s
        .edit_insert_block_after("p1", Some("b1"), "paragraph", "New")
        .unwrap();
    assert_eq!(s.ops().unwrap().len(), 1); // just the append_block op

    s.edit_reorder_block(&new_id, None, None, 0).unwrap();

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1); // still just one op — patched, not appended
    assert_eq!(ops[0].op_type, "append_block");
    let payload: serde_json::Value = serde_json::from_str(&ops[0].payload).unwrap();
    assert_eq!(payload["parent_id"], serde_json::Value::Null);
}

#[test]
fn reorder_to_same_position_is_a_no_op() {
    let mut s = store_with_page_and_blocks();
    s.edit_reorder_block("b1", None, None, 0).unwrap();
    assert!(s.ops().unwrap().is_empty());
    assert!(!s.is_page_dirty("p1").unwrap());
}
