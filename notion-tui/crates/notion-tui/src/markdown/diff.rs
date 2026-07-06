use std::collections::{HashMap, HashSet};

use notion_store::Store;
use similar::{ChangeTag, TextDiff};

use super::{parse_markdown, ParsedLine, Unit};

#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    pub inserted: u32,
    pub updated: u32,
    pub deleted: u32,
    pub reordered: u32,
    pub protected_missing: Vec<String>,
}

fn render_parsed_text(line: &ParsedLine) -> String {
    match line.block_type.as_str() {
        "heading_1" => format!("# {}", line.text),
        "heading_2" => format!("## {}", line.text),
        "heading_3" => format!("### {}", line.text),
        "to_do" => {
            let mark = if line.checked == Some(true) { "x" } else { " " };
            format!("- [{mark}] {}", line.text)
        }
        "bulleted_list_item" => format!("- {}", line.text),
        "numbered_list_item" => format!("1. {}", line.text),
        "quote" => format!("> {}", line.text),
        "divider" => "---".to_string(),
        "code" => format!("```\n{}\n```", line.text),
        _ => line.text.clone(),
    }
}

pub fn apply_edited_markdown(
    store: &mut Store,
    page_id: &str,
    before: &[Unit],
    edited_text: &str,
    delete_protected: &HashSet<String>,
) -> anyhow::Result<Applied> {
    let new_lines = parse_markdown(edited_text);
    let mut id_at_new_line: Vec<Option<String>> = vec![None; new_lines.len()];
    let mut result = Applied::default();

    // Pass 0: protected islands are matched by the explicit id in their marker.
    let still_present: HashSet<&str> = new_lines.iter().filter_map(|l| l.protected_id.as_deref()).collect();
    for u in before.iter().filter(|u| u.protected) {
        if !still_present.contains(u.id.as_str()) && !delete_protected.contains(&u.id) {
            result.protected_missing.push(u.id.clone());
        }
    }
    for (i, line) in new_lines.iter().enumerate() {
        if let Some(id) = &line.protected_id {
            id_at_new_line[i] = Some(id.clone());
        }
    }
    for id in delete_protected {
        store.edit_delete_block(id)?;
        result.deleted += 1;
    }

    // Pass 1a: match reordered-but-unchanged content by exact text, regardless of
    // position — a plain position-based diff can only preserve one side of a full
    // reversal (LCS can't keep both matches when order is swapped), which would
    // otherwise turn an unchanged, merely-moved block into a spurious delete+insert.
    let editable_before: Vec<&Unit> = before.iter().filter(|u| !u.protected).collect();
    let new_idx_of_editable: Vec<usize> = new_lines.iter().enumerate()
        .filter(|(_, l)| l.protected_id.is_none()).map(|(i, _)| i).collect();
    let new_texts: Vec<String> = new_idx_of_editable.iter().map(|&i| render_parsed_text(&new_lines[i])).collect();

    let mut available_by_text: HashMap<&str, std::collections::VecDeque<usize>> = HashMap::new();
    for (i, u) in editable_before.iter().enumerate() {
        available_by_text.entry(u.text.as_str()).or_default().push_back(i);
    }
    let mut matched_old: HashSet<usize> = HashSet::new();
    let mut matched_new: HashSet<usize> = HashSet::new();
    for (ni, text) in new_texts.iter().enumerate() {
        if let Some(queue) = available_by_text.get_mut(text.as_str()) {
            if let Some(oi) = queue.pop_front() {
                matched_old.insert(oi);
                matched_new.insert(ni);
                id_at_new_line[new_idx_of_editable[ni]] = Some(editable_before[oi].id.clone());
            }
        }
    }

    // Pass 1b: diff whatever's left (genuinely changed content) to pair up
    // insert/delete runs into updates that preserve the original block id.
    let leftover_old: Vec<usize> = (0..editable_before.len()).filter(|i| !matched_old.contains(i)).collect();
    let leftover_new: Vec<usize> = (0..new_texts.len()).filter(|i| !matched_new.contains(i)).collect();
    let leftover_old_texts: Vec<&str> = leftover_old.iter().map(|&i| editable_before[i].text.as_str()).collect();
    let leftover_new_texts: Vec<&str> = leftover_new.iter().map(|&i| new_texts[i].as_str()).collect();

    let diff = TextDiff::from_slices(&leftover_old_texts, &leftover_new_texts);
    let mut pending_deletes: Vec<usize> = Vec::new();
    let mut pending_inserts: Vec<usize> = Vec::new();

    // Anchor every insert after the current last top-level block (if any) so its
    // sibling-ordinal shift never touches a pre-existing block — otherwise a block
    // that never actually moved would get bumped and resync_order would (wrongly)
    // enqueue a real reorder for it. resync_order still does all the real
    // positioning/reparenting afterward; this only avoids collateral disturbance.
    let mut insert_anchor: Option<String> = store
        .page_blocks(page_id)?
        .iter()
        .filter(|b| b.parent_block_id.is_none())
        .max_by_key(|b| b.ordinal)
        .map(|b| b.id.clone());

    macro_rules! flush {
        () => {{
            let n = pending_deletes.len().min(pending_inserts.len());
            for k in 0..n {
                let old_id = editable_before[leftover_old[pending_deletes[k]]].id.clone();
                let new_line_idx = new_idx_of_editable[leftover_new[pending_inserts[k]]];
                let line = &new_lines[new_line_idx];
                store.edit_update_block_text(&old_id, &line.text)?;
                id_at_new_line[new_line_idx] = Some(old_id);
                result.updated += 1;
            }
            for &d in &pending_deletes[n..] {
                store.edit_delete_block(&editable_before[leftover_old[d]].id)?;
                result.deleted += 1;
            }
            for &ins in &pending_inserts[n..] {
                let new_line_idx = new_idx_of_editable[leftover_new[ins]];
                let line = &new_lines[new_line_idx];
                let (id, _receipt) = store.edit_insert_block_after(
                    page_id,
                    insert_anchor.as_deref(),
                    &line.block_type,
                    &line.text,
                )?;
                insert_anchor = Some(id.clone());
                id_at_new_line[new_line_idx] = Some(id);
                result.inserted += 1;
            }
            pending_deletes.clear();
            pending_inserts.clear();
        }};
    }

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                // Only reachable if a text value's old/new counts differ (so some
                // instances go through the leftover path) — treat defensively the
                // same as a genuine match.
                flush!();
                let old_i = leftover_old[change.old_index().unwrap()];
                let new_i = new_idx_of_editable[leftover_new[change.new_index().unwrap()]];
                id_at_new_line[new_i] = Some(editable_before[old_i].id.clone());
            }
            ChangeTag::Delete => pending_deletes.push(change.old_index().unwrap()),
            ChangeTag::Insert => pending_inserts.push(change.new_index().unwrap()),
        }
    }
    flush!();

    // Pass 2: resync every surviving/created block's parent/ordinal to match the
    // final edited document order (also catches pure reorders).
    result.reordered = resync_order(store, page_id, &id_at_new_line, &new_lines)?;

    Ok(result)
}

fn resync_order(
    store: &mut Store,
    page_id: &str,
    id_at_new_line: &[Option<String>],
    new_lines: &[ParsedLine],
) -> anyhow::Result<u32> {
    let current = store.page_blocks(page_id)?;
    let mut reordered = 0u32;
    let mut stack: Vec<(usize, String)> = Vec::new();
    let mut counters: HashMap<(usize, Option<String>), i64> = HashMap::new();
    let mut last_at: HashMap<(usize, Option<String>), String> = HashMap::new();

    for (i, line) in new_lines.iter().enumerate() {
        let Some(id) = &id_at_new_line[i] else { continue };
        while stack.last().is_some_and(|(d, _)| *d >= line.depth) {
            stack.pop();
        }
        let parent = stack.last().map(|(_, id)| id.clone());
        let key = (line.depth, parent.clone());
        let ordinal = *counters.entry(key.clone()).or_insert(0);
        counters.insert(key.clone(), ordinal + 1);
        let after = last_at.get(&key).cloned();

        let existing = current.iter().find(|b| &b.id == id);
        let needs_update = existing.is_none_or(|b| b.parent_block_id != parent || b.ordinal != ordinal);
        if needs_update {
            store.edit_reorder_block(id, parent.as_deref(), after.as_deref(), ordinal)?;
            reordered += 1;
        }
        last_at.insert(key, id.clone());
        stack.push((line.depth, id.clone()));
    }
    Ok(reordered)
}

#[cfg(test)]
mod tests {
    use crate::markdown::blocks_to_markdown;
    use notion_store::{BlockRec, PageRec, Store};
    use std::collections::HashSet;

    fn store_with_page() -> (Store, Vec<BlockRec>) {
        let mut s = Store::open_in_memory().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
            title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
        }).unwrap();
        let blocks = vec![
            BlockRec { id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
                block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false },
            BlockRec { id: "b2".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 1,
                block_type: "paragraph".into(), payload: "{}".into(), plain_text: "Second".into(), has_children: false },
        ];
        s.replace_page_blocks("p1", &blocks).unwrap();
        (s, blocks)
    }

    #[test]
    fn unchanged_text_produces_no_edits() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First\nSecond";
        let applied = super::apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!((applied.inserted, applied.updated, applied.deleted, applied.reordered), (0, 0, 0, 0));
    }

    #[test]
    fn changed_line_updates_the_same_block_id() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First\nSecond, edited";
        let applied = super::apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!(applied.updated, 1);
        let b2 = s.page_blocks("p1").unwrap().into_iter().find(|b| b.id == "b2").unwrap();
        assert_eq!(b2.plain_text, "Second, edited");
    }

    #[test]
    fn new_line_inserts_a_block() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First\nSecond\nThird";
        let applied = super::apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!(applied.inserted, 1);
        assert_eq!(s.page_blocks("p1").unwrap().len(), 3);
    }

    #[test]
    fn removed_line_deletes_the_block() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First";
        let applied = super::apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!(applied.deleted, 1);
        assert_eq!(s.page_blocks("p1").unwrap().len(), 1);
    }

    #[test]
    fn reordered_unchanged_lines_are_repositioned_not_recreated() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "Second\nFirst";
        let applied = super::apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!((applied.inserted, applied.updated, applied.deleted), (0, 0, 0));
        assert_eq!(applied.reordered, 2);
        let after = s.page_blocks("p1").unwrap();
        let b2 = after.iter().find(|b| b.id == "b2").unwrap();
        let b1 = after.iter().find(|b| b.id == "b1").unwrap();
        assert!(b2.ordinal < b1.ordinal);
    }

    #[test]
    fn protected_block_missing_from_edited_text_is_reported_but_not_deleted_without_confirmation() {
        let mut s = Store::open_in_memory().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
            title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
        }).unwrap();
        let blocks = vec![BlockRec { id: "tg1".into(), page_id: "p1".into(), parent_block_id: None,
            ordinal: 0, block_type: "toggle".into(), payload: "{}".into(), plain_text: "More".into(), has_children: false }];
        s.replace_page_blocks("p1", &blocks).unwrap();
        let (_md, units) = blocks_to_markdown(&blocks);

        let applied = super::apply_edited_markdown(&mut s, "p1", &units, "", &HashSet::new()).unwrap();
        assert_eq!(applied.protected_missing, vec!["tg1".to_string()]);
        assert_eq!(s.page_blocks("p1").unwrap().len(), 1); // not deleted yet

        let mut confirm = HashSet::new();
        confirm.insert("tg1".to_string());
        let applied2 = super::apply_edited_markdown(&mut s, "p1", &units, "", &confirm).unwrap();
        assert_eq!(applied2.deleted, 1);
        assert!(s.page_blocks("p1").unwrap().is_empty());
    }
}
