use notion_store::BlockRec;

pub struct Unit {
    pub id: String,
    pub depth: usize,
    pub block_type: String,
    pub text: String,
    pub protected: bool,
}

const EXPRESSIBLE: &[&str] = &[
    "paragraph",
    "heading_1",
    "heading_2",
    "heading_3",
    "to_do",
    "bulleted_list_item",
    "numbered_list_item",
    "quote",
    "divider",
    "code",
];

fn is_expressible(block_type: &str) -> bool {
    EXPRESSIBLE.contains(&block_type)
}

fn render_line(b: &BlockRec) -> String {
    let payload: serde_json::Value = serde_json::from_str(&b.payload).unwrap_or_default();
    match b.block_type.as_str() {
        "heading_1" => format!("# {}", b.plain_text),
        "heading_2" => format!("## {}", b.plain_text),
        "heading_3" => format!("### {}", b.plain_text),
        "to_do" => {
            let mark = if payload["checked"].as_bool().unwrap_or(false) {
                "x"
            } else {
                " "
            };
            format!("- [{mark}] {}", b.plain_text)
        }
        "bulleted_list_item" => format!("- {}", b.plain_text),
        "numbered_list_item" => format!("1. {}", b.plain_text),
        "quote" => format!("> {}", b.plain_text),
        "divider" => "---".to_string(),
        "code" => {
            let lang = payload["language"].as_str().unwrap_or("");
            format!("```{lang}\n{}\n```", b.plain_text)
        }
        _ => b.plain_text.clone(),
    }
}

/// Renders a page's blocks to Markdown. Returns the joined text plus the flat
/// list of `Unit`s (one per rendered top-level "atom", in document order) used
/// by `diff::apply_edited_markdown` to correlate the edited text back to block ids.
pub fn blocks_to_markdown(blocks: &[BlockRec]) -> (String, Vec<Unit>) {
    let mut units = Vec::new();
    push_children(blocks, None, 0, &mut units);
    let md = units
        .iter()
        .map(|u| format!("{}{}", "  ".repeat(u.depth), u.text))
        .collect::<Vec<_>>()
        .join("\n");
    (md, units)
}

fn push_children(blocks: &[BlockRec], parent: Option<&str>, depth: usize, out: &mut Vec<Unit>) {
    for b in blocks.iter().filter(|b| b.parent_block_id.as_deref() == parent) {
        if is_expressible(&b.block_type) {
            out.push(Unit {
                id: b.id.clone(),
                depth,
                block_type: b.block_type.clone(),
                text: render_line(b),
                protected: false,
            });
            if b.has_children {
                push_children(blocks, Some(&b.id), depth + 1, out);
            }
        } else {
            // Protected island: the entire subtree under a non-expressible block
            // is opaque and round-trips as a single marker line — do not recurse.
            out.push(Unit {
                id: b.id.clone(),
                depth,
                block_type: b.block_type.clone(),
                text: format!("<!--notion:block:{}-->", b.id),
                protected: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::BlockRec;

    fn rec(
        id: &str,
        parent: Option<&str>,
        ord: i64,
        ty: &str,
        text: &str,
        payload: &str,
        has_children: bool,
    ) -> BlockRec {
        BlockRec {
            id: id.into(),
            page_id: "p".into(),
            parent_block_id: parent.map(Into::into),
            ordinal: ord,
            block_type: ty.into(),
            payload: payload.into(),
            plain_text: text.into(),
            has_children,
        }
    }

    #[test]
    fn renders_headings_todos_and_lists() {
        let blocks = vec![
            rec("b1", None, 0, "heading_1", "Title", "{}", false),
            rec("b2", None, 1, "to_do", "Buy milk", r#"{"checked": false}"#, false),
            rec("b3", None, 2, "bulleted_list_item", "item", "{}", false),
        ];
        let (md, units) = blocks_to_markdown(&blocks);
        assert_eq!(md, "# Title\n- [ ] Buy milk\n- item");
        assert_eq!(units.len(), 3);
        assert_eq!(units[0].id, "b1");
    }

    #[test]
    fn protected_block_emits_marker_and_skips_children() {
        let blocks = vec![
            rec("t1", None, 0, "toggle", "More", "{}", true),
            rec("c1", Some("t1"), 0, "paragraph", "hidden", "{}", false),
        ];
        let (md, units) = blocks_to_markdown(&blocks);
        assert_eq!(units.len(), 1);
        assert!(units[0].protected);
        assert_eq!(md, "<!--notion:block:t1-->");
    }

    #[test]
    fn nested_expressible_blocks_indent_two_spaces_per_depth() {
        let blocks = vec![
            rec("p1", None, 0, "bulleted_list_item", "parent", "{}", true),
            rec("c1", Some("p1"), 0, "bulleted_list_item", "child", "{}", false),
        ];
        let (md, _) = blocks_to_markdown(&blocks);
        assert_eq!(md, "- parent\n  - child");
    }

    #[test]
    fn code_block_is_fenced_with_language() {
        let blocks = vec![rec(
            "b1",
            None,
            0,
            "code",
            "let x = 1;",
            r#"{"language": "rust"}"#,
            false,
        )];
        let (md, _) = blocks_to_markdown(&blocks);
        assert_eq!(md, "```rust\nlet x = 1;\n```");
    }
}
