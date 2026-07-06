use notion_store::BlockRec;
use notion_tui::markdown::{blocks_to_markdown, parse_markdown};
use proptest::prelude::*;

/// Only the expressible, flat (non-nested) subset — nesting and protected
/// islands are covered by the targeted unit tests in Tasks 1–2; this property
/// test focuses on content fidelity across the full type list.
// First char is always a letter so text never collides with Markdown-reserved
// prefixes ("-", "#", ">", digit-dot) or gets swallowed as blank/indentation —
// see the accepted v1 limitations noted at the assertion site below.
fn arb_block() -> impl Strategy<Value = (String, String)> {
    prop_oneof![
        "[a-zA-Z][a-zA-Z0-9 ]{0,19}".prop_map(|t| ("paragraph".to_string(), t)),
        "[a-zA-Z][a-zA-Z0-9 ]{0,19}".prop_map(|t| ("heading_1".to_string(), t)),
        "[a-zA-Z][a-zA-Z0-9 ]{0,19}".prop_map(|t| ("bulleted_list_item".to_string(), t)),
        "[a-zA-Z][a-zA-Z0-9 ]{0,19}".prop_map(|t| ("quote".to_string(), t)),
    ]
}

proptest! {
    #[test]
    fn roundtrips_flat_expressible_blocks(kind_text_pairs in prop::collection::vec(arb_block(), 1..8)) {
        let blocks: Vec<BlockRec> = kind_text_pairs.iter().enumerate().map(|(i, (kind, text))| BlockRec {
            id: format!("b{i}"),
            page_id: "p".into(),
            parent_block_id: None,
            ordinal: i as i64,
            block_type: kind.clone(),
            payload: "{}".into(),
            plain_text: text.clone(),
            has_children: false,
        }).collect();

        let (md, _units) = blocks_to_markdown(&blocks);
        let parsed = parse_markdown(&md);

        prop_assert_eq!(parsed.len(), blocks.len());
        for (p, b) in parsed.iter().zip(blocks.iter()) {
            prop_assert_eq!(&p.block_type, &b.block_type);
            prop_assert_eq!(&p.text, &b.plain_text);
        }
    }
}
