use notion_store::{PageRec, Store};

fn page(id: &str, parent_type: &str, parent_id: Option<&str>) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: parent_type.into(),
        parent_id: parent_id.map(String::from),
        title: "T".into(),
        icon: None,
        archived: false,
        last_edited_time: "t0".into(),
    }
}

#[test]
fn move_page_reparents_locally_marks_dirty_and_enqueues_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None)).unwrap();
    s.upsert_page(&page("p2", "workspace", None)).unwrap();

    let receipt = s.edit_move_page("p1", "p2").unwrap();

    let moved = s.get_page("p1").unwrap().unwrap();
    assert_eq!(moved.parent_type, "page_id");
    assert_eq!(moved.parent_id.as_deref(), Some("p2"));
    assert!(s.is_page_dirty("p1").unwrap());
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "move_page");
    let _ = receipt;
}
