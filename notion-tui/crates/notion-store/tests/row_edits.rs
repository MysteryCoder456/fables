use notion_store::{BlockRec, DataSourceRec, RowRec, Store};
use serde_json::{json, Value};

fn ds() -> DataSourceRec {
    DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: json!({"Name": {"type": "title"}, "Done": {"type": "checkbox"}}).to_string(),
        last_edited_time: "t".into(),
    }
}

fn setup() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&ds()).unwrap();
    s.replace_rows(
        "ds1",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Buy milk"}]},
                "Done": {"type": "checkbox", "checkbox": false}
            })
            .to_string(),
            last_edited_time: "2026-01-01T00:00:00.000Z".into(),
            archived: false,
        }],
    )
    .unwrap();
    s
}

#[test]
fn create_row_inserts_dirty_row_and_queues_op() {
    let mut s = setup();
    let props = json!({"Name": {"type": "title", "title": [{"plain_text": "New task"}]}});
    let (row_id, receipt) = s.edit_create_row("ds1", props.clone()).unwrap();

    assert!(row_id.starts_with("tmp-"));
    let rows = s.rows("ds1").unwrap();
    let created = rows.iter().find(|r| r.id == row_id).unwrap();
    let stored: Value = serde_json::from_str(&created.properties).unwrap();
    assert_eq!(stored["Name"]["title"][0]["plain_text"], "New task");
    assert!(s.is_row_dirty(&row_id).unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "create_row");
    assert_eq!(ops[0].target_id, row_id);
    assert_eq!(ops[0].seq, receipt.op_seq);
    let op_payload: Value = serde_json::from_str(&ops[0].payload).unwrap();
    assert_eq!(op_payload["data_source_id"], "ds1");
}

#[test]
fn update_row_merges_properties_and_queues_op() {
    let mut s = setup();
    let patch = json!({"Done": {"type": "checkbox", "checkbox": true}});
    s.edit_update_row("r1", patch.clone()).unwrap();

    let rows = s.rows("ds1").unwrap();
    let r1 = rows.iter().find(|r| r.id == "r1").unwrap();
    let stored: Value = serde_json::from_str(&r1.properties).unwrap();
    assert_eq!(stored["Done"]["checkbox"], true);
    // Untouched property survives the merge.
    assert_eq!(stored["Name"]["title"][0]["plain_text"], "Buy milk");
    assert!(s.is_row_dirty("r1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "update_row");
    assert_eq!(ops[0].target_id, "r1");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("2026-01-01T00:00:00.000Z"));
}

#[test]
fn delete_row_archives_and_queues_op() {
    let mut s = setup();
    s.edit_delete_row("r1").unwrap();

    // rows() filters out archived rows.
    assert!(s.rows("ds1").unwrap().is_empty());
    assert!(s.is_row_dirty("r1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "delete_row");
    assert_eq!(ops[0].target_id, "r1");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("2026-01-01T00:00:00.000Z"));
}

#[test]
fn rewrite_row_id_updates_row_and_pending_ops() {
    let mut s = setup();
    let props = json!({"Name": {"type": "title", "title": [{"plain_text": "New task"}]}});
    let (tmp_id, _receipt) = s.edit_create_row("ds1", props).unwrap();
    // A second edit queued against the still-temp id before the create has pushed.
    s.edit_update_row(&tmp_id, json!({"Done": {"type": "checkbox", "checkbox": true}})).unwrap();

    s.rewrite_row_id(&tmp_id, "real-row-id").unwrap();

    let rows = s.rows("ds1").unwrap();
    assert!(rows.iter().any(|r| r.id == "real-row-id"));
    assert!(!rows.iter().any(|r| r.id == tmp_id));

    let ops = s.ops().unwrap();
    assert!(ops.iter().all(|o| o.target_id != tmp_id));
    assert!(ops.iter().any(|o| o.target_id == "real-row-id" && o.op_type == "update_row"));
}

#[test]
fn rewrite_block_id_updates_block_children_and_pending_ops() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&notion_store::PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
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
            plain_text: "one".into(),
            has_children: false,
        }],
    )
    .unwrap();
    let (tmp_id, _) = s.edit_insert_block_after("p1", Some("b1"), "paragraph", "child soon").unwrap();

    s.rewrite_block_id(&tmp_id, "real-block-id").unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    assert!(blocks.iter().any(|b| b.id == "real-block-id"));
    assert!(!blocks.iter().any(|b| b.id == tmp_id));
    let ops = s.ops().unwrap();
    assert!(ops.iter().any(|o| o.target_id == "real-block-id"));
}
