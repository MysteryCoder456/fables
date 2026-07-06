use notion_store::{CommentRec, Store};

fn rec(id: &str, body: &str) -> CommentRec {
    CommentRec {
        id: id.into(), parent_id: "p1".into(), parent_kind: "page".into(),
        thread_id: Some("d1".into()), author: "u1".into(), body: body.into(),
        created_time: "2026-07-06T10:00:00.000Z".into(),
    }
}

#[test]
fn replace_and_read_comments() {
    let s = Store::open_in_memory().unwrap();
    s.replace_comments("p1", &[rec("c1", "first"), rec("c2", "second")]).unwrap();
    let got = s.comments_for("p1").unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].body, "first");

    // Replace is idempotent and removes stale rows for the same parent.
    s.replace_comments("p1", &[rec("c1", "first")]).unwrap();
    assert_eq!(s.comments_for("p1").unwrap().len(), 1);
}

#[test]
fn add_comment_inserts_locally_and_enqueues_op() {
    let mut s = Store::open_in_memory().unwrap();
    let tmp_id = s.edit_add_comment("p1", "page", None, "hello there").unwrap();
    assert!(tmp_id.starts_with("tmp-"));

    let got = s.comments_for("p1").unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].body, "hello there");
    assert_eq!(got[0].author, "me");

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "create_comment");
    assert_eq!(ops[0].target_id, tmp_id);
    let payload: serde_json::Value = serde_json::from_str(&ops[0].payload).unwrap();
    assert_eq!(payload["parent_id"], "p1");
    assert_eq!(payload["parent_kind"], "page");
    assert_eq!(payload["body"], "hello there");
}

#[test]
fn rewrite_comment_id_updates_row_and_pending_ops() {
    let mut s = Store::open_in_memory().unwrap();
    let tmp_id = s.edit_add_comment("p1", "page", None, "x").unwrap();
    s.rewrite_comment_id(&tmp_id, "real-c1").unwrap();
    assert_eq!(s.comments_for("p1").unwrap()[0].id, "real-c1");
    assert_eq!(s.ops().unwrap()[0].target_id, "real-c1");
}
