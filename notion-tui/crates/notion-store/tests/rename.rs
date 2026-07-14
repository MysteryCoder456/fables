use notion_store::{PageRec, Store};

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t0".into(),
    }
}

#[test]
fn rename_page_updates_title_marks_dirty_and_enqueues_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Old Title")).unwrap();

    let receipt = s.edit_rename_page("p1", "New Title").unwrap();

    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "New Title");
    assert!(s.is_page_dirty("p1").unwrap());
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "rename_page");
    assert_eq!(ops[0].target_id, "p1");
    assert_eq!(receipt.op_seq, ops[0].seq);
}

#[test]
fn undo_rename_restores_old_title() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Old Title")).unwrap();
    let receipt = s.edit_rename_page("p1", "New Title").unwrap();

    s.undo(receipt).unwrap();

    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "Old Title");
    assert!(s.ops().unwrap().is_empty());
}
