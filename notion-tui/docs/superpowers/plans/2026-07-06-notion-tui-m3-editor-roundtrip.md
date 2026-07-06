# notion-tui Milestone 3 — Editor Round-Trip Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the `$EDITOR` Markdown round-trip (design spec §4.4): pressing `e` on a page opens its blocks as Markdown in the user's editor; on save, the old and new Markdown are diffed and translated into per-block store edits (insert / update / delete / reorder) so blocks that didn't change keep their ids (and therefore their remote ids and any attached comments); block types Markdown can't express survive untouched as opaque "protected islands."

**Architecture:** Pure conversion logic lives in a new `notion_tui::markdown` module — `render` (blocks → Markdown), `parse` (Markdown → structural lines), and `diff` (correlates old/new and applies the result to `notion_store::Store`). A small `notion_tui::editor` module wraps the `$EDITOR` subprocess. `notion-store` gains one new local-only edit method, `edit_reorder_block`, that reuses the M2 `pending_ops`/dirty machinery. `notion-sync`'s pusher gains a handler for the new op type that is a local no-op (see Task 5 — Notion's Blocks API has no reorder endpoint).

**Tech Stack:** Adds `proptest` (property-based round-trip tests, per spec §7 — "the highest-risk correctness surface") and `similar` (text diffing) as new workspace dependencies. Promotes `tempfile` from dev-dependency to a regular dependency of `notion-tui` (the editor's scratch `.md` file).

## Global Constraints

- Notion API version `2025-09-03` (already set via `notion_api::NOTION_VERSION`).
- Every edit still applies to SQLite immediately and enqueues a `pending_ops` row in the same transaction — this milestone does not relax the M2 optimistic-write rule.
- No screen ever awaits the network. The `$EDITOR` subprocess blocks the terminal on local I/O only, exactly like any other TUI editor round-trip; it never talks to the network.
- The Notion Blocks API has no endpoint to move/reorder a block between parents or siblings. Reorders are represented as a local-only op whose "push" is a no-op success (Task 5) — this is a deliberate, documented v1 limitation, not an oversight.

---

## Design notes (read before starting)

**Protected islands.** A block whose type is not in the expressible whitelist (`paragraph`, `heading_1/2/3`, `to_do`, `bulleted_list_item`, `numbered_list_item`, `quote`, `divider`, `code`) is rendered as a single opaque line `<!--notion:block:{id}-->`, and its entire subtree is *not* recursed into — the whole subtree is swallowed by the marker. This covers toggles, callouts, child pages/databases, and any future/unknown block type, matching spec §4.4's list (callouts, toggles, synced blocks, embeds, colors) plus anything else we don't have a Markdown mapping for.

**Reorders and pending inserts.** A newly-inserted block (via `a`/`i` in M2, or newly created in this round-trip) is queued as an `append_block` op with a fixed `after`/`parent_id` at creation time. If the editor round-trip later moves that same not-yet-pushed block, there is nothing remote to reorder — instead of enqueuing a second op, `edit_reorder_block` rewrites the pending `append_block` op's payload in place. Only a block that has **no** pending creation op (i.e., it already exists remotely) gets a real `reorder_block` op enqueued; that op's push handler (Task 5) does not call the API at all — Notion has no such endpoint — it just clears the op and the page's dirty flag, exactly like every other op type, so the dirty-tracking invariant ("a page stays dirty until every op touching it drains") keeps holding.

**Two-pass apply.** Diffing directly for both content *and* position at once is a full tree-diff problem. Instead, `apply_edited_markdown` (Task 6) runs two passes: Pass 1 uses `similar::TextDiff` over unit *text* only to decide insert/update/delete (ignoring position); Pass 2 (`resync_order`) then walks the final edited document top-to-bottom and repositions every surviving/created block's `parent_block_id`/`ordinal` to match, calling `edit_reorder_block` wherever a block's position doesn't already match. This naturally also catches pure reorders (unchanged text, moved position) without any special-casing in the diff pass.

---

### Task 1: `markdown::render` — blocks → Markdown

**Files:**
- Create: `crates/notion-tui/src/markdown/mod.rs`
- Create: `crates/notion-tui/src/markdown/render.rs`
- Modify: `crates/notion-tui/src/lib.rs`

**Interfaces:**
- Consumes: `notion_store::BlockRec` (existing).
- Produces: `pub struct Unit { pub id: String, pub depth: usize, pub block_type: String, pub text: String, pub protected: bool }`, `pub fn blocks_to_markdown(blocks: &[BlockRec]) -> (String, Vec<Unit>)`.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/src/markdown/render.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::BlockRec;

    fn rec(id: &str, parent: Option<&str>, ord: i64, ty: &str, text: &str, payload: &str, has_children: bool) -> BlockRec {
        BlockRec {
            id: id.into(), page_id: "p".into(), parent_block_id: parent.map(Into::into),
            ordinal: ord, block_type: ty.into(), payload: payload.into(),
            plain_text: text.into(), has_children,
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
        let blocks = vec![rec("b1", None, 0, "code", "let x = 1;", r#"{"language": "rust"}"#, false)];
        let (md, _) = blocks_to_markdown(&blocks);
        assert_eq!(md, "```rust\nlet x = 1;\n```");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui markdown::render --lib`
Expected: FAIL — module `markdown` does not exist.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/markdown/mod.rs`:
```rust
pub mod render;

pub use render::{blocks_to_markdown, Unit};
```

`crates/notion-tui/src/markdown/render.rs` (above the `#[cfg(test)]` module):
```rust
use notion_store::BlockRec;

pub struct Unit {
    pub id: String,
    pub depth: usize,
    pub block_type: String,
    pub text: String,
    pub protected: bool,
}

const EXPRESSIBLE: &[&str] = &[
    "paragraph", "heading_1", "heading_2", "heading_3", "to_do",
    "bulleted_list_item", "numbered_list_item", "quote", "divider", "code",
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
            let mark = if payload["checked"].as_bool().unwrap_or(false) { "x" } else { " " };
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
```

`crates/notion-tui/src/lib.rs` — add the module:
```rust
pub mod app;
pub mod config;
pub mod markdown;
pub mod terminal;
pub mod ui;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui markdown::render --lib`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/markdown/mod.rs crates/notion-tui/src/markdown/render.rs crates/notion-tui/src/lib.rs
git commit -m "feat(notion-tui): blocks-to-markdown renderer"
```

---

### Task 2: `markdown::parse` — Markdown → structural lines

**Files:**
- Create: `crates/notion-tui/src/markdown/parse.rs`
- Modify: `crates/notion-tui/src/markdown/mod.rs`

**Interfaces:**
- Consumes: nothing beyond the standard library.
- Produces: `pub struct ParsedLine { pub depth: usize, pub block_type: String, pub text: String, pub checked: Option<bool>, pub protected_id: Option<String> }`, `pub fn parse_markdown(md: &str) -> Vec<ParsedLine>`.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/src/markdown/parse.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_headings_todo_and_bullets() {
        let lines = parse_markdown("# Title\n- [x] done\n- [ ] pending\n- bullet");
        assert_eq!(lines[0].block_type, "heading_1");
        assert_eq!(lines[0].text, "Title");
        assert_eq!(lines[1].block_type, "to_do");
        assert_eq!(lines[1].checked, Some(true));
        assert_eq!(lines[2].checked, Some(false));
        assert_eq!(lines[3].block_type, "bulleted_list_item");
        assert_eq!(lines[3].text, "bullet");
    }

    #[test]
    fn parses_numbered_list_regardless_of_number() {
        let lines = parse_markdown("1. first\n7. second");
        assert_eq!(lines[0].block_type, "numbered_list_item");
        assert_eq!(lines[0].text, "first");
        assert_eq!(lines[1].text, "second");
    }

    #[test]
    fn indentation_maps_to_depth() {
        let lines = parse_markdown("- parent\n  - child");
        assert_eq!(lines[0].depth, 0);
        assert_eq!(lines[1].depth, 1);
    }

    #[test]
    fn protected_marker_is_recognized() {
        let lines = parse_markdown("<!--notion:block:t1-->\n- after");
        assert_eq!(lines[0].protected_id.as_deref(), Some("t1"));
        assert_eq!(lines[1].protected_id, None);
    }

    #[test]
    fn fenced_code_block_is_one_line_with_full_body() {
        let lines = parse_markdown("```rust\nlet x = 1;\nlet y = 2;\n```\n- after");
        assert_eq!(lines[0].block_type, "code");
        assert_eq!(lines[0].text, "let x = 1;\nlet y = 2;");
        assert_eq!(lines[1].block_type, "bulleted_list_item");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui markdown::parse --lib`
Expected: FAIL — module `parse` does not exist.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/markdown/parse.rs` (above the test module):
```rust
pub struct ParsedLine {
    pub depth: usize,
    pub block_type: String,
    pub text: String,
    pub checked: Option<bool>,
    pub protected_id: Option<String>,
}

pub fn parse_markdown(md: &str) -> Vec<ParsedLine> {
    let mut out = Vec::new();
    let mut lines = md.lines().peekable();
    while let Some(raw) = lines.next() {
        if raw.trim().is_empty() {
            continue;
        }
        let indent = raw.chars().take_while(|c| *c == ' ').count();
        let depth = indent / 2;
        let trimmed = &raw[indent..];

        if let Some(id) = parse_protected_marker(trimmed) {
            out.push(ParsedLine { depth, block_type: "protected".into(), text: String::new(), checked: None, protected_id: Some(id) });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("```") {
            let _lang = rest.trim().to_string();
            let mut body = Vec::new();
            for l in lines.by_ref() {
                if l.trim() == "```" {
                    break;
                }
                body.push(l);
            }
            out.push(ParsedLine { depth, block_type: "code".into(), text: body.join("\n"), checked: None, protected_id: None });
            continue;
        }
        let (block_type, text, checked) = classify(trimmed);
        out.push(ParsedLine { depth, block_type, text, checked, protected_id: None });
    }
    out
}

fn parse_protected_marker(line: &str) -> Option<String> {
    line.strip_prefix("<!--notion:block:")
        .and_then(|rest| rest.strip_suffix("-->"))
        .map(str::to_string)
}

fn classify(line: &str) -> (String, String, Option<bool>) {
    if let Some(rest) = line.strip_prefix("### ") {
        return ("heading_3".into(), rest.into(), None);
    }
    if let Some(rest) = line.strip_prefix("## ") {
        return ("heading_2".into(), rest.into(), None);
    }
    if let Some(rest) = line.strip_prefix("# ") {
        return ("heading_1".into(), rest.into(), None);
    }
    if let Some(rest) = line.strip_prefix("- [x] ") {
        return ("to_do".into(), rest.into(), Some(true));
    }
    if let Some(rest) = line.strip_prefix("- [ ] ") {
        return ("to_do".into(), rest.into(), Some(false));
    }
    if let Some(rest) = line.strip_prefix("> ") {
        return ("quote".into(), rest.into(), None);
    }
    if line == "---" {
        return ("divider".into(), String::new(), None);
    }
    if let Some(rest) = line.strip_prefix("- ") {
        return ("bulleted_list_item".into(), rest.into(), None);
    }
    if let Some(dot) = line.find(". ") {
        if !line[..dot].is_empty() && line[..dot].chars().all(|c| c.is_ascii_digit()) {
            return ("numbered_list_item".into(), line[dot + 2..].into(), None);
        }
    }
    ("paragraph".into(), line.to_string(), None)
}
```

`crates/notion-tui/src/markdown/mod.rs` — add the module:
```rust
pub mod parse;
pub mod render;

pub use parse::{parse_markdown, ParsedLine};
pub use render::{blocks_to_markdown, Unit};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui markdown::parse --lib`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/markdown/parse.rs crates/notion-tui/src/markdown/mod.rs
git commit -m "feat(notion-tui): markdown-to-structural-lines parser"
```

---

### Task 3: Property-based round-trip test

**Files:**
- Create: `crates/notion-tui/tests/markdown_roundtrip.rs`
- Modify: `Cargo.toml` (workspace), `crates/notion-tui/Cargo.toml`

**Interfaces:**
- Consumes: `notion_tui::markdown::{blocks_to_markdown, parse_markdown}` (Tasks 1–2).
- Produces: nothing new — this is a test-only task confirming the two prior tasks are mutual inverses for the expressible subset.

- [ ] **Step 1: Add the `proptest` dependency**

`Cargo.toml` (workspace root) — add to `[workspace.dependencies]`:
```toml
proptest = "1"
```

`crates/notion-tui/Cargo.toml` — add to `[dev-dependencies]`:
```toml
proptest = { workspace = true }
```

- [ ] **Step 2: Write the failing test**

`crates/notion-tui/tests/markdown_roundtrip.rs`:
```rust
use notion_store::BlockRec;
use notion_tui::markdown::{blocks_to_markdown, parse_markdown};
use proptest::prelude::*;

/// Only the expressible, flat (non-nested) subset — nesting and protected
/// islands are covered by the targeted unit tests in Tasks 1–2; this property
/// test focuses on content fidelity across the full type list.
fn arb_block() -> impl Strategy<Value = (String, String)> {
    prop_oneof![
        "[a-zA-Z0-9 ]{1,20}".prop_map(|t| ("paragraph".to_string(), t)),
        "[a-zA-Z0-9 ]{1,20}".prop_map(|t| ("heading_1".to_string(), t)),
        "[a-zA-Z0-9 ]{1,20}".prop_map(|t| ("bulleted_list_item".to_string(), t)),
        "[a-zA-Z0-9 ]{1,20}".prop_map(|t| ("quote".to_string(), t)),
    ]
}

proptest! {
    #[test]
    fn roundtrips_flat_expressible_blocks((kind_text_pairs) in prop::collection::vec(arb_block(), 1..8)) {
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
```

- [ ] **Step 2b: Run test to verify it fails**

Run: `cargo test -p notion-tui --test markdown_roundtrip`
Expected: FAIL — `notion_tui::markdown` is not `pub` yet from the crate root in a way tests can reach it, or the property genuinely fails if there's a mismatch. (If Tasks 1–2 are already correct this may pass immediately; if so, tighten the generator — e.g. include a `-` or `#` character in the arbitrary text — until it exposes a real gap, then fix `render`/`parse` to close it. Do not skip this verification step.)

- [ ] **Step 3: Fix any gap found**

If the tightened generator reveals that literal `#`/`-`/`>`/digit-dot text collides with Markdown syntax (e.g. a paragraph whose text starts with `"- "`), note this as a known limitation in a doc comment on `render_line`/`classify` rather than attempting full escaping — v1 accepts that plain-text content starting with Markdown-reserved prefixes may round-trip as a different block type. Add a targeted `proptest::prop_assume!(!text.starts_with('-') && !text.starts_with('#') && !text.starts_with('>'))` to the test to document the accepted exclusion.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test markdown_roundtrip`
Expected: PASS (256 cases by default).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/notion-tui/Cargo.toml crates/notion-tui/tests/markdown_roundtrip.rs
git commit -m "test(notion-tui): property-based markdown round-trip test"
```

---

### Task 4: `Store::edit_reorder_block`

**Files:**
- Modify: `crates/notion-store/src/store.rs`
- Modify: `crates/notion-store/src/lib.rs`
- Test: `crates/notion-store/tests/reorder.rs` (new)

**Interfaces:**
- Consumes: existing `Store::ops()`, `Store::enqueue_op()`.
- Produces: `pub fn edit_reorder_block(&mut self, block_id: &str, new_parent: Option<&str>, new_after: Option<&str>, new_ordinal: i64) -> anyhow::Result<()>`. New `Inverse::ReorderBlock { block_id: String, old_parent: Option<String>, old_ordinal: i64 }` variant (undo restores prior position; no op re-enqueue needed since a pure position-only inverse never touched anything remote).

- [ ] **Step 1: Write the failing test**

`crates/notion-store/tests/reorder.rs`:
```rust
use notion_store::{BlockRec, PageRec, Store};

fn store_with_page_and_blocks() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[
        BlockRec { id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
            block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false },
        BlockRec { id: "b2".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 1,
            block_type: "paragraph".into(), payload: "{}".into(), plain_text: "Second".into(), has_children: false },
    ]).unwrap();
    s
}

#[test]
fn reordering_an_already_remote_block_enqueues_local_only_op_and_marks_dirty() {
    let mut s = store_with_page_and_blocks();
    s.edit_reorder_block("b2", None, None, 0).unwrap();

    let blocks = s.page_blocks("p1").unwrap();
    let b2 = blocks.iter().find(|b| b.id == "b2").unwrap();
    assert_eq!(b2.ordinal, 0);
    assert!(s.is_page_dirty("p1").unwrap());

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "reorder_block");
}

#[test]
fn reordering_a_still_pending_insert_patches_its_append_op_instead_of_enqueuing_a_new_one() {
    let mut s = store_with_page_and_blocks();
    let (new_id, _receipt) = s.edit_insert_block_after("p1", Some("b1"), "paragraph", "New").unwrap();
    assert_eq!(s.ops().unwrap().len(), 1); // just the append_block op

    s.edit_reorder_block(&new_id, None, None, 0).unwrap();

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1); // still just one op — patched, not appended
    assert_eq!(ops[0].op_type, "append_block");
    let payload: serde_json::Value = serde_json::from_str(&ops[0].payload).unwrap();
    assert_eq!(payload["parent_id"], serde_json::Value::Null);
}

#[test]
fn reorder_to_same_position_is_a_no_op() {
    let mut s = store_with_page_and_blocks();
    s.edit_reorder_block("b1", None, None, 0).unwrap();
    assert!(s.ops().unwrap().is_empty());
    assert!(!s.is_page_dirty("p1").unwrap());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-store --test reorder`
Expected: FAIL — `edit_reorder_block` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-store/src/store.rs` — add after `edit_delete_block`:
```rust
pub fn edit_reorder_block(
    &mut self,
    block_id: &str,
    new_parent: Option<&str>,
    new_after: Option<&str>,
    new_ordinal: i64,
) -> anyhow::Result<()> {
    let (page_id, old_parent, old_ordinal): (String, Option<String>, i64) = self.conn.query_row(
        "SELECT page_id, parent_block_id, ordinal FROM blocks WHERE id = ?1",
        [block_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    if old_parent.as_deref() == new_parent && old_ordinal == new_ordinal {
        return Ok(());
    }
    self.conn.execute(
        "UPDATE blocks SET parent_block_id = ?2, ordinal = ?3 WHERE id = ?1",
        rusqlite::params![block_id, new_parent, new_ordinal],
    )?;

    let pending_append = self.ops()?.into_iter().find(|o| o.op_type == "append_block" && o.target_id == block_id);
    if let Some(op) = pending_append {
        let mut payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
        payload["parent_id"] = json!(new_parent);
        payload["after"] = json!(new_after);
        self.conn.execute(
            "UPDATE pending_ops SET payload = ?2 WHERE seq = ?1",
            rusqlite::params![op.seq, payload.to_string()],
        )?;
        return Ok(());
    }

    self.conn.execute("UPDATE pages SET dirty = 1 WHERE id = ?1", [&page_id])?;
    self.enqueue_op("reorder_block", block_id, &json!({"page_id": page_id}).to_string(), None)?;
    let _ = old_ordinal; // captured for symmetry with the Inverse variant below; undo is out of scope for this op (see plan design notes)
    Ok(())
}
```

Note: `Inverse`/undo support for reorders is intentionally **not** added — the editor round-trip is a single batch operation covering many blocks at once, and per-block undo of a bulk reorder is not a coherent user action. `App::undo` (M2) continues to work for every other edit type; reorders simply aren't undo-able via `u`, consistent with them not producing an `EditReceipt` at all (`edit_reorder_block` returns `()`, not `EditReceipt`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-store --test reorder`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-store/src/store.rs crates/notion-store/tests/reorder.rs
git commit -m "feat(notion-store): local-only block reorder edit"
```

---

### Task 5: pusher `reorder_block` handler

**Files:**
- Modify: `crates/notion-sync/src/pusher.rs`
- Test: `crates/notion-sync/tests/push_reorder.rs` (new)

**Interfaces:**
- Consumes: `notion_store::OpRec`, existing `remaining_ops_reference_page` helper (already in `pusher.rs`).
- Produces: nothing new exported — extends the existing `push_one` dispatch.

- [ ] **Step 1: Write the failing test**

`crates/notion-sync/tests/push_reorder.rs`:
```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::push_once;
use wiremock::MockServer;

#[tokio::test]
async fn reorder_block_op_clears_without_any_http_call() {
    // No mocks registered at all — if the pusher made an HTTP request for this
    // op type, wiremock would return a 404 and the test would fail.
    let server = MockServer::start().await;
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));

    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[
        BlockRec { id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
            block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false },
    ]).unwrap();
    s.edit_reorder_block("b1", None, None, 0).unwrap(); // no-op position change forced below
    // Force a real position change so an op is actually enqueued:
    s.conn().execute("UPDATE blocks SET ordinal = 5 WHERE id = 'b1'", []).unwrap();
    s.conn().execute("UPDATE pages SET dirty = 1 WHERE id = 'p1'", []).unwrap();
    s.enqueue_op("reorder_block", "b1", &serde_json::json!({"page_id": "p1"}).to_string(), None).unwrap();

    let store = Arc::new(Mutex::new(s));
    let pushed = push_once(&client, &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-sync --test push_reorder`
Expected: FAIL — `push_once` returns `Ok(0)` and leaves the op pending (unknown op_type `reorder_block` falls to `Ok(PushOutcome::Failed(...))`, marking it `failed`, not clearing it).

- [ ] **Step 3: Write minimal implementation**

`crates/notion-sync/src/pusher.rs` — add a handler and wire it into `push_one`:
```rust
async fn push_reorder_block(store: &SharedStore, op: &OpRec) -> Result<PushOutcome, ApiError> {
    // Notion's Blocks API has no reorder/move endpoint — there is nothing to call.
    // This op exists purely to drive the same dirty-clearing machinery every
    // other op type uses (see plan design notes).
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();

    store.lock().unwrap().delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &page_id) {
        store.lock().unwrap().clear_page_dirty(&page_id).ok();
    }
    Ok(PushOutcome::Success)
}
```

Update `push_one`'s match (note: this handler doesn't need `client`, so it's called without it):
```rust
async fn push_one(client: &NotionClient, store: &SharedStore, op: &OpRec) -> Result<PushOutcome, ApiError> {
    match op.op_type.as_str() {
        "update_block" => push_update_block(client, store, op).await,
        "delete_block" => push_delete_block(client, store, op).await,
        "append_block" => push_append_block(client, store, op).await,
        "reorder_block" => push_reorder_block(store, op).await,
        "update_row" => push_update_row(client, store, op).await,
        "create_row" => push_create_row(client, store, op).await,
        "delete_row" => push_delete_row(client, store, op).await,
        "restore_row" => push_restore_row(client, store, op).await,
        other => Ok(PushOutcome::Failed(format!("unknown op_type {other}"))),
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-sync --test push_reorder`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/notion-sync/src/pusher.rs crates/notion-sync/tests/push_reorder.rs
git commit -m "feat(notion-sync): local-only push handler for block reorders"
```

---

### Task 6: `markdown::diff` — correlate and apply

**Files:**
- Create: `crates/notion-tui/src/markdown/diff.rs`
- Modify: `crates/notion-tui/src/markdown/mod.rs`, `crates/notion-tui/Cargo.toml`, `Cargo.toml` (workspace)

**Interfaces:**
- Consumes: `Unit`/`blocks_to_markdown` (Task 1), `ParsedLine`/`parse_markdown` (Task 2), `notion_store::Store::{edit_insert_block_after, edit_update_block_text, edit_delete_block, edit_reorder_block, page_blocks}` (existing + Task 4).
- Produces: `pub struct Applied { pub inserted: u32, pub updated: u32, pub deleted: u32, pub reordered: u32, pub protected_missing: Vec<String> }`, `pub fn apply_edited_markdown(store: &mut Store, page_id: &str, before: &[Unit], edited_text: &str, delete_protected: &std::collections::HashSet<String>) -> anyhow::Result<Applied>`.

- [ ] **Step 1: Add the `similar` dependency**

`Cargo.toml` (workspace root) — add to `[workspace.dependencies]`:
```toml
similar = "2"
```

`crates/notion-tui/Cargo.toml` — add to `[dependencies]`:
```toml
similar = { workspace = true }
```

- [ ] **Step 2: Write the failing test**

`crates/notion-tui/src/markdown/diff.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
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
        let applied = apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!((applied.inserted, applied.updated, applied.deleted, applied.reordered), (0, 0, 0, 0));
    }

    #[test]
    fn changed_line_updates_the_same_block_id() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First\nSecond, edited";
        let applied = apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!(applied.updated, 1);
        let b2 = s.page_blocks("p1").unwrap().into_iter().find(|b| b.id == "b2").unwrap();
        assert_eq!(b2.plain_text, "Second, edited");
    }

    #[test]
    fn new_line_inserts_a_block() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First\nSecond\nThird";
        let applied = apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!(applied.inserted, 1);
        assert_eq!(s.page_blocks("p1").unwrap().len(), 3);
    }

    #[test]
    fn removed_line_deletes_the_block() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "First";
        let applied = apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
        assert_eq!(applied.deleted, 1);
        assert_eq!(s.page_blocks("p1").unwrap().len(), 1);
    }

    #[test]
    fn reordered_unchanged_lines_are_repositioned_not_recreated() {
        let (mut s, blocks) = store_with_page();
        let (_md, units) = blocks_to_markdown(&blocks);
        let edited = "Second\nFirst";
        let applied = apply_edited_markdown(&mut s, "p1", &units, edited, &HashSet::new()).unwrap();
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

        let applied = apply_edited_markdown(&mut s, "p1", &units, "", &HashSet::new()).unwrap();
        assert_eq!(applied.protected_missing, vec!["tg1".to_string()]);
        assert_eq!(s.page_blocks("p1").unwrap().len(), 1); // not deleted yet

        let mut confirm = HashSet::new();
        confirm.insert("tg1".to_string());
        let applied2 = apply_edited_markdown(&mut s, "p1", &units, "", &confirm).unwrap();
        assert_eq!(applied2.deleted, 1);
        assert!(s.page_blocks("p1").unwrap().is_empty());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p notion-tui markdown::diff --lib`
Expected: FAIL — module `diff` does not exist.

- [ ] **Step 4: Write minimal implementation**

`crates/notion-tui/src/markdown/diff.rs` (above the test module):
```rust
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

    // Pass 1: diff non-protected unit text to find inserts/updates/deletes.
    let editable_before: Vec<&Unit> = before.iter().filter(|u| !u.protected).collect();
    let old_texts: Vec<&str> = editable_before.iter().map(|u| u.text.as_str()).collect();
    let new_idx_of_editable: Vec<usize> = new_lines.iter().enumerate()
        .filter(|(_, l)| l.protected_id.is_none()).map(|(i, _)| i).collect();
    let new_texts: Vec<String> = new_idx_of_editable.iter().map(|&i| render_parsed_text(&new_lines[i])).collect();
    let new_texts_ref: Vec<&str> = new_texts.iter().map(String::as_str).collect();

    let diff = TextDiff::from_slices(&old_texts, &new_texts_ref);
    let mut pending_deletes: Vec<usize> = Vec::new();
    let mut pending_inserts: Vec<usize> = Vec::new();

    macro_rules! flush {
        () => {{
            let n = pending_deletes.len().min(pending_inserts.len());
            for k in 0..n {
                let old_id = editable_before[pending_deletes[k]].id.clone();
                let new_line_idx = new_idx_of_editable[pending_inserts[k]];
                let line = &new_lines[new_line_idx];
                store.edit_update_block_text(&old_id, &line.text)?;
                id_at_new_line[new_line_idx] = Some(old_id);
                result.updated += 1;
            }
            for &d in &pending_deletes[n..] {
                store.edit_delete_block(&editable_before[d].id)?;
                result.deleted += 1;
            }
            for &ins in &pending_inserts[n..] {
                let new_line_idx = new_idx_of_editable[ins];
                let line = &new_lines[new_line_idx];
                let (id, _receipt) = store.edit_insert_block_after(page_id, None, &line.block_type, &line.text)?;
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
                flush!();
                let old_i = change.old_index().unwrap();
                let new_i = new_idx_of_editable[change.new_index().unwrap()];
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
```

`crates/notion-tui/src/markdown/mod.rs` — add the module:
```rust
pub mod diff;
pub mod parse;
pub mod render;

pub use diff::{apply_edited_markdown, Applied};
pub use parse::{parse_markdown, ParsedLine};
pub use render::{blocks_to_markdown, Unit};
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p notion-tui markdown::diff --lib`
Expected: PASS (6 tests).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/notion-tui/Cargo.toml crates/notion-tui/src/markdown/diff.rs crates/notion-tui/src/markdown/mod.rs
git commit -m "feat(notion-tui): diff edited markdown into store edits"
```

---

### Task 7: `$EDITOR` subprocess wrapper

**Files:**
- Create: `crates/notion-tui/src/editor.rs`
- Modify: `crates/notion-tui/src/lib.rs`, `crates/notion-tui/Cargo.toml`

**Interfaces:**
- Consumes: `std::env`, `std::process::Command`, `tempfile` (promoted to a regular dependency).
- Produces: `pub fn edit_text(editor_cmd: &str, initial: &str) -> anyhow::Result<String>`.

- [ ] **Step 1: Promote `tempfile` to a regular dependency**

`crates/notion-tui/Cargo.toml` — move `tempfile` from `[dev-dependencies]` to `[dependencies]`:
```toml
[dependencies]
# ...existing deps...
tempfile = { workspace = true }

[dev-dependencies]
insta = { workspace = true }
wiremock = { workspace = true }
proptest = { workspace = true }
```

- [ ] **Step 2: Write the failing test**

`crates/notion-tui/src/editor.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Builds a tiny "editor" shell script that appends a fixed suffix to the
    /// file it's given, simulating a user making an edit and saving.
    fn fake_editor_appending(suffix: &str) -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("fake-editor.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/sh").unwrap();
        writeln!(f, "printf '%s' \"{suffix}\" >> \"$1\"").unwrap();
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        (dir, script_path.to_string_lossy().to_string())
    }

    #[test]
    fn round_trips_through_a_real_subprocess() {
        let (_dir, editor) = fake_editor_appending("\nappended");
        let result = edit_text(&editor, "initial content").unwrap();
        assert_eq!(result, "initial content\nappended");
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p notion-tui editor:: --lib`
Expected: FAIL — `edit_text` not found.

- [ ] **Step 4: Write minimal implementation**

`crates/notion-tui/src/editor.rs` (above the test module):
```rust
use std::io::Write;

/// Writes `initial` to a scratch `.md` file, runs `editor_cmd <path>` to
/// completion, then reads the file back. Callers are responsible for
/// suspending/restoring the terminal's raw mode around this call (the
/// `TerminalGuard`'s `Drop` already restores the terminal on panic, but a
/// clean run needs an explicit temporary handoff — see Task 8).
pub fn edit_text(editor_cmd: &str, initial: &str) -> anyhow::Result<String> {
    let mut file = tempfile::Builder::new().suffix(".md").tempfile()?;
    file.write_all(initial.as_bytes())?;
    file.flush()?;
    let path = file.path().to_path_buf();

    let status = std::process::Command::new(editor_cmd).arg(&path).status()?;
    if !status.success() {
        anyhow::bail!("editor exited with {status}");
    }
    Ok(std::fs::read_to_string(&path)?)
}

/// Resolves the editor command: `$EDITOR`, falling back to `vi`.
pub fn editor_command() -> String {
    std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string())
}
```

`crates/notion-tui/src/lib.rs` — add the module:
```rust
pub mod app;
pub mod config;
pub mod editor;
pub mod markdown;
pub mod terminal;
pub mod ui;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p notion-tui editor:: --lib`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/notion-tui/Cargo.toml crates/notion-tui/src/editor.rs crates/notion-tui/src/lib.rs
git commit -m "feat(notion-tui): \$EDITOR subprocess round-trip helper"
```

---

### Task 8: confirm modal + `e` key wiring

**Files:**
- Create: `crates/notion-tui/src/ui/confirm.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`, `crates/notion-tui/src/app.rs`
- Test: `crates/notion-tui/tests/editor_flow.rs` (new)

**Interfaces:**
- Consumes: `notion_tui::markdown::{blocks_to_markdown, apply_edited_markdown, Unit}` (Tasks 1, 6), `notion_tui::editor::{edit_text, editor_command}` (Task 7).
- Produces: `pub struct ConfirmState { pub message: String, pub ids: Vec<String> }`, `pub enum ConfirmAction { None, Yes, No }`, `impl ConfirmState { pub fn on_key(&mut self, key: KeyEvent) -> ConfirmAction }`; `App` gains `pub confirm: Option<ConfirmState>`, a private `pending_editor: Option<(String, Vec<Unit>)>` (page_id + before-units, kept across the confirm round-trip), and `pub fn edit_in_editor(&mut self, run_editor: impl FnOnce(&str) -> anyhow::Result<String>)` — the App method takes an injectable editor function so tests never spawn a real subprocess (see Step 3).

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/editor_flow.rs`:
```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_page() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[
        BlockRec { id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
            block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false },
    ]).unwrap();
    Arc::new(Mutex::new(s))
}

fn app_on_page(store: notion_sync::SharedStore) -> App {
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let s = store.lock().unwrap();
    let page = s.get_page("p1").unwrap().unwrap();
    let blocks = s.page_blocks("p1").unwrap();
    drop(s);
    app.view = View::Page(PageView::new(page, blocks));
    app
}

#[test]
fn e_key_edits_via_injected_editor_and_applies_the_result() {
    let store = store_with_page();
    let mut app = app_on_page(store.clone());

    app.edit_in_editor(|_initial| Ok("First\nSecond".to_string()));

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 2);
}

#[test]
fn missing_protected_block_asks_for_confirmation_before_deleting() {
    let store = store_with_page();
    {
        let mut s = store.lock().unwrap();
        s.replace_page_blocks("p1", &[
            BlockRec { id: "tg1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
                block_type: "toggle".into(), payload: "{}".into(), plain_text: "More".into(), has_children: false },
        ]).unwrap();
    }
    let mut app = app_on_page(store.clone());
    if let View::Page(v) = &mut app.view {
        *v = PageView::new(v.page.clone(), store.lock().unwrap().page_blocks("p1").unwrap());
    }

    app.edit_in_editor(|_initial| Ok(String::new())); // user deleted the marker line

    assert!(app.confirm.is_some());
    assert_eq!(store.lock().unwrap().page_blocks("p1").unwrap().len(), 1); // not deleted yet

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));
    assert!(app.confirm.is_none());
    assert!(store.lock().unwrap().page_blocks("p1").unwrap().is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test editor_flow`
Expected: FAIL — `App::edit_in_editor` and `PageView.page` field access (already public) exist, but `edit_in_editor`/`confirm` do not.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/ui/confirm.rs`:
```rust
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub struct ConfirmState {
    pub message: String,
    pub ids: Vec<String>,
}

pub enum ConfirmAction {
    None,
    Yes,
    No,
}

impl ConfirmState {
    pub fn on_key(&mut self, key: KeyEvent) -> ConfirmAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmAction::Yes,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ConfirmAction::No,
            _ => ConfirmAction::None,
        }
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

pub fn render(f: &mut Frame, state: &ConfirmState) {
    let area = f.area();
    let popup = centered_rect((area.width * 2 / 3).clamp(30, 70), 5, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(format!("{}\n(y/n)", state.message))
            .block(Block::default().borders(Borders::ALL).title(" confirm ")),
        popup,
    );
}
```

`crates/notion-tui/src/ui/mod.rs` — register the module and render it:
```rust
pub mod confirm;
pub mod input;
pub mod props;
pub mod search;
pub mod sidebar;
pub mod table;
```
and in `draw()`, alongside the existing `props`/`input` rendering:
```rust
    if let Some(confirm_state) = &app.confirm {
        confirm::render(f, confirm_state);
    }
```

`crates/notion-tui/src/app.rs` — add fields, the editor entry point, and confirm-key routing. Add to `App`:
```rust
pub confirm: Option<crate::ui::confirm::ConfirmState>,
pending_editor: Option<(String, Vec<crate::markdown::Unit>)>, // (page_id, before-units)
```
and to `App::new`'s initializer: `confirm: None, pending_editor: None,`.

Add methods on `impl App`:
```rust
/// Opens the current page's Markdown in an editor. Takes an injectable
/// `run_editor` function so tests don't spawn a real subprocess; production
/// code passes a closure around `editor::edit_text(&editor::editor_command(), initial)`.
pub fn edit_in_editor(&mut self, run_editor: impl FnOnce(&str) -> anyhow::Result<String>) {
    let View::Page(view) = &self.view else { return };
    let page_id = view.page.id.clone();
    let blocks = view.blocks.clone();
    let (md, units) = crate::markdown::blocks_to_markdown(&blocks);

    let edited = match run_editor(&md) {
        Ok(text) => text,
        Err(_) => return,
    };

    let empty = std::collections::HashSet::new();
    let mut guard = self.store.lock().unwrap();
    let applied = crate::markdown::apply_edited_markdown(&mut guard, &page_id, &units, &edited, &empty);
    drop(guard);

    match applied {
        Ok(result) if !result.protected_missing.is_empty() => {
            self.confirm = Some(crate::ui::confirm::ConfirmState {
                message: format!("Delete {} block(s) that no longer appear in the edited text?", result.protected_missing.len()),
                ids: result.protected_missing,
            });
            self.pending_editor = Some((page_id, units));
        }
        Ok(_) => self.refresh_current_view(),
        Err(_) => {}
    }
}

fn confirm_delete_protected(&mut self, confirm: bool) {
    if let Some((page_id, units)) = self.pending_editor.take() {
        if confirm {
            if let Some(state) = self.confirm.take() {
                let ids: std::collections::HashSet<String> = state.ids.into_iter().collect();
                let mut guard = self.store.lock().unwrap();
                let _ = crate::markdown::apply_edited_markdown(&mut guard, &page_id, &units, "", &ids);
                drop(guard);
            }
        } else {
            self.confirm = None;
        }
    }
    self.refresh_current_view();
}
```

Wire `e` into `dispatch_key` (in the `Focus::Main` / `View::Page` branch alongside `i`/`a`):
```rust
            if key.code == KeyCode::Char('e') {
                app.edit_in_editor(|initial| {
                    crate::editor::edit_text(&crate::editor::editor_command(), initial)
                });
                return;
            }
```

And route confirm-modal keys at the very top of `dispatch_key`, before the `props`/`input`/`search` checks:
```rust
    if app.confirm.is_some() {
        let action = app.confirm.as_mut().unwrap().on_key(key);
        match action {
            crate::ui::confirm::ConfirmAction::None => {}
            crate::ui::confirm::ConfirmAction::Yes => app.confirm_delete_protected(true),
            crate::ui::confirm::ConfirmAction::No => app.confirm_delete_protected(false),
        }
        return;
    }
```

`PageView` needs `Clone` for the `pending_editor`-free path above (`view.blocks.clone()` only clones `Vec<BlockRec>`, which is already `Clone`; no change needed there — `BlockRec` already derives `Clone`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test editor_flow`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/ui/confirm.rs crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/app.rs crates/notion-tui/tests/editor_flow.rs
git commit -m "feat(notion-tui): e key opens \$EDITOR round-trip with protected-block confirmation"
```

---

### Task 9: workspace e2e test

**Files:**
- Create: `crates/notion-tui/tests/e2e_editor.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–8 plus the M1/M2 crawl (`notion_sync::pull_once`).
- Produces: nothing new — end-to-end confirmation the whole path works together.

- [ ] **Step 1: Write the test**

`crates/notion-tui/tests/e2e_editor.rs`:
```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::pull_once;
use notion_tui::app::App;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn crawl_then_editor_round_trip_updates_and_inserts_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-06T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title", "title": [{"plain_text": "Notes"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph", "has_children": false,
                "paragraph": {"rich_text": [{"plain_text": "First"}]}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    pull_once(&client, &store).await.unwrap();

    let mut app = App::new(store.clone());
    app.refresh_sidebar();
    let node = app.sidebar.selected().cloned().unwrap();
    app.open_node(&node);

    app.edit_in_editor(|initial| {
        assert_eq!(initial, "First");
        Ok(format!("{initial}, edited\nSecond paragraph"))
    });

    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 2);
    let b1 = blocks.iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "First, edited");
    assert_eq!(store.lock().unwrap().ops().unwrap().len(), 2); // update_block + append_block
}
```

- [ ] **Step 2: Run test**

Run: `cargo test -p notion-tui --test e2e_editor`
Expected: PASS (this task has no separate implementation — it is the integration checkpoint for Tasks 1–8; if it fails, the bug is in one of those tasks, not in new code).

- [ ] **Step 3: Run the full workspace suite**

Run: `cargo test`
Expected: PASS across all crates — no regressions in M1/M2 behavior.

- [ ] **Step 4: Commit**

```bash
git add crates/notion-tui/tests/e2e_editor.rs
git commit -m "test(notion-tui): editor round-trip end-to-end smoke test"
```

---

## Verification (final)

- `cargo test` at the workspace root: all green.
- Manual smoke test against the user's real Notion workspace: open a page with a paragraph and a toggle, press `e`, edit the paragraph text and add a new bullet in `$EDITOR`, save and quit — confirm the paragraph updates in place (same block, so any existing comment on it survives) and the new bullet appears; delete the toggle's marker line, save, confirm the `y/n` prompt appears, confirm `y`, and verify the toggle is gone both locally and (after the next push) in the Notion app.
