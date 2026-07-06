# notion-tui Milestone 4 — Board View, Comments, Conflicts UI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the remaining v1 surface (design spec §8 milestone 4): kanban board view with optimistic card moves, comments (read and write, page- and block-level), and the conflicts/queue screen with all three resolution actions (keep mine / take theirs / merge in `$EDITOR`).

**Architecture:** Board view is a pure UI module over the existing `rows`/`data_sources` store tables — moving a card is just `App::update_row_property` (M2). Comments reuse the whole M2 write pipeline: a `comments` table method set in `notion-store`, a `create_comment` op type in the pusher, and comment pulls piggy-backed on the puller's changed-page fetch. The conflicts screen renders `pending_ops` directly; resolutions that need remote data (take-theirs refetch, merge) run through a new `RemoteHandle` — an `Arc<NotionClient>` plus an mpsc channel back into the main event loop — so no screen ever awaits the network.

**Tech Stack:** No new dependencies. `wiremock`/`TestBackend`/in-memory SQLite testing patterns carried over from M1–M3.

**Depends on:** Milestone 3 (`markdown` module, `editor::edit_text`, the confirm-modal pattern, `edit_reorder_block`). Do not start this plan until M3 is merged.

## Global Constraints

- Notion API version `2025-09-03`.
- The UI renders exclusively from `notion-store`; user actions never await the network. Background fetches (merge, take-theirs refetch, comment refresh) run in spawned tokio tasks that write to the store and notify the event loop via channel.
- Conflicts are surfaced, never silently resolved (spec §2). Ops are never silently dropped.
- Notion's API cannot delete comments — comment creation is therefore not undoable and returns no `EditReceipt`.

---

## Design notes (read before starting)

**Board grouping.** The board groups by the data source's first `status` property, falling back to the first `select` property. Column list = that property's schema options in schema order, plus a trailing `(none)` column for rows with no value. `J`/`K` move the cursor card to the next/previous column by writing the new option name through `App::update_row_property` — the same optimistic path the property form uses, so queueing/undo/pending-count all come for free.

**Comment pulling.** The official API lists comments per block (`GET /v1/comments?block_id=…`, where a page id is also a valid block id). Pulling comments for every block of every page would explode request counts, so the puller fetches page-level comments only (one extra request per changed page). Block-level comments are refreshed on demand: opening the comments panel on a cursor block triggers a background fetch for that one block via `RemoteHandle`. Both refresh paths write to the same `comments` table and repaint via the notify channel.

**Conflict resolutions.**
- *Retry* (`r`, failed ops): state back to `pending`, error cleared. Next push cycle re-attempts.
- *Keep mine* (`p`, conflicted ops): clear `base_edited_time` and set state `pending` — the pusher's conflict check is skipped when base is NULL, so the op pushes over the remote change.
- *Take theirs* (`t`): delete the op; clear the target page/row dirty flag if no other ops reference it; then request a background refetch of that page (block tree) or data source (rows) so the remote version replaces the stale local one. The refetch is needed because the puller's high-water mark has usually already passed the remote edit (the pull saw it but skipped the dirty record).
- *Merge* (`e`, conflicted `update_block` ops only in v1): background-fetch the remote page's current block text and `last_edited_time`, then open `$EDITOR` on a conflict-marker document (`<<<<<<< local / ======= / >>>>>>> remote`). On save, `Store::resolve_conflict_merge` transactionally deletes the conflicted op, advances the page's stored `last_edited_time` to the fetched remote value, and applies the merged text as a fresh `edit_update_block_text` — producing a new pending op whose base is the remote time, so it pushes cleanly. Conflicted row ops offer keep-mine/take-theirs only; this v1 narrowing is deliberate (property JSON has no meaningful line-merge representation).

**Queue screen navigation.** `Q` swaps the current `View` into `App.queue_return` and shows `View::Queue`; `Q`/`Esc` swaps back. This preserves the user's page/table/board cursor state exactly.

---

### Task 1: `BoardView` — grouping, cursor, card moves

**Files:**
- Create: `crates/notion-tui/src/ui/board.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`

**Interfaces:**
- Consumes: `notion_store::{DataSourceRec, RowRec}`, `crate::ui::table::cell_text`.
- Produces: `pub struct BoardView { pub ds: DataSourceRec, pub group_prop: String, pub group_type: String, pub columns: Vec<String>, pub rows: Vec<RowRec>, pub col: usize, pub card: usize }`, `pub fn group_property(schema_json: &str) -> Option<(String, String)>`, methods `cards_in(&self, col: usize) -> Vec<&RowRec>`, `selected_row_id(&self) -> Option<String>`, `move_cursor_card(&mut self, delta: isize)`, `move_cursor_col(&mut self, delta: isize)`, `move_card(&mut self, delta: isize) -> Option<(String, String)>` (row_id, new option value — `""` for the `(none)` column), `pub fn render(f, area, view, focused)`.

- [ ] **Step 1: Write the failing test**

Inline `#[cfg(test)]` module in `crates/notion-tui/src/ui/board.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{DataSourceRec, RowRec};
    use serde_json::json;

    fn ds() -> DataSourceRec {
        DataSourceRec {
            id: "ds".into(), database_id: "db".into(), title: "Tasks".into(),
            schema_json: json!({
                "Name": {"type": "title"},
                "Status": {"type": "status", "status": {"options": [
                    {"name": "Todo"}, {"name": "Doing"}, {"name": "Done"}]}}
            }).to_string(),
            last_edited_time: "t".into(),
        }
    }

    fn row(id: &str, name: &str, status: Option<&str>) -> RowRec {
        let status_val = match status {
            Some(s) => json!({"type": "status", "status": {"name": s}}),
            None => json!({"type": "status", "status": null}),
        };
        RowRec {
            id: id.into(), data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": name}]},
                "Status": status_val
            }).to_string(),
            last_edited_time: "t".into(), archived: false,
        }
    }

    #[test]
    fn groups_rows_by_status_options_plus_none_column() {
        let v = BoardView::new(ds(), vec![row("r1", "A", Some("Todo")), row("r2", "B", Some("Done")), row("r3", "C", None)]);
        assert_eq!(v.group_prop, "Status");
        assert_eq!(v.columns, vec!["Todo", "Doing", "Done", "(none)"]);
        assert_eq!(v.cards_in(0).len(), 1);
        assert_eq!(v.cards_in(1).len(), 0);
        assert_eq!(v.cards_in(3)[0].id, "r3");
    }

    #[test]
    fn move_card_returns_row_and_target_option() {
        let mut v = BoardView::new(ds(), vec![row("r1", "A", Some("Todo"))]);
        v.col = 0;
        v.card = 0;
        assert_eq!(v.move_card(1), Some(("r1".to_string(), "Doing".to_string())));
        // Moving off either end is a no-op:
        v.col = 3; // (none) column is empty now locally — nothing to move
        assert_eq!(v.move_card(1), None);
    }

    #[test]
    fn no_groupable_property_means_no_board() {
        let plain = DataSourceRec {
            id: "ds".into(), database_id: "db".into(), title: "T".into(),
            schema_json: json!({"Name": {"type": "title"}}).to_string(),
            last_edited_time: "t".into(),
        };
        assert!(group_property(&plain.schema_json).is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui ui::board --lib`
Expected: FAIL — module `board` does not exist.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/ui/board.rs` (above the test module):
```rust
use notion_store::{DataSourceRec, RowRec};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;
use serde_json::Value;

use crate::ui::table::cell_text;

pub struct BoardView {
    pub ds: DataSourceRec,
    pub group_prop: String,
    pub group_type: String,
    pub columns: Vec<String>,
    pub rows: Vec<RowRec>,
    pub col: usize,
    pub card: usize,
}

/// Picks the grouping property: first `status`, else first `select`, in schema
/// key order. Returns (property name, property type).
pub fn group_property(schema_json: &str) -> Option<(String, String)> {
    let schema: Value = serde_json::from_str(schema_json).ok()?;
    let map = schema.as_object()?;
    for wanted in ["status", "select"] {
        if let Some((name, _)) = map.iter().find(|(_, def)| def["type"] == wanted) {
            return Some((name.clone(), wanted.to_string()));
        }
    }
    None
}

fn schema_options(schema_json: &str, prop: &str, prop_type: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(schema_json).unwrap_or_default();
    schema[prop][prop_type]["options"]
        .as_array()
        .map(|a| a.iter().filter_map(|o| o["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

impl BoardView {
    pub fn new(ds: DataSourceRec, rows: Vec<RowRec>) -> BoardView {
        let (group_prop, group_type) =
            group_property(&ds.schema_json).unwrap_or_else(|| ("".into(), "".into()));
        let mut columns = schema_options(&ds.schema_json, &group_prop, &group_type);
        columns.push("(none)".to_string());
        BoardView { ds, group_prop, group_type, columns, rows, col: 0, card: 0 }
    }

    fn row_group(&self, row: &RowRec) -> String {
        let props: Value = serde_json::from_str(&row.properties).unwrap_or_default();
        let name = props[&self.group_prop][&self.group_type]["name"].as_str().unwrap_or("");
        if name.is_empty() { "(none)".to_string() } else { name.to_string() }
    }

    pub fn cards_in(&self, col: usize) -> Vec<&RowRec> {
        let Some(col_name) = self.columns.get(col) else { return Vec::new() };
        self.rows.iter().filter(|r| &self.row_group(r) == col_name).collect()
    }

    pub fn selected_row_id(&self) -> Option<String> {
        self.cards_in(self.col).get(self.card).map(|r| r.id.clone())
    }

    pub fn move_cursor_card(&mut self, delta: isize) {
        let len = self.cards_in(self.col).len();
        if len == 0 { return; }
        self.card = (self.card as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn move_cursor_col(&mut self, delta: isize) {
        if self.columns.is_empty() { return; }
        self.col = (self.col as isize + delta).clamp(0, self.columns.len() as isize - 1) as usize;
        self.card = self.card.min(self.cards_in(self.col).len().saturating_sub(1));
    }

    /// Returns (row_id, new option name) for the optimistic write, or None if
    /// there is no selected card or the move runs off the board. Moving into
    /// the trailing `(none)` column clears the value (empty string sentinel).
    pub fn move_card(&mut self, delta: isize) -> Option<(String, String)> {
        let row_id = self.selected_row_id()?;
        let target = self.col as isize + delta;
        if target < 0 || target as usize >= self.columns.len() {
            return None;
        }
        let target = target as usize;
        let value = if self.columns[target] == "(none)" { String::new() } else { self.columns[target].clone() };
        self.col = target;
        Some((row_id, value))
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &BoardView, focused: bool) {
    let n = view.columns.len().max(1) as u32;
    let constraints: Vec<Constraint> = view.columns.iter().map(|_| Constraint::Ratio(1, n)).collect();
    let cols = Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area);
    for (ci, rect) in cols.iter().enumerate() {
        let cards = view.cards_in(ci);
        let items: Vec<ListItem> = cards
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let props: Value = serde_json::from_str(&r.properties).unwrap_or_default();
                let title = props
                    .as_object()
                    .and_then(|m| m.values().find(|p| p["type"] == "title"))
                    .map(cell_text)
                    .unwrap_or_default();
                let mut item = ListItem::new(title);
                if focused && ci == view.col && i == view.card {
                    item = item.style(Style::default().add_modifier(Modifier::REVERSED));
                }
                item
            })
            .collect();
        let title = format!(" {} ({}) ", view.columns[ci], cards.len());
        f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(title)), *rect);
    }
}
```

`crates/notion-tui/src/ui/mod.rs` — add `pub mod board;` to the module list.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui ui::board --lib`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/ui/board.rs crates/notion-tui/src/ui/mod.rs
git commit -m "feat(notion-tui): kanban board view widget"
```

---

### Task 2: `v` toggle + board keys in the app

**Files:**
- Modify: `crates/notion-tui/src/app.rs`, `crates/notion-tui/src/ui/mod.rs`
- Test: `crates/notion-tui/tests/board_flow.rs` (new)

**Interfaces:**
- Consumes: `BoardView` (Task 1), `App::update_row_property` (M2).
- Produces: `View::Board(BoardView)` variant; `v` toggles Table↔Board (no-op when the schema has no status/select property); board keys `h`/`l` column, `j`/`k` card, `J`/`K` move card, `Enter` open row as page. `Action` gains `MoveCard { row_id: String, prop_name: String, prop_type: String, value: String }`.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/board_flow.rs`:
```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::{DataSourceRec, RowRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;

fn store_with_board_ds() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds".into(), database_id: "db".into(), title: "Tasks".into(),
        schema_json: json!({
            "Name": {"type": "title"},
            "Status": {"type": "status", "status": {"options": [{"name": "Todo"}, {"name": "Done"}]}}
        }).to_string(),
        last_edited_time: "t".into(),
    }).unwrap();
    s.replace_rows("ds", &[RowRec {
        id: "r1".into(), data_source_id: "ds".into(),
        properties: json!({
            "Name": {"type": "title", "title": [{"plain_text": "Task A"}]},
            "Status": {"type": "status", "status": {"name": "Todo"}}
        }).to_string(),
        last_edited_time: "t0".into(), archived: false,
    }]).unwrap();
    Arc::new(Mutex::new(s))
}

fn key(c: char) -> KeyEvent { KeyEvent::from(KeyCode::Char(c)) }
fn shift(c: char) -> KeyEvent { KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT) }

#[test]
fn v_toggles_table_to_board_and_back() {
    let mut app = App::new(store_with_board_ds());
    app.focus = Focus::Main;
    app.open_page("ds");
    assert!(matches!(app.view, View::Table(_)));
    dispatch_key(&mut app, key('v'));
    assert!(matches!(app.view, View::Board(_)));
    dispatch_key(&mut app, key('v'));
    assert!(matches!(app.view, View::Table(_)));
}

#[test]
fn shift_j_moves_card_and_queues_an_update_row_op() {
    let store = store_with_board_ds();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    app.open_page("ds");
    dispatch_key(&mut app, key('v'));

    dispatch_key(&mut app, shift('J')); // Todo -> Done

    let s = store.lock().unwrap();
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_eq!(ops[0].op_type, "update_row");
    let rows = s.rows("ds").unwrap();
    let props: serde_json::Value = serde_json::from_str(&rows[0].properties).unwrap();
    assert_eq!(props["Status"]["status"]["name"], "Done");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test board_flow`
Expected: FAIL — no `View::Board` variant.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/app.rs`:

Add the variant and import:
```rust
use crate::ui::board::BoardView;

pub enum View {
    Empty,
    Page(PageView),
    Table(TableView),
    Board(BoardView),
}
```

Extend `Action`:
```rust
    MoveCard { row_id: String, prop_name: String, prop_type: String, value: String },
```

In `refresh_current_view`, handle the new variant (rebuild, preserving cursors):
```rust
            View::Board(v) => {
                let (id, col, card) = (v.ds.id.clone(), v.col, v.card);
                let guard = self.store.lock().unwrap();
                let ds = guard.get_data_source(&id).ok().flatten();
                let rows = guard.rows(&id).unwrap_or_default();
                drop(guard);
                if let Some(ds) = ds {
                    let mut b = BoardView::new(ds, rows);
                    b.col = col.min(b.columns.len().saturating_sub(1));
                    b.card = card.min(b.cards_in(b.col).len().saturating_sub(1));
                    self.view = View::Board(b);
                }
            }
```

In `dispatch_key`'s `Focus::Main` section (alongside the Table `o`/`p` openers), add the `v` toggle:
```rust
        if key.code == KeyCode::Char('v') {
            match &self.view {
                _ => {}
            }
        }
```
Concretely, add this before the `handle_key` fallthrough:
```rust
        if key.code == KeyCode::Char('v') {
            match std::mem::replace(&mut app.view, View::Empty) {
                View::Table(t) => {
                    if crate::ui::board::group_property(&t.ds.schema_json).is_some() {
                        app.view = View::Board(BoardView::new(t.ds, t.rows));
                    } else {
                        app.view = View::Table(t);
                    }
                }
                View::Board(b) => {
                    app.view = View::Table(crate::ui::table::TableView::new(b.ds, b.rows));
                }
                other => app.view = other,
            }
            return;
        }
```

In `handle_key`'s `Focus::Main` section, add a Board arm (mirroring the Table arm):
```rust
        if let View::Board(view) = &mut app.view {
            match (key.code, key.modifiers) {
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => view.move_cursor_card(1),
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => view.move_cursor_card(-1),
                (KeyCode::Char('h'), _) => view.move_cursor_col(-1),
                (KeyCode::Char('l'), _) => view.move_cursor_col(1),
                (KeyCode::Char('J'), _) => {
                    if let Some((row_id, value)) = view.move_card(1) {
                        return Action::MoveCard {
                            row_id,
                            prop_name: view.group_prop.clone(),
                            prop_type: view.group_type.clone(),
                            value,
                        };
                    }
                }
                (KeyCode::Char('K'), _) => {
                    if let Some((row_id, value)) = view.move_card(-1) {
                        return Action::MoveCard {
                            row_id,
                            prop_name: view.group_prop.clone(),
                            prop_type: view.group_type.clone(),
                            value,
                        };
                    }
                }
                (KeyCode::Enter, _) => {
                    if let Some(row_id) = view.selected_row_id() {
                        return Action::OpenPage(row_id);
                    }
                }
                _ => {}
            }
        }
```
Note: crossterm reports `Char('J')` with `SHIFT` in `modifiers`; the `(KeyCode::Char('J'), _)` pattern matches regardless. The `j`/`k` arms are pinned to `KeyModifiers::NONE` so they don't shadow `J`/`K`.

Handle the new action at the bottom of `dispatch_key`:
```rust
        Action::MoveCard { row_id, prop_name, prop_type, value } => {
            app.update_row_property(&row_id, &prop_name, &prop_type, &value)
        }
```

Also extend the two `dd`-target and `View::…` matches that enumerate variants (`handle_key`'s `d`-key `target_id` match and `scroll`) with a Board arm:
```rust
                View::Board(view) => view.selected_row_id(),   // in the dd target match; dd deletes the selected card's row
```
```rust
        View::Board(v) => v.move_cursor_card(delta),           // in scroll()
```
And in the `dd` action dispatch match, `View::Board(_) => Action::DeleteRow(id)`.

`crates/notion-tui/src/ui/mod.rs` — render the new variant in `draw()`:
```rust
        View::Board(view) => board::render(f, main_area, view, main_focused),
```

`update_row_property` (existing) rebuilds `app.props` from a `View::Table` only; its `if let View::Table(view) = &self.view` already ignores Board — no change needed.

One store nuance: `build_property_value("status", "")` currently produces `{"status": {"name": ""}}`. Moving to `(none)` must clear the value. Update `build_property_value` in `crates/notion-tui/src/ui/props.rs` for `select`/`status` to emit `null` on empty text:
```rust
        "select" => {
            if text.is_empty() { json!({"type": "select", "select": null}) }
            else { json!({"type": "select", "select": {"name": text}}) }
        }
        "status" => {
            if text.is_empty() { json!({"type": "status", "status": null}) }
            else { json!({"type": "status", "status": {"name": text}}) }
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test board_flow`
Expected: PASS (2 tests). Also run `cargo test -p notion-tui` to confirm no regressions in the exhaustive `View` matches.

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/app.rs crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/ui/props.rs crates/notion-tui/tests/board_flow.rs
git commit -m "feat(notion-tui): board view toggle and optimistic card moves"
```

---

### Task 3: comments endpoints in `notion-api`

**Files:**
- Modify: `crates/notion-api/src/endpoints.rs`, `crates/notion-api/src/types.rs`, `crates/notion-api/src/lib.rs`
- Test: `crates/notion-api/tests/comments.rs` (new)

**Interfaces:**
- Consumes: existing `get_json`/`post_json`.
- Produces: `pub struct Comment { pub id: String, pub discussion_id: String, pub author: String, pub body: String, pub created_time: String }`, `pub async fn list_comments(&self, block_id: &str) -> Result<Vec<Comment>, ApiError>` (paginates internally), `pub async fn create_comment(&self, parent: Value, body_text: &str) -> Result<Value, ApiError>`.

- [ ] **Step 1: Write the failing test**

`crates/notion-api/tests/comments.rs`:
```rust
use std::time::Duration;

use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn list_comments_parses_and_paginates() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/comments")).and(query_param("block_id", "p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "id": "c1", "discussion_id": "d1",
                "created_time": "2026-07-06T10:00:00.000Z",
                "created_by": {"object": "user", "id": "u1"},
                "rich_text": [{"plain_text": "Nice page"}]
            }],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;

    let comments = client(&server).list_comments("p1").await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, "c1");
    assert_eq!(comments[0].discussion_id, "d1");
    assert_eq!(comments[0].body, "Nice page");
    assert_eq!(comments[0].author, "u1");
}

#[tokio::test]
async fn create_comment_posts_parent_and_rich_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/comments"))
        .and(body_partial_json(json!({
            "parent": {"page_id": "p1"},
            "rich_text": [{"type": "text", "text": {"content": "hello"}}]
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "c9", "discussion_id": "d9"})))
        .mount(&server).await;

    let v = client(&server)
        .create_comment(json!({"page_id": "p1"}), "hello")
        .await
        .unwrap();
    assert_eq!(v["id"], "c9");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-api --test comments`
Expected: FAIL — `list_comments` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-api/src/types.rs` — add:
```rust
#[derive(Debug, Clone)]
pub struct Comment {
    pub id: String,
    pub discussion_id: String,
    pub author: String,
    pub body: String,
    pub created_time: String,
}

impl Comment {
    pub fn parse(v: &Value) -> Comment {
        Comment {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            discussion_id: v["discussion_id"].as_str().unwrap_or_default().to_string(),
            author: v["created_by"]["id"].as_str().unwrap_or_default().to_string(),
            body: rich_text_plain(&v["rich_text"]),
            created_time: v["created_time"].as_str().unwrap_or_default().to_string(),
        }
    }
}
```

`crates/notion-api/src/endpoints.rs` — add to the `impl NotionClient` (and `use crate::types::Comment;`):
```rust
    /// All comments attached to a block (a page id is a valid block id here).
    pub async fn list_comments(&self, block_id: &str) -> Result<Vec<Comment>, ApiError> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut path = format!("/v1/comments?block_id={block_id}&page_size=100");
            if let Some(c) = &cursor {
                path.push_str(&format!("&start_cursor={c}"));
            }
            let v = self.get_json(&path).await?;
            for item in v["results"].as_array().into_iter().flatten() {
                out.push(Comment::parse(item));
            }
            cursor = v["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok(out);
            }
        }
    }

    /// `parent` is either `{"page_id": ...}` / `{"block_id": ...}` for a new
    /// thread, or callers pass a body with `discussion_id` via `create_comment_reply`.
    pub async fn create_comment(&self, parent: Value, body_text: &str) -> Result<Value, ApiError> {
        let body = json!({
            "parent": parent,
            "rich_text": [{"type": "text", "text": {"content": body_text}}]
        });
        self.post_json("/v1/comments", &body).await
    }

    pub async fn create_comment_reply(&self, discussion_id: &str, body_text: &str) -> Result<Value, ApiError> {
        let body = json!({
            "discussion_id": discussion_id,
            "rich_text": [{"type": "text", "text": {"content": body_text}}]
        });
        self.post_json("/v1/comments", &body).await
    }
```

`crates/notion-api/src/lib.rs` — add `Comment` to the `pub use types::{...}` list.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-api --test comments`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-api/src/endpoints.rs crates/notion-api/src/types.rs crates/notion-api/src/lib.rs crates/notion-api/tests/comments.rs
git commit -m "feat(notion-api): list and create comment endpoints"
```

---

### Task 4: comment persistence + optimistic comment creation in `notion-store`

**Files:**
- Modify: `crates/notion-store/src/store.rs`, `crates/notion-store/src/lib.rs`
- Test: `crates/notion-store/tests/comments.rs` (new)

**Interfaces:**
- Consumes: existing `comments` table (schema v1), `enqueue_op`.
- Produces: `pub struct CommentRec { pub id: String, pub parent_id: String, pub parent_kind: String, pub thread_id: Option<String>, pub author: String, pub body: String, pub created_time: String }`; `replace_comments(&self, parent_id: &str, recs: &[CommentRec])`, `comments_for(&self, parent_id: &str) -> Vec<CommentRec>`, `edit_add_comment(&mut self, parent_id: &str, parent_kind: &str, thread_id: Option<&str>, body: &str) -> anyhow::Result<String>` (returns the temp comment id; enqueues a `create_comment` op; no receipt — comments are not undoable), `rewrite_comment_id(&mut self, old_id, new_id)`.

- [ ] **Step 1: Write the failing test**

`crates/notion-store/tests/comments.rs`:
```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-store --test comments`
Expected: FAIL — `CommentRec` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-store/src/store.rs` — add the struct near the other `*Rec` types:
```rust
#[derive(Debug, Clone)]
pub struct CommentRec {
    pub id: String,
    pub parent_id: String,
    pub parent_kind: String,
    pub thread_id: Option<String>,
    pub author: String,
    pub body: String,
    pub created_time: String,
}
```

and the methods on `impl Store`:
```rust
    pub fn replace_comments(&self, parent_id: &str, recs: &[CommentRec]) -> anyhow::Result<()> {
        // Keep locally-created (still-pending, tmp-id) comments; replace the synced rest.
        self.conn.execute(
            "DELETE FROM comments WHERE parent_id = ?1 AND id NOT LIKE 'tmp-%'",
            [parent_id],
        )?;
        let mut stmt = self.conn.prepare(
            "INSERT OR REPLACE INTO comments (id, parent_id, parent_kind, thread_id, author, body, created_time)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )?;
        for c in recs {
            stmt.execute(rusqlite::params![
                c.id, c.parent_id, c.parent_kind, c.thread_id, c.author, c.body, c.created_time
            ])?;
        }
        Ok(())
    }

    pub fn comments_for(&self, parent_id: &str) -> anyhow::Result<Vec<CommentRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_id, parent_kind, thread_id, author, body, created_time
             FROM comments WHERE parent_id = ?1 ORDER BY created_time, id",
        )?;
        let out = stmt
            .query_map([parent_id], |r| {
                Ok(CommentRec {
                    id: r.get(0)?, parent_id: r.get(1)?, parent_kind: r.get(2)?,
                    thread_id: r.get(3)?, author: r.get(4)?, body: r.get(5)?,
                    created_time: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// Optimistic comment creation. Not undoable (the API cannot delete
    /// comments), so this returns the temp id rather than an EditReceipt.
    pub fn edit_add_comment(
        &mut self,
        parent_id: &str,
        parent_kind: &str,
        thread_id: Option<&str>,
        body: &str,
    ) -> anyhow::Result<String> {
        let id = format!(
            "tmp-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        );
        self.conn.execute(
            "INSERT INTO comments (id, parent_id, parent_kind, thread_id, author, body, created_time)
             VALUES (?1, ?2, ?3, ?4, 'me', ?5, '')",
            rusqlite::params![id, parent_id, parent_kind, thread_id, body],
        )?;
        let payload = json!({
            "parent_id": parent_id,
            "parent_kind": parent_kind,
            "thread_id": thread_id,
            "body": body,
        })
        .to_string();
        self.enqueue_op("create_comment", &id, &payload, None)?;
        Ok(id)
    }

    pub fn rewrite_comment_id(&mut self, old_id: &str, new_id: &str) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE comments SET id = ?2 WHERE id = ?1", rusqlite::params![old_id, new_id])?;
        tx.execute(
            "UPDATE pending_ops SET target_id = ?2 WHERE target_id = ?1",
            rusqlite::params![old_id, new_id],
        )?;
        tx.commit()?;
        Ok(())
    }
```

`crates/notion-store/src/lib.rs` — add `CommentRec` to the re-export list.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-store --test comments`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-store/src/store.rs crates/notion-store/src/lib.rs crates/notion-store/tests/comments.rs
git commit -m "feat(notion-store): comment persistence and optimistic comment creation"
```

---

### Task 5: sync comments — pull with changed pages, push `create_comment` ops

**Files:**
- Modify: `crates/notion-sync/src/puller.rs`, `crates/notion-sync/src/pusher.rs`
- Test: `crates/notion-sync/tests/comments_sync.rs` (new)

**Interfaces:**
- Consumes: `NotionClient::{list_comments, create_comment, create_comment_reply}` (Task 3), `Store::{replace_comments, rewrite_comment_id, CommentRec}` (Task 4).
- Produces: puller stores page-level comments for every changed page; pusher handles op type `create_comment` (new thread on page or block, or reply to a discussion), rewriting the temp comment id on success.

- [ ] **Step 1: Write the failing test**

`crates/notion-sync/tests/comments_sync.rs`:
```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::{pull_once, push_once};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn client(server: &MockServer) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn pull_stores_page_comments_for_changed_pages() {
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
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"id": "c1", "discussion_id": "d1",
                "created_time": "2026-07-06T09:00:00.000Z",
                "created_by": {"id": "u1"},
                "rich_text": [{"plain_text": "remote comment"}]}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    pull_once(&client(&server), &store).await.unwrap();

    let comments = store.lock().unwrap().comments_for("p1").unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "remote comment");
    assert_eq!(comments[0].parent_kind, "page");
}

#[tokio::test]
async fn push_create_comment_calls_api_and_rewrites_temp_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/comments"))
        .and(body_partial_json(json!({"parent": {"page_id": "p1"}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "real-c1", "discussion_id": "d1"})))
        .mount(&server).await;

    let mut s = Store::open_in_memory().unwrap();
    let tmp = s.edit_add_comment("p1", "page", None, "hello").unwrap();
    let store = Arc::new(Mutex::new(s));

    let pushed = push_once(&client(&server), &store).await.unwrap();
    assert_eq!(pushed, 1);
    let s = store.lock().unwrap();
    assert!(s.ops().unwrap().is_empty());
    assert_eq!(s.comments_for("p1").unwrap()[0].id, "real-c1");
    assert_ne!(tmp, "real-c1");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-sync --test comments_sync`
Expected: FAIL — pull stores no comments; push marks the op `failed` (unknown op_type).

- [ ] **Step 3: Write minimal implementation**

`crates/notion-sync/src/puller.rs` — in the `SearchItem::Page(p)` arm, after `replace_page_blocks` (and only on that non-dirty path, since it costs one request per changed page):
```rust
                    let comments = client.list_comments(&p.id).await?;
                    let comment_recs: Vec<notion_store::CommentRec> = comments
                        .iter()
                        .map(|c| notion_store::CommentRec {
                            id: c.id.clone(),
                            parent_id: p.id.clone(),
                            parent_kind: "page".into(),
                            thread_id: Some(c.discussion_id.clone()),
                            author: c.author.clone(),
                            body: c.body.clone(),
                            created_time: c.created_time.clone(),
                        })
                        .collect();
                    store.lock().unwrap().replace_comments(&p.id, &comment_recs).ok();
```

`crates/notion-sync/src/pusher.rs` — add a handler and dispatch arm:
```rust
async fn push_create_comment(
    client: &NotionClient,
    store: &SharedStore,
    op: &OpRec,
) -> Result<PushOutcome, ApiError> {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    let body = payload["body"].as_str().unwrap_or_default();
    let resp = if let Some(discussion_id) = payload["thread_id"].as_str() {
        client.create_comment_reply(discussion_id, body).await?
    } else {
        let parent_id = payload["parent_id"].as_str().unwrap_or_default();
        let parent = match payload["parent_kind"].as_str() {
            Some("block") => json!({"block_id": parent_id}),
            _ => json!({"page_id": parent_id}),
        };
        client.create_comment(parent, body).await?
    };
    let real_id = resp["id"].as_str().unwrap_or_default().to_string();

    let mut guard = store.lock().unwrap();
    guard.rewrite_comment_id(&op.target_id, &real_id).ok();
    guard.delete_op(op.seq).ok();
    Ok(PushOutcome::Success)
}
```
and in `push_one`'s match: `"create_comment" => push_create_comment(client, store, op).await,`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-sync --test comments_sync`
Expected: PASS (2 tests). Existing `notion-sync` pull tests will now need a comments mock — check `cargo test -p notion-sync`; if `pull.rs`/`dirty_pull.rs`/`sync_loop.rs` tests fail on the missing `/v1/comments` route, add a catch-all empty mock to each:
```rust
    Mock::given(method("GET")).and(path("/v1/comments"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [], "has_more": false, "next_cursor": null})))
        .mount(&server).await;
```
(The same applies to `crates/notion-tui/tests/e2e.rs` / `e2e_write.rs` / `e2e_editor.rs` — run `cargo test` at the root and patch every fixture that pulls.)

- [ ] **Step 5: Commit**

```bash
git add crates/notion-sync/src/puller.rs crates/notion-sync/src/pusher.rs crates/notion-sync/tests/comments_sync.rs
git add crates/notion-sync/tests crates/notion-tui/tests   # fixture updates
git commit -m "feat(notion-sync): pull page comments and push comment creation"
```

---

### Task 6: comments side panel (`c`)

**Files:**
- Create: `crates/notion-tui/src/ui/comments.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`, `crates/notion-tui/src/app.rs`
- Test: `crates/notion-tui/tests/comments_flow.rs` (new)

**Interfaces:**
- Consumes: `Store::{comments_for, edit_add_comment}` (Task 4), the existing `InputState`/`InputPurpose` modal machinery.
- Produces: `pub struct CommentsState { pub parent_id: String, pub parent_kind: String, pub items: Vec<CommentRec>, pub cursor: usize }` with `on_key(&mut self, key) -> CommentsAction`, `pub enum CommentsAction { None, Close, NewThread, Reply }`, `pub fn render(f, area, state)`; `App` gains `pub comments: Option<CommentsState>` and `InputPurpose::NewComment { parent_id: String, parent_kind: String, thread_id: Option<String> }`. `c` toggles the panel for the cursor block (Page view, `parent_kind: "block"`) or the page itself when the cursor block has no comments; on a Table/Board row, for that row's page id (`parent_kind: "page"`). Inside the panel: `n` composes a new thread, `r` replies to the selected comment's thread, `Esc`/`c` closes.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/comments_flow.rs`:
```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, CommentRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::page::PageView;

fn store_with_commented_page() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "First".into(), has_children: false,
    }]).unwrap();
    s.replace_comments("p1", &[CommentRec {
        id: "c1".into(), parent_id: "p1".into(), parent_kind: "page".into(),
        thread_id: Some("d1".into()), author: "u1".into(), body: "existing".into(),
        created_time: "t".into(),
    }]).unwrap();
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

fn key(c: char) -> KeyEvent { KeyEvent::from(KeyCode::Char(c)) }

#[test]
fn c_opens_panel_with_page_comments_and_c_closes() {
    let mut app = app_on_page(store_with_commented_page());
    dispatch_key(&mut app, key('c'));
    let panel = app.comments.as_ref().expect("panel open");
    assert_eq!(panel.items.len(), 1);
    assert_eq!(panel.items[0].body, "existing");
    dispatch_key(&mut app, key('c'));
    assert!(app.comments.is_none());
}

#[test]
fn n_composes_a_new_thread_through_the_input_modal() {
    let store = store_with_commented_page();
    let mut app = app_on_page(store.clone());
    dispatch_key(&mut app, key('c'));
    dispatch_key(&mut app, key('n'));
    assert!(app.input.is_some());
    for ch in "my reply".chars() {
        dispatch_key(&mut app, key(ch));
    }
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));

    let s = store.lock().unwrap();
    let comments = s.comments_for("p1").unwrap();
    assert_eq!(comments.len(), 2);
    assert!(comments.iter().any(|c| c.body == "my reply"));
    assert_eq!(s.ops().unwrap()[0].op_type, "create_comment");
    drop(s);
    // Panel refreshed with the new comment:
    assert_eq!(app.comments.as_ref().unwrap().items.len(), 2);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test comments_flow`
Expected: FAIL — `App` has no `comments` field.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/ui/comments.rs`:
```rust
use crossterm::event::{KeyCode, KeyEvent};
use notion_store::CommentRec;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct CommentsState {
    pub parent_id: String,
    pub parent_kind: String,
    pub items: Vec<CommentRec>,
    pub cursor: usize,
}

pub enum CommentsAction {
    None,
    Close,
    NewThread,
    Reply,
}

impl CommentsState {
    pub fn on_key(&mut self, key: KeyEvent) -> CommentsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('c') => CommentsAction::Close,
            KeyCode::Char('n') => CommentsAction::NewThread,
            KeyCode::Char('r') => CommentsAction::Reply,
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.items.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.items.len() - 1);
                }
                CommentsAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                CommentsAction::None
            }
            _ => CommentsAction::None,
        }
    }

    pub fn selected_thread(&self) -> Option<String> {
        self.items.get(self.cursor).and_then(|c| c.thread_id.clone())
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &CommentsState) {
    let items: Vec<ListItem> = state
        .items
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut item = ListItem::new(format!("{}: {}", c.author, c.body));
            if i == state.cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    f.render_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" comments (n new · r reply) ")),
        area,
    );
}
```

`crates/notion-tui/src/ui/mod.rs` — add `pub mod comments;`; in `draw()`, when `app.comments.is_some()`, split the main area to make room for the panel. Replace the `let main_area = *cols.last().unwrap();` line with:
```rust
    let full_main = *cols.last().unwrap();
    let main_area = if let Some(comments_state) = &app.comments {
        let halves = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(36)])
            .split(full_main);
        comments::render(f, halves[1], comments_state);
        halves[0]
    } else {
        full_main
    };
```

`crates/notion-tui/src/app.rs`:
- Add field `pub comments: Option<crate::ui::comments::CommentsState>` (init `None`).
- Extend `InputPurpose`:
```rust
    NewComment { parent_id: String, parent_kind: String, thread_id: Option<String> },
```
- Add method:
```rust
    fn open_comments(&mut self) {
        // Page view: prefer the cursor block's comments if it has any; else page-level.
        // Table/Board: the selected row is itself a page — show its comments.
        let (parent_id, parent_kind) = match &self.view {
            View::Page(v) => {
                let block = v.block_id_at_cursor();
                let page = v.page.id.clone();
                match block {
                    Some(b) if !self.store.lock().unwrap().comments_for(&b).unwrap_or_default().is_empty() => {
                        (b, "block".to_string())
                    }
                    _ => (page, "page".to_string()),
                }
            }
            View::Table(v) => match v.selected_row_id() {
                Some(id) => (id, "page".to_string()),
                None => return,
            },
            View::Board(v) => match v.selected_row_id() {
                Some(id) => (id, "page".to_string()),
                None => return,
            },
            _ => return,
        };
        let items = self.store.lock().unwrap().comments_for(&parent_id).unwrap_or_default();
        self.comments = Some(crate::ui::comments::CommentsState { parent_id, parent_kind, items, cursor: 0 });
        self.request_comment_refresh(); // background; no-op without a RemoteHandle (Task 9)
    }

    pub fn refresh_comments(&mut self) {
        if let Some(panel) = &mut self.comments {
            panel.items = self.store.lock().unwrap().comments_for(&panel.parent_id).unwrap_or_default();
            panel.cursor = panel.cursor.min(panel.items.len().saturating_sub(1));
        }
    }

    fn add_comment(&mut self, parent_id: &str, parent_kind: &str, thread_id: Option<&str>, body: &str) {
        self.store.lock().unwrap().edit_add_comment(parent_id, parent_kind, thread_id, body).ok();
        self.refresh_comments();
    }

    fn request_comment_refresh(&mut self) {}  // replaced with a real impl in Task 9
```
- In `dispatch_key`, route panel keys after the confirm/props/input/search modal checks but before the global openers (the panel is a side panel, not a modal, but its `n`/`r`/`j`/`k` keys take priority while open):
```rust
    if app.comments.is_some() {
        let action = app.comments.as_mut().unwrap().on_key(key);
        match action {
            crate::ui::comments::CommentsAction::None => return,
            crate::ui::comments::CommentsAction::Close => {
                app.comments = None;
                return;
            }
            crate::ui::comments::CommentsAction::NewThread => {
                let panel = app.comments.as_ref().unwrap();
                app.input = Some(InputState::new("new comment", ""));
                app.input_purpose = Some(InputPurpose::NewComment {
                    parent_id: panel.parent_id.clone(),
                    parent_kind: panel.parent_kind.clone(),
                    thread_id: None,
                });
                return;
            }
            crate::ui::comments::CommentsAction::Reply => {
                let panel = app.comments.as_ref().unwrap();
                let thread = panel.selected_thread();
                app.input = Some(InputState::new("reply", ""));
                app.input_purpose = Some(InputPurpose::NewComment {
                    parent_id: panel.parent_id.clone(),
                    parent_kind: panel.parent_kind.clone(),
                    thread_id: thread,
                });
                return;
            }
        }
    }
```
- In the `InputAction::Submit` purpose match, add:
```rust
                        InputPurpose::NewComment { parent_id, parent_kind, thread_id } => {
                            app.add_comment(&parent_id, &parent_kind, thread_id.as_deref(), &text)
                        }
```
- Add the `c` opener in `dispatch_key` before the `handle_key` fallthrough (any main view):
```rust
    if matches!(app.focus, Focus::Main) && key.code == KeyCode::Char('c') {
        app.open_comments();
        return;
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test comments_flow`
Expected: PASS (2 tests). Run `cargo test -p notion-tui` for regressions.

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/ui/comments.rs crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/app.rs crates/notion-tui/tests/comments_flow.rs
git commit -m "feat(notion-tui): comments side panel with new-thread and reply"
```

---

### Task 7: conflict-resolution store methods

**Files:**
- Modify: `crates/notion-store/src/store.rs`
- Test: `crates/notion-store/tests/resolutions.rs` (new)

**Interfaces:**
- Consumes: existing queue API, `edit_update_block_text`.
- Produces:
  - `pub fn retry_op(&self, seq: i64) -> anyhow::Result<()>` — state → `pending`, error cleared.
  - `pub fn resolve_keep_mine(&self, seq: i64) -> anyhow::Result<()>` — `base_edited_time` → NULL, state → `pending`.
  - `pub enum ResolvedTarget { Page(String), Row { row_id: String, data_source_id: String } }`
  - `pub fn resolve_take_theirs(&mut self, seq: i64) -> anyhow::Result<Option<ResolvedTarget>>` — deletes the op; clears the page/row dirty flag if no remaining ops reference it; returns what to refetch (None for comment ops, which have nothing local to revert).
  - `pub fn resolve_conflict_merge(&mut self, seq: i64, block_id: &str, merged_text: &str, remote_edited_time: &str) -> anyhow::Result<()>` — deletes the conflicted op, sets the page's `last_edited_time` to the remote value, then applies `edit_update_block_text(block_id, merged_text)` so the new op's base is the remote time.

- [ ] **Step 1: Write the failing test**

`crates/notion-store/tests/resolutions.rs`:
```rust
use notion_store::{BlockRec, PageRec, ResolvedTarget, Store};

fn store_with_todo() -> (Store, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "local text".into(), has_children: false,
    }]).unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    (s, receipt.op_seq)
}

#[test]
fn retry_resets_failed_op_to_pending() {
    let (s, seq) = store_with_todo();
    s.set_op_state(seq, "failed", Some("boom")).unwrap();
    s.retry_op(seq).unwrap();
    let op = &s.ops().unwrap()[0];
    assert_eq!(op.state, "pending");
    assert!(op.error.is_none());
}

#[test]
fn keep_mine_clears_base_so_push_skips_conflict_check() {
    let (s, seq) = store_with_todo();
    s.set_op_state(seq, "conflicted", None).unwrap();
    s.resolve_keep_mine(seq).unwrap();
    let op = &s.ops().unwrap()[0];
    assert_eq!(op.state, "pending");
    assert!(op.base_edited_time.is_none());
}

#[test]
fn take_theirs_drops_op_clears_dirty_and_names_the_page_to_refetch() {
    let (mut s, seq) = store_with_todo();
    s.set_op_state(seq, "conflicted", None).unwrap();
    let target = s.resolve_take_theirs(seq).unwrap();
    assert!(matches!(target, Some(ResolvedTarget::Page(ref p)) if p == "p1"));
    assert!(s.ops().unwrap().is_empty());
    assert!(!s.is_page_dirty("p1").unwrap());
}

#[test]
fn merge_replaces_conflicted_op_with_fresh_edit_based_on_remote_time() {
    let (mut s, seq) = store_with_todo();
    s.set_op_state(seq, "conflicted", None).unwrap();
    s.resolve_conflict_merge(seq, "b1", "merged text", "t9-remote").unwrap();

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_ne!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "pending");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("t9-remote"));

    let b1 = s.page_blocks("p1").unwrap().into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "merged text");
    assert!(s.is_page_dirty("p1").unwrap());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-store --test resolutions`
Expected: FAIL — `retry_op` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-store/src/store.rs`:
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedTarget {
    Page(String),
    Row { row_id: String, data_source_id: String },
}

impl Store {
    pub fn retry_op(&self, seq: i64) -> anyhow::Result<()> {
        self.set_op_state(seq, "pending", None)
    }

    pub fn resolve_keep_mine(&self, seq: i64) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pending_ops SET base_edited_time = NULL, state = 'pending', error = NULL WHERE seq = ?1",
            [seq],
        )?;
        Ok(())
    }

    pub fn resolve_take_theirs(&mut self, seq: i64) -> anyhow::Result<Option<ResolvedTarget>> {
        let op = self.ops()?.into_iter().find(|o| o.seq == seq);
        let Some(op) = op else { return Ok(None) };
        self.delete_op(seq)?;

        match op.op_type.as_str() {
            "update_block" | "append_block" | "delete_block" | "reorder_block" => {
                let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
                let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
                let others = self.ops()?.iter().any(|o| {
                    let p: Value = serde_json::from_str(&o.payload).unwrap_or_default();
                    p["page_id"].as_str() == Some(page_id.as_str())
                });
                if !others {
                    self.clear_page_dirty(&page_id)?;
                }
                Ok(Some(ResolvedTarget::Page(page_id)))
            }
            "update_row" | "create_row" | "delete_row" | "restore_row" => {
                let row_id = op.target_id.clone();
                let others = self.has_ops_for(&row_id)?;
                if !others {
                    self.clear_row_dirty(&row_id).ok(); // row may be a deleted temp id
                }
                let ds: Option<String> = self
                    .conn
                    .query_row("SELECT data_source_id FROM rows WHERE id = ?1", [&row_id], |r| r.get(0))
                    .ok();
                Ok(ds.map(|data_source_id| ResolvedTarget::Row { row_id, data_source_id }))
            }
            _ => Ok(None),
        }
    }

    pub fn resolve_conflict_merge(
        &mut self,
        seq: i64,
        block_id: &str,
        merged_text: &str,
        remote_edited_time: &str,
    ) -> anyhow::Result<()> {
        let page_id: String =
            self.conn
                .query_row("SELECT page_id FROM blocks WHERE id = ?1", [block_id], |r| r.get(0))?;
        self.delete_op(seq)?;
        // Advance our record of the remote timestamp so the fresh edit's base
        // matches what the server currently has (bypasses the dirty guard on purpose).
        self.conn.execute(
            "UPDATE pages SET last_edited_time = ?2 WHERE id = ?1",
            rusqlite::params![page_id, remote_edited_time],
        )?;
        self.edit_update_block_text(block_id, merged_text)?;
        Ok(())
    }
}
```
Add `ResolvedTarget` to `crates/notion-store/src/lib.rs`'s re-exports.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-store --test resolutions`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-store/src/store.rs crates/notion-store/src/lib.rs crates/notion-store/tests/resolutions.rs
git commit -m "feat(notion-store): conflict resolution primitives (retry, keep-mine, take-theirs, merge)"
```

---

### Task 8: queue/conflicts screen (`Q`)

**Files:**
- Create: `crates/notion-tui/src/ui/queue.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`, `crates/notion-tui/src/app.rs`
- Test: `crates/notion-tui/tests/queue_flow.rs` (new)

**Interfaces:**
- Consumes: `Store::{ops, retry_op, resolve_keep_mine, resolve_take_theirs}` (Task 7), `ResolvedTarget`.
- Produces: `View::Queue(QueueView)` where `pub struct QueueView { pub ops: Vec<OpRec>, pub cursor: usize }`; global key `Q` swaps in/out (previous view preserved in `App.queue_return`); inside: `j`/`k` move, `r` retry, `p` keep mine, `t` take theirs (triggers refetch via `App::request_refetch`, a no-op until Task 9 wires the `RemoteHandle`), `e` request merge (conflicted `update_block` ops only; no-op until Task 9), `Esc`/`Q` back. `status_line` gains a `conflicted: u32` parameter rendered as `· N conflicted`; `App.conflicted: u32` maintained alongside `pending`.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/queue_flow.rs`:
```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};

fn store_with_conflicted_op() -> (notion_sync::SharedStore, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "x".into(), has_children: false,
    }]).unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    s.set_op_state(receipt.op_seq, "conflicted", None).unwrap();
    (Arc::new(Mutex::new(s)), receipt.op_seq)
}

fn shift_q() -> KeyEvent { KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT) }
fn key(c: char) -> KeyEvent { KeyEvent::from(KeyCode::Char(c)) }

#[test]
fn q_opens_queue_listing_ops_and_q_returns_to_previous_view() {
    let (store, _) = store_with_conflicted_op();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("p1");

    dispatch_key(&mut app, shift_q());
    match &app.view {
        View::Queue(q) => {
            assert_eq!(q.ops.len(), 1);
            assert_eq!(q.ops[0].state, "conflicted");
        }
        _ => panic!("expected queue view"),
    }
    dispatch_key(&mut app, shift_q());
    assert!(matches!(app.view, View::Page(_)));
}

#[test]
fn keep_mine_from_queue_clears_base_and_repends_op() {
    let (store, seq) = store_with_conflicted_op();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    dispatch_key(&mut app, shift_q());

    dispatch_key(&mut app, key('p'));

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "pending");
    assert!(ops[0].base_edited_time.is_none());
    // Screen refreshed in place:
    if let View::Queue(q) = &app.view {
        assert_eq!(q.ops[0].state, "pending");
    } else {
        panic!("still on queue view");
    }
}

#[test]
fn take_theirs_from_queue_deletes_op() {
    let (store, _) = store_with_conflicted_op();
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    dispatch_key(&mut app, shift_q());

    dispatch_key(&mut app, key('t'));

    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test queue_flow`
Expected: FAIL — no `View::Queue`.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/ui/queue.rs`:
```rust
use notion_store::OpRec;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct QueueView {
    pub ops: Vec<OpRec>,
    pub cursor: usize,
}

impl QueueView {
    pub fn new(ops: Vec<OpRec>) -> QueueView {
        QueueView { ops, cursor: 0 }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.ops.is_empty() { return; }
        self.cursor = (self.cursor as isize + delta).clamp(0, self.ops.len() as isize - 1) as usize;
    }

    pub fn selected(&self) -> Option<&OpRec> {
        self.ops.get(self.cursor)
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &QueueView, focused: bool) {
    let items: Vec<ListItem> = view
        .ops
        .iter()
        .enumerate()
        .map(|(i, op)| {
            let err = op.error.as_deref().unwrap_or("");
            let mut item = ListItem::new(format!(
                "#{} {} {} [{}] {}",
                op.seq, op.op_type, op.target_id, op.state, err
            ));
            if i == view.cursor && focused {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" queue (r retry · p keep mine · t take theirs · e merge) "),
        ),
        area,
    );
}
```

`crates/notion-tui/src/ui/mod.rs` — add `pub mod queue;`, render `View::Queue(view) => queue::render(f, main_area, view, main_focused),` in `draw()`, and change `status_line`:
```rust
pub fn status_line(status: &SyncStatus, pending: u32, conflicted: u32) -> String {
    let base = match status { /* unchanged match */ };
    let mut out = base;
    if pending > 0 {
        out = format!("{out} · {pending} pending");
    }
    if conflicted > 0 {
        out = format!("{out} · ⚠ {conflicted} conflicted");
    }
    out
}
```
with the `draw()` call site becoming `status_line(&app.sync_status, app.pending, app.conflicted)`. Update the existing inline `status_line` test to pass `0` and add:
```rust
    assert_eq!(
        status_line(&SyncStatus::Idle { updated: 0 }, 1, 2),
        "✓ synced · 1 pending · ⚠ 2 conflicted"
    );
```

`crates/notion-tui/src/app.rs`:
- Add `View::Queue(crate::ui::queue::QueueView)`, fields `pub queue_return: Option<Box<View>>` and `pub conflicted: u32` (init `None`/`0`).
- Add `App` methods:
```rust
    pub fn toggle_queue(&mut self) {
        match std::mem::replace(&mut self.view, View::Empty) {
            View::Queue(_) => {
                self.view = self.queue_return.take().map(|b| *b).unwrap_or(View::Empty);
            }
            other => {
                self.queue_return = Some(Box::new(other));
                let ops = self.store.lock().unwrap().ops().unwrap_or_default();
                self.view = View::Queue(crate::ui::queue::QueueView::new(ops));
            }
        }
    }

    pub fn refresh_conflicted(&mut self) {
        self.conflicted = self
            .store
            .lock()
            .unwrap()
            .ops()
            .unwrap_or_default()
            .iter()
            .filter(|o| o.state == "conflicted")
            .count() as u32;
    }

    fn refresh_queue(&mut self) {
        if let View::Queue(q) = &mut self.view {
            let cursor = q.cursor;
            let ops = self.store.lock().unwrap().ops().unwrap_or_default();
            let mut nq = crate::ui::queue::QueueView::new(ops);
            nq.cursor = cursor.min(nq.ops.len().saturating_sub(1));
            self.view = View::Queue(nq);
        }
        self.refresh_conflicted();
    }

    pub fn request_refetch(&mut self, _target: notion_store::ResolvedTarget) {}  // wired in Task 9
    pub fn request_merge(&mut self, _op: notion_store::OpRec) {}                  // wired in Task 9
```
- In `dispatch_key`, before the `handle_key` fallthrough, add the global toggle and the queue keys:
```rust
    if key.code == KeyCode::Char('Q') {
        app.toggle_queue();
        return;
    }
    if let View::Queue(q) = &mut app.view {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => q.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => q.move_cursor(-1),
            KeyCode::Esc => app.toggle_queue(),
            KeyCode::Char('r') => {
                if let Some(op) = q.selected() {
                    let seq = op.seq;
                    app.store.lock().unwrap().retry_op(seq).ok();
                    app.refresh_queue();
                }
            }
            KeyCode::Char('p') => {
                if let Some(op) = q.selected() {
                    let seq = op.seq;
                    app.store.lock().unwrap().resolve_keep_mine(seq).ok();
                    app.refresh_queue();
                }
            }
            KeyCode::Char('t') => {
                if let Some(op) = q.selected() {
                    let seq = op.seq;
                    let target = app.store.lock().unwrap().resolve_take_theirs(seq).ok().flatten();
                    if let Some(target) = target {
                        app.request_refetch(target);
                    }
                    app.refresh_queue();
                }
            }
            KeyCode::Char('e') => {
                if let Some(op) = q.selected() {
                    if op.state == "conflicted" && op.op_type == "update_block" {
                        let op = op.clone();
                        app.request_merge(op);
                    }
                }
            }
            _ => {}
        }
        return;
    }
```
(Borrow note: the `q.selected()` calls inside arms that then call `app.*` need the seq/op extracted first, exactly as written, because `q` borrows `app.view` — copy the pattern above.)

Also update the two exhaustive `View` matches (`handle_key`'s `d`-target match: `View::Queue(_) => None`; `scroll`: `View::Queue(v) => v.move_cursor(delta)`), `refresh_current_view` (`View::Queue(_) => self.refresh_queue()` — restructure to avoid double borrow by matching on a discriminant first if needed), and update `main.rs`'s pending-watch arm to also call `app.refresh_conflicted();`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test queue_flow`
Expected: PASS (3 tests). Run `cargo test -p notion-tui` for the `status_line` signature change fallout (fix `ui/mod.rs` tests and any snapshot/render tests that call it).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/ui/queue.rs crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/app.rs crates/notion-tui/src/main.rs crates/notion-tui/tests/queue_flow.rs
git commit -m "feat(notion-tui): conflicts/queue screen with retry, keep-mine, take-theirs"
```

---

### Task 9: `RemoteHandle` — background refetch, comment refresh, and merge editor

**Files:**
- Modify: `crates/notion-tui/src/app.rs`, `crates/notion-tui/src/main.rs`
- Test: `crates/notion-tui/tests/merge_flow.rs` (new)

**Interfaces:**
- Consumes: `NotionClient::{fetch_block_tree, query_data_source_all, get_page_edited_time, list_comments}`, `Store::{replace_page_blocks, replace_rows, replace_comments, resolve_conflict_merge}`.
- Produces:
```rust
pub enum AppMsg {
    Refreshed,
    MergeReady { op_seq: i64, block_id: String, local_text: String, remote_text: String, remote_edited_time: String },
}
pub struct RemoteHandle {
    pub client: std::sync::Arc<notion_api::NotionClient>,
    pub tx: tokio::sync::mpsc::UnboundedSender<AppMsg>,
}
```
`App.remote: Option<RemoteHandle>`; real bodies for `request_refetch`, `request_merge` (Task 8 stubs), `request_comment_refresh` (Task 6 stub); `pub fn open_merge_editor(&mut self, msg: AppMsg, run_editor: impl FnOnce(&str) -> anyhow::Result<String>)` which builds the conflict-marker document, runs the editor, and applies `resolve_conflict_merge`. `main.rs` creates the channel, sets `app.remote`, and adds a `select!` arm handling `AppMsg`.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/merge_flow.rs`:
```rust
use std::sync::{Arc, Mutex};

use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{App, AppMsg};

fn store_with_conflicted_op() -> (notion_sync::SharedStore, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false, last_edited_time: "t1".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "orig".into(), has_children: false,
    }]).unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    s.set_op_state(receipt.op_seq, "conflicted", None).unwrap();
    (Arc::new(Mutex::new(s)), receipt.op_seq)
}

#[test]
fn merge_editor_shows_both_versions_and_applies_the_result() {
    let (store, seq) = store_with_conflicted_op();
    let mut app = App::new(store.clone());

    let msg = AppMsg::MergeReady {
        op_seq: seq,
        block_id: "b1".into(),
        local_text: "local edit".into(),
        remote_text: "remote edit".into(),
        remote_edited_time: "t9".into(),
    };
    app.open_merge_editor(msg, |initial| {
        assert!(initial.contains("<<<<<<< local"));
        assert!(initial.contains("local edit"));
        assert!(initial.contains("remote edit"));
        assert!(initial.contains(">>>>>>> remote"));
        Ok("merged result".to_string())
    });

    let s = store.lock().unwrap();
    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 1);
    assert_ne!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "pending");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("t9"));
    let b1 = s.page_blocks("p1").unwrap().into_iter().find(|b| b.id == "b1").unwrap();
    assert_eq!(b1.plain_text, "merged result");
}

#[test]
fn merge_editor_abort_leaves_op_conflicted() {
    let (store, seq) = store_with_conflicted_op();
    let mut app = App::new(store.clone());
    let msg = AppMsg::MergeReady {
        op_seq: seq, block_id: "b1".into(), local_text: "l".into(),
        remote_text: "r".into(), remote_edited_time: "t9".into(),
    };
    app.open_merge_editor(msg, |_| anyhow::bail!("editor aborted"));
    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops[0].seq, seq);
    assert_eq!(ops[0].state, "conflicted");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test merge_flow`
Expected: FAIL — `AppMsg` not found.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/app.rs`:
```rust
pub enum AppMsg {
    Refreshed,
    MergeReady {
        op_seq: i64,
        block_id: String,
        local_text: String,
        remote_text: String,
        remote_edited_time: String,
    },
}

pub struct RemoteHandle {
    pub client: std::sync::Arc<notion_api::NotionClient>,
    pub tx: tokio::sync::mpsc::UnboundedSender<AppMsg>,
}
```
Add `pub remote: Option<RemoteHandle>` to `App` (init `None`). Replace the Task 6/8 stubs:
```rust
    pub fn request_refetch(&mut self, target: notion_store::ResolvedTarget) {
        let Some(remote) = &self.remote else { return };
        let (client, tx, store) = (remote.client.clone(), remote.tx.clone(), self.store.clone());
        tokio::spawn(async move {
            match target {
                notion_store::ResolvedTarget::Page(page_id) => {
                    if let Ok(flat) = client.fetch_block_tree(&page_id).await {
                        let recs: Vec<notion_store::BlockRec> = flat
                            .iter()
                            .map(|f| notion_store::BlockRec {
                                id: f.block.id.clone(),
                                page_id: page_id.clone(),
                                parent_block_id: f.parent_block_id.clone(),
                                ordinal: f.ordinal,
                                block_type: f.block.block_type.clone(),
                                payload: f.block.payload.to_string(),
                                plain_text: f.block.plain_text.clone(),
                                has_children: f.block.has_children,
                            })
                            .collect();
                        store.lock().unwrap().replace_page_blocks(&page_id, &recs).ok();
                    }
                }
                notion_store::ResolvedTarget::Row { data_source_id, .. } => {
                    if let Ok(rows) = client.query_data_source_all(&data_source_id).await {
                        let recs: Vec<notion_store::RowRec> = rows
                            .iter()
                            .map(|r| notion_store::RowRec {
                                id: r.id.clone(),
                                data_source_id: data_source_id.clone(),
                                properties: r.properties.to_string(),
                                last_edited_time: r.last_edited_time.clone(),
                                archived: r.archived,
                            })
                            .collect();
                        store.lock().unwrap().replace_rows(&data_source_id, &recs).ok();
                    }
                }
            }
            tx.send(AppMsg::Refreshed).ok();
        });
    }

    pub fn request_merge(&mut self, op: notion_store::OpRec) {
        let Some(remote) = &self.remote else { return };
        let (client, tx, store) = (remote.client.clone(), remote.tx.clone(), self.store.clone());
        tokio::spawn(async move {
            let payload: serde_json::Value = serde_json::from_str(&op.payload).unwrap_or_default();
            let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
            let block_id = op.target_id.clone();
            let local_text = store
                .lock()
                .unwrap()
                .page_blocks(&page_id)
                .unwrap_or_default()
                .into_iter()
                .find(|b| b.id == block_id)
                .map(|b| b.plain_text)
                .unwrap_or_default();

            let (Ok(remote_time), Ok(flat)) = (
                client.get_page_edited_time(&page_id).await,
                client.fetch_block_tree(&page_id).await,
            ) else {
                return;
            };
            let remote_text = flat
                .iter()
                .find(|f| f.block.id == block_id)
                .map(|f| f.block.plain_text.clone())
                .unwrap_or_default();
            tx.send(AppMsg::MergeReady {
                op_seq: op.seq,
                block_id,
                local_text,
                remote_text,
                remote_edited_time: remote_time,
            })
            .ok();
        });
    }

    fn request_comment_refresh(&mut self) {
        let Some(panel) = &self.comments else { return };
        let Some(remote) = &self.remote else { return };
        let (parent_id, parent_kind) = (panel.parent_id.clone(), panel.parent_kind.clone());
        let (client, tx, store) = (remote.client.clone(), remote.tx.clone(), self.store.clone());
        tokio::spawn(async move {
            if let Ok(comments) = client.list_comments(&parent_id).await {
                let recs: Vec<notion_store::CommentRec> = comments
                    .iter()
                    .map(|c| notion_store::CommentRec {
                        id: c.id.clone(),
                        parent_id: parent_id.clone(),
                        parent_kind: parent_kind.clone(),
                        thread_id: Some(c.discussion_id.clone()),
                        author: c.author.clone(),
                        body: c.body.clone(),
                        created_time: c.created_time.clone(),
                    })
                    .collect();
                store.lock().unwrap().replace_comments(&parent_id, &recs).ok();
                tx.send(AppMsg::Refreshed).ok();
            }
        });
    }

    pub fn open_merge_editor(
        &mut self,
        msg: AppMsg,
        run_editor: impl FnOnce(&str) -> anyhow::Result<String>,
    ) {
        let AppMsg::MergeReady { op_seq, block_id, local_text, remote_text, remote_edited_time } = msg
        else {
            return;
        };
        let doc = format!("<<<<<<< local\n{local_text}\n=======\n{remote_text}\n>>>>>>> remote\n");
        let Ok(merged) = run_editor(&doc) else { return };
        let merged = merged.trim_end_matches('\n').to_string();
        self.store
            .lock()
            .unwrap()
            .resolve_conflict_merge(op_seq, &block_id, &merged, &remote_edited_time)
            .ok();
        self.refresh_queue();
        self.refresh_current_view();
    }
```
(`refresh_queue` was private in Task 8 — make it `pub(crate)` or call it via the existing pattern; keep it private and call it from `open_merge_editor` in the same module, which is fine.)

`crates/notion-tui/src/main.rs`:
```rust
    let client = std::sync::Arc::new(notion_api::NotionClient::new(cfg.token.clone()));
    let sync_client = notion_api::NotionClient::new(cfg.token.clone());
    let mut handle =
        notion_sync::spawn_sync(sync_client, store.clone(), Duration::from_secs(cfg.poll_interval_secs));
    let (app_tx, mut app_rx) = tokio::sync::mpsc::unbounded_channel();
    // ... after App::new:
    app.remote = Some(app::RemoteHandle { client, tx: app_tx });
```
and a new `select!` arm:
```rust
            msg = app_rx.recv() => match msg {
                Some(app::AppMsg::Refreshed) => {
                    app.refresh_current_view();
                    app.refresh_comments();
                }
                Some(m @ app::AppMsg::MergeReady { .. }) => {
                    app.open_merge_editor(m, |initial| {
                        notion_tui::editor::edit_text(&notion_tui::editor::editor_command(), initial)
                    });
                }
                None => {}
            },
```
(Note: `spawn_sync` takes an owned `NotionClient`, so a second client instance is constructed for the interactive handle; both share the same token and pacing rules independently. This doubles the theoretical request ceiling but interactive fetches are rare, user-triggered one-shots.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test merge_flow`
Expected: PASS (2 tests). Run `cargo test` at the workspace root.

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/app.rs crates/notion-tui/src/main.rs crates/notion-tui/tests/merge_flow.rs
git commit -m "feat(notion-tui): background remote fetches for take-theirs, comments, and merge-in-editor"
```

---

### Task 10: end-to-end conflict lifecycle test

**Files:**
- Create: `crates/notion-tui/tests/e2e_conflict.rs`

**Interfaces:**
- Consumes: everything above plus `notion_sync::push_once`.
- Produces: nothing new — integration checkpoint.

- [ ] **Step 1: Write the test**

`crates/notion-tui/tests/e2e_conflict.rs`:
```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_api::NotionClient;
use notion_store::{BlockRec, PageRec, Store};
use notion_sync::push_once;
use notion_tui::app::{dispatch_key, App, Focus, View};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn conflicted_push_surfaces_in_queue_and_keep_mine_pushes_through() {
    let server = MockServer::start().await;
    // Remote page has moved past our base time -> conflict on first push.
    Mock::given(method("GET")).and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "p1", "last_edited_time": "2026-07-06T12:00:00.000Z"})))
        .mount(&server).await;
    Mock::given(method("PATCH")).and(path("/v1/blocks/b1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "b1"})))
        .mount(&server).await;

    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(), parent_type: "workspace".into(), parent_id: None,
        title: "P".into(), icon: None, archived: false,
        last_edited_time: "2026-07-06T10:00:00.000Z".into(),
    }).unwrap();
    s.replace_page_blocks("p1", &[BlockRec {
        id: "b1".into(), page_id: "p1".into(), parent_block_id: None, ordinal: 0,
        block_type: "paragraph".into(), payload: "{}".into(), plain_text: "x".into(), has_children: false,
    }]).unwrap();
    s.edit_update_block_text("b1", "local edit").unwrap();
    let store = Arc::new(Mutex::new(s));

    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));

    // First push: conflict detected, op marked conflicted, nothing pushed.
    assert_eq!(push_once(&client, &store).await.unwrap(), 0);
    assert_eq!(store.lock().unwrap().ops().unwrap()[0].state, "conflicted");

    // Queue screen shows it; keep-mine re-pends it without a base.
    let mut app = App::new(store.clone());
    app.focus = Focus::Main;
    app.refresh_conflicted();
    assert_eq!(app.conflicted, 1);
    dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('Q'), KeyModifiers::SHIFT));
    assert!(matches!(app.view, View::Queue(_)));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));

    // Second push: base is gone, PATCH succeeds, queue drains.
    assert_eq!(push_once(&client, &store).await.unwrap(), 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
    assert!(!store.lock().unwrap().is_page_dirty("p1").unwrap());
}
```

- [ ] **Step 2: Run the test and the full suite**

Run: `cargo test -p notion-tui --test e2e_conflict`
Expected: PASS.
Run: `cargo test`
Expected: PASS workspace-wide.

- [ ] **Step 3: Commit**

```bash
git add crates/notion-tui/tests/e2e_conflict.rs
git commit -m "test(notion-tui): end-to-end conflict detect, surface, keep-mine, drain"
```

---

## Verification (final)

- `cargo test` at the workspace root: all green.
- Manual smoke against the real workspace:
  - Open a database with a status property, press `v` — board renders with the schema's option columns; `J` moves a card and `✓ synced · 1 pending` appears until the pusher drains it; verify the status changed in the Notion app.
  - Press `c` on a page — remote comments appear (after the background refresh lands); `n` + text + Enter posts a comment visible in the Notion app after the next push.
  - Edit a block in the TUI while offline (stop networking), edit the same page in the Notion app, reconnect — status bar shows `⚠ 1 conflicted`; `Q` opens the queue; try each of `r`/`p`/`t`/`e` on separate conflicts and verify the resulting state both locally and remotely.
