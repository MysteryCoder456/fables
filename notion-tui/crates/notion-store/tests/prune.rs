use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec, Store};

fn page(id: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "T".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    }
}

#[test]
fn prune_missing_removes_pages_and_data_sources_absent_from_the_seen_set() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.upsert_page(&page("p2")).unwrap(); // will be "deleted remotely"
    s.replace_page_blocks(
        "p2",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p2".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
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
        &[RowRec {
            id: "r1".into(),
            data_source_id: "ds1".into(),
            properties: "{}".into(),
            last_edited_time: "t".into(),
            archived: false,
        }],
    )
    .unwrap();

    let (removed_pages, removed_ds) = s.prune_missing(&["p1".to_string()], &[]).unwrap();

    assert_eq!(removed_pages, 1);
    assert_eq!(removed_ds, 1);
    assert!(s.get_page("p1").unwrap().is_some());
    assert!(s.get_page("p2").unwrap().is_none());
    assert!(
        s.page_blocks("p2").unwrap().is_empty(),
        "orphaned blocks must be cleaned up"
    );
    assert!(s.get_data_source("ds1").unwrap().is_none());
    assert!(
        s.rows("ds1").unwrap().is_empty(),
        "orphaned rows must be cleaned up"
    );
}

#[test]
fn prune_missing_never_removes_a_page_with_a_pending_local_edit() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
    s.edit_delete_block("b1").unwrap(); // queues a pending_ops row targeting b1...
                                        // ...but the guard checks pending_ops against the *page* id for page-level ops too:
    s.edit_rename_page("p1", "Renamed").unwrap();

    let (removed_pages, _) = s.prune_missing(&[], &[]).unwrap();

    assert_eq!(
        removed_pages, 0,
        "a page with pending ops must survive even if absent from `seen`"
    );
    assert!(s.get_page("p1").unwrap().is_some());
}

#[test]
fn prune_missing_never_removes_a_data_source_with_a_pending_row_edit() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    // Creates a local row under a temp id and enqueues a `create_row` pending op
    // whose target_id is that temp row id, not ds1's id — but the row belongs to ds1.
    // A stale/partial crawl that omits ds1 from `seen_ds` (racing the create) must
    // not delete ds1 (and cascade-orphan the pending temp row) out from under it.
    let another_ds = "ds-other".to_string();
    s.upsert_data_source(&DataSourceRec {
        id: another_ds.clone(),
        database_id: "db2".into(),
        title: "Other".into(),
        schema_json: "{}".into(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    let (_row_id, _receipt) = s
        .edit_create_row("ds1", serde_json::json!({"Name": "x"}))
        .unwrap();

    // ds1 absent from seen; ds-other present (so this isn't the empty-seen circuit breaker case).
    let (_, removed_ds) = s.prune_missing(&[], std::slice::from_ref(&another_ds)).unwrap();

    assert_eq!(
        removed_ds, 0,
        "a data source with an in-flight row edit must survive even if absent from `seen`"
    );
    assert!(s.get_data_source("ds1").unwrap().is_some());
}

#[test]
fn prune_missing_never_removes_a_page_with_only_a_pending_block_level_edit() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.upsert_page(&page("p-other")).unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
    // Only a block-level op (target_id = b1, not p1) is pending — no page-level op.
    s.edit_delete_block("b1").unwrap();

    // p1 absent from seen; p-other present (so this isn't the empty-seen circuit breaker case).
    let (removed_pages, _) = s.prune_missing(&["p-other".to_string()], &[]).unwrap();

    assert_eq!(
        removed_pages, 0,
        "a page with only a pending block-level edit must survive even if absent from `seen`"
    );
    assert!(s.get_page("p1").unwrap().is_some());
}

#[test]
fn prune_missing_skips_pruning_when_seen_is_empty_but_store_is_not() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t".into(),
    })
    .unwrap();

    // An empty seen set on both fronts while the local store is non-empty looks like
    // an anomalous/incomplete crawl, not a genuinely empty workspace — must not wipe
    // the whole local cache.
    let (removed_pages, removed_ds) = s.prune_missing(&[], &[]).unwrap();

    assert_eq!(removed_pages, 0);
    assert_eq!(removed_ds, 0);
    assert!(s.get_page("p1").unwrap().is_some());
    assert!(s.get_data_source("ds1").unwrap().is_some());
}

#[test]
fn forget_page_deletes_the_page_and_its_blocks() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();

    let forgot = s.forget_page("p1").unwrap();

    assert!(
        forgot,
        "no pending ops => forget_page must delete and return true"
    );
    assert!(s.get_page("p1").unwrap().is_none());
    assert!(s.page_blocks("p1").unwrap().is_empty());
}

#[test]
fn forget_page_skips_a_page_with_a_pending_rename_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.edit_rename_page("p1", "Renamed").unwrap(); // queues a rename_page op targeting p1

    let forgot = s.forget_page("p1").unwrap();

    assert!(
        !forgot,
        "a page with a pending local edit must survive a false-positive liveness check"
    );
    assert!(s.get_page("p1").unwrap().is_some());
}

#[test]
fn forget_page_skips_a_page_with_only_a_pending_block_level_edit() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.replace_page_blocks(
        "p1",
        &[BlockRec {
            id: "b1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "paragraph".into(),
            payload: "{}".into(),
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
    s.edit_delete_block("b1").unwrap(); // target_id = b1, not p1

    let forgot = s.forget_page("p1").unwrap();

    assert!(
        !forgot,
        "a pending block-level edit on the page must also guard it"
    );
    assert!(s.get_page("p1").unwrap().is_some());
}
