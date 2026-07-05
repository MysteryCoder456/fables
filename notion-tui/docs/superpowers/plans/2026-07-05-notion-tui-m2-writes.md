# notion-tui Milestone 2 — Writes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Offline-first writes: a local `pending_ops` queue, a background pusher that drains it to the Notion API with conflict detection, and inline TUI edits (to-do toggle, block text edit/add/delete, row create/delete, property form, undo).

**Architecture:** Every edit is applied to SQLite immediately (optimistic) inside the same transaction that enqueues a `pending_ops` row. The pusher drains ops in `seq` order (FIFO per target), maps them to API calls, and rewrites temporary local ids to real Notion ids on create. The puller never overwrites dirty (locally edited) records. The UI still renders exclusively from the store.

**Tech Stack:** Rust workspace (existing crates `notion-api`, `notion-store`, `notion-sync`, `notion-tui`), rusqlite, tokio, reqwest, ratatui, wiremock + insta for tests.

## Global Constraints

- Notion API version header: `2025-09-03` (already set in `notion-api`).
- Rate limiting/backoff stays inside `notion-api` (~3 req/s pacing; retries on 429/5xx).
- The TUI renders only from `notion-store`; no view awaits the network.
- Failed pushes are never silently dropped: ops get state `failed`/`conflicted` with error text.
- All store mutations transactional.
- TDD: every task = failing test → verify fail → implement → verify pass → commit.
- Run tests from the workspace root `/home/pi/Developer/fables/notion-tui`.
- Commit from the git root `/home/pi/Developer/fables` on branch `notion-tui-m2`; end every commit message with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

## Op vocabulary (used by Tasks 1–7)

`pending_ops` columns already exist (schema v1): `seq, op_type, target_id, payload, base_edited_time, state, error`. States: `pending`, `inflight`, `failed`, `conflicted`. Ops are deleted on successful push.

| op_type | target_id | payload JSON | base_edited_time |
|---|---|---|---|
| `update_block` | block id | `{"block_type": "to_do", "block_payload": {…new payload…}}` | page's `last_edited_time` |
| `append_block` | temp block id (`tmp-…`) | `{"page_id": "...", "parent_id": "...", "after": "...or null", "block": {"type": "paragraph", "paragraph": {…}}}` | null |
| `delete_block` | block id | `{"page_id": "..."}` | page's `last_edited_time` |
| `update_row` | row id | `{"properties": {…partial properties…}}` | row's `last_edited_time` |
| `create_row` | temp row id (`tmp-…`) | `{"data_source_id": "...", "properties": {…}}` | null |
| `delete_row` | row id | `{}` | row's `last_edited_time` |

Temp ids are `format!("tmp-{}", nanos_since_unix_epoch)` — never collide with Notion UUIDs.

---

### Task 1: Store — pending_ops queue API

**Files:**
- Modify: `crates/notion-store/src/store.rs` (append methods + `OpRec` struct)
- Modify: `crates/notion-store/src/lib.rs` (re-export `OpRec`)
- Test: `crates/notion-store/tests/ops_queue.rs` (new)

**Interfaces:**
- Consumes: existing `Store`, `pending_ops` table from schema v1.
- Produces (later tasks rely on these exact signatures):
  - `pub struct OpRec { pub seq: i64, pub op_type: String, pub target_id: String, pub payload: String, pub base_edited_time: Option<String>, pub state: String, pub error: Option<String> }`
  - `Store::enqueue_op(&self, op_type: &str, target_id: &str, payload: &str, base: Option<&str>) -> anyhow::Result<i64>` (returns seq)
  - `Store::ops(&self) -> anyhow::Result<Vec<OpRec>>` (all ops, seq ascending)
  - `Store::set_op_state(&self, seq: i64, state: &str, error: Option<&str>) -> anyhow::Result<()>`
  - `Store::delete_op(&self, seq: i64) -> anyhow::Result<()>`
  - `Store::pending_count(&self) -> anyhow::Result<u32>` (count of ALL rows in pending_ops — anything not yet synced)
  - `Store::has_ops_for(&self, target_id: &str) -> anyhow::Result<bool>`

- [x] **Step 1: Write the failing test**

Create `crates/notion-store/tests/ops_queue.rs`:

```rust
use notion_store::Store;

#[test]
fn enqueue_list_update_delete_roundtrip() {
    let s = Store::open_in_memory().unwrap();
    let seq1 = s.enqueue_op("update_block", "b1", r#"{"x":1}"#, Some("2026-01-01T00:00:00.000Z")).unwrap();
    let seq2 = s.enqueue_op("delete_row", "r1", "{}", None).unwrap();
    assert!(seq2 > seq1);

    let ops = s.ops().unwrap();
    assert_eq!(ops.len(), 2);
    assert_eq!(ops[0].seq, seq1);
    assert_eq!(ops[0].op_type, "update_block");
    assert_eq!(ops[0].target_id, "b1");
    assert_eq!(ops[0].base_edited_time.as_deref(), Some("2026-01-01T00:00:00.000Z"));
    assert_eq!(ops[0].state, "pending");
    assert_eq!(ops[1].base_edited_time, None);

    assert_eq!(s.pending_count().unwrap(), 2);
    assert!(s.has_ops_for("b1").unwrap());
    assert!(!s.has_ops_for("nope").unwrap());

    s.set_op_state(seq1, "failed", Some("validation_error: bad")).unwrap();
    let ops = s.ops().unwrap();
    assert_eq!(ops[0].state, "failed");
    assert_eq!(ops[0].error.as_deref(), Some("validation_error: bad"));

    s.delete_op(seq1).unwrap();
    s.delete_op(seq2).unwrap();
    assert_eq!(s.pending_count().unwrap(), 0);
    assert!(s.ops().unwrap().is_empty());
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-store --test ops_queue`
Expected: compile error — `enqueue_op` not found.

- [x] **Step 3: Write minimal implementation**

Append to `crates/notion-store/src/store.rs` (inside `impl Store`, before the closing brace):

```rust
    pub fn enqueue_op(
        &self,
        op_type: &str,
        target_id: &str,
        payload: &str,
        base: Option<&str>,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_ops (op_type, target_id, payload, base_edited_time)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![op_type, target_id, payload, base],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn ops(&self) -> anyhow::Result<Vec<OpRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, op_type, target_id, payload, base_edited_time, state, error
             FROM pending_ops ORDER BY seq",
        )?;
        let out = stmt
            .query_map([], |r| {
                Ok(OpRec {
                    seq: r.get(0)?,
                    op_type: r.get(1)?,
                    target_id: r.get(2)?,
                    payload: r.get(3)?,
                    base_edited_time: r.get(4)?,
                    state: r.get(5)?,
                    error: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn set_op_state(&self, seq: i64, state: &str, error: Option<&str>) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pending_ops SET state = ?2, error = ?3 WHERE seq = ?1",
            rusqlite::params![seq, state, error],
        )?;
        Ok(())
    }

    pub fn delete_op(&self, seq: i64) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM pending_ops WHERE seq = ?1", [seq])?;
        Ok(())
    }

    pub fn pending_count(&self) -> anyhow::Result<u32> {
        Ok(self.conn.query_row("SELECT count(*) FROM pending_ops", [], |r| r.get(0))?)
    }

    pub fn has_ops_for(&self, target_id: &str) -> anyhow::Result<bool> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM pending_ops WHERE target_id = ?1",
            [target_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
```

Add the struct at the bottom of `store.rs` next to the other record structs:

```rust
#[derive(Debug, Clone)]
pub struct OpRec {
    pub seq: i64,
    pub op_type: String,
    pub target_id: String,
    pub payload: String,
    pub base_edited_time: Option<String>,
    pub state: String,
    pub error: Option<String>,
}
```

In `crates/notion-store/src/lib.rs`, extend the re-export:

```rust
pub use store::{BlockRec, DataSourceRec, NodeKind, OpRec, PageRec, RowRec, SearchHit, Store, TreeNode};
```

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-store --test ops_queue`
Expected: PASS (1 test).

- [x] **Step 5: Commit**

```bash
cd /home/pi/Developer/fables
git add notion-tui/crates/notion-store
git commit -m "feat(notion-store): pending_ops queue API"
```

---

## Tasks 2–12: completed

Tasks 2–12 were carried out with the same TDD cycle (failing test → verify fail → implement → verify pass → commit) directly against the codebase rather than being pre-written here task-by-task. Each is committed separately on `notion-tui-m2`. Summary, with the tests that lock in each interface:

### Task 2 — Store: block edit API + dirty flags + undo receipts
`crates/notion-store/src/store.rs`: `edit_toggle_todo`, `edit_update_block_text`, `edit_insert_block_after`, `edit_delete_block`, `is_page_dirty`/`clear_page_dirty`, `undo`, plus `EditReceipt`/`Inverse` (`ToggleTodo`, `UpdateBlockText`, `DeleteInsertedBlock`, `RecreateBlock`). Test: `crates/notion-store/tests/block_edits.rs` (6 tests).

### Task 3 — Store: row edit API + temp-id rewrite
`edit_create_row`, `edit_update_row`, `edit_delete_row` (soft-delete via `archived`), `is_row_dirty`/`clear_row_dirty`, `rewrite_row_id`, `rewrite_block_id`. `Inverse` gained `DeleteInsertedRow`/`UpdateRowProperties`/`RestoreRow`. Test: `crates/notion-store/tests/row_edits.rs` (5 tests).

### Task 4 — Store: pull protection (dirty guards)
`upsert_page` and `replace_rows` became conditional upserts (`... WHERE dirty = 0`); `notion-sync`'s puller skips `fetch_block_tree`/`replace_page_blocks` when `is_page_dirty`. Tests: `crates/notion-store/tests/pull_protection.rs` (3), `crates/notion-sync/tests/dirty_pull.rs` (1).

### Task 5 — notion-api: write endpoints
`NotionClient::patch_json`/`delete_json`; `update_block`, `delete_block`, `append_children`, `create_page`, `update_page`, `get_block_edited_time`, `get_page_edited_time`. Test: `crates/notion-api/tests/writes.rs` (7 tests).

### Task 6 — notion-sync: pusher success paths
`crates/notion-sync/src/pusher.rs`: `push_once` dispatches each op type, rewrites temp block/row ids to real ids on create, clears dirty flags once no pending op still references the owning page/row. Test: `crates/notion-sync/tests/push.rs` (6 tests).

### Task 7 — notion-sync: conflicts/failed/offline + spawn_sync
Conflict check compares `base_edited_time` against the owning page/row's *current* remote `last_edited_time` (fetched via `get_page_edited_time`) before pushing; mismatch → op `conflicted`, blocking only that op's target. API rejection → op `failed` with reason. Network error aborts the pass, leaving ops `pending`. `spawn_puller` was replaced by `lib.rs::spawn_sync` (push then pull each cycle; `SyncHandle` gained a `pending: watch::Receiver<u32>`). Tests: `crates/notion-sync/tests/push_conflicts.rs` (4), `crates/notion-sync/tests/sync_loop.rs` (1).

### Task 8 — TUI: pending count + Space/dd/u
`ui::status_line` takes a pending count (`✓ synced · N pending`); `App` gained `pending`, `pending_d`, `undo_stack`; `Action::ToggleTodo/DeleteBlock/Undo`. `Space` on a to-do writes through `edit_toggle_todo`; `dd` (two presses) deletes the block under the cursor; `u` undoes the last local edit via `Store::undo`. Test: `crates/notion-tui/tests/edit_flow.rs` (4 tests).

### Task 9 — TUI: input modal, `i` edit text, `a` add block
New `ui/input.rs` (`InputState`/`InputAction`, centered popup). `i` opens it pre-filled with the cursor block's text; `a` opens it empty and inserts a new paragraph after the cursor block. Test: `crates/notion-tui/tests/input_flow.rs` (3 tests).

### Task 10 — TUI: table `o` new row, `dd` delete row
`o` opens the input modal for the title property and calls `edit_create_row`; the `dd` state machine (shared with Task 8) was generalized to also delete the selected row in table view. Test: `crates/notion-tui/tests/table_edit_flow.rs` (2 tests).

### Task 11 — TUI: property form modal (`p`)
New `ui/props.rs` (`PropsState`/`PropsAction`, `build_fields`, `build_property_value` per Notion property type). `p` lists the selected row's properties; `Enter` edits text-like properties inline or toggles checkboxes immediately. Test: `crates/notion-tui/tests/property_form_flow.rs` (4 tests).

### Task 12 — Workspace e2e: edit → queue → push
`crates/notion-tui/tests/e2e_write.rs`: crawls a page via `pull_once`, opens it in the TUI, toggles a to-do through `dispatch_key`, asserts the optimistic write is visible in a rendered frame (`[x] Buy milk`, `1 pending`), then drains the queue via `push_once` and asserts the store ends clean.

## Verification (final)

`cargo test` at the workspace root: all crates pass (notion-api, notion-store, notion-sync, notion-tui), no snapshot regressions.
