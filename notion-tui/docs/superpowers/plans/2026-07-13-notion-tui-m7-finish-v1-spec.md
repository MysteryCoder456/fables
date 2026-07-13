# notion-tui M7 — Finish the v1 Spec: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close every promised-but-missing v1 behavior — palette parity, mouse support, universal back-history, table filtering, first-crawl progress/checkpointing, status-bar completeness, block placeholders, and remote-deletion propagation — so notion-tui matches its own spec.

**Architecture:** No new crates. `notion-tui` gains a shared subsequence-fuzzy matcher module (`fuzzy.rs`) reused by the command palette and workspace search re-ranking, a generic `Picker` popup reused by "move page" and "group by", stored last-frame layout rects for mouse hit-testing, a centralized `apply_action` used by both keyboard and mouse dispatch (needed for universal history), table filter state, and status-bar breadcrumb/pending-key rendering. `notion-store` gains rename/move-page edit ops, a `meta_delete`, and a mark-and-sweep `prune_missing`. `notion-sync` gains a `SyncHandle` wake-notify (manual "sync now"), incrementally checkpointed first-crawl progress (`SyncStatus::Syncing{done,total}`), and a periodic full-crawl reconciliation pass. `notion-api` gains a `get_page` wrapper (404/archived detection).

**Tech Stack:** Existing: ratatui/crossterm (mouse events + `ListState::offset()`), tokio (`watch`, `Notify`), reqwest(rustls), rusqlite(bundled), wiremock, insta.

## Global Constraints

- Notion API version `2025-09-03` (unchanged).
- The git root is the monorepo `/Users/rehatbir/Developer/fables`; the workspace lives in `notion-tui/`. All `cargo` commands run from `/Users/rehatbir/Developer/fables/notion-tui`.
- Quality gates that must stay green after every task: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- Never weaken an existing test to make it pass; if a behavior intentionally changes (e.g. `pull_once`'s signature, `status_line`'s parameter order), update the test and say so in the step.
- Mandated user-facing copy: sync progress renders as `⟳ syncing {done}/{total} pages` with thousands separators, e.g. `⟳ syncing 240/1,893 pages`, until `total` is known, then falls back to `⟳ syncing…`.
- Palette/search matching is subsequence-fuzzy (fzf-style), never typo-tolerant (spec §5 descope) — workspace search stays FTS-prefix first, subsequence re-ranks on top.
- Mouse support is on by default but never required — every mouse action must have an existing keyboard equivalent.
- Commit steps inside each task are plan content for **executors** to run — the planning agent that wrote this document never runs `git commit` itself.

---

### Task 1: Shared subsequence-fuzzy matcher module

**Files:**
- Create: `crates/notion-tui/src/fuzzy.rs`
- Modify: `crates/notion-tui/src/lib.rs` (add `pub mod fuzzy;`)

**Interfaces:**
- Consumes: nothing.
- Produces (Tasks 2, 3, 4, 5 depend on these exact signatures):
```rust
pub fn subsequence_score(query: &str, candidate: &str) -> Option<i64>;
pub fn subsequence_rank<T>(query: &str, items: Vec<(T, String)>) -> Vec<T>;
```
  `subsequence_score` is case-insensitive; returns `None` when `query`'s characters don't all appear in `candidate` in order, `Some(score)` otherwise (higher is better; empty query always scores `Some(0)`, matching everything). `subsequence_rank` filters out non-matches and stable-sorts the rest descending by score.

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-tui/src/fuzzy.rs`:

```rust
/// Case-insensitive subsequence match with a lightweight fzf-style score:
/// +10 per matched character, +15 when it's contiguous with the previous
/// match, +20 when the match starts at position 0 (prefix bonus). Returns
/// `None` when `query` is not a subsequence of `candidate`.
pub fn subsequence_score(query: &str, candidate: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c: Vec<char> = candidate.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut score: i64 = 0;
    let mut last_match: Option<usize> = None;
    for (ci, ch) in c.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if *ch == q[qi] {
            score += 10;
            if last_match == Some(ci.wrapping_sub(1)) {
                score += 15;
            }
            if ci == 0 {
                score += 20;
            }
            last_match = Some(ci);
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

/// Filters `items` to those whose label is a subsequence match for `query`,
/// stable-sorted descending by `subsequence_score` (ties keep `items`' order).
pub fn subsequence_rank<T>(query: &str, items: Vec<(T, String)>) -> Vec<T> {
    let mut scored: Vec<(i64, T)> = items
        .into_iter()
        .filter_map(|(item, label)| subsequence_score(query, &label).map(|s| (s, item)))
        .collect();
    scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    scored.into_iter().map(|(_, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything_with_zero_score() {
        assert_eq!(subsequence_score("", "anything"), Some(0));
    }

    #[test]
    fn non_subsequence_is_none() {
        assert_eq!(subsequence_score("xyz", "queue"), None);
    }

    #[test]
    fn out_of_order_subsequence_is_none() {
        assert_eq!(subsequence_score("eq", "queue"), None);
    }

    #[test]
    fn scattered_subsequence_matches() {
        assert!(subsequence_score("qee", "queue").is_some());
    }

    #[test]
    fn contiguous_prefix_match_scores_higher_than_scattered_match() {
        let contiguous = subsequence_score("que", "queue").unwrap();
        let scattered = subsequence_score("que", "q_u_e_ntity").unwrap();
        assert!(contiguous > scattered, "{contiguous} should beat {scattered}");
    }

    #[test]
    fn rank_filters_non_matches_and_orders_by_score() {
        let items = vec![
            ("no-match", "zzz".to_string()),
            ("scattered", "q_u_e".to_string()),
            ("exact-prefix", "queue".to_string()),
        ];
        let ranked = subsequence_rank("que", items);
        assert_eq!(ranked, vec!["exact-prefix", "scattered"]);
    }
}
```

- [ ] **Step 2: Run to verify it fails to even compile as a module**

Run: `cargo test -p notion-tui fuzzy`
Expected: FAIL — `fuzzy` is not yet declared as a module in `lib.rs`, so `cargo test -p notion-tui fuzzy` reports no tests matched (0 run) rather than the 6 tests above.

- [ ] **Step 3: Wire the module in**

In `crates/notion-tui/src/lib.rs`, add `pub mod fuzzy;` alongside the other `pub mod` lines.

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui fuzzy`
Expected: `6 passed`.

---

### Task 2: Command palette — subsequence matching + "sync now" + "rename"

**Files:**
- Modify: `crates/notion-tui/src/ui/palette.rs`
- Modify: `crates/notion-tui/src/app.rs` (`InputPurpose`, `run_command`, `dispatch_key`'s `InputAction::Submit` match, new `rename_page`/`start_rename`/`request_sync_now` methods)
- Modify: `crates/notion-sync/src/lib.rs` (`SyncHandle` gains `notify`; wired in Task 12 — this task only adds the `App`-side field and a no-op-safe call)
- Modify: `crates/notion-store/src/store.rs` (`edit_rename_page`, `Inverse::RenamePage`)
- Modify: `crates/notion-sync/src/pusher.rs` (`push_rename_page`, `extract_page_id` gains `"rename_page"`)
- Test: `crates/notion-tui/tests/palette_command_flow.rs` (new), inline tests in `palette.rs` and `store.rs`

**Interfaces:**
- Consumes: `fuzzy::subsequence_rank` (Task 1).
- Produces (Task 3 and Task 12 depend on these):
  - `PaletteState::matches(&self) -> Vec<&'static str>` now subsequence-filtered/ranked instead of substring-filtered.
  - `COMMANDS` gains `"rename"` and `"sync now"` (Task 3 adds `"move page"`, Task 4 adds `"group by"`).
  - `App::request_sync_now(&mut self)` — no-ops safely with a notice when `self.sync_notify` is `None` (Task 12 populates it from `main.rs`).
  - `Store::edit_rename_page(&mut self, page_id: &str, new_title: &str) -> anyhow::Result<EditReceipt>`.

- [ ] **Step 1: Write the failing test for subsequence palette matching**

Append to `crates/notion-tui/src/ui/palette.rs`'s test module (create one if absent, following the pattern in `search.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_subsequence_not_substring() {
        let mut p = PaletteState::new();
        p.input = "bd".to_string(); // not a substring of "board", but is a subsequence
        assert!(p.matches().contains(&"board"));
    }

    #[test]
    fn non_subsequence_input_matches_nothing() {
        let mut p = PaletteState::new();
        p.input = "zzz".to_string();
        assert!(p.matches().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui palette::tests`
Expected: FAIL — `matches()` uses `c.contains(self.input.as_str())`, so `"bd"` doesn't match `"board"`.

- [ ] **Step 3: Implement subsequence matching + new commands**

```rust
pub const COMMANDS: &[&str] = &[
    "help", "queue", "board", "table", "quit", "rename", "move page", "group by", "sync now",
];
```

```rust
pub fn matches(&self) -> Vec<&'static str> {
    let items: Vec<(&'static str, String)> = COMMANDS.iter().map(|c| (*c, c.to_string())).collect();
    crate::fuzzy::subsequence_rank(&self.input, items)
}
```

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui palette::tests`
Expected: `2 passed`.

- [ ] **Step 5: Write the failing store test for page rename**

Append to `crates/notion-store/tests/` — create `crates/notion-store/tests/rename.rs`:

```rust
use notion_store::{PageRec, Store};

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: title.into(),
        icon: None,
        archived: false,
        last_edited_time: "t0".into(),
    }
}

#[test]
fn rename_page_updates_title_marks_dirty_and_enqueues_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Old Title")).unwrap();

    let receipt = s.edit_rename_page("p1", "New Title").unwrap();

    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "New Title");
    assert!(s.is_page_dirty("p1").unwrap());
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "rename_page");
    assert_eq!(ops[0].target_id, "p1");
    assert_eq!(receipt.op_seq, ops[0].seq);
}

#[test]
fn undo_rename_restores_old_title() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "Old Title")).unwrap();
    let receipt = s.edit_rename_page("p1", "New Title").unwrap();

    s.undo(receipt).unwrap();

    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "Old Title");
    assert!(s.ops().unwrap().is_empty());
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test -p notion-tui-store --test rename`
Expected: FAIL — `edit_rename_page` doesn't exist.

- [ ] **Step 7: Implement `Store::edit_rename_page`**

In `crates/notion-store/src/store.rs`, add near `edit_update_block_text`:

```rust
pub fn edit_rename_page(&mut self, page_id: &str, new_title: &str) -> anyhow::Result<EditReceipt> {
    let old_title: String =
        self.conn
            .query_row("SELECT title FROM pages WHERE id = ?1", [page_id], |r| r.get(0))?;
    let base: String = self.conn.query_row(
        "SELECT last_edited_time FROM pages WHERE id = ?1",
        [page_id],
        |r| r.get(0),
    )?;
    self.conn.execute(
        "UPDATE pages SET title = ?2, dirty = 1 WHERE id = ?1",
        rusqlite::params![page_id, new_title],
    )?;

    let op_payload = json!({"title": new_title}).to_string();
    let op_seq = self.enqueue_op("rename_page", page_id, &op_payload, Some(&base))?;

    Ok(EditReceipt {
        op_seq,
        inverse: Inverse::RenamePage {
            page_id: page_id.to_string(),
            old_title,
        },
    })
}
```

Add the variant to `Inverse`:

```rust
    RenamePage { page_id: String, old_title: String },
```

Handle it in `apply_inverse_locally`:

```rust
            Inverse::RenamePage { page_id, old_title } => {
                self.conn.execute(
                    "UPDATE pages SET title = ?2 WHERE id = ?1",
                    rusqlite::params![page_id, old_title],
                )?;
            }
```

and in `apply_inverse_as_new_edit`:

```rust
            Inverse::RenamePage { page_id, old_title } => {
                self.edit_rename_page(page_id, old_title)?;
            }
```

- [ ] **Step 8: Run to pass**

Run: `cargo test -p notion-tui-store --test rename`
Expected: `2 passed`.

- [ ] **Step 9: Push the rename op — write the failing pusher test**

Append to `crates/notion-sync/tests/push.rs` (reuse its scaffolding: an in-memory store with a queued op + a wiremock server):

```rust
#[tokio::test]
async fn push_once_sends_a_rename_page_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Old".into(),
        icon: None,
        archived: false,
        last_edited_time: "2026-07-05T10:00:00.000Z".into(),
    })
    .unwrap();
    s.edit_rename_page("p1", "New").unwrap();
    let store: SharedStore = Arc::new(Mutex::new(s));

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "p1"})))
        .mount(&server)
        .await;

    let client = fast_client(server.uri());
    let pushed = push_once(&client, &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
```

(Reuse `push.rs`'s existing `fast_client` helper and imports; add `PageRec` to the `use notion_store::{...}` line if not already imported.)

- [ ] **Step 10: Run to verify it fails**

Run: `cargo test -p notion-tui-sync --test push push_once_sends_a_rename_page_op`
Expected: FAIL — `push_one` returns `Failed("unknown op_type rename_page")`.

- [ ] **Step 11: Implement `push_rename_page`**

In `crates/notion-sync/src/pusher.rs`, add:

```rust
async fn push_rename_page(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&op.target_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let title = payload["title"].as_str().unwrap_or_default();
    let body = json!({
        "properties": {"title": {"title": [{"type": "text", "text": {"content": title}}]}}
    });
    client.update_page(&op.target_id, body).await?;

    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &op.target_id) {
        lock_store(store).clear_page_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}
```

Wire it into `push_one`'s match: `"rename_page" => push_rename_page(client, store, op).await,`.

Extend `extract_page_id` so `remaining_ops_reference_page` also sees rename ops:

```rust
fn extract_page_id(op: &OpRec) -> Option<String> {
    match op.op_type.as_str() {
        "update_block" | "append_block" | "delete_block" | "reorder_block" => {
            let v: Value = serde_json::from_str(&op.payload).ok()?;
            v["page_id"].as_str().map(str::to_string)
        }
        "rename_page" => Some(op.target_id.clone()),
        _ => None,
    }
}
```

- [ ] **Step 12: Run to pass**

Run: `cargo test -p notion-tui-sync --test push`
Expected: PASS, including the new test.

- [ ] **Step 13: Wire "rename" and "sync now" into the palette on the app side**

In `crates/notion-tui/src/app.rs`, add to `InputPurpose`:

```rust
    RenamePage { page_id: String },
    RenameRow { row_id: String, title_prop_name: String },
```

Add to `App`: `pub sync_notify: Option<std::sync::Arc<tokio::sync::Notify>>` (init `None` in `App::new`).

Add methods:

```rust
pub fn start_rename(&mut self) {
    match &self.view {
        View::Page(v) => {
            self.input = Some(InputState::new("rename page", v.page.title.clone()));
            self.input_purpose = Some(InputPurpose::RenamePage { page_id: v.page.id.clone() });
        }
        View::Table(v) => {
            let title_col = v.columns.iter().find(|c| c.prop_type == "title").cloned();
            if let (Some(row), Some(col)) = (v.rows.get(v.cursor), title_col) {
                self.input = Some(InputState::new("rename", v.cell(row, &col)));
                self.input_purpose = Some(InputPurpose::RenameRow {
                    row_id: row.id.clone(),
                    title_prop_name: col.name,
                });
            }
        }
        View::Board(v) => {
            let title_col = crate::ui::table::schema_columns(&v.ds.schema_json)
                .into_iter()
                .find(|c| c.prop_type == "title");
            if let (Some(row), Some(col)) = (v.cards_in(v.col).get(v.card).copied(), title_col) {
                let text = crate::ui::table::cell_text(
                    &serde_json::from_str::<serde_json::Value>(&row.properties).unwrap_or_default()
                        [&col.name],
                );
                self.input = Some(InputState::new("rename", text));
                self.input_purpose = Some(InputPurpose::RenameRow {
                    row_id: row.id.clone(),
                    title_prop_name: col.name,
                });
            }
        }
        _ => self.notice = Some("nothing to rename here".into()),
    }
}

pub fn rename_page(&mut self, page_id: &str, new_title: &str) {
    if let Ok(receipt) = self.store.lock().unwrap().edit_rename_page(page_id, new_title) {
        self.undo_stack.push(receipt);
    }
    self.refresh_current_view();
    self.refresh_sidebar();
}

pub fn request_sync_now(&mut self) {
    match &self.sync_notify {
        Some(n) => {
            n.notify_one();
            self.notice = Some("sync requested".into());
        }
        None => self.notice = Some("sync not available".into()),
    }
}
```

Note: `TableView::cell` takes `&Column` by reference and `Column` isn't `Clone` today — add `#[derive(Clone)]` to `pub struct Column` in `crates/notion-tui/src/ui/table.rs` (it's two owned `String` fields, trivially cloneable) so `title_col.cloned()` above compiles.

Extend `run_command`:

```rust
            "rename" => self.start_rename(),
            "sync now" => self.request_sync_now(),
```

Extend the `InputAction::Submit` match in `dispatch_key`:

```rust
                        InputPurpose::RenamePage { page_id } => app.rename_page(&page_id, &text),
                        InputPurpose::RenameRow {
                            row_id,
                            title_prop_name,
                        } => app.update_row_property(&row_id, &title_prop_name, "title", &text),
```

- [ ] **Step 14: Write the end-to-end flow test**

Create `crates/notion-tui/tests/palette_command_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_page() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Old Title".into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    })
    .unwrap();
    Arc::new(Mutex::new(s))
}

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}

#[test]
fn palette_rename_command_renames_the_open_page() {
    let store = store_with_page();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let page = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    app.view = View::Page(PageView::new(page, Vec::new()));

    dispatch_key(&mut app, key(':')); // open palette
    for c in "rename".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // run "rename"
    assert!(app.input.is_some(), "rename should open a text prompt");

    // Replace the prefilled title.
    for _ in 0.."Old Title".len() {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    }
    for c in "New Title".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    assert_eq!(store.lock().unwrap().get_page("p1").unwrap().unwrap().title, "New Title");
}

#[test]
fn sync_now_without_a_running_sync_reports_unavailable() {
    let mut app = App::new(store_with_page());
    app.request_sync_now();
    assert_eq!(app.notice.as_deref(), Some("sync not available"));
}
```

- [ ] **Step 15: Run to pass**

Run: `cargo test -p notion-tui --test palette_command_flow`
Expected: `2 passed`.

- [ ] **Step 16: Full crate gate**

Run: `cargo test -p notion-tui-store -p notion-tui-sync -p notion-tui`
Expected: PASS.

---

### Task 3: Generic `Picker` popup + "move page" command

**Files:**
- Create: `crates/notion-tui/src/ui/picker.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs` (register module, render when open)
- Modify: `crates/notion-tui/src/app.rs` (`picker`/`picker_purpose` fields, `start_move_page`, `move_page`, palette wiring, `dispatch_key` routing)
- Modify: `crates/notion-store/src/store.rs` (`edit_move_page`, `Inverse::MovePage`)
- Modify: `crates/notion-sync/src/pusher.rs` (`push_move_page`)
- Test: inline in `picker.rs`, `crates/notion-tui/tests/palette_command_flow.rs` (extend)

**Interfaces:**
- Consumes: `fuzzy::subsequence_rank` (Task 1).
- Produces (Task 4 depends on `PickerState`/`PickerAction`):
```rust
pub struct PickerState { pub title: String, pub input: String, pub items: Vec<(String, String)>, pub cursor: usize, pub list_state: ListState }
pub enum PickerAction { None, Changed, Close, Choose(String) }
impl PickerState {
    pub fn new(title: impl Into<String>, items: Vec<(String, String)>) -> Self;
    pub fn matches(&self) -> Vec<&(String, String)>;
    pub fn on_key(&mut self, key: KeyEvent) -> PickerAction;
}
```
  `items` are `(id, label)` pairs; `matches()` subsequence-filters/ranks by `label`.
  - `App::picker: Option<PickerState>`, `App::picker_purpose: Option<PickerPurpose>` with `pub enum PickerPurpose { MovePage { page_id: String }, GroupBy { data_source_id: String } }` (the `GroupBy` arm is unused until Task 4).

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-tui/src/ui/picker.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

pub struct PickerState {
    pub title: String,
    pub input: String,
    pub items: Vec<(String, String)>,
    pub cursor: usize,
    pub list_state: ListState,
}

pub enum PickerAction {
    None,
    Changed,
    Close,
    Choose(String),
}

impl PickerState {
    pub fn new(title: impl Into<String>, items: Vec<(String, String)>) -> PickerState {
        PickerState {
            title: title.into(),
            input: String::new(),
            items,
            cursor: 0,
            list_state: ListState::default(),
        }
    }

    pub fn matches(&self) -> Vec<&(String, String)> {
        let candidates: Vec<(&(String, String), String)> =
            self.items.iter().map(|it| (it, it.1.clone())).collect();
        crate::fuzzy::subsequence_rank(&self.input, candidates)
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Esc => PickerAction::Close,
            KeyCode::Enter => match self.matches().get(self.cursor) {
                Some((id, _)) => PickerAction::Choose(id.clone()),
                None => PickerAction::Close,
            },
            KeyCode::Down => {
                let n = self.matches().len();
                if n > 0 {
                    self.cursor = (self.cursor + 1).min(n - 1);
                }
                PickerAction::None
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerAction::None
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.cursor = 0;
                PickerAction::Changed
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.cursor = 0;
                PickerAction::Changed
            }
            _ => PickerAction::None,
        }
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

pub fn render(f: &mut Frame, state: &mut PickerState) {
    let area = f.area();
    let popup = centered_rect((area.width * 2 / 3).clamp(24, 60), 14, area);
    f.render_widget(Clear, popup);
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);
    f.render_widget(
        Paragraph::new(state.input.as_str())
            .block(Block::default().borders(Borders::ALL).title(format!(" {} ", state.title))),
        inner[0],
    );
    let items: Vec<ListItem> = state.matches().iter().map(|(_, label)| ListItem::new(label.as_str())).collect();
    state.list_state.select(Some(state.cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .block(Block::default().borders(Borders::ALL)),
        inner[1],
        &mut state.list_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<(String, String)> {
        vec![
            ("id1".into(), "Roadmap".into()),
            ("id2".into(), "Retro Notes".into()),
            ("id3".into(), "Budget".into()),
        ]
    }

    #[test]
    fn typing_filters_by_subsequence_on_label() {
        let mut p = PickerState::new("move to…", items());
        for c in "rmap".chars() {
            p.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let labels: Vec<&str> = p.matches().iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, vec!["Roadmap"]);
    }

    #[test]
    fn enter_chooses_the_highlighted_id() {
        let mut p = PickerState::new("move to…", items());
        match p.on_key(KeyEvent::from(KeyCode::Enter)) {
            PickerAction::Choose(id) => assert_eq!(id, "id1"),
            _ => panic!("expected Choose"),
        }
    }

    #[test]
    fn esc_closes() {
        let mut p = PickerState::new("move to…", items());
        assert!(matches!(p.on_key(KeyEvent::from(KeyCode::Esc)), PickerAction::Close));
    }
}
```

- [ ] **Step 2: Register the module and run**

Add `pub mod picker;` to `crates/notion-tui/src/ui/mod.rs`.

Run: `cargo test -p notion-tui picker::tests`
Expected: `3 passed`.

- [ ] **Step 3: Render the picker from `ui::draw`**

In `crates/notion-tui/src/ui/mod.rs`'s `draw`, alongside the existing `if let Some(palette_state) = &mut app.palette { ... }`:

```rust
    if let Some(picker_state) = &mut app.picker {
        picker::render(f, picker_state);
    }
```

- [ ] **Step 4: Write the failing store test for move-page**

Create `crates/notion-store/tests/move_page.rs`:

```rust
use notion_store::{PageRec, Store};

fn page(id: &str, parent_type: &str, parent_id: Option<&str>) -> PageRec {
    PageRec {
        id: id.into(),
        parent_type: parent_type.into(),
        parent_id: parent_id.map(String::from),
        title: "T".into(),
        icon: None,
        archived: false,
        last_edited_time: "t0".into(),
    }
}

#[test]
fn move_page_reparents_locally_marks_dirty_and_enqueues_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None)).unwrap();
    s.upsert_page(&page("p2", "workspace", None)).unwrap();

    let receipt = s.edit_move_page("p1", "p2").unwrap();

    let moved = s.get_page("p1").unwrap().unwrap();
    assert_eq!(moved.parent_type, "page_id");
    assert_eq!(moved.parent_id.as_deref(), Some("p2"));
    assert!(s.is_page_dirty("p1").unwrap());
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "move_page");
    let _ = receipt;
}
```

- [ ] **Step 5: Run to verify it fails**

Run: `cargo test -p notion-tui-store --test move_page`
Expected: FAIL — `edit_move_page` doesn't exist.

- [ ] **Step 6: Implement `Store::edit_move_page`**

In `crates/notion-store/src/store.rs`:

```rust
pub fn edit_move_page(&mut self, page_id: &str, new_parent_id: &str) -> anyhow::Result<EditReceipt> {
    let (old_parent_type, old_parent_id, base): (String, Option<String>, String) = self.conn.query_row(
        "SELECT parent_type, parent_id, last_edited_time FROM pages WHERE id = ?1",
        [page_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    self.conn.execute(
        "UPDATE pages SET parent_type = 'page_id', parent_id = ?2, dirty = 1 WHERE id = ?1",
        rusqlite::params![page_id, new_parent_id],
    )?;

    let op_payload = json!({"new_parent_id": new_parent_id}).to_string();
    let op_seq = self.enqueue_op("move_page", page_id, &op_payload, Some(&base))?;

    Ok(EditReceipt {
        op_seq,
        inverse: Inverse::MovePage {
            page_id: page_id.to_string(),
            old_parent_type,
            old_parent_id,
        },
    })
}
```

Add the `Inverse` variant `MovePage { page_id: String, old_parent_type: String, old_parent_id: Option<String> }`, and in `apply_inverse_locally`:

```rust
            Inverse::MovePage { page_id, old_parent_type, old_parent_id } => {
                self.conn.execute(
                    "UPDATE pages SET parent_type = ?2, parent_id = ?3 WHERE id = ?1",
                    rusqlite::params![page_id, old_parent_type, old_parent_id],
                )?;
            }
```

and in `apply_inverse_as_new_edit`, treat a move-undo as a no-op-safe re-move only if there's a concrete prior page parent (skip workspace-root restores — YAGNI for v1, note it in the match with a comment):

```rust
            Inverse::MovePage { page_id, old_parent_id, .. } => {
                if let Some(parent) = old_parent_id {
                    self.edit_move_page(page_id, parent)?;
                }
            }
```

- [ ] **Step 7: Run to pass**

Run: `cargo test -p notion-tui-store --test move_page`
Expected: PASS.

- [ ] **Step 8: Push the move — write the failing pusher test**

Append to `crates/notion-sync/tests/push.rs`:

```rust
#[tokio::test]
async fn push_once_sends_a_move_page_op() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "T".into(), icon: None, archived: false,
        last_edited_time: "2026-07-05T10:00:00.000Z".into(),
    }).unwrap();
    s.edit_move_page("p1", "p2").unwrap();
    let store: SharedStore = Arc::new(Mutex::new(s));

    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T10:00:00.000Z"
        })))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "p1"})))
        .mount(&server).await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}
```

- [ ] **Step 9: Run to verify it fails, then implement**

Run: `cargo test -p notion-tui-sync --test push push_once_sends_a_move_page_op`
Expected: FAIL — unknown op type.

In `pusher.rs`:

```rust
async fn push_move_page(client: &NotionClient, store: &SharedStore, op: &OpRec) -> Result<PushOutcome, ApiError> {
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&op.target_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let new_parent_id = payload["new_parent_id"].as_str().unwrap_or_default();
    client
        .update_page(&op.target_id, json!({"parent": {"page_id": new_parent_id}}))
        .await?;
    lock_store(store).delete_op(op.seq).ok();
    if !remaining_ops_reference_page(store, &op.target_id) {
        lock_store(store).clear_page_dirty(&op.target_id).ok();
    }
    Ok(PushOutcome::Success)
}
```

Add `"move_page" => push_move_page(client, store, op).await,` to `push_one`, and `"move_page" => Some(op.target_id.clone()),` to `extract_page_id`.

- [ ] **Step 10: Run to pass**

Run: `cargo test -p notion-tui-sync --test push`
Expected: PASS.

- [ ] **Step 11: Wire "move page" into the app**

In `crates/notion-tui/src/app.rs`, add fields:

```rust
    pub picker: Option<crate::ui::picker::PickerState>,
```
```rust
pub enum PickerPurpose {
    MovePage { page_id: String },
    GroupBy { data_source_id: String }, // unused until group-by (next task)
}
```
Add `pub picker_purpose: Option<PickerPurpose>` to `App`; init both to `None` in `App::new`.

```rust
pub fn start_move_page(&mut self) {
    let View::Page(v) = &self.view else {
        self.notice = Some("open a page to move it".into());
        return;
    };
    let page_id = v.page.id.clone();
    let items: Vec<(String, String)> = self
        .store
        .lock()
        .unwrap()
        .sidebar_nodes()
        .unwrap_or_default()
        .into_iter()
        .filter(|n| n.kind == NodeKind::Page && n.id != page_id)
        .map(|n| (n.id, n.title))
        .collect();
    self.picker = Some(crate::ui::picker::PickerState::new("move to…", items));
    self.picker_purpose = Some(PickerPurpose::MovePage { page_id });
}

pub fn move_page(&mut self, page_id: &str, new_parent_id: &str) {
    if let Ok(receipt) = self.store.lock().unwrap().edit_move_page(page_id, new_parent_id) {
        self.undo_stack.push(receipt);
    }
    self.refresh_current_view();
    self.refresh_sidebar();
}
```

Extend `run_command`: `"move page" => self.start_move_page(),`.

In `dispatch_key`, add picker routing right after the existing `if app.palette.is_some() { ... }` block:

```rust
    if app.picker.is_some() {
        let action = app.picker.as_mut().unwrap().on_key(key);
        match action {
            crate::ui::picker::PickerAction::None | crate::ui::picker::PickerAction::Changed => {}
            crate::ui::picker::PickerAction::Close => {
                app.picker = None;
                app.picker_purpose = None;
            }
            crate::ui::picker::PickerAction::Choose(id) => {
                app.picker = None;
                if let Some(purpose) = app.picker_purpose.take() {
                    match purpose {
                        PickerPurpose::MovePage { page_id } => app.move_page(&page_id, &id),
                        PickerPurpose::GroupBy { .. } => {} // wired in the next task
                    }
                }
            }
        }
        return;
    }
```

- [ ] **Step 12: Write and run the flow test**

Append to `crates/notion-tui/tests/palette_command_flow.rs`:

```rust
#[test]
fn move_page_command_reparents_via_the_picker() {
    let store = store_with_page();
    {
        let mut s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "dest".into(), parent_type: "workspace".into(), parent_id: None,
            title: "Destination".into(), icon: None, archived: false, last_edited_time: "t1".into(),
        })
        .unwrap();
    }
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    let page = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    app.view = View::Page(PageView::new(page, Vec::new()));

    app.start_move_page();
    assert!(app.picker.is_some());
    for c in "Dest".chars() {
        notion_tui::app::dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let moved = store.lock().unwrap().get_page("p1").unwrap().unwrap();
    assert_eq!(moved.parent_id.as_deref(), Some("dest"));
}
```

Run: `cargo test -p notion-tui --test palette_command_flow`
Expected: `3 passed`.

- [ ] **Step 13: Full crate gate**

Run: `cargo test -p notion-tui-store -p notion-tui-sync -p notion-tui`
Expected: PASS.

---

### Task 4: Board "group by" command (fixes the hard-picked first-status default)

**Files:**
- Modify: `crates/notion-tui/src/ui/board.rs` (`BoardView::with_group`, `group_override` field)
- Modify: `crates/notion-tui/src/app.rs` (`refresh_current_view`'s `View::Board` arm, `start_group_by`, `set_board_group`, palette wiring, picker `Choose` arm)
- Test: inline in `board.rs`, `crates/notion-tui/tests/board_flow.rs` (extend)

**Interfaces:**
- Consumes: `PickerState`/`PickerPurpose::GroupBy` (Task 3).
- Produces: `BoardView::with_group(ds, rows, override_: Option<(String, String)>) -> BoardView`; `BoardView::new` becomes a thin wrapper (`Self::with_group(ds, rows, None)`), so every existing call site keeps compiling unchanged.

- [ ] **Step 1: Write the failing test**

Append to `board.rs`'s test module:

```rust
    #[test]
    fn with_group_override_beats_the_first_status_heuristic() {
        let schema = json!({
            "Name": {"type": "title"},
            "Status": {"type": "status", "status": {"options": [{"name": "Todo"}, {"name": "Done"}]}},
            "Priority": {"type": "select", "select": {"options": [{"name": "Low"}, {"name": "High"}]}}
        })
        .to_string();
        let ds = DataSourceRec {
            id: "ds".into(), database_id: "db".into(), title: "T".into(),
            schema_json: schema, last_edited_time: "t".into(),
        };
        let v = BoardView::with_group(ds, vec![], Some(("Priority".to_string(), "select".to_string())));
        assert_eq!(v.group_prop, "Priority");
        assert_eq!(v.columns, vec!["Low", "High", "(none)"]);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui board::tests::with_group_override`
Expected: FAIL — `with_group` doesn't exist.

- [ ] **Step 3: Implement**

```rust
pub struct BoardView {
    pub ds: DataSourceRec,
    pub group_prop: String,
    pub group_type: String,
    pub columns: Vec<String>,
    pub rows: Vec<RowRec>,
    pub col: usize,
    pub card: usize,
    pub list_state: ListState,
    /// User-chosen grouping property (via the "group by" palette command),
    /// carried across `refresh_current_view` rebuilds so a sync tick doesn't
    /// silently revert to the first-status/first-select heuristic.
    pub group_override: Option<(String, String)>,
}

impl BoardView {
    pub fn new(ds: DataSourceRec, rows: Vec<RowRec>) -> BoardView {
        Self::with_group(ds, rows, None)
    }

    pub fn with_group(ds: DataSourceRec, rows: Vec<RowRec>, group_override: Option<(String, String)>) -> BoardView {
        let (group_prop, group_type) = group_override
            .clone()
            .or_else(|| group_property(&ds.schema_json))
            .unwrap_or_else(|| ("".into(), "".into()));
        let mut columns = schema_options(&ds.schema_json, &group_prop, &group_type);
        columns.push("(none)".to_string());
        BoardView {
            ds,
            group_prop,
            group_type,
            columns,
            rows,
            col: 0,
            card: 0,
            list_state: ListState::default(),
            group_override,
        }
    }
```

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui board::tests`
Expected: PASS (existing `BoardView::new` tests are unaffected — `new` still constructs the same defaults).

- [ ] **Step 5: Preserve the override across `refresh_current_view`, and add the command**

In `app.rs`'s `refresh_current_view`, `View::Board` arm — replace `BoardView::new(ds, rows)` with:

```rust
                let group_override = v.group_override.clone();
                ...
                if let Some(ds) = ds {
                    let mut b = BoardView::with_group(ds, rows, group_override);
```

(Keep the rest of the relocation logic — `col`/`card`/selected-row relocation — unchanged; only the constructor call and its extra captured variable change.)

Add to `App`:

```rust
pub fn start_group_by(&mut self) {
    let View::Board(v) = &self.view else {
        self.notice = Some("open a board to change its grouping".into());
        return;
    };
    let data_source_id = v.ds.id.clone();
    let items: Vec<(String, String)> = crate::ui::table::schema_columns(&v.ds.schema_json)
        .into_iter()
        .filter(|c| c.prop_type == "select" || c.prop_type == "status")
        .map(|c| (format!("{}\u{1}{}", c.name, c.prop_type), c.name))
        .collect();
    if items.is_empty() {
        self.notice = Some("no select/status property to group by".into());
        return;
    }
    self.picker = Some(crate::ui::picker::PickerState::new("group by…", items));
    self.picker_purpose = Some(PickerPurpose::GroupBy { data_source_id });
}

pub fn set_board_group(&mut self, encoded: &str) {
    let Some((name, ptype)) = encoded.split_once('\u{1}') else { return };
    if let View::Board(v) = std::mem::replace(&mut self.view, View::Empty) {
        self.view = View::Board(BoardView::with_group(v.ds, v.rows, Some((name.to_string(), ptype.to_string()))));
    }
}
```

Extend `run_command`: `"group by" => self.start_group_by(),`.

In `dispatch_key`'s picker `Choose(id)` match, fill in the previously-empty arm:

```rust
                        PickerPurpose::GroupBy { .. } => app.set_board_group(&id),
```

- [ ] **Step 6: Write the flow test**

Append to `crates/notion-tui/tests/board_flow.rs`:

```rust
#[test]
fn group_by_command_overrides_the_default_status_grouping() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds".into(), database_id: "db".into(), title: "Tasks".into(),
        schema_json: json!({
            "Name": {"type": "title"},
            "Status": {"type": "status", "status": {"options": [{"name": "Todo"}]}},
            "Priority": {"type": "select", "select": {"options": [{"name": "Low"}, {"name": "High"}]}}
        }).to_string(),
        last_edited_time: "t".into(),
    }).unwrap();
    s.replace_rows("ds", &[RowRec {
        id: "r1".into(), data_source_id: "ds".into(),
        properties: json!({
            "Name": {"type": "title", "title": [{"plain_text": "A"}]},
            "Status": {"type": "status", "status": {"name": "Todo"}},
            "Priority": {"type": "select", "select": {"name": "High"}}
        }).to_string(),
        last_edited_time: "t0".into(), archived: false,
    }]).unwrap();
    let store = Arc::new(Mutex::new(s));

    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds");
    dispatch_key(&mut app, key('v')); // table -> board (defaults to Status)

    app.start_group_by();
    assert!(app.picker.is_some());
    for c in "Prio".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let View::Board(v) = &app.view else { panic!("expected a board view") };
    assert_eq!(v.group_prop, "Priority");
    assert_eq!(v.columns, vec!["Low", "High", "(none)"]);
}
```

- [ ] **Step 7: Run to pass**

Run: `cargo test -p notion-tui --test board_flow`
Expected: PASS.

- [ ] **Step 8: Full crate gate**

Run: `cargo test -p notion-tui`
Expected: PASS.

---

### Task 5: Workspace search — subsequence re-ranking on top of FTS prefix matching

**Files:**
- Modify: `crates/notion-tui/src/app.rs` (`refresh_search`)
- Test: `crates/notion-tui/tests/search_rerank_flow.rs` (new)

**Interfaces:**
- Consumes: `fuzzy::subsequence_rank` (Task 1), existing `Store::search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>>` (unchanged).
- Produces: `App::refresh_search` re-orders (never drops) the FTS hits by subsequence score against `{title} {snippet}`.

- [ ] **Step 1: Write the failing test**

Create `crates/notion-tui/tests/search_rerank_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use notion_store::{PageRec, Store};
use notion_tui::app::App;
use notion_tui::ui::search::SearchState;

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(), parent_type: "workspace".into(), parent_id: None,
        title: title.into(), icon: None, archived: false, last_edited_time: "t".into(),
    }
}

#[test]
fn subsequence_matches_are_promoted_above_weaker_fts_hits() {
    let mut s = Store::open_in_memory().unwrap();
    // Both titles contain "road" as an FTS prefix hit for query "road", but
    // only "Roadmap" is additionally a tight subsequence match for "rdmp".
    s.upsert_page(&page("p1", "Road closure notice")).unwrap();
    s.upsert_page(&page("p2", "Roadmap")).unwrap();
    let store = Arc::new(Mutex::new(s));

    let mut app = App::new(store);
    app.search = Some(SearchState::new());
    app.search.as_mut().unwrap().input = "road".to_string();
    app.refresh_search();

    let titles: Vec<String> = app.search.as_ref().unwrap().results.iter().map(|h| h.title.clone()).collect();
    assert_eq!(titles[0], "Roadmap", "titles were: {titles:?}");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui --test search_rerank_flow`
Expected: FAIL (or flaky-pass on FTS's own rank ordering) — `refresh_search` uses the FTS hit order verbatim with no subsequence re-ranking.

- [ ] **Step 3: Implement**

In `app.rs`:

```rust
pub fn refresh_search(&mut self) {
    let query = match &self.search {
        Some(s) => s.input.clone(),
        None => return,
    };
    let mut hits = self.store.lock().unwrap().search(&query).unwrap_or_default();
    if !query.trim().is_empty() {
        let candidates: Vec<(notion_store::SearchHit, String)> = hits
            .into_iter()
            .map(|h| {
                let text = format!("{} {}", h.title, h.snippet);
                (h, text)
            })
            .collect();
        hits = crate::fuzzy::subsequence_rank(&query, candidates);
    }
    if let Some(search) = &mut self.search {
        search.results = hits;
        search.cursor = 0;
    }
}
```

Note: `subsequence_rank` drops non-matches; since `query` here is the same text already used for FTS's prefix match, a hit returned by FTS is normally also a subsequence match — but a query containing characters FTS treats specially (e.g. multi-word queries where word order differs) could fail the strict subsequence test. Guard against silently losing hits: fall back to the original FTS order for any hit `subsequence_rank` drops.

Revise to preserve every original hit:

```rust
    if !query.trim().is_empty() {
        let scored_ids: Vec<String> = crate::fuzzy::subsequence_rank(
            &query,
            hits.iter().map(|h| (h.page_id.clone(), format!("{} {}", h.title, h.snippet))).collect(),
        );
        let mut by_id: std::collections::HashMap<String, notion_store::SearchHit> =
            hits.into_iter().map(|h| (h.page_id.clone(), h)).collect();
        let mut reordered: Vec<notion_store::SearchHit> =
            scored_ids.into_iter().filter_map(|id| by_id.remove(&id)).collect();
        reordered.extend(by_id.into_values()); // any FTS hit that wasn't a strict subsequence match
        hits = reordered;
    }
```

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui --test search_rerank_flow`
Expected: PASS.

- [ ] **Step 5: Full crate gate**

Run: `cargo test -p notion-tui`
Expected: PASS.

---

### Task 6: Universal back-history

**Files:**
- Modify: `crates/notion-tui/src/app.rs` (`Action::Back`, `apply_action` extraction, `handle_key`'s back-key arm, `push_history`/`current_location_id`)
- Test: `crates/notion-tui/tests/history_flow.rs` (new)

**Interfaces:**
- Consumes: nothing new.
- Produces (Task 7 depends on this): `pub fn apply_action(app: &mut App, action: Action)` — the single place every navigating `Action` is applied, pushing history before every forward navigation. Both `dispatch_key` (keyboard) and the mouse dispatcher (Task 7) call it.

- [ ] **Step 1: Write the failing test**

Create `crates/notion-tui/tests/history_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, NodeKind, View};
use notion_tui::ui::sidebar::SidebarState;

fn page(id: &str, title: &str) -> PageRec {
    PageRec {
        id: id.into(), parent_type: "workspace".into(), parent_id: None,
        title: title.into(), icon: None, archived: false, last_edited_time: "t".into(),
    }
}

fn store_with_two_pages() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "First")).unwrap();
    s.upsert_page(&page("p2", "Second")).unwrap();
    Arc::new(Mutex::new(s))
}

#[test]
fn sidebar_open_pushes_history_so_backspace_returns() {
    let store = store_with_two_pages();
    let mut app = App::new(store);
    app.focus = Focus::Sidebar;
    app.sidebar = SidebarState::new(vec![
        notion_store::TreeNode { id: "p1".into(), title: "First".into(), parent_id: None, kind: NodeKind::Page },
        notion_store::TreeNode { id: "p2".into(), title: "Second".into(), parent_id: None, kind: NodeKind::Page },
    ]);

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // open "First"
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));

    app.focus = Focus::Sidebar;
    app.sidebar.cursor = 1;
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter)); // open "Second" (sidebar-driven, no link follow)
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p2"));

    app.focus = Focus::Main;
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace));
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"), "backspace should return to First");
}

#[test]
fn back_navigation_itself_does_not_grow_history() {
    let store = store_with_two_pages();
    let mut app = App::new(store);
    app.open_page("p1");
    app.push_history_for_test(); // p1 pushed
    app.open_page("p2");
    app.history.push("p1".to_string());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace)); // back to p1
    assert!(app.history.is_empty(), "going back must not re-push where we came from");
}
```

Note: `push_history_for_test` is a thin test-only wrapper — expose it as `#[doc(hidden)] pub fn push_history_for_test(&mut self) { self.push_history(); }` so the test can seed history without duplicating the private `push_history` logic.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui --test history_flow`
Expected: FAIL — sidebar-driven opens don't touch `history` today, and `push_history_for_test` doesn't exist yet.

- [ ] **Step 3: Implement**

In `app.rs`, extend `Action`:

```rust
pub enum Action {
    None,
    OpenNode(TreeNode),
    OpenPage(String),
    /// Returned only by the back/backspace key: `open_page` without pushing
    /// history (the entry was already popped by the caller).
    Back(String),
    ToggleTodo(String),
    DeleteBlock(String),
    DeleteRow(String),
    MoveCard { row_id: String, prop_name: String, prop_type: String, value: String },
    Undo,
}
```

In `handle_key`'s `View::Page` back-key arm, change the return from `Action::OpenPage(prev)` to `Action::Back(prev)`:

```rust
            } else if km.is("back", key) || key.code == KeyCode::Backspace {
                if let Some(prev) = app.history.pop() {
                    return Action::Back(prev);
                }
            }
```

Add to `impl App`:

```rust
fn current_location_id(&self) -> Option<String> {
    match &self.view {
        View::Page(v) => Some(v.page.id.clone()),
        View::Table(v) => Some(v.ds.id.clone()),
        View::Board(v) => Some(v.ds.id.clone()),
        View::Queue(_) | View::Empty => None,
    }
}

fn push_history(&mut self) {
    if let Some(id) = self.current_location_id() {
        self.history.push(id);
    }
}

#[doc(hidden)]
pub fn push_history_for_test(&mut self) {
    self.push_history();
}
```

Extract the tail of `dispatch_key` (the `match handle_key(app, key) { ... }` block) into a standalone function, adding the `Back` arm and the history push on `OpenNode`/`OpenPage`:

```rust
/// Applies a navigation/edit `Action` against the store. This is the single
/// funnel for both keyboard (`dispatch_key`) and mouse dispatch: every
/// forward navigation pushes history here, so sidebar opens, table/board row
/// opens, and search jumps all participate in `Backspace`/`-` the same way
/// in-page link follows already did.
pub fn apply_action(app: &mut App, action: Action) {
    match action {
        Action::None => {}
        Action::OpenNode(node) => {
            app.push_history();
            app.open_node(&node);
        }
        Action::OpenPage(id) => {
            app.push_history();
            app.open_page(&id);
        }
        Action::Back(id) => app.open_page(&id),
        Action::ToggleTodo(id) => app.toggle_todo(&id),
        Action::DeleteBlock(id) => app.delete_block(&id),
        Action::DeleteRow(id) => app.delete_row(&id),
        Action::MoveCard { row_id, prop_name, prop_type, value } => {
            app.update_row_property(&row_id, &prop_name, &prop_type, &value)
        }
        Action::Undo => app.undo(),
    }
}
```

Replace the old inline match at the bottom of `dispatch_key` with `apply_action(app, handle_key(app, key));`.

Also update the `SearchAction::Open(id)` arm (search jumps must push history too):

```rust
            SearchAction::Open(id) => {
                app.search = None;
                app.push_history();
                app.open_page(&id);
            }
```

`push_history` is a private method (`fn`, not `pub fn`) but `apply_action` and `refresh_search`'s sibling code live in the same module (`app.rs`), so visibility is unaffected.

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui --test history_flow`
Expected: `2 passed`.

- [ ] **Step 5: Full crate gate**

Run: `cargo test -p notion-tui`
Expected: PASS — existing in-page-link-follow history tests (if any in `e2e.rs`/`board_flow.rs`) keep passing since `Action::OpenPage`'s push behavior is unchanged, just centralized.

---

### Task 7: Mouse support — click-to-focus, sidebar/link clicks, header-sort, card drag

**Files:**
- Modify: `crates/notion-tui/src/ui/mod.rs` (`LayoutRects`, populate during `draw`)
- Modify: `crates/notion-tui/src/ui/table.rs` (`column_widths`, `column_x_starts` helpers)
- Modify: `crates/notion-tui/src/ui/board.rs` (`BoardView::move_card_to`)
- Modify: `crates/notion-tui/src/app.rs` (`last_layout`, `drag_card`/`drag_target_col` fields, `dispatch_mouse`)
- Modify: `crates/notion-tui/src/main.rs` (route `Event::Mouse` through `dispatch_mouse`)
- Test: `crates/notion-tui/tests/mouse_flow.rs` (new)

**Interfaces:**
- Consumes: `apply_action` (Task 6).
- Produces: `pub fn dispatch_mouse(app: &mut App, m: crossterm::event::MouseEvent)`.

- [ ] **Step 1: Expose last-frame layout rects**

In `crates/notion-tui/src/ui/mod.rs`:

```rust
use ratatui::layout::Rect;

#[derive(Clone)]
pub struct LayoutRects {
    pub sidebar: Option<Rect>,
    pub main: Rect,
    pub table_header_y: Option<u16>,
    pub table_col_x: Vec<u16>,
    pub board_columns: Vec<Rect>,
}

impl Default for LayoutRects {
    fn default() -> Self {
        LayoutRects {
            sidebar: None,
            main: Rect::new(0, 0, 0, 0),
            table_header_y: None,
            table_col_x: Vec::new(),
            board_columns: Vec::new(),
        }
    }
}
```

At the end of `draw` (after every widget is rendered, so the rects reflect exactly what the user sees this frame), populate `app.last_layout`:

```rust
    let mut layout = LayoutRects {
        sidebar: (!app.sidebar.hidden).then_some(cols[0]),
        main: main_area,
        ..LayoutRects::default()
    };
    if let View::Table(view) = &app.view {
        layout.table_header_y = Some(main_area.y + 1); // border + header row
        layout.table_col_x = table::column_x_starts(main_area, &view.columns);
    }
    if let View::Board(view) = &app.view {
        let n = view.columns.len().max(1) as u32;
        let constraints: Vec<Constraint> = view.columns.iter().map(|_| Constraint::Ratio(1, n)).collect();
        layout.board_columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(main_area)
            .to_vec();
    }
    app.last_layout = layout;
```

Add `pub last_layout: crate::ui::LayoutRects` to `App` (init `Default::default()` in `App::new`).

- [ ] **Step 2: Factor out table column-width helpers**

In `crates/notion-tui/src/ui/table.rs`, extract the widths logic already inline in `render`:

```rust
pub fn column_widths(columns: &[Column]) -> Vec<Constraint> {
    columns
        .iter()
        .enumerate()
        .map(|(i, _)| if i == 0 { Constraint::Min(20) } else { Constraint::Length(14) })
        .collect()
}

/// x-coordinate where each column starts, for header-click hit-testing.
/// Mirrors the same `Layout::horizontal(widths)` split the `Table` widget
/// itself performs, so header clicks line up with rendered columns (modulo
/// ratatui's own internal cell-spacing, which this doesn't attempt to
/// replicate exactly — acceptable for a best-effort, non-required input).
pub fn column_x_starts(area: Rect, columns: &[Column]) -> Vec<u16> {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(column_widths(columns))
        .split(inner)
        .iter()
        .map(|r| r.x)
        .collect()
}
```

Add `use ratatui::layout::{Direction, Layout};` to the top of `table.rs`. Update `render` to call `column_widths(&view.columns)` instead of its inline `map` (same output, now shared).

- [ ] **Step 3: Write the failing hit-testing test**

Append to `table.rs`'s test module:

```rust
    #[test]
    fn column_x_starts_matches_column_widths_order() {
        let cols = vec![
            Column { name: "Name".into(), prop_type: "title".into() },
            Column { name: "Done".into(), prop_type: "checkbox".into() },
        ];
        let area = Rect::new(0, 0, 50, 10);
        let xs = column_x_starts(area, &cols);
        assert_eq!(xs.len(), 2);
        assert!(xs[1] > xs[0], "second column must start after the first");
    }
```

Run: `cargo test -p notion-tui table::tests::column_x_starts`
Expected: FAIL — `column_x_starts` doesn't exist yet (write this test alongside Step 2's implementation, run once both exist).

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui table::tests`
Expected: PASS.

- [ ] **Step 5: `BoardView::move_card_to` (absolute column, for drag-drop)**

Append to `board.rs`:

```rust
    /// Like `move_card` but targets an absolute column index (drag-drop),
    /// rather than a signed delta (keyboard `J`/`K`).
    pub fn move_card_to(&mut self, target_col: usize) -> Option<(String, String)> {
        let row_id = self.selected_row_id()?;
        if target_col >= self.columns.len() || target_col == self.col {
            return None;
        }
        let value = if self.columns[target_col] == "(none)" {
            String::new()
        } else {
            self.columns[target_col].clone()
        };
        self.col = target_col;
        Some((row_id, value))
    }
```

with a test:

```rust
    #[test]
    fn move_card_to_targets_an_absolute_column() {
        let mut v = BoardView::new(ds(), vec![row("r1", "A", Some("Todo"))]);
        v.col = 0;
        v.card = 0;
        assert_eq!(v.move_card_to(2), Some(("r1".to_string(), "Done".to_string())));
        assert_eq!(v.move_card_to(0), None, "already moved to col 2 above — same target is a no-op");
    }
```

Run: `cargo test -p notion-tui board::tests::move_card_to`
Expected: FAIL then PASS after adding the method.

- [ ] **Step 6: Implement `dispatch_mouse`**

In `app.rs`, add fields to `App`: `pub drag_card: Option<(usize, usize)>` (the board `(col, card)` a left-button-down started on).

```rust
/// Routes a crossterm mouse event. Modals (palette/picker/confirm/props/
/// input/search/comments) don't participate in mouse routing yet — clicks
/// while one is open are ignored, matching how they already swallow all
/// keyboard input except their own handlers.
pub fn dispatch_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    use crossterm::event::{MouseButton, MouseEventKind};
    if app.palette.is_some()
        || app.picker.is_some()
        || app.confirm.is_some()
        || app.props.is_some()
        || app.input.is_some()
        || app.search.is_some()
        || app.comments.is_some()
    {
        return;
    }
    let pt = (m.column, m.row);
    match m.kind {
        MouseEventKind::ScrollDown => scroll(app, 3),
        MouseEventKind::ScrollUp => scroll(app, -3),
        MouseEventKind::Down(MouseButton::Left) => {
            let action = handle_click(app, pt);
            apply_action(app, action);
        }
        MouseEventKind::Drag(MouseButton::Left) => handle_drag(app, pt),
        MouseEventKind::Up(MouseButton::Left) => {
            let action = handle_drop(app, pt);
            apply_action(app, action);
        }
        _ => {}
    }
}

fn point_in(rect: ratatui::layout::Rect, pt: (u16, u16)) -> bool {
    pt.0 >= rect.x && pt.0 < rect.x + rect.width && pt.1 >= rect.y && pt.1 < rect.y + rect.height
}

fn handle_click(app: &mut App, pt: (u16, u16)) -> Action {
    let layout = app.last_layout.clone();
    if let Some(sidebar) = layout.sidebar {
        if point_in(sidebar, pt) {
            app.focus = Focus::Sidebar;
            let row = pt.1.saturating_sub(sidebar.y + 1) as usize;
            let idx = app.sidebar.list_state.offset() + row;
            if let Some(node) = app.sidebar.visible().get(idx).map(|v| v.node.clone()) {
                app.sidebar.cursor = idx;
                return Action::OpenNode(node);
            }
            return Action::None;
        }
    }
    if !point_in(layout.main, pt) {
        return Action::None;
    }
    app.focus = Focus::Main;

    if let (Some(header_y), View::Table(_)) = (layout.table_header_y, &app.view) {
        if pt.1 == header_y {
            let col = layout
                .table_col_x
                .iter()
                .rposition(|&x| pt.0 >= x)
                .unwrap_or(0);
            if let View::Table(v) = &mut app.view {
                v.toggle_sort(col);
            }
            return Action::None;
        }
    }
    if let View::Page(v) = &app.view {
        let row = pt.1.saturating_sub(layout.main.y + 1) as usize;
        let idx = v.list_state.offset() + row;
        if let Some(line) = v.lines().get(idx) {
            if let Some(target) = line.link_page_id.clone() {
                return Action::OpenPage(target);
            }
        }
        return Action::None;
    }
    if let View::Board(_) = &app.view {
        if let Some((ci, _)) = layout.board_columns.iter().enumerate().find(|(_, r)| point_in(**r, pt)) {
            if let View::Board(v) = &mut app.view {
                let row = pt.1.saturating_sub(layout.board_columns[ci].y + 1) as usize;
                let card_idx = v.list_state.offset() + row;
                if card_idx < v.cards_in(ci).len() {
                    v.col = ci;
                    v.card = card_idx;
                    app.drag_card = Some((ci, card_idx));
                }
            }
        }
    }
    Action::None
}

fn handle_drag(app: &mut App, pt: (u16, u16)) {
    if app.drag_card.is_none() {
        return;
    }
    let layout = app.last_layout.clone();
    if let Some((ci, _)) = layout.board_columns.iter().enumerate().find(|(_, r)| point_in(**r, pt)) {
        if let View::Board(v) = &mut app.view {
            v.col = ci; // live preview: highlight the column under the cursor
        }
    }
}

fn handle_drop(app: &mut App, _pt: (u16, u16)) -> Action {
    let Some((origin_col, _)) = app.drag_card.take() else { return Action::None };
    let View::Board(v) = &mut app.view else { return Action::None };
    let target = v.col;
    v.col = origin_col; // move_card_to validates target != current col itself
    match v.move_card_to(target) {
        Some((row_id, value)) => Action::MoveCard {
            row_id,
            prop_name: v.group_prop.clone(),
            prop_type: v.group_type.clone(),
            value,
        },
        None => Action::None,
    }
}
```

- [ ] **Step 7: Wire into `main.rs`**

Replace the current inline `Event::Mouse(m)` match in `main.rs`:

```rust
                Some(Ok(Event::Mouse(m))) => notion_tui::app::dispatch_mouse(&mut app, m),
```

(This subsumes the old scroll-only handling — `dispatch_mouse` handles `ScrollDown`/`ScrollUp` too.)

- [ ] **Step 8: Write the flow test**

Create `crates/notion-tui/tests/mouse_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use notion_store::{NodeKind, Store, TreeNode};
use notion_tui::app::{dispatch_mouse, App, Focus, View};
use notion_tui::ui;
use notion_tui::ui::sidebar::SidebarState;
use ratatui::{backend::TestBackend, Terminal};

fn click(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    }
}

#[test]
fn clicking_a_sidebar_entry_opens_it() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let mut s = store.lock().unwrap();
        s.upsert_page(&notion_store::PageRec {
            id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
            title: "Roadmap".into(), icon: None, archived: false, last_edited_time: "t".into(),
        })
        .unwrap();
    }
    let mut app = App::new(store);
    app.sidebar = SidebarState::new(vec![TreeNode {
        id: "p1".into(), title: "Roadmap".into(), parent_id: None, kind: NodeKind::Page,
    }]);

    // Render once so `app.last_layout` reflects a real frame.
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();

    let sidebar_rect = app.last_layout.sidebar.expect("sidebar should be visible");
    dispatch_mouse(&mut app, click(sidebar_rect.x + 2, sidebar_rect.y + 1));

    assert!(matches!(app.focus, Focus::Sidebar));
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));
}
```

- [ ] **Step 9: Run to verify it fails, then pass**

Run: `cargo test -p notion-tui --test mouse_flow`
Expected: FAIL first (`dispatch_mouse` doesn't exist / `last_layout` field missing), then PASS once Steps 1–7 are in place.

- [ ] **Step 10: Full crate gate**

Run: `cargo test -p notion-tui && cargo clippy -p notion-tui --all-targets -- -D warnings`
Expected: PASS.

---

### Task 8: Table filtering

**Files:**
- Modify: `crates/notion-tui/src/ui/table.rs` (`FilterState`, `TableView::visible`, `matches_filter`, render title suffix)
- Modify: `crates/notion-tui/src/keymap.rs` (`filter`, `filter_row` actions)
- Modify: `crates/notion-tui/src/app.rs` (`InputPurpose::FilterTable`, dispatch wiring, `refresh_current_view`'s Table arm carries `filter` forward)
- Test: inline in `table.rs`, `crates/notion-tui/tests/table_filter_flow.rs` (new)

**Interfaces:**
- Consumes: nothing new.
- Produces: `TableView::visible(&self) -> Vec<&RowRec>` (filtered + already-sorted display list) — `render`, `move_cursor`, `selected_row_id`, `rows_iter_position`, `row_count` all switch to this.

- [ ] **Step 1: Write the failing tests**

Append to `table.rs`'s test module:

```rust
    #[test]
    fn column_filter_hides_non_matching_rows() {
        let mut v = TableView::new(ds(), vec![
            row("r1", "Buy milk", false, "High"),
            row("r2", "Buy eggs", false, "Low"),
        ]);
        v.filter = Some(FilterState { col: Some(2), query: "high".into() }); // Prio column
        let visible: Vec<&str> = v.visible().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(visible, vec!["r1"]);
    }

    #[test]
    fn free_text_filter_matches_any_column() {
        let mut v = TableView::new(ds(), vec![
            row("r1", "Buy milk", false, "High"),
            row("r2", "Buy eggs", false, "Low"),
        ]);
        v.filter = Some(FilterState { col: None, query: "eggs".into() });
        let visible: Vec<&str> = v.visible().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(visible, vec!["r2"]);
    }

    #[test]
    fn no_filter_shows_every_row() {
        let v = TableView::new(ds(), vec![row("r1", "Buy milk", false, "High")]);
        assert_eq!(v.visible().len(), 1);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui table::tests`
Expected: FAIL — `FilterState`/`filter`/`visible` don't exist.

- [ ] **Step 3: Implement filtering**

```rust
#[derive(Clone)]
pub struct FilterState {
    /// `Some(col_idx)` filters that one column; `None` is free-text across the whole row.
    pub col: Option<usize>,
    pub query: String,
}

pub struct TableView {
    pub ds: DataSourceRec,
    pub columns: Vec<Column>,
    pub rows: Vec<RowRec>,
    pub cursor: usize,
    pub sort: Option<(usize, bool)>,
    pub sort_col: usize,
    pub table_state: TableState,
    pub filter: Option<FilterState>,
}
```

Add `filter: None` to `TableView::new`'s literal.

```rust
    fn matches_filter(&self, row: &RowRec) -> bool {
        let Some(f) = &self.filter else { return true };
        let q = f.query.to_lowercase();
        match f.col {
            Some(i) => self
                .columns
                .get(i)
                .map(|c| self.cell(row, c).to_lowercase().contains(&q))
                .unwrap_or(true),
            None => self.columns.iter().any(|c| self.cell(row, c).to_lowercase().contains(&q)),
        }
    }

    /// The rows actually shown: `self.rows` (already sorted by `apply_sort`)
    /// with the active filter, if any, applied on top.
    pub fn visible(&self) -> Vec<&RowRec> {
        self.rows.iter().filter(|r| self.matches_filter(r)).collect()
    }
```

Update the methods that previously indexed `self.rows` directly to use `self.visible()`:

```rust
    pub fn rows_iter_position(&self, id: &str) -> Option<usize> {
        self.visible().iter().position(|r| r.id == id)
    }

    pub fn row_count(&self) -> usize {
        self.visible().len()
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn selected_row_id(&self) -> Option<String> {
        self.visible().get(self.cursor).map(|r| r.id.clone())
    }
```

In `render`, build `rows`/highlighting off `view.visible()` instead of `view.rows`, and append the filter to the title:

```rust
fn filter_suffix(filter: &Option<FilterState>, columns: &[Column]) -> String {
    match filter {
        None => String::new(),
        Some(f) => match f.col.and_then(|i| columns.get(i)) {
            Some(c) => format!(" — filter: {}~\"{}\"", c.name, f.query),
            None => format!(" — filter: \"{}\"", f.query),
        },
    }
}
```

```rust
    let visible = view.visible();
    let rows: Vec<TRow> = visible
        .iter()
        .map(|r| {
            let cells: Vec<String> = view.columns.iter().map(|c| view.cell(r, c)).collect();
            TRow::new(cells)
        })
        .collect();
    ...
                    .title(Span::styled(
                        format!(" {}{} ", view.ds.title, filter_suffix(&view.filter, &view.columns)),
                        theme.title,
                    )),
```

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui table::tests`
Expected: PASS.

- [ ] **Step 5: Keymap actions + app wiring**

In `keymap.rs`, add to `DEFAULTS` (and mirror in `actions()`):

```rust
    ("filter", "filter column", KeyCode::Char('f')),
    ("filter_row", "filter row (free text)", KeyCode::Char('F')),
```

In `app.rs`, add to `InputPurpose`:

```rust
    FilterTable { data_source_id: String, col: Option<usize> },
```

In `dispatch_key`'s `if let View::Table(view) = &app.view` block (alongside `new_row`/`props`):

```rust
            if km.is("filter", key) {
                let prefill = view
                    .filter
                    .as_ref()
                    .filter(|f| f.col == Some(view.sort_col))
                    .map(|f| f.query.clone())
                    .unwrap_or_default();
                let col_name = view.columns.get(view.sort_col).map(|c| c.name.clone()).unwrap_or_default();
                app.input = Some(InputState::new(format!("filter {col_name}"), prefill));
                app.input_purpose = Some(InputPurpose::FilterTable {
                    data_source_id: view.ds.id.clone(),
                    col: Some(view.sort_col),
                });
                return;
            }
            if km.is("filter_row", key) {
                let prefill = view
                    .filter
                    .as_ref()
                    .filter(|f| f.col.is_none())
                    .map(|f| f.query.clone())
                    .unwrap_or_default();
                app.input = Some(InputState::new("filter row", prefill));
                app.input_purpose = Some(InputPurpose::FilterTable {
                    data_source_id: view.ds.id.clone(),
                    col: None,
                });
                return;
            }
```

Add `App::set_table_filter`:

```rust
pub fn set_table_filter(&mut self, data_source_id: &str, col: Option<usize>, text: &str) {
    let View::Table(v) = &mut self.view else { return };
    if v.ds.id != data_source_id {
        return;
    }
    v.filter = if text.is_empty() {
        None
    } else {
        Some(crate::ui::table::FilterState { col, query: text.to_string() })
    };
    v.cursor = 0;
}
```

Extend the `InputAction::Submit` match:

```rust
                        InputPurpose::FilterTable { data_source_id, col } => {
                            app.set_table_filter(&data_source_id, col, &text)
                        }
```

- [ ] **Step 6: Preserve filter across sync ticks**

In `refresh_current_view`'s `View::Table` arm, capture and restore `filter` alongside `sort`/`sort_col`:

```rust
            View::Table(v) => {
                let (id, cursor, sort, sort_col, selected, filter) =
                    (v.ds.id.clone(), v.cursor, v.sort, v.sort_col, v.selected_row_id(), v.filter.clone());
                self.open_table(&id);
                if let View::Table(nv) = &mut self.view {
                    nv.sort = sort.filter(|(col_idx, _)| *col_idx < nv.columns.len());
                    nv.sort_col = sort_col.min(nv.columns.len().saturating_sub(1));
                    nv.apply_sort();
                    nv.filter = filter;
                    nv.cursor = selected
                        .and_then(|sid| nv.rows_iter_position(&sid))
                        .unwrap_or_else(|| cursor.min(nv.row_count().saturating_sub(1)));
                }
            }
```

- [ ] **Step 7: Write the flow test**

Create `crates/notion-tui/tests/table_filter_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use notion_store::{DataSourceRec, RowRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use crossterm::event::{KeyCode, KeyEvent};
use serde_json::json;

fn store_with_rows() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds".into(), database_id: "db".into(), title: "Tasks".into(),
        schema_json: json!({"Name": {"type": "title"}}).to_string(),
        last_edited_time: "t".into(),
    }).unwrap();
    s.replace_rows("ds", &[
        RowRec { id: "r1".into(), data_source_id: "ds".into(),
            properties: json!({"Name": {"type": "title", "title": [{"plain_text": "Buy milk"}]}}).to_string(),
            last_edited_time: "t0".into(), archived: false },
        RowRec { id: "r2".into(), data_source_id: "ds".into(),
            properties: json!({"Name": {"type": "title", "title": [{"plain_text": "Buy eggs"}]}}).to_string(),
            last_edited_time: "t0".into(), archived: false },
    ]).unwrap();
    Arc::new(Mutex::new(s))
}

fn key(c: char) -> KeyEvent { KeyEvent::from(KeyCode::Char(c)) }

#[test]
fn filter_key_narrows_the_table_and_survives_a_refresh() {
    let store = store_with_rows();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("ds");

    dispatch_key(&mut app, key('f')); // filter current (title) column
    for c in "eggs".chars() {
        dispatch_key(&mut app, key(c));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let View::Table(v) = &app.view else { panic!("expected table") };
    assert_eq!(v.visible().len(), 1);
    assert_eq!(v.visible()[0].id, "r2");

    app.refresh_current_view(); // simulate a sync tick
    let View::Table(v) = &app.view else { panic!("expected table") };
    assert_eq!(v.visible().len(), 1, "filter must survive a refresh");
    assert_eq!(v.visible()[0].id, "r2");
}
```

- [ ] **Step 8: Run to pass**

Run: `cargo test -p notion-tui --test table_filter_flow`
Expected: PASS.

- [ ] **Step 9: Full crate gate**

Run: `cargo test -p notion-tui && cargo clippy -p notion-tui --all-targets -- -D warnings`
Expected: PASS.

---

### Task 9: Status bar completeness — breadcrumb + pending-keystroke

**Files:**
- Modify: `crates/notion-tui/src/ui/mod.rs` (`status_line` signature + `format_count`)
- Modify: `crates/notion-tui/src/app.rs` (`App::breadcrumb`)
- Test: inline in `ui/mod.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: `status_line(breadcrumb: Option<&str>, status: &SyncStatus, pending: u32, conflicted: u32, notice: Option<&str>, hints: Option<&str>, pending_key: Option<char>) -> String` (breaking parameter-order change — updates the 4 existing inline tests).

- [ ] **Step 1: Write the failing tests**

Update `ui/mod.rs`'s existing test module (adding two new params to every call, plus two new tests):

```rust
    #[test]
    fn status_line_appends_pending_count() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 0, 0, None, None, None),
            "✓ synced"
        );
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 3, 0, None, None, None),
            "✓ synced · 3 pending"
        );
    }

    #[test]
    fn status_line_appends_conflicted_count() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 1, 2, None, None, None),
            "✓ synced · 1 pending · ⚠ 2 conflicted"
        );
    }

    #[test]
    fn status_line_appends_notice() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 0, 0, Some("no table open to show as a board"), None, None),
            "✓ synced · no table open to show as a board"
        );
    }

    #[test]
    fn status_line_appends_hints() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 0, 0, None, Some("J move card right"), None),
            "✓ synced · J move card right"
        );
    }

    #[test]
    fn status_line_prepends_breadcrumb() {
        assert_eq!(
            status_line(Some("workspace → Roadmap"), &SyncStatus::Idle { updated: 0 }, 0, 0, None, None, None),
            "workspace → Roadmap · ✓ synced"
        );
    }

    #[test]
    fn status_line_appends_pending_key() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 0, 0, None, None, Some('d')),
            "✓ synced · d pending"
        );
    }

    #[test]
    fn syncing_status_shows_done_over_total_with_thousands_separators() {
        assert_eq!(
            status_line(None, &SyncStatus::Syncing { done: 240, total: 1893 }, 0, 0, None, None, None),
            "⟳ syncing 240/1,893 pages"
        );
        assert_eq!(
            status_line(None, &SyncStatus::Syncing { done: 0, total: 0 }, 0, 0, None, None, None),
            "⟳ syncing…"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui ui::tests`
Expected: FAIL — `status_line`'s signature and `SyncStatus::Syncing`'s shape haven't changed yet (this step also requires `SyncStatus::Syncing { done, total }`, landed in Task 12 — if executing tasks out of order, stub the field locally first; recommended order is to run Task 12 before this task's `Syncing`-specific test, or accept a temporary compile error on those two tests until Task 12 lands. Note this cross-task dependency explicitly in the PR/commit message.)

- [ ] **Step 3: Implement `format_count` and the new `status_line` signature**

```rust
fn format_count(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

pub fn status_line(
    breadcrumb: Option<&str>,
    status: &SyncStatus,
    pending: u32,
    conflicted: u32,
    notice: Option<&str>,
    hints: Option<&str>,
    pending_key: Option<char>,
) -> String {
    let base = match status {
        SyncStatus::Starting => "starting…".into(),
        SyncStatus::Syncing { total, .. } if *total == 0 => "⟳ syncing…".into(),
        SyncStatus::Syncing { done, total } => {
            format!("⟳ syncing {}/{} pages", format_count(*done), format_count(*total))
        }
        SyncStatus::Idle { updated: 0 } => "✓ synced".into(),
        SyncStatus::Idle { updated } => format!("✓ synced ({updated} updated)"),
        SyncStatus::Offline => "⚠ offline".into(),
        SyncStatus::Failed(msg) => format!("✗ sync failed: {msg}"),
    };
    let mut out = match breadcrumb {
        Some(b) => format!("{b} · {base}"),
        None => base,
    };
    if pending > 0 {
        out = format!("{out} · {pending} pending");
    }
    if conflicted > 0 {
        out = format!("{out} · ⚠ {conflicted} conflicted");
    }
    if let Some(notice) = notice {
        out = format!("{out} · {notice}");
    }
    if let Some(hints) = hints {
        out = format!("{out} · {hints}");
    }
    if let Some(k) = pending_key {
        out = format!("{out} · {k} pending");
    }
    out
}
```

Update the call site in `draw`:

```rust
    f.render_widget(
        Paragraph::new(status_line(
            app.breadcrumb().as_deref(),
            &app.sync_status,
            app.pending,
            app.conflicted,
            app.notice.as_deref(),
            hints.as_deref(),
            app.pending_d.then_some('d'),
        ))
        .style(theme.status),
        rows[1],
    );
```

- [ ] **Step 4: Add `App::breadcrumb`**

In `app.rs`:

```rust
/// "workspace → Ancestor → … → current" for the open page; for a table/board,
/// just "workspace → {data source title}" (data sources have no tracked page
/// ancestry in this schema). `None` when nothing is open.
pub fn breadcrumb(&self) -> Option<String> {
    match &self.view {
        View::Page(v) => {
            let guard = self.store.lock().unwrap();
            let mut chain = vec![v.page.title.clone()];
            let mut parent_id = v.page.parent_id.clone();
            while let Some(pid) = parent_id {
                let Some(p) = guard.get_page(&pid).ok().flatten() else { break };
                chain.push(p.title.clone());
                parent_id = p.parent_id.clone();
            }
            drop(guard);
            chain.reverse();
            Some(format!("workspace → {}", chain.join(" → ")))
        }
        View::Table(v) => Some(format!("workspace → {}", v.ds.title)),
        View::Board(v) => Some(format!("workspace → {}", v.ds.title)),
        View::Queue(_) | View::Empty => None,
    }
}
```

- [ ] **Step 5: Run to pass**

Run: `cargo test -p notion-tui ui::tests`
Expected: PASS (once `SyncStatus::Syncing`'s `total` field exists — see Step 2's note; if this task runs before Task 12, add `total: u32` to `SyncStatus::Syncing` here as a minimal stub and let Task 12 own the actual progress-emitting logic).

- [ ] **Step 6: Full crate gate**

Run: `cargo test -p notion-tui && cargo test -p notion-tui-sync`
Expected: PASS (any other `SyncStatus::Syncing { done }` construction site — grep for `SyncStatus::Syncing` — needs its literal updated to `SyncStatus::Syncing { done, total }`; there is exactly one today, in `notion-sync/src/lib.rs`'s `one_cycle`, updated in Task 12).

---

### Task 10: Block placeholders

**Files:**
- Modify: `crates/notion-tui/src/ui/page.rs`
- Test: inline in `page.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: no signature change — only the rendered `text` for unsupported block types.

- [ ] **Step 1: Write the failing tests**

Append to `page.rs`'s test module:

```rust
    #[test]
    fn image_block_shows_caption_placeholder() {
        let v = PageView::new(
            page(),
            vec![rec("i1", None, 0, "image", "", r#"{"caption": [{"plain_text": "A chart"}]}"#)],
        );
        assert_eq!(v.lines()[0].text, "[image: A chart]");
    }

    #[test]
    fn image_block_without_caption_shows_untitled() {
        let v = PageView::new(page(), vec![rec("i1", None, 0, "image", "", "{}")]);
        assert_eq!(v.lines()[0].text, "[image: untitled]");
    }

    #[test]
    fn bookmark_block_shows_url_placeholder() {
        let v = PageView::new(
            page(),
            vec![rec("b1", None, 0, "bookmark", "", r#"{"url": "https://example.com"}"#)],
        );
        assert_eq!(v.lines()[0].text, "[bookmark: https://example.com]");
    }

    #[test]
    fn table_and_embed_and_other_unsupported_blocks_get_named_placeholders() {
        let v = PageView::new(
            page(),
            vec![
                rec("t1", None, 0, "table", "", "{}"),
                rec("e1", None, 1, "embed", "", r#"{"url": "https://youtu.be/x"}"#),
                rec("v1", None, 2, "video", "", "{}"),
            ],
        );
        let texts: Vec<&str> = v.lines().iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["[table]", "[embed: https://youtu.be/x]", "[video]"]);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui page::tests`
Expected: FAIL — every unsupported block currently renders `"⍰ {plain_text}"`.

- [ ] **Step 3: Implement**

Add a caption helper above `push_children`:

```rust
fn caption_text(payload: &serde_json::Value) -> String {
    payload["caption"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t["plain_text"].as_str()).collect::<Vec<_>>().join(""))
        .unwrap_or_default()
}
```

Replace the catch-all arm in `push_children`'s match:

```rust
                "image" => {
                    let caption = caption_text(&payload);
                    let caption = if caption.is_empty() { "untitled".to_string() } else { caption };
                    (format!("[image: {caption}]"), None)
                }
                "bookmark" => (format!("[bookmark: {}]", payload["url"].as_str().unwrap_or("")), None),
                "embed" => (format!("[embed: {}]", payload["url"].as_str().unwrap_or("")), None),
                _ => (format!("[{}]", b.block_type), None),
```

(`table` and any other unsupported type fall through to the final `_` arm, producing `[table]`, `[video]`, etc.)

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui page::tests`
Expected: PASS.

- [ ] **Step 5: Full crate gate + snapshot check**

Run: `cargo test -p notion-tui`
Expected: PASS. If any `insta` snapshot in `render_smoke.rs` happens to render a block type affected by this change, review the diff with `cargo insta review` and accept only if it matches the new placeholder format (none of today's snapshot fixtures use image/bookmark/table/embed blocks, so no snapshot changes are expected).

---

### Task 11: notion-api — `get_page` wrapper (404/archived detection)

**Files:**
- Modify: `crates/notion-api/src/endpoints.rs`
- Test: `crates/notion-api/tests/get_page.rs` (new)

**Interfaces:**
- Consumes: nothing new (reuses `PageMeta::parse`, already imported in `endpoints.rs`).
- Produces (Task 14 depends on this): `pub async fn get_page(&self, page_id: &str) -> Result<PageMeta, ApiError>`.

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-api/tests/get_page.rs`:

```rust
use notion_api::NotionClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn get_page_returns_metadata_including_archived() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "page", "id": "p1", "archived": true,
            "parent": {"type": "workspace", "workspace": true},
            "last_edited_time": "2026-07-05T10:00:00.000Z",
            "properties": {"title": {"type": "title", "title": [{"plain_text": "Gone"}]}}
        })))
        .mount(&server)
        .await;
    let client = NotionClient::with_base_url("t", server.uri());

    let meta = client.get_page("p1").await.unwrap();
    assert!(meta.archived);
    assert_eq!(meta.title, "Gone");
}

#[tokio::test]
async fn get_page_surfaces_404_as_an_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/gone"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "code": "object_not_found", "message": "not found"
        })))
        .mount(&server)
        .await;
    let client = NotionClient::with_base_url("t", server.uri());

    let err = client.get_page("gone").await.unwrap_err();
    assert!(matches!(err, notion_api::ApiError::Api { status: 404, .. }));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui-api --test get_page`
Expected: FAIL — `get_page` doesn't exist.

- [ ] **Step 3: Implement**

In `crates/notion-api/src/endpoints.rs`, add next to `get_page_edited_time`:

```rust
    pub async fn get_page(&self, page_id: &str) -> Result<PageMeta, ApiError> {
        let v = self.get_json(&format!("/v1/pages/{page_id}")).await?;
        Ok(PageMeta::parse(&v))
    }
```

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui-api --test get_page`
Expected: PASS.

- [ ] **Step 5: Full crate gate**

Run: `cargo test -p notion-tui-api`
Expected: PASS.

---

### Task 12: notion-sync — sync-now trigger + first-crawl progress + incremental checkpointing

**Files:**
- Modify: `crates/notion-sync/src/lib.rs` (`SyncStatus::Syncing{done,total}`, `SyncHandle.notify`, `one_cycle`/`spawn_sync`)
- Modify: `crates/notion-sync/src/puller.rs` (`pull_once` signature + checkpointing)
- Modify: `crates/notion-store/src/store.rs` (`meta_delete`)
- Modify: `crates/notion-tui/src/main.rs` (wire `app.sync_notify`)
- Modify: `crates/notion-tui/src/wizard.rs` (one-line first-sync handoff message)
- Test: `crates/notion-sync/tests/pull.rs` (extend), `crates/notion-sync/tests/progress.rs` (new); update existing `pull_once` call sites

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `pub async fn pull_once(client: &NotionClient, store: &SharedStore, status_tx: &watch::Sender<SyncStatus>) -> Result<u32, SyncError>` (signature change — `client`/`store` args unchanged, `status_tx` is new).
  - `SyncStatus::Syncing { done: u32, total: u32 }` (was `{ done: u32 }`).
  - `SyncHandle.notify: std::sync::Arc<tokio::sync::Notify>`; `SyncHandle::request_sync_now(&self)`.
  - `Store::meta_delete(&self, key: &str) -> anyhow::Result<()>`.

- [ ] **Step 1: `Store::meta_delete` — write the failing test**

Append to `crates/notion-store`'s inline tests in `store.rs` (or a small new `crates/notion-store/tests/meta.rs`):

```rust
#[test]
fn meta_delete_removes_the_key() {
    let s = notion_store::Store::open_in_memory().unwrap();
    s.meta_set("k", "v").unwrap();
    assert_eq!(s.meta_get("k").unwrap().as_deref(), Some("v"));
    s.meta_delete("k").unwrap();
    assert_eq!(s.meta_get("k").unwrap(), None);
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test -p notion-tui-store --test meta`
Expected: FAIL — `meta_delete` doesn't exist.

In `store.rs`, next to `meta_set`:

```rust
    pub fn meta_delete(&self, key: &str) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM sync_meta WHERE key = ?1", [key])?;
        Ok(())
    }
```

Run: `cargo test -p notion-tui-store --test meta`
Expected: PASS.

- [ ] **Step 3: `SyncStatus::Syncing` gains `total`; `SyncHandle` gains `notify`**

In `crates/notion-sync/src/lib.rs`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    Starting,
    Syncing { done: u32, total: u32 },
    Idle { updated: u32 },
    Offline,
    Failed(String),
}

pub struct SyncHandle {
    pub status: watch::Receiver<SyncStatus>,
    pub data_version: watch::Receiver<u64>,
    pub pending: watch::Receiver<u32>,
    pub notify: std::sync::Arc<tokio::sync::Notify>,
}

impl SyncHandle {
    /// Wakes the sync loop immediately instead of waiting out its poll interval.
    pub fn request_sync_now(&self) {
        self.notify.notify_one();
    }
}
```

Update `one_cycle`'s initial marker and `pull_once` call:

```rust
async fn one_cycle(
    client: &Arc<NotionClient>,
    store: &SharedStore,
    status_tx: &watch::Sender<SyncStatus>,
    data_tx: &watch::Sender<u64>,
    pending_tx: &watch::Sender<u32>,
) {
    status_tx.send_replace(SyncStatus::Syncing { done: 0, total: 0 });

    match push_once(client, store).await { ... } // unchanged
    pending_tx.send_replace(lock_store(store).pending_count().unwrap_or(0));

    match pull_once(client, store, status_tx).await {
        Ok(updated) => {
            if updated > 0 {
                data_tx.send_modify(|v| *v += 1);
            }
            status_tx.send_replace(SyncStatus::Idle { updated });
        }
        Err(SyncError::Api(ApiError::Network(_))) => {
            status_tx.send_replace(SyncStatus::Offline);
        }
        Err(e) => {
            status_tx.send_replace(SyncStatus::Failed(e.to_string()));
        }
    }
    pending_tx.send_replace(lock_store(store).pending_count().unwrap_or(0));
}
```

Update `spawn_sync` to wake on either the interval or a manual request:

```rust
pub fn spawn_sync(client: NotionClient, store: SharedStore, interval: Duration) -> SyncHandle {
    let (status_tx, status_rx) = watch::channel(SyncStatus::Starting);
    let (data_tx, data_rx) = watch::channel(0u64);
    let (pending_tx, pending_rx) = watch::channel(0u32);
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify_loop = notify.clone();
    tokio::spawn(async move {
        let client = Arc::new(client);
        loop {
            let cycle = one_cycle(&client, &store, &status_tx, &data_tx, &pending_tx);
            if let Err(payload) = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(cycle)).await
            {
                let _ = payload;
                status_tx.send_replace(SyncStatus::Failed(
                    "sync engine crashed — restart notion-tui".into(),
                ));
            }
            tokio::select! {
                _ = tokio::time::sleep(interval) => {}
                _ = notify_loop.notified() => {}
            }
        }
    });
    SyncHandle {
        status: status_rx,
        data_version: data_rx,
        pending: pending_rx,
        notify,
    }
}
```

- [ ] **Step 4: Update every existing `pull_once`/`SyncStatus::Syncing` call site to compile**

Update these test files' `pull_once(&client, &store)` calls to `pull_once(&client, &store, &tokio::sync::watch::channel(notion_sync::SyncStatus::Starting).0).await`:
- `crates/notion-sync/tests/pull.rs` (4 call sites)
- `crates/notion-sync/tests/dirty_pull.rs` (1 call site)
- `crates/notion-sync/tests/comments_sync.rs` (1 call site)
- `crates/notion-tui/tests/e2e.rs`, `e2e_editor.rs`, `e2e_write.rs` (1 call site each — add `use tokio::sync::watch;` and `use notion_sync::SyncStatus;` to each file's imports if not already present)

Concretely, in each file replace e.g. `pull_once(&fast_client(server.uri()), &store).await.unwrap();` with:

```rust
let (status_tx, _status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
pull_once(&fast_client(server.uri()), &store, &status_tx).await.unwrap();
```

Factor this two-line pattern into each file's existing fixture helper where one exists (e.g. `pull.rs`'s `pull_fixture_one_page` can return the `status_tx` alongside `server`/`client`/`store`, or tests can just declare the channel inline — inline is simpler and keeps this step mechanical).

Run: `cargo build --workspace --tests`
Expected: now compiles (this step is purely mechanical signature-following; no behavior test yet).

- [ ] **Step 5: Write the failing checkpointing/progress test**

Append to `crates/notion-sync/tests/pull.rs`:

```rust
#[tokio::test]
async fn interrupted_first_crawl_resumes_from_the_checkpoint_instead_of_restarting() {
    let server = MockServer::start().await;
    // Page 1: p1, followed by cursor "c2".
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(wiremock::matchers::body_string_contains(r#""page_size":100}"#)) // no start_cursor
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_json("p1", "First", "2026-07-05T10:00:00.000Z")],
            "has_more": true, "next_cursor": "c2"
        })))
        .expect(1)
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": [], "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    // Page 2 (cursor "c2") fails the first time.
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(wiremock::matchers::body_string_contains("c2"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());
    let (status_tx, status_rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);

    let err = notion_sync::pull_once(&client, &store, &status_tx).await;
    assert!(err.is_err(), "the second page's 500 must fail this pass");
    assert_eq!(store.lock().unwrap().meta_get("pull_cursor").unwrap().as_deref(), Some("c2"));
    assert_eq!(store.lock().unwrap().get_page("p1").unwrap().unwrap().title, "First");
    match &*status_rx.borrow() {
        notion_sync::SyncStatus::Syncing { done: 1, total } => assert!(*total >= 1),
        other => panic!("expected a Syncing progress update, got {other:?}"),
    }

    // Second attempt: page 2 now succeeds and is the last page.
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(wiremock::matchers::body_string_contains("c2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_json("p2", "Second", "2026-07-04T10:00:00.000Z")],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server).await;

    let updated = notion_sync::pull_once(&client, &store, &status_tx).await.unwrap();
    assert_eq!(updated, 1, "only p2 should be (re-)processed — p1 must not be re-fetched");
    assert_eq!(store.lock().unwrap().meta_get("hwm").unwrap().as_deref(), Some("2026-07-05T10:00:00.000Z"));
    assert_eq!(store.lock().unwrap().meta_get("pull_cursor").unwrap(), None, "checkpoint clears on success");
    // The page-1 search mock's `.expect(1)` (asserted on drop) proves it wasn't re-hit.
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test -p notion-tui-sync --test pull interrupted_first_crawl`
Expected: FAIL — no checkpointing exists; a failed pass leaves nothing persisted and `SyncStatus` update never happens (old `pull_once` doesn't take `status_tx`, so this won't even compile until Step 4 is done — sequence Steps 4 and 5/6 together).

- [ ] **Step 7: Implement checkpointed, progress-reporting `pull_once`**

Rewrite `crates/notion-sync/src/puller.rs`'s `pull_once`:

```rust
pub async fn pull_once(
    client: &NotionClient,
    store: &SharedStore,
    status_tx: &watch::Sender<crate::SyncStatus>,
) -> Result<u32, SyncError> {
    let hwm = lock_store(store)
        .meta_get("hwm")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .unwrap_or_default();
    let mut cursor = lock_store(store)
        .meta_get("pull_cursor")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .filter(|c| !c.is_empty());
    let mut max_seen = lock_store(store)
        .meta_get("pull_max_seen")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .unwrap_or_else(|| hwm.clone());
    let mut done: u32 = lock_store(store)
        .meta_get("pull_done")
        .map_err(|e| SyncError::Store(e.to_string()))?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut updated: u32 = 0;
    let mut finished = false;

    while !finished {
        let page = client.search_page(cursor.as_deref()).await?;
        let has_more = page.next_cursor.is_some();
        for item in &page.items {
            let edited = match item {
                SearchItem::Page(p) => p.last_edited_time.clone(),
                SearchItem::DataSource(d) => d.last_edited_time.clone(),
                SearchItem::Other => continue,
            };
            if !hwm.is_empty() && edited.as_str() <= hwm.as_str() {
                finished = true;
                break;
            }
            if edited > max_seen {
                max_seen = edited.clone();
            }
            match item {
                // ... existing SearchItem::Page / SearchItem::DataSource bodies, unchanged ...
                SearchItem::Other => {}
            }
            done += 1;
            updated += 1;
            let total_estimate = (done + if has_more { 100 } else { 0 }).max(done);
            status_tx.send_replace(crate::SyncStatus::Syncing { done, total: total_estimate });
            lock_store(store)
                .meta_set("pull_done", &done.to_string())
                .map_err(|e| SyncError::Store(e.to_string()))?;
        }
        cursor = page.next_cursor.clone();
        lock_store(store)
            .meta_set("pull_cursor", cursor.as_deref().unwrap_or(""))
            .map_err(|e| SyncError::Store(e.to_string()))?;
        lock_store(store)
            .meta_set("pull_max_seen", &max_seen)
            .map_err(|e| SyncError::Store(e.to_string()))?;
        if cursor.is_none() {
            finished = true;
        }
    }

    if max_seen > hwm {
        lock_store(store)
            .meta_set("hwm", &max_seen)
            .map_err(|e| SyncError::Store(e.to_string()))?;
    }
    lock_store(store).meta_delete("pull_cursor").map_err(|e| SyncError::Store(e.to_string()))?;
    lock_store(store).meta_delete("pull_max_seen").map_err(|e| SyncError::Store(e.to_string()))?;
    lock_store(store).meta_delete("pull_done").map_err(|e| SyncError::Store(e.to_string()))?;
    Ok(updated)
}
```

Keep every line inside the `match item { SearchItem::Page(p) => { ... } SearchItem::DataSource(d) => { ... } }` arms byte-for-byte identical to today's implementation (only the surrounding checkpoint/progress bookkeeping is new) — this preserves the dirty-page-skips-block-fetch behavior (`dirty_pull.rs`) and comments/rows persistence untouched.

- [ ] **Step 8: Run to pass**

Run: `cargo test -p notion-tui-sync --test pull`
Expected: PASS, including `interrupted_first_crawl_resumes_from_the_checkpoint_instead_of_restarting` and every pre-existing test in the file.

- [ ] **Step 9: Wire "sync now" end-to-end in `main.rs`, add the wizard handoff line**

In `crates/notion-tui/src/main.rs`, after `spawn_sync`:

```rust
    let mut handle =
        notion_sync::spawn_sync(client, store.clone(), Duration::from_secs(cfg.poll_interval_secs));
    ...
    app.sync_notify = Some(handle.notify.clone());
```

In `crates/notion-tui/src/wizard.rs`'s `run_with`, after the "ok — connected" line:

```rust
            Some(name) => {
                writeln!(output, "ok — connected as \"{name}\"")?;
                writeln!(
                    output,
                    "starting first sync now — large workspaces show progress in the status bar \
                     and can take a few minutes."
                )?;
                return Ok(token);
            }
```

Update `accepts_first_valid_token`'s assertion (it currently only checks the output contains `"My Bot"`) to also assert the handoff line, so the addition is covered:

```rust
        assert!(printed.contains("My Bot"));
        assert!(printed.contains("starting first sync"));
```

- [ ] **Step 10: Full workspace gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

---

### Task 13: notion-store + notion-sync — remote-deletion mark-and-sweep reconciliation

**Files:**
- Modify: `crates/notion-store/src/store.rs` (`prune_missing`)
- Modify: `crates/notion-sync/src/puller.rs` (`reconcile_deletions`)
- Modify: `crates/notion-sync/src/lib.rs` (periodic invocation from the sync loop)
- Test: `crates/notion-store/tests/prune.rs` (new), `crates/notion-sync/tests/reconcile.rs` (new)

**Interfaces:**
- Consumes: nothing new.
- Produces: `Store::prune_missing(&mut self, seen_page_ids: &[String], seen_ds_ids: &[String]) -> anyhow::Result<(u32, u32)>` (returns `(pages_removed, data_sources_removed)`); `pub async fn reconcile_deletions(client: &NotionClient, store: &SharedStore) -> Result<(u32, u32), SyncError>`.

- [ ] **Step 1: Write the failing store test**

Create `crates/notion-store/tests/prune.rs`:

```rust
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec, Store};

fn page(id: &str) -> PageRec {
    PageRec { id: id.into(), parent_type: "workspace".into(), parent_id: None,
        title: "T".into(), icon: None, archived: false, last_edited_time: "t".into() }
}

#[test]
fn prune_missing_removes_pages_and_data_sources_absent_from_the_seen_set() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.upsert_page(&page("p2")).unwrap(); // will be "deleted remotely"
    s.replace_page_blocks("p2", &[BlockRec {
        id: "b1".into(), page_id: "p2".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "x".into(), has_children: false,
    }]).unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(), database_id: "db1".into(), title: "Tasks".into(),
        schema_json: "{}".into(), last_edited_time: "t".into(),
    }).unwrap();
    s.replace_rows("ds1", &[RowRec {
        id: "r1".into(), data_source_id: "ds1".into(), properties: "{}".into(),
        last_edited_time: "t".into(), archived: false,
    }]).unwrap();

    let (removed_pages, removed_ds) =
        s.prune_missing(&["p1".to_string()], &[]).unwrap();

    assert_eq!(removed_pages, 1);
    assert_eq!(removed_ds, 1);
    assert!(s.get_page("p1").unwrap().is_some());
    assert!(s.get_page("p2").unwrap().is_none());
    assert!(s.page_blocks("p2").unwrap().is_empty(), "orphaned blocks must be cleaned up");
    assert!(s.get_data_source("ds1").unwrap().is_none());
    assert!(s.rows("ds1").unwrap().is_empty(), "orphaned rows must be cleaned up");
}

#[test]
fn prune_missing_never_removes_a_page_with_a_pending_local_edit() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "x".into(), has_children: false,
    }]).unwrap();
    s.edit_delete_block("b1").unwrap(); // queues a pending_ops row targeting b1... 
    // ...but the guard checks pending_ops against the *page* id for page-level ops too:
    s.edit_rename_page("p1", "Renamed").unwrap();

    let (removed_pages, _) = s.prune_missing(&[], &[]).unwrap();

    assert_eq!(removed_pages, 0, "a page with pending ops must survive even if absent from `seen`");
    assert!(s.get_page("p1").unwrap().is_some());
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p notion-tui-store --test prune`
Expected: FAIL — `prune_missing` doesn't exist.

- [ ] **Step 3: Implement**

In `store.rs`:

```rust
    /// Mark-and-sweep reconciliation: removes local pages/data sources that
    /// no longer appear in a fresh full workspace search (i.e. were deleted,
    /// trashed, or un-shared from the integration remotely), together with
    /// their orphaned blocks/rows. A page or data source with any pending
    /// local op targeting it survives regardless of `seen` — an edit racing
    /// a stale reconciliation pass must never be silently discarded.
    pub fn prune_missing(
        &mut self,
        seen_page_ids: &[String],
        seen_ds_ids: &[String],
    ) -> anyhow::Result<(u32, u32)> {
        let tx = self.conn.transaction()?;
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS seen_pages (id TEXT PRIMARY KEY);
             DELETE FROM seen_pages;
             CREATE TEMP TABLE IF NOT EXISTS seen_ds (id TEXT PRIMARY KEY);
             DELETE FROM seen_ds;",
        )?;
        {
            let mut stmt = tx.prepare("INSERT OR IGNORE INTO seen_pages (id) VALUES (?1)")?;
            for id in seen_page_ids {
                stmt.execute([id])?;
            }
            let mut stmt = tx.prepare("INSERT OR IGNORE INTO seen_ds (id) VALUES (?1)")?;
            for id in seen_ds_ids {
                stmt.execute([id])?;
            }
        }
        let removed_pages = tx.execute(
            "DELETE FROM pages
             WHERE id NOT IN (SELECT id FROM seen_pages)
               AND NOT EXISTS (SELECT 1 FROM pending_ops WHERE target_id = pages.id)",
            [],
        )? as u32;
        let removed_ds = tx.execute("DELETE FROM data_sources WHERE id NOT IN (SELECT id FROM seen_ds)", [])? as u32;
        tx.execute("DELETE FROM blocks WHERE page_id NOT IN (SELECT id FROM pages)", [])?;
        tx.execute("DELETE FROM rows WHERE data_source_id NOT IN (SELECT id FROM data_sources)", [])?;
        tx.commit()?;
        Ok((removed_pages, removed_ds))
    }
```

- [ ] **Step 4: Run to pass**

Run: `cargo test -p notion-tui-store --test prune`
Expected: PASS.

- [ ] **Step 5: Write the failing sync-level test**

Create `crates/notion-sync/tests/reconcile.rs`:

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::{PageRec, Store};
use notion_sync::{reconcile_deletions, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn reconcile_deletions_removes_a_page_absent_from_a_full_crawl() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title", "title": [{"plain_text": "Still here"}]}}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    {
        let mut s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
            title: "Still here".into(), icon: None, archived: false, last_edited_time: "t".into(),
        }).unwrap();
        s.upsert_page(&PageRec {
            id: "trashed".into(), parent_type: "workspace".into(), parent_id: None,
            title: "Trashed elsewhere".into(), icon: None, archived: false, last_edited_time: "t".into(),
        }).unwrap();
    }

    let (removed_pages, _) = reconcile_deletions(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(removed_pages, 1);
    assert!(store.lock().unwrap().get_page("p1").unwrap().is_some());
    assert!(store.lock().unwrap().get_page("trashed").unwrap().is_none());
}
```

- [ ] **Step 6: Run to verify it fails**

Run: `cargo test -p notion-tui-sync --test reconcile`
Expected: FAIL — `reconcile_deletions` doesn't exist.

- [ ] **Step 7: Implement**

In `crates/notion-sync/src/puller.rs`:

```rust
/// A full, unfiltered workspace crawl (no hwm cutoff) used purely to build
/// the "seen" set for `Store::prune_missing`. Runs far less often than
/// `pull_once` (see `spawn_sync`'s cycle counter) since it always pages
/// through the entire workspace.
pub async fn reconcile_deletions(client: &NotionClient, store: &SharedStore) -> Result<(u32, u32), SyncError> {
    let mut seen_pages = Vec::new();
    let mut seen_ds = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let page = client.search_page(cursor.as_deref()).await?;
        for item in &page.items {
            match item {
                SearchItem::Page(p) => seen_pages.push(p.id.clone()),
                SearchItem::DataSource(d) => seen_ds.push(d.id.clone()),
                SearchItem::Other => {}
            }
        }
        cursor = page.next_cursor.clone();
        if cursor.is_none() {
            break;
        }
    }
    lock_store(store)
        .prune_missing(&seen_pages, &seen_ds)
        .map_err(|e| SyncError::Store(e.to_string()))
}
```

Re-export it from `crates/notion-sync/src/lib.rs`: `pub use puller::{pull_once, reconcile_deletions};`.

- [ ] **Step 8: Run to pass**

Run: `cargo test -p notion-tui-sync --test reconcile`
Expected: PASS.

- [ ] **Step 9: Call it periodically from the sync loop**

In `lib.rs`'s `spawn_sync`, thread a cycle counter and call `reconcile_deletions` every 20 cycles, after a successful pull:

```rust
const RECONCILE_EVERY_N_CYCLES: u32 = 20;

async fn one_cycle(
    client: &Arc<NotionClient>,
    store: &SharedStore,
    status_tx: &watch::Sender<SyncStatus>,
    data_tx: &watch::Sender<u64>,
    pending_tx: &watch::Sender<u32>,
    cycle_count: u32,
) {
    status_tx.send_replace(SyncStatus::Syncing { done: 0, total: 0 });
    match push_once(client, store).await { /* unchanged */ };
    pending_tx.send_replace(lock_store(store).pending_count().unwrap_or(0));

    match pull_once(client, store, status_tx).await {
        Ok(updated) => {
            if updated > 0 {
                data_tx.send_modify(|v| *v += 1);
            }
            status_tx.send_replace(SyncStatus::Idle { updated });
            if cycle_count % RECONCILE_EVERY_N_CYCLES == 0 {
                if let Ok((removed_pages, removed_ds)) = reconcile_deletions(client, store).await {
                    if removed_pages + removed_ds > 0 {
                        data_tx.send_modify(|v| *v += 1);
                    }
                }
            }
        }
        Err(SyncError::Api(ApiError::Network(_))) => status_tx.send_replace(SyncStatus::Offline),
        Err(e) => status_tx.send_replace(SyncStatus::Failed(e.to_string())),
    }
    pending_tx.send_replace(lock_store(store).pending_count().unwrap_or(0));
}
```

Thread `cycle_count` through `spawn_sync`'s loop (`let mut cycle_count: u32 = 0;` before the `loop`, `cycle_count = cycle_count.wrapping_add(1);` at the end of each iteration, passed by value into `one_cycle`).

- [ ] **Step 10: Run to pass**

Run: `cargo test -p notion-tui-sync`
Expected: PASS — existing `sync_loop.rs` tests are unaffected since `cycle_count % 20 == 0` is only true on the very first cycle (`0 % 20 == 0`), meaning reconciliation also runs once immediately on startup; existing fixtures mount a matching empty `/v1/search` response already reused by `pull_once`, so the extra full crawl this triggers hits the same already-mocked endpoint and returns an empty result — no new mocks required. If a `sync_loop.rs` test's mock is `.expect(1)`-bound to `/v1/search`, relax it to accommodate the extra reconciliation call this task intentionally adds, and note the change.

- [ ] **Step 11: Full workspace gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

---

### Task 14: 404/archived check on page open

**Files:**
- Modify: `crates/notion-tui/src/app.rs` (`AppMsg::PageGone`, `request_page_liveness_check`, `handle_page_gone`, `open_page` hook)
- Modify: `crates/notion-tui/src/main.rs` (route the new `AppMsg` variant)
- Modify: `crates/notion-store/src/store.rs` (`forget_page`)
- Test: `crates/notion-tui/tests/page_gone_flow.rs` (new)

**Interfaces:**
- Consumes: `NotionClient::get_page` (Task 11).
- Produces: `Store::forget_page(&mut self, page_id: &str) -> anyhow::Result<()>`; `App::handle_page_gone(&mut self, page_id: &str)`.

- [ ] **Step 1: Write the failing store test**

Append to `crates/notion-store/tests/prune.rs`:

```rust
#[test]
fn forget_page_deletes_the_page_and_its_blocks() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1")).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "x".into(), has_children: false,
    }]).unwrap();

    s.forget_page("p1").unwrap();

    assert!(s.get_page("p1").unwrap().is_none());
    assert!(s.page_blocks("p1").unwrap().is_empty());
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

Run: `cargo test -p notion-tui-store --test prune forget_page`
Expected: FAIL — `forget_page` doesn't exist.

In `store.rs`:

```rust
    pub fn forget_page(&mut self, page_id: &str) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM blocks WHERE page_id = ?1", [page_id])?;
        tx.execute("DELETE FROM pages WHERE id = ?1", [page_id])?;
        tx.commit()?;
        Ok(())
    }
```

Run: `cargo test -p notion-tui-store --test prune forget_page`
Expected: PASS.

- [ ] **Step 3: Write the failing app-level test**

Create `crates/notion-tui/tests/page_gone_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use notion_store::{PageRec, Store};
use notion_tui::app::{App, View};

fn store_with_page() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "T".into(), icon: None, archived: false, last_edited_time: "t".into(),
    }).unwrap();
    Arc::new(Mutex::new(s))
}

#[test]
fn page_gone_forgets_the_page_and_shows_a_notice() {
    let store = store_with_page();
    let mut app = App::new(store.clone());
    app.open_page("p1");
    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));

    app.handle_page_gone("p1");

    assert!(store.lock().unwrap().get_page("p1").unwrap().is_none());
    assert!(app.notice.as_deref().unwrap_or("").contains("deleted or unshared"));
    assert!(matches!(app.view, View::Empty), "no history to fall back to, so the view clears");
}

#[test]
fn page_gone_falls_back_to_history_when_available() {
    let store = store_with_page();
    {
        let mut s = store.lock().unwrap();
        s.upsert_page(&PageRec {
            id: "p2".into(), parent_type: "workspace".into(), parent_id: None,
            title: "Second".into(), icon: None, archived: false, last_edited_time: "t".into(),
        }).unwrap();
    }
    let mut app = App::new(store.clone());
    app.open_page("p1");
    app.history.push("p1".to_string());
    app.open_page("p2");

    app.handle_page_gone("p2");

    assert!(matches!(&app.view, View::Page(v) if v.page.id == "p1"));
}
```

- [ ] **Step 4: Run to verify it fails**

Run: `cargo test -p notion-tui --test page_gone_flow`
Expected: FAIL — `handle_page_gone` doesn't exist.

- [ ] **Step 5: Implement**

In `app.rs`, extend `AppMsg`:

```rust
    PageGone(String),
```

Add to `App`:

```rust
pub fn handle_page_gone(&mut self, page_id: &str) {
    self.store.lock().unwrap().forget_page(page_id).ok();
    self.notice = Some("that page was deleted or unshared in Notion".into());
    let showing_it = matches!(&self.view, View::Page(v) if v.page.id == page_id);
    if showing_it {
        match self.history.pop() {
            Some(prev) => self.open_page(&prev),
            None => self.view = View::Empty,
        }
    }
    self.refresh_sidebar();
}

/// Fires a background 404/archived check for a page the user just opened;
/// on either signal, `AppMsg::PageGone` lets the caller forget it locally
/// instead of leaving a ghost that resurfaces every sidebar refresh.
fn request_page_liveness_check(&mut self, page_id: &str) {
    let Some(remote) = &self.remote else { return };
    let (client, tx) = (remote.client.clone(), remote.tx.clone());
    let page_id = page_id.to_string();
    tokio::spawn(async move {
        match client.get_page(&page_id).await {
            Ok(meta) if meta.archived => {
                tx.send(AppMsg::PageGone(page_id)).ok();
            }
            Err(notion_api::ApiError::Api { status: 404, .. }) => {
                tx.send(AppMsg::PageGone(page_id)).ok();
            }
            _ => {}
        }
    });
}
```

In `open_page`, right before each successful `return;` that sets `View::Page`, add the check (only the page-view branch — tables/data sources aren't covered by this task):

```rust
        if let Ok(Some(page)) = guard.get_page(page_id) {
            let blocks = guard.page_blocks(page_id).unwrap_or_default();
            drop(guard);
            self.view = View::Page(PageView::new(page, blocks));
            self.request_page_liveness_check(page_id);
            return;
        }
```

- [ ] **Step 6: Route `AppMsg::PageGone` in `main.rs`**

In the `app_rx.recv()` match:

```rust
                Some(app::AppMsg::PageGone(page_id)) => app.handle_page_gone(&page_id),
```

- [ ] **Step 7: Run to pass**

Run: `cargo test -p notion-tui --test page_gone_flow`
Expected: `2 passed`.

- [ ] **Step 8: Full crate gate**

Run: `cargo test -p notion-tui-store -p notion-tui`
Expected: PASS.

---

### Task 15: Integration sweep

**Files:** none new.

- [ ] **Step 1: Full workspace gates**

Run (from `notion-tui/`): `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green.

- [ ] **Step 2: Cross-task interaction checks**

- `cargo test -p notion-tui-sync --test pull --test reconcile --test sync_loop` — Task 12's checkpointing and Task 13's reconciliation compose in the same sync cycle.
- `cargo test -p notion-tui --test mouse_flow --test history_flow --test table_filter_flow --test palette_command_flow` — mouse/history/filter/palette all touch `app.rs`'s dispatch surface; confirm they don't regress each other.
- `cargo test -p notion-tui --test page_gone_flow --test history_flow` — a page-gone fallback must itself go through `open_page`, not double-push history.
- `git -C /Users/rehatbir/Developer/fables status` — confirm only intended files changed; report the full file list.

- [ ] **Step 3: Manual acceptance pass (report, don't fix)**

Per spec §7's verification bar:
- Large-workspace crawl shows progress and survives interruption: if a real token/workspace is available, run `cargo run -p notion-tui` against a workspace with hundreds of pages, kill the process mid-first-sync (before the status bar reaches `✓ synced`), restart, and confirm the status bar resumes near where it left off rather than restarting from `⟳ syncing 0/…`. Otherwise, rely on Task 12's `interrupted_first_crawl_resumes_from_the_checkpoint` test as the automated proxy and note that the manual pass was skipped.
- Deletion reconciliation removes a trashed page: trash or un-share a page from Notion directly, wait for the next reconciliation cycle (or restart notion-tui, since cycle 0 always reconciles), and confirm it disappears from the sidebar. Otherwise rely on Task 13's `reconcile_deletions_removes_a_page_absent_from_a_full_crawl` test and note the skip.
- Mouse: click a sidebar entry, a page link, a table header (sorts), and drag a board card between columns; confirm each has a working keyboard equivalent per spec §4.5's "never required."

Report results; the session owner decides on commits.

---

## Spec coverage

| M7 item | Tasks |
|---|---|
| 1. Command palette parity (rename/move/group-by/sync-now, fuzzy palette + search re-rank) | 1, 2, 3, 4, 5 |
| 2. Mouse support (click-to-focus, sidebar/link clicks, header-sort, card drag) | 6 (history funnel), 7 |
| 3. Universal back-history | 6 |
| 4. Table filtering (per-column/free-text, survives sync, visible in title) | 8 |
| 5. First-crawl progress + checkpointing | 9 (status-bar rendering), 11 (`get_page`, unrelated prerequisite for 14), 12 |
| 6. Status bar completeness (breadcrumb + pending-keystroke) | 9 |
| 7. Block placeholders | 10 |
| 8. Remote deletion propagation (mark-and-sweep + 404/archived on open) | 11, 13, 14 |

## Design decisions and resolved ambiguities

- **Progress `total` is a live estimate, not a precise count.** Notion's search API exposes `has_more`/`next_cursor` but never a total item count. `total` is computed as `done_so_far + 100 (one page_size) if another page is known to exist, else done_so_far` — it grows as pagination proceeds and becomes exact on the final page. This is the only way to honor the mandated `"syncing 240/1,893 pages"` format without inventing an API capability that doesn't exist.
- **Checkpointing resumes via a persisted search cursor (`pull_cursor`), not just the hwm.** The hwm is only ever advanced after a fully clean pass (preserving M6's guarantee); `pull_cursor`/`pull_max_seen`/`pull_done` are a separate, more granular checkpoint cleared only on success, so a crawl interrupted on page 19 of 19 resumes from page 19 rather than page 1.
- **Board "group by" fix keeps the existing first-status/first-select default** (spec item 1 says it "also fixes the hard-picked first-status behavior") **by making it overridable per-board via a new palette command**, rather than removing the heuristic outright — a data source with an unconfigured board must still show something sensible when opened for the first time.
- **Mouse hit-testing approximates ratatui's internal `Table` column layout** via a standalone `Layout::horizontal(widths).split()` rather than reaching into ratatui internals; this is exact for the sidebar/page-view list (which use `ListState::offset()` directly) and best-effort for table header clicks, acceptable since spec §4.5 mandates mouse support is never required.
- **Board card drag has no rendered "ghost"** — `Drag` events retarget `BoardView::col` live (visible as the highlighted column moving), and `Up` commits the move; a separate floating drag indicator was scoped out as YAGN for v1's "never required" mouse tier.
- **Breadcrumb for table/board views is `"workspace → {data source title}"`**, not a full ancestry chain — `DataSourceRec` has no parent-page tracking in the current schema, and the spec's own example (`workspace → page → subpage`) only describes page nesting.
- **Rename/move-page reuse the existing `dirty`-flag guard** blocks that already protect in-flight block edits from being clobbered by a concurrent pull, rather than introducing a second dirty channel — consistent with how `update_row`/`delete_row` already gate on `rows.dirty`.
- **Reconciliation runs every 20 sync cycles (and once on startup, cycle 0)**, not on every cycle — a full unfiltered workspace crawl is unbounded work per pass; the spec calls for "periodic," not "every tick."

## Assumptions about other milestones

- M8+ (referenced nowhere in this plan) is assumed not to touch `app.rs`'s `Action` enum, `SyncStatus`, `PickerPurpose`, or `TableView`/`BoardView` fields added here; if a parallel milestone is mid-flight against the same files, re-run Task 15's cross-task checks after merging.
- The wizard's plain first-run flow (no resumed-session concept) is assumed to remain synchronous prompt-then-exit; if a future milestone makes the wizard itself launch the TUI in-process, Task 12's handoff-line addition in `wizard.rs` may need to move to wherever that handoff actually happens.
