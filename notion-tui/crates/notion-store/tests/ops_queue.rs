use notion_store::Store;

#[test]
fn enqueue_list_update_delete_roundtrip() {
    let s = Store::open_in_memory().unwrap();
    let seq1 = s
        .enqueue_op(
            "update_block",
            "b1",
            r#"{"x":1}"#,
            Some("2026-01-01T00:00:00.000Z"),
        )
        .unwrap();
    let seq2 = s.enqueue_op("delete_row", "r1", "{}", None).unwrap();
    assert!(seq2 > seq1);

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].seq, seq1);
    assert_eq!(ops[0].op_type, "update_block");
    assert_eq!(ops[0].target_id, "b1");
    assert_eq!(
        ops[0].base_edited_time.as_deref(),
        Some("2026-01-01T00:00:00.000Z")
    );
    assert_eq!(ops[0].state, "pending");
    assert_eq!(ops[1].base_edited_time, None);

    assert_eq!(s.pending_count().unwrap(), 2);
    assert!(s.has_ops_for("b1").unwrap());
    assert!(!s.has_ops_for("nope").unwrap());

    s.set_op_state(seq1, "failed", Some("validation_error: bad"))
        .unwrap();
    let ops = s.ops().unwrap();
    assert_eq!(ops[0].state, "failed");
    assert_eq!(ops[0].error.as_deref(), Some("validation_error: bad"));

    s.delete_op(seq1).unwrap();
    s.delete_op(seq2).unwrap();
    assert_eq!(s.pending_count().unwrap(), 0);
    assert!(s.ops().unwrap().is_empty());
}
