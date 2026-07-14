use notion_store::{OpRec, Store};
use serde_json::Value;

/// A human-readable rendering of a pending op. `summary` is what the queue
/// shows; the structured fields exist for M10's conflict detail view.
pub struct OpDescription {
    /// One-line human summary, e.g. "edit ¶ in 'Meeting Notes'" or "set Status on 'Q3 Launch'".
    pub summary: String,
    /// Title of the page / row / comment parent the op targets, when resolvable from the store.
    pub target_title: Option<String>,
    /// The property name (row ops), block-text snippet (block ops), or comment-body snippet.
    pub detail: Option<String>,
}

fn short_id(id: &str) -> String {
    let mut s: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        s.push('…');
    }
    s
}

fn snippet(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let mut s: String = text.chars().take(24).collect();
    if text.chars().count() > 24 {
        s.push('…');
    }
    Some(s)
}

fn title_from_props(props: &Value) -> Option<String> {
    props
        .as_object()?
        .values()
        .find(|p| p["type"] == "title")
        .map(crate::ui::table::cell_text)
        .filter(|t| !t.is_empty())
}

fn row_title(store: &Store, row_id: &str) -> Option<String> {
    let row = store.get_row(row_id).ok().flatten()?;
    let props: Value = serde_json::from_str(&row.properties).ok()?;
    title_from_props(&props)
}

fn block_snippet(store: &Store, page_id: &str, block_id: &str) -> Option<String> {
    let text = store
        .page_blocks(page_id)
        .ok()?
        .into_iter()
        .find(|b| b.id == block_id)?
        .plain_text;
    snippet(&text)
}

pub fn describe_op(op: &OpRec, store: &Store) -> OpDescription {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    match op.op_type.as_str() {
        "update_block" | "append_block" | "delete_block" | "reorder_block" => {
            let verb = match op.op_type.as_str() {
                "update_block" => "edit",
                "append_block" => "add",
                "delete_block" => "delete",
                _ => "move",
            };
            let page_id = payload["page_id"].as_str().unwrap_or("");
            let target_title = store.get_page(page_id).ok().flatten().map(|p| p.title);
            let shown = target_title.clone().unwrap_or_else(|| short_id(page_id));
            OpDescription {
                summary: format!("{verb} ¶ in '{shown}'"),
                target_title,
                detail: block_snippet(store, page_id, &op.target_id),
            }
        }
        "create_row" => {
            let target_title = title_from_props(&payload["properties"]);
            let ds_id = payload["data_source_id"].as_str().unwrap_or("");
            let ds_title = store
                .get_data_source(ds_id)
                .ok()
                .flatten()
                .map(|d| d.title)
                .unwrap_or_else(|| short_id(ds_id));
            let shown = target_title.clone().unwrap_or_else(|| "untitled".into());
            OpDescription {
                summary: format!("add row '{shown}' to '{ds_title}'"),
                target_title,
                detail: None,
            }
        }
        "update_row" => {
            let target_title = row_title(store, &op.target_id);
            let prop = payload["properties"]
                .as_object()
                .and_then(|m| m.keys().next().cloned());
            let shown = target_title.clone().unwrap_or_else(|| short_id(&op.target_id));
            let summary = match &prop {
                Some(p) => format!("set {p} on '{shown}'"),
                None => format!("edit '{shown}'"),
            };
            OpDescription {
                summary,
                target_title,
                detail: prop,
            }
        }
        "delete_row" | "restore_row" => {
            let verb = if op.op_type == "delete_row" {
                "delete"
            } else {
                "restore"
            };
            let target_title = row_title(store, &op.target_id);
            let shown = target_title.clone().unwrap_or_else(|| short_id(&op.target_id));
            OpDescription {
                summary: format!("{verb} row '{shown}'"),
                target_title,
                detail: None,
            }
        }
        "create_comment" => {
            let parent_id = payload["parent_id"].as_str().unwrap_or("");
            let target_title = store
                .get_page(parent_id)
                .ok()
                .flatten()
                .map(|p| p.title)
                .or_else(|| row_title(store, parent_id));
            let shown = target_title.clone().unwrap_or_else(|| short_id(parent_id));
            OpDescription {
                summary: format!("comment on '{shown}'"),
                target_title,
                detail: payload["body"].as_str().and_then(snippet),
            }
        }
        other => {
            // M7 adds rename_page/move_page; future ops land here too.
            let target_title = store.get_page(&op.target_id).ok().flatten().map(|p| p.title);
            let shown = target_title.clone().unwrap_or_else(|| short_id(&op.target_id));
            OpDescription {
                summary: format!("{} on '{shown}'", other.replace('_', " ")),
                target_title,
                detail: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{BlockRec, OpRec, PageRec, Store};
    use serde_json::json;

    fn store_with_page() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Meeting Notes".into(),
            icon: None,
            archived: false,
            last_edited_time: "t1".into(),
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
                plain_text: "agenda item one".into(),
                has_children: false,
            }],
        )
        .unwrap();
        s
    }

    #[test]
    fn update_block_op_names_page_and_snippets_block_text() {
        let mut s = store_with_page();
        s.edit_update_block_text("b1", "agenda item one v2").unwrap();
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "edit ¶ in 'Meeting Notes'");
        assert_eq!(d.target_title.as_deref(), Some("Meeting Notes"));
        assert!(d.detail.as_deref().unwrap().starts_with("agenda item one"));
    }

    #[test]
    fn update_row_op_names_property_and_row_title() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn()
            .execute(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived, dirty)
                 VALUES ('r1', 'ds1', ?1, 't1', 0, 0)",
                [json!({
                    "Name": {"type": "title", "title": [{"plain_text": "Q3 Launch"}]},
                    "Status": {"type": "status", "status": {"name": "Todo"}}
                })
                .to_string()],
            )
            .unwrap();
        s.edit_update_row(
            "r1",
            json!({"Status": {"type": "status", "status": {"name": "Doing"}}}),
        )
        .unwrap();
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "set Status on 'Q3 Launch'");
        assert_eq!(d.detail.as_deref(), Some("Status"));
    }

    #[test]
    fn create_comment_op_names_the_parent_page() {
        let mut s = store_with_page();
        s.edit_add_comment("p1", "page", None, "looks good to me")
            .unwrap();
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "comment on 'Meeting Notes'");
        assert_eq!(d.detail.as_deref(), Some("looks good to me"));
    }

    #[test]
    fn unknown_op_type_falls_back_to_short_id_never_full_uuid() {
        let s = Store::open_in_memory().unwrap();
        let op = OpRec {
            seq: 1,
            op_type: "rename_page".into(),
            target_id: "0123456789abcdef0123".into(),
            payload: "{}".into(),
            base_edited_time: None,
            state: "pending".into(),
            error: None,
        };
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "rename page on '01234567…'");
        assert!(d.target_title.is_none());
    }

    #[test]
    fn delete_row_op_resolves_title_of_archived_row() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn()
            .execute(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived, dirty)
                 VALUES ('r1', 'ds1', ?1, 't1', 0, 0)",
                [json!({"Name": {"type": "title", "title": [{"plain_text": "Q3 Launch"}]}}).to_string()],
            )
            .unwrap();
        s.edit_delete_row("r1").unwrap(); // archives the row
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "delete row 'Q3 Launch'");
    }
}
