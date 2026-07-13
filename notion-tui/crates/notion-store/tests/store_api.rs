use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec, Store};

fn page(id: &str, parent_type: &str, parent_id: Option<&str>, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: parent_type.into(),
        parent_id: parent_id.map(Into::into),
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    }
}

fn block(id: &str, page_id: &str, ordinal: i64, text: &str) -> BlockRec {
    BlockRec {
        id: id.into(),
        page_id: page_id.into(),
        parent_block_id: None,
        ordinal,
        block_type: "paragraph".into(),
        payload: "{}".into(),
        plain_text: text.into(),
        has_children: false,
    }
}

#[test]
fn page_roundtrip_and_upsert_overwrites() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None, "Old title"))
        .unwrap();
    s.upsert_page(&page("p1", "workspace", None, "New title"))
        .unwrap();
    let got = s.get_page("p1").unwrap().unwrap();
    assert_eq!(got.title, "New title");
}

#[test]
fn replace_page_blocks_swaps_content() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None, "P")).unwrap();
    s.replace_page_blocks("p1", &[block("b1", "p1", 0, "one")])
        .unwrap();
    s.replace_page_blocks("p1", &[block("b2", "p1", 0, "two")])
        .unwrap();
    let blocks = s.page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].id, "b2");
    // FTS reflects the replacement.
    assert!(s.search("one").unwrap().is_empty());
    assert_eq!(s.search("two").unwrap()[0].page_id, "p1");
}

#[test]
fn sidebar_excludes_rows_and_archived() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None, "Top")).unwrap();
    s.upsert_page(&page("p2", "page_id", Some("p1"), "Child"))
        .unwrap();
    s.upsert_page(&page("r1", "data_source_id", Some("ds1"), "Row page"))
        .unwrap();
    let mut archived = page("p3", "workspace", None, "Gone");
    archived.archived = true;
    s.upsert_page(&archived).unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t1".into(),
    })
    .unwrap();

    let nodes = s.sidebar_nodes().unwrap();
    let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"p1") && ids.contains(&"p2") && ids.contains(&"ds1"));
    assert!(!ids.contains(&"r1") && !ids.contains(&"p3"));
}

#[test]
fn get_data_source_by_database_id_finds_by_database_id_not_data_source_id() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t1".into(),
    })
    .unwrap();

    let got = s.get_data_source_by_database_id("db1").unwrap().unwrap();
    assert_eq!(got.id, "ds1");
    assert!(s.get_data_source_by_database_id("ds1").unwrap().is_none());
}

#[test]
fn rows_and_meta_roundtrip() {
    let mut s = Store::open_in_memory().unwrap();
    s.replace_rows(
        "ds1",
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: r#"{"Name":{"type":"title","title":[{"plain_text":"Buy milk"}]}}"#.into(),
            last_edited_time: "t1".into(),
            archived: false,
        }],
    )
    .unwrap();
    assert_eq!(s.rows("ds1").unwrap().len(), 1);

    assert_eq!(s.meta_get("hwm").unwrap(), None);
    s.meta_set("hwm", "2026-07-05").unwrap();
    s.meta_set("hwm", "2026-07-06").unwrap();
    assert_eq!(s.meta_get("hwm").unwrap().as_deref(), Some("2026-07-06"));
}
