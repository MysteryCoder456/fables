use notion_store::{DataSourceRec, PageRec, RowRec, Store};
use serde_json::{json, Value};

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    }
}

#[test]
fn upsert_page_skips_metadata_when_dirty() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Original title")).unwrap();
    s.conn()
        .execute("UPDATE pages SET dirty = 1 WHERE id = 'p1'", [])
        .unwrap();

    s.upsert_page(&page("p1", "Remote renamed title")).unwrap();

    let got = s.get_page("p1").unwrap().unwrap();
    assert_eq!(got.title, "Original title");
}

#[test]
fn upsert_page_applies_metadata_when_not_dirty() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Original title")).unwrap();

    s.upsert_page(&page("p1", "Remote renamed title")).unwrap();

    let got = s.get_page("p1").unwrap().unwrap();
    assert_eq!(got.title, "Remote renamed title");
}

#[test]
fn replace_rows_preserves_dirty_row_but_updates_others() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    s.replace_rows(
        "ds1",
        &[
            RowRec {
                id: "r1".into(),
                data_source_id: "ds1".into(),
                properties: json!({"Name": "old r1"}).to_string(),
                last_edited_time: "t1".into(),
                archived: false,
            },
            RowRec {
                id: "r2".into(),
                data_source_id: "ds1".into(),
                properties: json!({"Name": "old r2"}).to_string(),
                last_edited_time: "t1".into(),
                archived: false,
            },
        ],
    )
    .unwrap();

    // Local edit dirties r1 before the next remote pull lands.
    s.edit_update_row("r1", json!({"Name": "locally edited r1"}))
        .unwrap();

    // Simulated remote pull: r1 has different (stale, pre-edit) data, r2 has a genuine update,
    // and a brand-new row r3 shows up.
    s.replace_rows(
        "ds1",
        &[
            RowRec {
                id: "r1".into(),
                data_source_id: "ds1".into(),
                properties: json!({"Name": "remote r1 (stale)"}).to_string(),
                last_edited_time: "t2".into(),
                archived: false,
            },
            RowRec {
                id: "r2".into(),
                data_source_id: "ds1".into(),
                properties: json!({"Name": "remote r2 updated"}).to_string(),
                last_edited_time: "t2".into(),
                archived: false,
            },
            RowRec {
                id: "r3".into(),
                data_source_id: "ds1".into(),
                properties: json!({"Name": "new r3"}).to_string(),
                last_edited_time: "t2".into(),
                archived: false,
            },
        ],
    )
    .unwrap();

    let rows = s.rows("ds1").unwrap();
    let by_id: std::collections::HashMap<&str, &RowRec> = rows.iter().map(|r| (r.id.as_str(), r)).collect();

    let r1_props: Value = serde_json::from_str(&by_id["r1"].properties).unwrap();
    assert_eq!(r1_props["Name"], "locally edited r1");
    let r2_props: Value = serde_json::from_str(&by_id["r2"].properties).unwrap();
    assert_eq!(r2_props["Name"], "remote r2 updated");
    let r3_props: Value = serde_json::from_str(&by_id["r3"].properties).unwrap();
    assert_eq!(r3_props["Name"], "new r3");
}
