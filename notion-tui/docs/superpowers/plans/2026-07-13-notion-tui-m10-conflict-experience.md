# notion-tui M10 — Conflict Experience: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make conflicts legible end-to-end — visible at their source (sidebar/table/board/status-bar), identifiable (who/when/what) in the queue, and resolvable from a detail pane that shows local vs. remote before committing to keep-mine/take-theirs/merge.

**Architecture:** `notion-store` gains two new `pending_ops` columns (`created_at`, set automatically on enqueue; `remote_edited_time`/`remote_edited_by`, set when a conflict is detected) via a schema v2 migration. `notion-api` gains a `get_page_edited_meta` call that also reads `last_edited_by`. `notion-sync`'s pusher captures that remote metadata at the moment it detects a conflict instead of discarding it. `notion-tui` gains: a pure `conflict` module (which ids are conflicted, for markers; a timestamp suffix for queue rows), marker glyphs in sidebar/table/board, a status-bar jump action, and a new modal `ConflictDetailState` (opened by `Enter` on a conflicted queue row) that renders local vs. remote side-by-side with a unified diff and one-line resolution consequences, wired to the existing keep-mine/take-theirs/merge store operations.

**Tech Stack:** Existing: ratatui/crossterm, tokio, reqwest(rustls), rusqlite(bundled), `similar` (already a `notion-tui` dependency — its `unified_diff()` builder is reused, no new dependency). New: none.

## Global Constraints

- Notion API version `2025-09-03` (unchanged).
- The git root is the monorepo `/Users/rehatbir/Developer/fables`; the workspace lives in `notion-tui/`. All `cargo` commands run from `/Users/rehatbir/Developer/fables/notion-tui`.
- Exact copy strings (must appear verbatim where noted): status-bar conflict count `⚠ N conflicts` (renamed from the current `⚠ N conflicted` — see Task 8); resolution consequence line `keep mine — overwrites the remote edit shown right` (spec-mandated, verbatim).
- Never weaken an existing test to make it pass; if a behavior intentionally changes, update the test and say so.
- All quality gates must stay green after every task: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.
- The working tree already contains M6 (schema guard/backup, editor-failure surfacing, fence warnings) — treat it as baseline; do not re-do it.

## Cross-milestone dependency (M8.3)

Tasks 9 and 10 call a helper this plan does **not** implement: M8's Task 4 (plan `2026-07-13-notion-tui-m8-interaction-polish.md`, same directory) produces — this is the reconciled, final contract:

```rust
// crates/notion-tui/src/describe.rs (M8 Task 4)
pub struct OpDescription {
    /// One-line human summary, e.g. "edit ¶ in 'Meeting Notes'" or "set Status on 'Q3 Launch'".
    pub summary: String,
    /// Title of the page / row / comment parent the op targets, when resolvable from the store.
    pub target_title: Option<String>,
    /// The property name (row ops), block-text snippet (block ops), or comment-body snippet.
    pub detail: Option<String>,
}
pub fn describe_op(op: &notion_store::OpRec, store: &notion_store::Store) -> OpDescription;
```

Both call sites (Task 9's queue row line, Task 10's detail-view fetch) are isolated one-liners. **Task 9 and Task 10 must not start until M8 Task 4 has landed** — every other task in this plan is independent of it.

## Execution waves (parallelization map)

- **Wave 1 (two parallel tracks, disjoint crates):**
  - Track STORE: Task 1 (`crates/notion-store`)
  - Track API: Task 2 (`crates/notion-api`)
- **Wave 2 (serial, needs both of Wave 1):** Task 3 (`crates/notion-sync/src/pusher.rs`)
- **Wave 3 (serial chain, all in `crates/notion-tui` — mostly the same files so tasks run one after another):** Task 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 (Tasks 9 and 10 gated on M8.3, see above)
- **Wave 4 (serial):** Task 12 (e2e verification) → Task 13 (integration sweep)

---

### Task 1: notion-store — schema v2: op creation time + remote conflict metadata

**Files:**
- Modify: `crates/notion-store/src/schema.rs`
- Modify: `crates/notion-store/src/store.rs` (`enqueue_op` at line 314, `ops()` at line 329, new `mark_conflicted`, `OpRec` struct at line 1183)

**Interfaces:**
- Consumes: nothing.
- Produces (Task 3 depends on these):
  - `schema::LATEST_VERSION` becomes `2`.
  - `pending_ops` gains `created_at TEXT NOT NULL DEFAULT ''` (populated by `enqueue_op` itself via a SQL `strftime` expression, not a static default — see below), `remote_edited_time TEXT`, `remote_edited_by TEXT`.
  - `OpRec` gains `pub created_at: String`, `pub remote_edited_time: Option<String>`, `pub remote_edited_by: Option<String>`.
  - `pub fn Store::mark_conflicted(&self, seq: i64, remote_edited_time: &str, remote_edited_by: Option<&str>) -> anyhow::Result<()>` — sets `state='conflicted', error=NULL`, plus the two remote columns.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/notion-store/src/schema.rs`:

```rust
    #[test]
    fn fresh_db_lands_on_schema_v2_with_op_metadata_columns() {
        let s = Store::open_in_memory().unwrap();
        let v: i64 = s
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
        // Columns exist and are queryable (would error at prepare-time if missing).
        s.conn()
            .execute(
                "INSERT INTO pending_ops (op_type, target_id, payload, created_at, remote_edited_time, remote_edited_by)
                 VALUES ('update_block', 'b1', '{}', 't', 'rt', 'ru')",
                [],
            )
            .unwrap();
    }

    #[test]
    fn migrates_v1_db_to_v2_preserving_existing_ops_and_backing_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            // Simulate a database left behind by the v1 build.
            use rusqlite::Connection;
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(super::SCHEMA_V1).unwrap();
            conn.execute(
                "INSERT INTO pending_ops (op_type, target_id, payload) VALUES ('update_block', 'b1', '{}')",
                [],
            )
            .unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
        }

        let s = Store::open(&path).unwrap();
        let v: i64 = s
            .conn()
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);

        let (target, created_at): (String, String) = s
            .conn()
            .query_row(
                "SELECT target_id, created_at FROM pending_ops WHERE target_id = 'b1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(target, "b1", "pre-existing op must survive the migration");
        assert_eq!(created_at, "", "pre-migration rows get the column default, not a backfilled timestamp");

        assert!(dir.path().join("n.db.bak-v1").exists(), "must back up before altering the table");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-store schema::tests`
Expected: FAIL — `fresh_db_lands_on_schema_v2...` fails on `assert_eq!(v, 2)` (currently 1) and the INSERT fails (`no such column: created_at`); `migrates_v1_db_to_v2...` fails the same way.

- [ ] **Step 3: Implement the migration**

In `crates/notion-store/src/schema.rs`:

```rust
const SCHEMA_V2: &str = r#"
ALTER TABLE pending_ops ADD COLUMN created_at TEXT NOT NULL DEFAULT '';
ALTER TABLE pending_ops ADD COLUMN remote_edited_time TEXT;
ALTER TABLE pending_ops ADD COLUMN remote_edited_by TEXT;
"#;

pub const LATEST_VERSION: i64 = 2;

pub fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version > LATEST_VERSION {
        anyhow::bail!(
            "this database (schema v{version}) was created by a newer notion-tui; \
             this build supports up to v{LATEST_VERSION} — upgrade notion-tui, \
             or point --db-path at a fresh file"
        );
    }
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    if version < 2 {
        conn.execute_batch(SCHEMA_V2)?;
        conn.pragma_update(None, "user_version", 2)?;
    }
    Ok(())
}
```

Also update the pre-existing `migrates_fresh_db_and_is_idempotent` test (a few lines above in the same file), which currently asserts `assert_eq!(v, 1)` — change to `assert_eq!(v, 2)` (intentional behavior change: LATEST_VERSION grew).

- [ ] **Step 4: `enqueue_op` sets `created_at`; `ops()`/`OpRec` read the new columns; add `mark_conflicted`**

In `crates/notion-store/src/store.rs`:

```rust
    pub fn enqueue_op(
        &self,
        op_type: &str,
        target_id: &str,
        payload: &str,
        base: Option<&str>,
    ) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO pending_ops (op_type, target_id, payload, base_edited_time, created_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            rusqlite::params![op_type, target_id, payload, base],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn ops(&self) -> anyhow::Result<Vec<OpRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, op_type, target_id, payload, base_edited_time, state, error,
                    created_at, remote_edited_time, remote_edited_by
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
                    created_at: r.get(7)?,
                    remote_edited_time: r.get(8)?,
                    remote_edited_by: r.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    /// Marks an op `conflicted` and records the remote state seen at detection
    /// time, so the queue/detail view can show "when and by whom the remote
    /// changed" without a second network round-trip.
    pub fn mark_conflicted(
        &self,
        seq: i64,
        remote_edited_time: &str,
        remote_edited_by: Option<&str>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            "UPDATE pending_ops SET state = 'conflicted', error = NULL,
                                     remote_edited_time = ?2, remote_edited_by = ?3
             WHERE seq = ?1",
            rusqlite::params![seq, remote_edited_time, remote_edited_by],
        )?;
        Ok(())
    }
```

And `OpRec` (near line 1183):

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
    pub created_at: String,
    pub remote_edited_time: Option<String>,
    pub remote_edited_by: Option<String>,
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p notion-store && cargo test --workspace`
Expected: PASS. No other test constructs `OpRec { .. }` literally (confirmed by search — all go through `store.ops()`), so the added fields don't break anything; no test asserts `LATEST_VERSION == 1` except the one just updated.

- [ ] **Step 6: Commit**

```
git add crates/notion-store/src/schema.rs crates/notion-store/src/store.rs
git commit -m "$(cat <<'EOF'
feat(notion-store): schema v2 — op creation time and remote conflict metadata

Adds pending_ops.created_at (set at enqueue time) and remote_edited_time/
remote_edited_by (set when a conflict is detected), needed by M10's conflict
identity and detail views.
EOF
)"
```

---

### Task 2: notion-api — `get_page_edited_meta` (adds `last_edited_by`)

**Files:**
- Modify: `crates/notion-api/src/endpoints.rs` (near `get_page_edited_time` at line 144)
- Test: `crates/notion-api/tests/writes.rs` (extend)

**Interfaces:**
- Consumes: nothing.
- Produces (Task 3 depends on this):
  - `pub struct PageEditMeta { pub last_edited_time: String, pub last_edited_by: Option<String> }` (`#[derive(Debug, Clone, PartialEq)]`), exported from `notion_api`.
  - `pub async fn NotionClient::get_page(&self, page_id: &str) -> Result<Value, ApiError>` — raw page JSON (reused by M10's row-property detail fetch later).
  - `pub async fn NotionClient::get_page_edited_meta(&self, page_id: &str) -> Result<PageEditMeta, ApiError>` — reads `last_edited_time` and `last_edited_by.id` (the latter `None` if absent, matching every existing wiremock fixture that omits it).
  - `get_page_edited_time` is unchanged (still used by `notion-tui`'s legacy merge path — see Task 10).

- [ ] **Step 1: Write the failing test**

Append to `crates/notion-api/tests/writes.rs`:

```rust
#[tokio::test]
async fn get_page_edited_meta_reads_time_and_author_when_present() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T13:00:00.000Z",
            "last_edited_by": {"object": "user", "id": "u-42"}
        })))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());
    let meta = client.get_page_edited_meta("p1").await.unwrap();
    assert_eq!(meta.last_edited_time, "2026-07-05T13:00:00.000Z");
    assert_eq!(meta.last_edited_by.as_deref(), Some("u-42"));
}

#[tokio::test]
async fn get_page_edited_meta_author_is_none_when_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T13:00:00.000Z"
        })))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());
    let meta = client.get_page_edited_meta("p1").await.unwrap();
    assert_eq!(meta.last_edited_by, None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-api get_page_edited_meta`
Expected: FAIL — `get_page_edited_meta` and `PageEditMeta` don't exist yet.

- [ ] **Step 3: Implement**

In `crates/notion-api/src/types.rs`, add near the top (after `ParentRef`):

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct PageEditMeta {
    pub last_edited_time: String,
    pub last_edited_by: Option<String>,
}
```

In `crates/notion-api/src/endpoints.rs`, add next to `get_page_edited_time`:

```rust
    pub async fn get_page(&self, page_id: &str) -> Result<Value, ApiError> {
        self.get_json(&format!("/v1/pages/{page_id}")).await
    }

    pub async fn get_page_edited_meta(&self, page_id: &str) -> Result<PageEditMeta, ApiError> {
        let v = self.get_page(page_id).await?;
        Ok(PageEditMeta {
            last_edited_time: v["last_edited_time"].as_str().unwrap_or_default().to_string(),
            last_edited_by: v["last_edited_by"]["id"].as_str().map(str::to_string),
        })
    }
```

Add `PageEditMeta` to the `use crate::types::{...}` import list at the top of `endpoints.rs`, and export it from `crates/notion-api/src/lib.rs` (check the existing `pub use types::{...}` line and add `PageEditMeta` to it).

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-api && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-api/src/types.rs crates/notion-api/src/endpoints.rs crates/notion-api/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(notion-api): get_page_edited_meta — capture last_edited_by alongside the timestamp

Needed so a detected conflict can record who changed the remote side, not
just when, for M10's conflict identity view.
EOF
)"
```

---

### Task 3: notion-sync — pusher captures remote conflict metadata

**Files:**
- Modify: `crates/notion-sync/src/pusher.rs` (`PushOutcome` at line 30, the five conflict-check call sites at lines 87, 118, 307, 356, 380, and the `Ok(PushOutcome::Conflicted)` arm in `push_once` at line 519)
- Test: `crates/notion-sync/tests/push_conflicts.rs` (extend)

**Interfaces:**
- Consumes: Task 1's `Store::mark_conflicted`; Task 2's `get_page_edited_meta`/`PageEditMeta`.
- Produces (Task 10 depends on the data now being present on conflicted `OpRec`s):
  - `PushOutcome::Conflicted { remote_edited_time: String, remote_edited_by: Option<String> }` (was a unit variant).
  - Every one of the five conflict-detection call sites switches from `client.get_page_edited_time(...)` to `client.get_page_edited_meta(...)`.
  - `push_once` calls `store.mark_conflicted(op.seq, &remote_edited_time, remote_edited_by.as_deref())` instead of `set_op_state(op.seq, "conflicted", None)`.

- [ ] **Step 1: Write the failing test**

Append to `crates/notion-sync/tests/push_conflicts.rs`:

```rust
#[tokio::test]
async fn conflict_records_remote_edited_time_and_author() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T12:00:00.000Z",
            "last_edited_by": {"object": "user", "id": "u-9"}
        })))
        .mount(&server)
        .await;

    let pushed = push_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(pushed, 0);

    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops[0].state, "conflicted");
    assert_eq!(ops[0].remote_edited_time.as_deref(), Some("2026-07-05T12:00:00.000Z"));
    assert_eq!(ops[0].remote_edited_by.as_deref(), Some("u-9"));
}

#[tokio::test]
async fn conflict_author_is_none_when_the_api_does_not_provide_one() {
    let store = page_store_with_todo();
    store.lock().unwrap().edit_toggle_todo("b1").unwrap();

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/pages/p1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "page", "id": "p1", "last_edited_time": "2026-07-05T12:00:00.000Z"
        })))
        .mount(&server)
        .await;

    push_once(&fast_client(server.uri()), &store).await.unwrap();
    let ops = store.lock().unwrap().ops().unwrap();
    assert_eq!(ops[0].remote_edited_by, None);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-sync --test push_conflicts remote_edited`
Expected: FAIL — `ops[0].remote_edited_time`/`remote_edited_by` are always `None` (the pusher never writes them today).

- [ ] **Step 3: Implement**

In `crates/notion-sync/src/pusher.rs`, change the `PushOutcome` enum:

```rust
enum PushOutcome {
    Success,
    Conflicted { remote_edited_time: String, remote_edited_by: Option<String> },
    Failed(String),
}
```

In each of the five conflict-check blocks (`push_update_block`, `push_delete_block`, `push_update_row`, `push_delete_row`, `push_restore_row`), replace:

```rust
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&page_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
```

with:

```rust
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let meta = client.get_page_edited_meta(&page_id).await?;
            if meta.last_edited_time > *base {
                return Ok(PushOutcome::Conflicted {
                    remote_edited_time: meta.last_edited_time,
                    remote_edited_by: meta.last_edited_by,
                });
            }
        }
    }
```

`push_update_row`, `push_delete_row`, and `push_restore_row` each open with the identical six-line conflict check, keyed on `op.target_id` (a row id) instead of a `page_id` derived from the payload. In each of the three, replace:

```rust
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let current = client.get_page_edited_time(&op.target_id).await?;
            if current > *base {
                return Ok(PushOutcome::Conflicted);
            }
        }
    }
```

with:

```rust
    if let Some(base) = &op.base_edited_time {
        if !base.is_empty() {
            let meta = client.get_page_edited_meta(&op.target_id).await?;
            if meta.last_edited_time > *base {
                return Ok(PushOutcome::Conflicted {
                    remote_edited_time: meta.last_edited_time,
                    remote_edited_by: meta.last_edited_by,
                });
            }
        }
    }
```

In `push_once`'s match on `push_one(...)`'s result:

```rust
            Ok(PushOutcome::Conflicted { remote_edited_time, remote_edited_by }) => {
                lock_store(store)
                    .mark_conflicted(op.seq, &remote_edited_time, remote_edited_by.as_deref())
                    .ok();
                blocked.insert(op.target_id.clone());
            }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-sync && cargo test --workspace`
Expected: PASS, including the pre-existing `push_conflicts.rs` tests (they only assert `state == "conflicted"`/`is_page_dirty`, unaffected by the new fields).

- [ ] **Step 5: Commit**

```
git add crates/notion-sync/src/pusher.rs
git commit -m "$(cat <<'EOF'
feat(notion-sync): pusher records remote edited time/author when a conflict is detected

Threads notion-api's get_page_edited_meta and notion-store's mark_conflicted
through every conflict-detection call site, so the data M10's conflict UI
needs is captured at detection time instead of being discarded.
EOF
)"
```

---

### Task 4: notion-tui — pure conflict-id and conflict-suffix helpers

**Files:**
- Create: `crates/notion-tui/src/conflict.rs`
- Modify: `crates/notion-tui/src/lib.rs` (add `pub mod conflict;`)

**Interfaces:**
- Consumes: `notion_store::OpRec` (Task 1's new fields).
- Produces (Tasks 5–9 depend on these):
  - `pub fn conflicted_target_ids(ops: &[OpRec]) -> HashSet<String>` — for `update_block`/`append_block`/`delete_block`/`reorder_block` ops, extracts `payload["page_id"]`; for `update_row`/`create_row`/`delete_row`/`restore_row` ops, uses `op.target_id`. Only `state == "conflicted"` ops contribute.
  - `pub fn conflict_suffix(op: &OpRec) -> Option<String>` — `None` unless `op.state == "conflicted"`; otherwise `" · yours {created_at}"`, plus `" · theirs {remote_edited_time}"` and `" by {remote_edited_by}"` when those are present.

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-tui/src/conflict.rs`:

```rust
use std::collections::HashSet;

use notion_store::OpRec;
use serde_json::Value;

/// Ids (page ids for block-type ops, row ids for row-type ops) that currently
/// have at least one `state == "conflicted"` op, so the sidebar/table/board
/// can mark the affected item at its source.
pub fn conflicted_target_ids(ops: &[OpRec]) -> HashSet<String> {
    ops.iter()
        .filter(|o| o.state == "conflicted")
        .filter_map(|o| match o.op_type.as_str() {
            "update_block" | "append_block" | "delete_block" | "reorder_block" => {
                let v: Value = serde_json::from_str(&o.payload).ok()?;
                v["page_id"].as_str().map(str::to_string)
            }
            "update_row" | "create_row" | "delete_row" | "restore_row" => Some(o.target_id.clone()),
            _ => None,
        })
        .collect()
}

/// A one-line suffix giving conflict identity: when the local edit was
/// queued and, if known, when/by whom the remote side changed. `None` for
/// non-conflicted ops (nothing to append).
pub fn conflict_suffix(op: &OpRec) -> Option<String> {
    if op.state != "conflicted" {
        return None;
    }
    let mut s = format!(" · yours {}", op.created_at);
    if let Some(remote_time) = &op.remote_edited_time {
        s.push_str(&format!(" · theirs {remote_time}"));
        if let Some(by) = &op.remote_edited_by {
            s.push_str(&format!(" by {by}"));
        }
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(op_type: &str, target_id: &str, payload: &str, state: &str) -> OpRec {
        OpRec {
            seq: 1,
            op_type: op_type.into(),
            target_id: target_id.into(),
            payload: payload.into(),
            base_edited_time: None,
            state: state.into(),
            error: None,
            created_at: "2026-07-13T10:00:00.000Z".into(),
            remote_edited_time: None,
            remote_edited_by: None,
        }
    }

    #[test]
    fn block_op_contributes_its_page_id() {
        let ops = vec![op("update_block", "b1", r#"{"page_id":"p1"}"#, "conflicted")];
        assert_eq!(conflicted_target_ids(&ops), HashSet::from(["p1".to_string()]));
    }

    #[test]
    fn row_op_contributes_its_own_target_id() {
        let ops = vec![op("update_row", "r1", "{}", "conflicted")];
        assert_eq!(conflicted_target_ids(&ops), HashSet::from(["r1".to_string()]));
    }

    #[test]
    fn non_conflicted_ops_are_excluded() {
        let ops = vec![op("update_block", "b1", r#"{"page_id":"p1"}"#, "pending")];
        assert!(conflicted_target_ids(&ops).is_empty());
    }

    #[test]
    fn suffix_is_none_when_not_conflicted() {
        let o = op("update_block", "b1", "{}", "pending");
        assert_eq!(conflict_suffix(&o), None);
    }

    #[test]
    fn suffix_includes_remote_time_and_author_when_present() {
        let mut o = op("update_block", "b1", "{}", "conflicted");
        o.remote_edited_time = Some("2026-07-05T12:00:00.000Z".into());
        o.remote_edited_by = Some("u-9".into());
        assert_eq!(
            conflict_suffix(&o).as_deref(),
            Some(" · yours 2026-07-13T10:00:00.000Z · theirs 2026-07-05T12:00:00.000Z by u-9")
        );
    }

    #[test]
    fn suffix_omits_remote_fields_when_unknown() {
        let o = op("update_block", "b1", "{}", "conflicted");
        assert_eq!(conflict_suffix(&o).as_deref(), Some(" · yours 2026-07-13T10:00:00.000Z"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui conflict::`
Expected: FAIL to compile — `crate::conflict` module doesn't exist yet / isn't declared.

- [ ] **Step 3: Wire the module**

In `crates/notion-tui/src/lib.rs`, add `pub mod conflict;` alongside the other `pub mod` declarations.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui conflict:: && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/conflict.rs crates/notion-tui/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): pure helpers for conflict ids and queue-row timestamps

conflicted_target_ids drives the new sidebar/table/board markers;
conflict_suffix appends "yours <time> · theirs <time> by <who>" to a
conflicted queue row. Both are plain functions over OpRec, independently
testable ahead of the UI wiring.
EOF
)"
```

---

### Task 5: notion-tui — sidebar conflict marker

**Files:**
- Modify: `crates/notion-tui/src/ui/sidebar.rs` (`SidebarState` at line 16, `render` at line 92)
- Modify: `crates/notion-tui/src/app.rs` (`refresh_conflicted` at line 493)
- Test: `crates/notion-tui/tests/queue_flow.rs` (extend) or inline in `sidebar.rs`

**Interfaces:**
- Consumes: Task 4's `conflicted_target_ids`.
- Produces: `SidebarState.conflicted_ids: HashSet<String>`; `App::refresh_conflicted` populates it every time it recomputes the count.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/notion-tui/src/ui/sidebar.rs`:

```rust
    #[test]
    fn render_marks_a_conflicted_page_with_a_warning_glyph() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut s = SidebarState::new(nodes()); // "Alpha", "Alpha child", "Beta", "Tasks"
        s.conflicted_ids.insert("b".to_string()); // "Beta"'s id
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(30, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut s, true, &theme)).unwrap();
        let rendered = format!("{:?}", term.backend().buffer());
        assert!(rendered.contains("⚠ ▸ Beta"), "rendered:\n{rendered}");
        assert!(!rendered.contains("⚠ ▸ Alpha"), "only Beta should be marked:\n{rendered}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui sidebar::tests::render_marks_a_conflicted_page`
Expected: FAIL to compile — `SidebarState` has no `conflicted_ids` field.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/ui/sidebar.rs`, add the field:

```rust
pub struct SidebarState {
    pub nodes: Vec<TreeNode>,
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub hidden: bool,
    pub list_state: ListState,
    pub conflicted_ids: HashSet<String>,
}
```

and initialize it in `SidebarState::new`:

```rust
            conflicted_ids: HashSet::new(),
```

In `render`, prefix the marker:

```rust
    let items: Vec<ListItem> = state
        .visible()
        .iter()
        .map(|v| {
            let marker = match v.node.kind {
                NodeKind::Page => "▸ ",
                NodeKind::DataSource => "▦ ",
            };
            let warn = if state.conflicted_ids.contains(&v.node.id) { "⚠ " } else { "" };
            let line = format!("{}{}{}{}", "  ".repeat(v.depth), warn, marker, v.node.title);
            ListItem::new(Line::from(line))
        })
        .collect();
```

In `crates/notion-tui/src/app.rs`, extend `refresh_conflicted`:

```rust
    pub fn refresh_conflicted(&mut self) {
        let ops = self.store.lock().unwrap().ops().unwrap_or_default();
        self.conflicted = ops.iter().filter(|o| o.state == "conflicted").count() as u32;
        self.sidebar.conflicted_ids = crate::conflict::conflicted_target_ids(&ops);
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/ui/sidebar.rs crates/notion-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): mark conflicted pages in the sidebar

App::refresh_conflicted now populates SidebarState.conflicted_ids from the
pure conflicted_target_ids helper; the sidebar prefixes a "⚠ " glyph.
EOF
)"
```

---

### Task 6: notion-tui — table row conflict marker

**Files:**
- Modify: `crates/notion-tui/src/ui/table.rs` (`TableView` at line 16, `render` at line 195)
- Modify: `crates/notion-tui/src/app.rs` (`open_table` at line 645, `refresh_conflicted`)
- Test: inline in `table.rs`

**Interfaces:**
- Consumes: Task 4's `conflicted_target_ids`; Task 5's `SidebarState.conflicted_ids` pattern (same shape).
- Produces: `TableView.conflicted_ids: HashSet<String>` + `pub fn set_conflicted(&mut self, ids: HashSet<String>)`; `App::refresh_conflicted` and `App::open_table` apply it.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/notion-tui/src/ui/table.rs`:

```rust
    #[test]
    fn render_marks_a_conflicted_row_with_a_warning_glyph() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut v = TableView::new(ds(), vec![row("r1", "Buy milk", true, "High")]);
        v.set_conflicted(std::collections::HashSet::from(["r1".to_string()]));
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(60, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut v, true, &theme)).unwrap();
        let rendered = format!("{:?}", term.backend().buffer());
        assert!(rendered.contains("⚠ Buy milk"), "rendered:\n{rendered}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui table::tests::render_marks_a_conflicted_row`
Expected: FAIL to compile — no `set_conflicted` method.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/ui/table.rs`:

```rust
use std::collections::HashSet;

pub struct TableView {
    pub ds: DataSourceRec,
    pub columns: Vec<Column>,
    pub rows: Vec<RowRec>,
    pub cursor: usize,
    pub sort: Option<(usize, bool)>,
    pub sort_col: usize,
    pub table_state: TableState,
    pub conflicted_ids: HashSet<String>,
}
```

Initialize `conflicted_ids: HashSet::new()` in `TableView::new`, and add:

```rust
    pub fn set_conflicted(&mut self, ids: HashSet<String>) {
        self.conflicted_ids = ids;
    }
```

In `render`, prefix the title-column cell:

```rust
    let rows: Vec<TRow> = view
        .rows
        .iter()
        .map(|r| {
            let mut cells: Vec<String> = view.columns.iter().map(|c| view.cell(r, c)).collect();
            if view.conflicted_ids.contains(&r.id) {
                if let Some(first) = cells.first_mut() {
                    *first = format!("⚠ {first}");
                }
            }
            TRow::new(cells)
        })
        .collect();
```

In `crates/notion-tui/src/app.rs`:

```rust
    pub fn refresh_conflicted(&mut self) {
        let ops = self.store.lock().unwrap().ops().unwrap_or_default();
        self.conflicted = ops.iter().filter(|o| o.state == "conflicted").count() as u32;
        let ids = crate::conflict::conflicted_target_ids(&ops);
        self.sidebar.conflicted_ids = ids.clone();
        if let View::Table(v) = &mut self.view {
            v.set_conflicted(ids);
        }
    }
```

and in `open_table`, apply marks to the freshly built view (so opening a table with existing conflicts shows them immediately, not just after the next `refresh_conflicted`):

```rust
    fn open_table(&mut self, data_source_id: &str) {
        let guard = self.store.lock().unwrap();
        let ds = guard.get_data_source(data_source_id).ok().flatten();
        let rows = guard.rows(data_source_id).unwrap_or_default();
        let ops = guard.ops().unwrap_or_default();
        drop(guard);
        match ds {
            Some(ds) => {
                let mut view = TableView::new(ds, rows);
                view.set_conflicted(crate::conflict::conflicted_target_ids(&ops));
                self.view = View::Table(view);
            }
            None => self.notice = Some(format!("database not found (not synced yet?): {data_source_id}")),
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/ui/table.rs crates/notion-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): mark conflicted rows in the table view
EOF
)"
```

---

### Task 7: notion-tui — board card conflict marker

**Files:**
- Modify: `crates/notion-tui/src/ui/board.rs` (`BoardView` at line 10, `render` at line 114)
- Modify: `crates/notion-tui/src/app.rs` (`toggle_board` at line 439, `refresh_conflicted`)
- Test: inline in `board.rs`

**Interfaces:**
- Consumes: Task 4's `conflicted_target_ids`; same `set_conflicted` shape as Task 6.
- Produces: `BoardView.conflicted_ids: HashSet<String>` + `set_conflicted`; `App::refresh_conflicted`/`toggle_board` apply it.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/notion-tui/src/ui/board.rs`:

```rust
    #[test]
    fn render_marks_a_conflicted_card_with_a_warning_glyph() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut v = BoardView::new(ds(), vec![row("r1", "A", Some("Todo"))]);
        v.set_conflicted(std::collections::HashSet::from(["r1".to_string()]));
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(60, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut v, true, &theme)).unwrap();
        let rendered = format!("{:?}", term.backend().buffer());
        assert!(rendered.contains("⚠ A"), "rendered:\n{rendered}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui board::tests::render_marks_a_conflicted_card`
Expected: FAIL to compile — no `set_conflicted` method.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/ui/board.rs`:

```rust
use std::collections::HashSet;

pub struct BoardView {
    pub ds: DataSourceRec,
    pub group_prop: String,
    pub group_type: String,
    pub columns: Vec<String>,
    pub rows: Vec<RowRec>,
    pub col: usize,
    pub card: usize,
    pub list_state: ListState,
    pub conflicted_ids: HashSet<String>,
}
```

Initialize `conflicted_ids: HashSet::new()` in `BoardView::new`, and add:

```rust
    pub fn set_conflicted(&mut self, ids: HashSet<String>) {
        self.conflicted_ids = ids;
    }
```

In `render`, where the card title is built:

```rust
                let title = props
                    .as_object()
                    .and_then(|m| m.values().find(|p| p["type"] == "title"))
                    .map(cell_text)
                    .unwrap_or_default();
                let title = if view.conflicted_ids.contains(&r.id) {
                    format!("⚠ {title}")
                } else {
                    title
                };
                ListItem::new(title)
```

In `crates/notion-tui/src/app.rs`, extend `refresh_conflicted` once more:

```rust
    pub fn refresh_conflicted(&mut self) {
        let ops = self.store.lock().unwrap().ops().unwrap_or_default();
        self.conflicted = ops.iter().filter(|o| o.state == "conflicted").count() as u32;
        let ids = crate::conflict::conflicted_target_ids(&ops);
        self.sidebar.conflicted_ids = ids.clone();
        match &mut self.view {
            View::Table(v) => v.set_conflicted(ids),
            View::Board(v) => v.set_conflicted(ids),
            _ => {}
        }
    }
```

and in `toggle_board`, apply marks right after constructing a fresh `BoardView` (mirrors Task 6's `open_table` change):

```rust
    pub fn toggle_board(&mut self) {
        match std::mem::replace(&mut self.view, View::Empty) {
            View::Table(t) => {
                if crate::ui::board::group_property(&t.ds.schema_json).is_some() {
                    let ops = self.store.lock().unwrap().ops().unwrap_or_default();
                    let mut b = BoardView::new(t.ds, t.rows);
                    b.set_conflicted(crate::conflict::conflicted_target_ids(&ops));
                    self.view = View::Board(b);
                } else {
                    self.notice = Some("board view needs a status or select property".into());
                    self.view = View::Table(t);
                }
            }
            View::Board(b) => {
                self.view = View::Table(crate::ui::table::TableView::new(b.ds, b.rows));
            }
            other => {
                self.notice = Some("no table open to show as a board".into());
                self.view = other;
            }
        }
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/ui/board.rs crates/notion-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): mark conflicted cards in the board view
EOF
)"
```

---

### Task 8: notion-tui — status-bar jump action ("conflicts")

**Files:**
- Modify: `crates/notion-tui/src/ui/mod.rs` (`status_line` at line 22 — rename `conflicted` copy to `conflicts`)
- Modify: `crates/notion-tui/src/keymap.rs` (`DEFAULTS` at line 5, `actions()` at line 90)
- Modify: `crates/notion-tui/src/ui/palette.rs` (`COMMANDS` at line 7)
- Modify: `crates/notion-tui/src/app.rs` (`run_command` at line 459, new `jump_to_first_conflict`)
- Test: `crates/notion-tui/tests/queue_flow.rs` (extend), inline in `ui/mod.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces: keymap action `"conflicts"` (default key `C`); palette command `"conflicts"`; `App::jump_to_first_conflict(&mut self)` — opens the queue (if not already open) and selects the first `state == "conflicted"` row.

- [ ] **Step 1: Write the failing tests**

In `crates/notion-tui/src/ui/mod.rs`, update the existing test (intentional copy change) and it will fail until Step 3:

```rust
    #[test]
    fn status_line_appends_conflicted_count() {
        assert_eq!(
            status_line(&SyncStatus::Idle { updated: 0 }, 1, 2, None, None),
            "✓ synced · 1 pending · ⚠ 2 conflicts"
        );
    }
```

Append to `crates/notion-tui/tests/queue_flow.rs`:

```rust
fn store_with_two_ops_second_conflicted() -> (notion_sync::SharedStore, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[
            BlockRec {
                id: "b1".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 0,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "x".into(),
                has_children: false,
            },
            BlockRec {
                id: "b2".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 1,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "y".into(),
                has_children: false,
            },
        ],
    )
    .unwrap();
    s.edit_update_block_text("b1", "local edit 1").unwrap(); // stays pending
    let receipt2 = s.edit_update_block_text("b2", "local edit 2").unwrap();
    s.set_op_state(receipt2.op_seq, "conflicted", None).unwrap();
    (Arc::new(Mutex::new(s)), receipt2.op_seq)
}

#[test]
fn conflicts_key_jumps_straight_to_the_first_conflicted_op() {
    let (store, seq) = store_with_two_ops_second_conflicted();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    app.open_page("p1");

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('C')));

    match &app.view {
        View::Queue(q) => {
            assert_eq!(q.ops[q.cursor].seq, seq);
            assert_eq!(q.ops[q.cursor].state, "conflicted");
        }
        _ => panic!("expected queue view"),
    }
}

#[test]
fn conflicts_palette_command_does_the_same_jump() {
    let (store, seq) = store_with_two_ops_second_conflicted();
    let mut app = App::new(store);
    app.run_command("conflicts");
    match &app.view {
        View::Queue(q) => assert_eq!(q.ops[q.cursor].seq, seq),
        _ => panic!("expected queue view"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui status_line_appends_conflicted_count conflicts_key_jumps conflicts_palette_command`
Expected: FAIL — status line still says "conflicted"; `'C'` isn't bound to anything; `run_command("conflicts")` is a no-op.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/ui/mod.rs`:

```rust
    if conflicted > 0 {
        out = format!("{out} · ⚠ {conflicted} conflicts");
    }
```

In `crates/notion-tui/src/keymap.rs`, add to `DEFAULTS`:

```rust
    ("conflicts", "jump to first conflict", KeyCode::Char('C')),
```

and the matching entry in `actions()`:

```rust
            ("conflicts", "jump to first conflict"),
```

In `crates/notion-tui/src/ui/palette.rs`:

```rust
pub const COMMANDS: &[&str] = &["help", "queue", "conflicts", "board", "table", "quit"];
```

In `crates/notion-tui/src/app.rs`, extend `run_command` and add the new method:

```rust
    pub fn run_command(&mut self, cmd: &str) {
        match cmd {
            "help" => self.help_open = true,
            "queue" => {
                if !matches!(self.view, View::Queue(_)) {
                    self.toggle_queue();
                }
            }
            "conflicts" => self.jump_to_first_conflict(),
            "board" | "table" => {
                let want_board = cmd == "board";
                let is_board = matches!(self.view, View::Board(_));
                let is_table = matches!(self.view, View::Table(_));
                if (want_board && is_table) || (!want_board && is_board) {
                    self.toggle_board();
                }
            }
            "quit" => self.should_quit = true,
            _ => {}
        }
    }

    /// Opens the queue (if not already open) and selects the first conflicted
    /// op, so the status-bar `⚠ N conflicts` count is one keypress from the
    /// exact row that needs attention.
    pub fn jump_to_first_conflict(&mut self) {
        if !matches!(self.view, View::Queue(_)) {
            self.toggle_queue();
        }
        if let View::Queue(q) = &mut self.view {
            if let Some(idx) = q.ops.iter().position(|o| o.state == "conflicted") {
                q.cursor = idx;
            }
        }
    }
```

Wire the key in `dispatch_key`, next to the existing `km.is("queue", key)` check:

```rust
    if km.is("conflicts", key) {
        app.jump_to_first_conflict();
        return;
    }
    if km.is("queue", key) {
        app.toggle_queue();
        return;
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/keymap.rs crates/notion-tui/src/ui/palette.rs crates/notion-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): "conflicts" key/palette command jumps to the first conflict

Also renames the status-bar copy from "N conflicted" to "N conflicts" to
match the M10 spec's exact wording.
EOF
)"
```

---

### Task 9: notion-tui — queue rows show conflict identity timestamps

*(Gated on M8.3 having landed — see "Cross-milestone dependency" above. M8.3 is expected to have already switched `queue.rs`'s row text to `describe_op(op, store).summary`; this task only appends the timestamp suffix and does not otherwise touch the row-text format.)*

**Files:**
- Modify: `crates/notion-tui/src/ui/queue.rs` (`render` at line 36)
- Test: inline in `queue.rs`

**Interfaces:**
- Consumes: Task 4's `conflict_suffix`; M8 Task 4's `describe_op`/`OpDescription` (at `crate::describe::describe_op`) for the base row text — this task does not call `describe_op` directly, it only appends after whatever text M8 already produces.
- Produces: every conflicted row's line ends with `conflict_suffix(op)`'s text.

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/notion-tui/src/ui/queue.rs` (add one if none exists yet):

```rust
#[cfg(test)]
mod conflict_suffix_tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn conflicted_op() -> notion_store::OpRec {
        notion_store::OpRec {
            seq: 1,
            op_type: "update_block".into(),
            target_id: "b1".into(),
            payload: r#"{"page_id":"p1"}"#.into(),
            base_edited_time: None,
            state: "conflicted".into(),
            error: None,
            created_at: "2026-07-13T10:00:00.000Z".into(),
            remote_edited_time: Some("2026-07-05T12:00:00.000Z".into()),
            remote_edited_by: Some("u-9".into()),
        }
    }

    #[test]
    fn conflicted_row_shows_yours_and_theirs_timestamps() {
        let mut view = QueueView::new(vec![conflicted_op()]);
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(100, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut view, true, &theme)).unwrap();
        let rendered = format!("{:?}", term.backend().buffer());
        assert!(rendered.contains("yours 2026-07-13T10:00:00.000Z"), "rendered:\n{rendered}");
        assert!(rendered.contains("theirs 2026-07-05T12:00:00.000Z by u-9"), "rendered:\n{rendered}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui queue::conflict_suffix_tests`
Expected: FAIL — the suffix isn't appended yet.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/ui/queue.rs`, extend the row-building closure (keep whatever base line M8.3 left in place — this only appends):

```rust
    let items: Vec<ListItem> = view
        .ops
        .iter()
        .map(|op| {
            let err = op.error.as_deref().unwrap_or("");
            let mut line = format!(
                "#{} {} {} [{}] {}",
                op.seq, op.op_type, op.target_id, op.state, err
            );
            if let Some(suffix) = crate::conflict::conflict_suffix(op) {
                line.push_str(&suffix);
            }
            ListItem::new(line)
        })
        .collect();
```

(If M8 Task 4 already replaced the `format!("#{} {} {} [{}] {}", ...)` line with the `describe_op(op, store)` summary — M8's Task 4 changes `QueueView::new` to take `(ops: Vec<OpRec>, summaries: Vec<String>)` — apply the same `if let Some(suffix) = ... { line.push_str(&suffix); }` pattern to whatever variable holds that summary string instead — the append is the only part this task owns.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/ui/queue.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): queue rows show conflict identity timestamps

Conflicted rows now append "yours <local edit time> · theirs <remote time>
by <remote author>" using the pure conflict_suffix helper.
EOF
)"
```

---

### Task 10: notion-tui — conflict detail view (data + fetch)

*(Gated on M8.3 having landed — see "Cross-milestone dependency" above.)*

**Files:**
- Create: `crates/notion-tui/src/ui/conflict_detail.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs` (add `pub mod conflict_detail;`, render wiring in `draw`)
- Modify: `crates/notion-tui/src/app.rs` (`AppMsg` at line 42, new `conflict_detail` field, `request_conflict_detail`, `open_conflict_detail`)
- Modify: `crates/notion-tui/src/main.rs` (route the new `AppMsg` variant)
- Test: `crates/notion-tui/tests/conflict_detail_flow.rs` (new)

**Interfaces:**
- Consumes: M8 Task 4's `describe_op(op: &notion_store::OpRec, store: &notion_store::Store) -> OpDescription` (at `crate::describe::describe_op`, fields `summary: String`/`target_title: Option<String>`/`detail: Option<String>` — see "Cross-milestone dependency"); Task 2's `client.get_page_edited_meta`/`client.get_page`; Task 1's `OpRec.remote_edited_time`/`remote_edited_by` (used as a fallback display value before the live fetch lands, not required by this task's code).
- Produces (Task 11 depends on these):
  - `pub enum ConflictDetailKind { Block { page_id: String, block_id: String, block_type: String, block_payload: String }, Property { row_id: String, prop_name: String } }`
  - `pub struct ConflictDetailState { pub op_seq: i64, pub kind: ConflictDetailKind, pub target_title: String, pub field_label: String, pub local_text: String, pub remote_text: String, pub remote_edited_time: String, pub remote_edited_by: Option<String> }` with `pub fn can_merge(&self) -> bool` and `pub fn rendered_diff(&self) -> String`.
  - `AppMsg::ConflictDetailReady { op_seq: i64, kind: ConflictDetailKind, target_title: String, field_label: String, local_text: String, remote_text: String, remote_edited_time: String, remote_edited_by: Option<String> }`.
  - `App.conflict_detail: Option<ConflictDetailState>`.
  - `App::request_conflict_detail(&mut self, op: OpRec)` — spawns the remote fetch; `App::open_conflict_detail(&mut self, msg: AppMsg)` — populates `self.conflict_detail` from `AppMsg::ConflictDetailReady`.

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-tui/tests/conflict_detail_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{App, AppMsg};
use notion_tui::ui::conflict_detail::ConflictDetailKind;

fn store_with_conflicted_block_op() -> (notion_sync::SharedStore, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Roadmap".into(),
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
            plain_text: "orig".into(),
            has_children: false,
        }],
    )
    .unwrap();
    let receipt = s.edit_update_block_text("b1", "local edit").unwrap();
    s.set_op_state(receipt.op_seq, "conflicted", None).unwrap();
    (Arc::new(Mutex::new(s)), receipt.op_seq)
}

#[test]
fn open_conflict_detail_populates_state_from_the_ready_message() {
    let (store, seq) = store_with_conflicted_block_op();
    let mut app = App::new(store);

    app.open_conflict_detail(AppMsg::ConflictDetailReady {
        op_seq: seq,
        kind: ConflictDetailKind::Block {
            page_id: "p1".into(),
            block_id: "b1".into(),
            block_type: "paragraph".into(),
            block_payload: "{}".into(),
        },
        target_title: "Roadmap".into(),
        field_label: "page body".into(),
        local_text: "local edit".into(),
        remote_text: "remote edit".into(),
        remote_edited_time: "2026-07-05T12:00:00.000Z".into(),
        remote_edited_by: Some("u-9".into()),
    });

    let detail = app.conflict_detail.as_ref().expect("detail state should be set");
    assert_eq!(detail.op_seq, seq);
    assert_eq!(detail.target_title, "Roadmap");
    assert_eq!(detail.local_text, "local edit");
    assert_eq!(detail.remote_text, "remote edit");
    assert!(detail.can_merge());
}

#[test]
fn rendered_diff_shows_both_sides_via_the_markdown_renderer() {
    let (store, seq) = store_with_conflicted_block_op();
    let mut app = App::new(store);
    app.open_conflict_detail(AppMsg::ConflictDetailReady {
        op_seq: seq,
        kind: ConflictDetailKind::Block {
            page_id: "p1".into(),
            block_id: "b1".into(),
            block_type: "to_do".into(),
            block_payload: r#"{"checked": false}"#.into(),
        },
        target_title: "Roadmap".into(),
        field_label: "page body".into(),
        local_text: "Buy milk".into(),
        remote_text: "Buy oat milk".into(),
        remote_edited_time: "t".into(),
        remote_edited_by: None,
    });
    let diff = app.conflict_detail.as_ref().unwrap().rendered_diff();
    // Rendered through blocks_to_markdown, so a to_do gets its "- [ ]" prefix.
    assert!(diff.contains("- [ ] Buy milk"), "diff:\n{diff}");
    assert!(diff.contains("- [ ] Buy oat milk"), "diff:\n{diff}");
}

#[test]
fn property_conflict_detail_cannot_merge() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_data_source(&notion_store::DataSourceRec {
        id: "ds1".into(),
        database_id: "db1".into(),
        title: "Tasks".into(),
        schema_json: "{}".into(),
        last_edited_time: "t".into(),
    })
    .unwrap();
    let store = Arc::new(Mutex::new(s));
    let mut app = App::new(store);
    app.open_conflict_detail(AppMsg::ConflictDetailReady {
        op_seq: 1,
        kind: ConflictDetailKind::Property {
            row_id: "r1".into(),
            prop_name: "Status".into(),
        },
        target_title: "Ship it".into(),
        field_label: "Status".into(),
        local_text: "Done".into(),
        remote_text: "In Progress".into(),
        remote_edited_time: "t".into(),
        remote_edited_by: None,
    });
    assert!(!app.conflict_detail.as_ref().unwrap().can_merge());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test conflict_detail_flow`
Expected: FAIL to compile — `notion_tui::ui::conflict_detail`, `AppMsg::ConflictDetailReady`, `App::open_conflict_detail`, `App.conflict_detail` don't exist yet.

- [ ] **Step 3: Implement the detail-view data model**

Create `crates/notion-tui/src/ui/conflict_detail.rs`:

```rust
use notion_store::BlockRec;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::ui::theme::Theme;

#[derive(Debug, Clone)]
pub enum ConflictDetailKind {
    Block {
        page_id: String,
        block_id: String,
        block_type: String,
        block_payload: String,
    },
    Property {
        row_id: String,
        prop_name: String,
    },
}

pub struct ConflictDetailState {
    pub op_seq: i64,
    pub kind: ConflictDetailKind,
    pub target_title: String,
    pub field_label: String,
    pub local_text: String,
    pub remote_text: String,
    pub remote_edited_time: String,
    pub remote_edited_by: Option<String>,
}

impl ConflictDetailState {
    /// Only a page/block body (a single string that can be re-edited as
    /// Markdown) can go through the merge editor; a property's old/new
    /// values cannot.
    pub fn can_merge(&self) -> bool {
        matches!(self.kind, ConflictDetailKind::Block { .. })
    }

    /// Local and remote rendered the same way the page view would show them
    /// (via the existing Markdown renderer), for a block conflict; the raw
    /// values for a property conflict.
    fn rendered_sides(&self) -> (String, String) {
        match &self.kind {
            ConflictDetailKind::Block {
                block_id,
                block_type,
                block_payload,
                ..
            } => {
                let render_one = |text: &str| {
                    crate::markdown::blocks_to_markdown(&[BlockRec {
                        id: block_id.clone(),
                        page_id: String::new(),
                        parent_block_id: None,
                        ordinal: 0,
                        block_type: block_type.clone(),
                        payload: block_payload.clone(),
                        plain_text: text.to_string(),
                        has_children: false,
                    }])
                    .0
                };
                (render_one(&self.local_text), render_one(&self.remote_text))
            }
            ConflictDetailKind::Property { .. } => (self.local_text.clone(), self.remote_text.clone()),
        }
    }

    /// An inline unified diff between the two sides (local as "yours", remote
    /// as "theirs"), reusing the `similar` crate already used elsewhere for
    /// Markdown diffing — no new dependency.
    pub fn rendered_diff(&self) -> String {
        let (local, remote) = self.rendered_sides();
        similar::TextDiff::from_lines(&remote, &local)
            .unified_diff()
            .context_radius(3)
            .header("theirs (remote)", "yours (local)")
            .to_string()
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

pub fn render(f: &mut Frame, state: &ConflictDetailState, theme: &Theme) {
    let area = f.area();
    let popup = centered_rect((area.width * 4 / 5).clamp(50, 120), (area.height * 4 / 5).clamp(16, 40), area);
    f.render_widget(Clear, popup);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(6), Constraint::Length(6)])
        .split(popup);

    let (local_md, remote_md) = state.rendered_sides();
    f.render_widget(
        Paragraph::new(format!("{} — {}", state.target_title, state.field_label)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(" conflict "),
        ),
        rows[0],
    );

    let sides = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);
    f.render_widget(
        Paragraph::new(local_md)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(" yours (local) ")),
        sides[0],
    );
    let remote_title = match &state.remote_edited_by {
        Some(who) => format!(" theirs (remote, {} at {}) ", who, state.remote_edited_time),
        None => format!(" theirs (remote, at {}) ", state.remote_edited_time),
    };
    f.render_widget(
        Paragraph::new(remote_md)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(remote_title)),
        sides[1],
    );

    let mut consequences = vec![
        "keep mine (p) — overwrites the remote edit shown right".to_string(),
        "take theirs (t) — discards your edit shown left and adopts the remote version shown right".to_string(),
    ];
    if state.can_merge() {
        consequences.push("merge (m) — opens $EDITOR with both versions to combine them by hand".to_string());
    }
    consequences.push("esc — back to the queue".to_string());
    f.render_widget(
        Paragraph::new(consequences.join("\n")).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(" resolve "),
        ),
        rows[2],
    );
}
```

In `crates/notion-tui/src/ui/mod.rs`, add `pub mod conflict_detail;` to the module list, and in `draw`, alongside the other overlay renders:

```rust
    if let Some(detail) = &app.conflict_detail {
        conflict_detail::render(f, detail, &theme);
    }
```

- [ ] **Step 4: Implement the App-side fetch/open plumbing**

In `crates/notion-tui/src/app.rs`:

Add the field to `App`:

```rust
    pub conflict_detail: Option<crate::ui::conflict_detail::ConflictDetailState>,
```

and initialize `conflict_detail: None,` in `App::new`.

Extend `AppMsg`:

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
    ConflictDetailReady {
        op_seq: i64,
        kind: crate::ui::conflict_detail::ConflictDetailKind,
        target_title: String,
        field_label: String,
        local_text: String,
        remote_text: String,
        remote_edited_time: String,
        remote_edited_by: Option<String>,
    },
}
```

Add the request + open functions (near `request_merge`):

```rust
    /// Kicks off the async fetch behind the conflict detail pane: fetches the
    /// remote side (block text, or a row's remote property value) and pairs it
    /// with the target's human-readable identity from M8 Task 4's `describe_op`.
    pub fn request_conflict_detail(&mut self, op: notion_store::OpRec) {
        let Some(remote) = &self.remote else { return };
        let (target_title, field_label) = {
            let guard = self.store.lock().unwrap();
            let desc = crate::describe::describe_op(&op, &guard);
            (
                desc.target_title.unwrap_or_else(|| "(untitled)".to_string()),
                desc.detail.unwrap_or_else(|| "page body".to_string()),
            )
        };
        let (client, tx, store) = (remote.client.clone(), remote.tx.clone(), self.store.clone());
        tokio::spawn(async move {
            let payload: serde_json::Value = serde_json::from_str(&op.payload).unwrap_or_default();
            match op.op_type.as_str() {
                "update_block" => {
                    let page_id = payload["page_id"].as_str().unwrap_or_default().to_string();
                    let block_id = op.target_id.clone();
                    let (block_type, block_payload, local_text) = {
                        let guard = store.lock().unwrap();
                        guard
                            .page_blocks(&page_id)
                            .unwrap_or_default()
                            .into_iter()
                            .find(|b| b.id == block_id)
                            .map(|b| (b.block_type, b.payload, b.plain_text))
                            .unwrap_or_default()
                    };
                    let (Ok(meta), Ok(flat)) = (
                        client.get_page_edited_meta(&page_id).await,
                        client.fetch_block_tree(&page_id).await,
                    ) else {
                        return;
                    };
                    let remote_text = flat
                        .iter()
                        .find(|f| f.block.id == block_id)
                        .map(|f| f.block.plain_text.clone())
                        .unwrap_or_default();
                    tx.send(AppMsg::ConflictDetailReady {
                        op_seq: op.seq,
                        kind: crate::ui::conflict_detail::ConflictDetailKind::Block {
                            page_id,
                            block_id,
                            block_type,
                            block_payload,
                        },
                        target_title,
                        field_label,
                        local_text,
                        remote_text,
                        remote_edited_time: meta.last_edited_time,
                        remote_edited_by: meta.last_edited_by,
                    })
                    .ok();
                }
                "update_row" => {
                    let row_id = op.target_id.clone();
                    let Some((prop_name, local_value)) = payload["properties"]
                        .as_object()
                        .and_then(|m| m.iter().next())
                        .map(|(k, v)| (k.clone(), crate::ui::table::cell_text(v)))
                    else {
                        return;
                    };
                    let (Ok(meta), Ok(remote_page)) =
                        (client.get_page_edited_meta(&row_id).await, client.get_page(&row_id).await)
                    else {
                        return;
                    };
                    let remote_value = crate::ui::table::cell_text(&remote_page["properties"][&prop_name]);
                    tx.send(AppMsg::ConflictDetailReady {
                        op_seq: op.seq,
                        kind: crate::ui::conflict_detail::ConflictDetailKind::Property { row_id, prop_name },
                        target_title,
                        field_label,
                        local_text: local_value,
                        remote_text: remote_value,
                        remote_edited_time: meta.last_edited_time,
                        remote_edited_by: meta.last_edited_by,
                    })
                    .ok();
                }
                _ => {}
            }
        });
    }

    pub fn open_conflict_detail(&mut self, msg: AppMsg) {
        let AppMsg::ConflictDetailReady {
            op_seq,
            kind,
            target_title,
            field_label,
            local_text,
            remote_text,
            remote_edited_time,
            remote_edited_by,
        } = msg
        else {
            return;
        };
        self.conflict_detail = Some(crate::ui::conflict_detail::ConflictDetailState {
            op_seq,
            kind,
            target_title,
            field_label,
            local_text,
            remote_text,
            remote_edited_time,
            remote_edited_by,
        });
    }
```

- [ ] **Step 5: Route the new message in `main.rs`**

In `crates/notion-tui/src/main.rs`, extend the `app_rx.recv()` match:

```rust
                Some(m @ app::AppMsg::ConflictDetailReady { .. }) => {
                    app.open_conflict_detail(m);
                }
```

- [ ] **Step 6: Run tests**

Run: `cargo test -p notion-tui --test conflict_detail_flow && cargo test --workspace`
Expected: PASS.

- [ ] **Step 7: Commit**

```
git add crates/notion-tui/src/ui/conflict_detail.rs crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/app.rs crates/notion-tui/src/main.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): conflict detail view — local vs. remote, with a unified diff

request_conflict_detail fetches the remote side (block text or a row's
property value) and pairs it with describe_op's target title; the detail
pane renders both sides via the existing Markdown renderer plus a unified
diff (similar::TextDiff), matching M10 item 3.
EOF
)"
```

---

### Task 11: notion-tui — informed resolution + non-blocking flow

**Files:**
- Modify: `crates/notion-tui/src/app.rs` (`dispatch_key` — new branch before the queue-key block; new resolution methods)
- Modify: `crates/notion-tui/src/app.rs` (queue key handling — wire `Enter` to open the detail view)
- Test: `crates/notion-tui/tests/conflict_detail_flow.rs` (extend)

**Interfaces:**
- Consumes: Task 10's `ConflictDetailState`/`AppMsg::ConflictDetailReady`; existing `Store::resolve_keep_mine`/`resolve_take_theirs`/`resolve_conflict_merge` (all pre-existing, unchanged).
- Produces:
  - `App::resolve_conflict_detail_keep_mine(&mut self)`, `App::resolve_conflict_detail_take_theirs(&mut self)`, `App::merge_conflict_detail(&mut self, run_editor: impl FnOnce(&str) -> anyhow::Result<String>)`.
  - Non-blocking flow: after any resolution, if another conflicted op remains in the queue, the queue stays open with that op selected; if none remain, the queue closes back to `queue_return` (same mechanism `toggle_queue` already uses).
  - Queue key handling gains `Enter` on a conflicted row → `request_conflict_detail`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/notion-tui/tests/conflict_detail_flow.rs`:

```rust
fn store_with_two_conflicted_block_ops() -> notion_sync::SharedStore {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
        icon: None,
        archived: false,
        last_edited_time: "t1".into(),
    })
    .unwrap();
    s.replace_page_blocks(
        "p1",
        &[
            BlockRec {
                id: "b1".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 0,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "x".into(),
                has_children: false,
            },
            BlockRec {
                id: "b2".into(),
                page_id: "p1".into(),
                parent_block_id: None,
                ordinal: 1,
                block_type: "paragraph".into(),
                payload: "{}".into(),
                plain_text: "y".into(),
                has_children: false,
            },
        ],
    )
    .unwrap();
    let r1 = s.edit_update_block_text("b1", "local 1").unwrap();
    s.set_op_state(r1.op_seq, "conflicted", None).unwrap();
    let r2 = s.edit_update_block_text("b2", "local 2").unwrap();
    s.set_op_state(r2.op_seq, "conflicted", None).unwrap();
    Arc::new(Mutex::new(s))
}

fn detail_msg(op_seq: i64, block_id: &str) -> AppMsg {
    AppMsg::ConflictDetailReady {
        op_seq,
        kind: ConflictDetailKind::Block {
            page_id: "p1".into(),
            block_id: block_id.into(),
            block_type: "paragraph".into(),
            block_payload: "{}".into(),
        },
        target_title: "P".into(),
        field_label: "page body".into(),
        local_text: "local".into(),
        remote_text: "remote".into(),
        remote_edited_time: "t9".into(),
        remote_edited_by: None,
    }
}

#[test]
fn resolving_one_of_two_conflicts_returns_to_queue_with_the_next_selected() {
    let store = store_with_two_conflicted_block_ops();
    let ops = store.lock().unwrap().ops().unwrap();
    let (seq1, seq2) = (ops[0].seq, ops[1].seq);
    let mut app = App::new(store.clone());
    app.toggle_queue(); // enter queue so there's a previous view to return to later

    app.open_conflict_detail(detail_msg(seq1, "b1"));
    app.resolve_conflict_detail_keep_mine();

    assert!(app.conflict_detail.is_none(), "detail pane should close after resolving");
    match &app.view {
        notion_tui::app::View::Queue(q) => {
            let selected = q.ops[q.cursor].seq;
            assert_eq!(selected, seq2, "the remaining conflict should be selected");
        }
        _ => panic!("expected to remain on the queue — one conflict is left"),
    }
}

#[test]
fn resolving_the_last_conflict_returns_to_the_previous_view() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "P".into(),
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
            plain_text: "x".into(),
            has_children: false,
        }],
    )
    .unwrap();
    let r = s.edit_update_block_text("b1", "local").unwrap();
    s.set_op_state(r.op_seq, "conflicted", None).unwrap();
    let store = Arc::new(Mutex::new(s));

    let mut app = App::new(store);
    app.open_page("p1"); // the "previous view" to return to
    app.toggle_queue();
    app.open_conflict_detail(detail_msg(r.op_seq, "b1"));

    app.resolve_conflict_detail_keep_mine();

    assert!(app.conflict_detail.is_none());
    assert!(matches!(app.view, notion_tui::app::View::Page(_)), "no conflicts left: back to the page");
}

#[test]
fn merge_from_detail_view_applies_and_advances_like_keep_mine() {
    let (store, seq) = store_with_conflicted_block_op();
    let mut app = App::new(store.clone());
    app.toggle_queue();
    app.open_conflict_detail(detail_msg(seq, "b1"));

    app.merge_conflict_detail(|initial| {
        assert!(initial.contains("<<<<<<< local"));
        assert!(initial.contains("remote"));
        Ok("merged text".to_string())
    });

    assert!(app.conflict_detail.is_none());
    let b1 = store
        .lock()
        .unwrap()
        .page_blocks("p1")
        .unwrap()
        .into_iter()
        .find(|b| b.id == "b1")
        .unwrap();
    assert_eq!(b1.plain_text, "merged text");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test conflict_detail_flow`
Expected: FAIL to compile — `resolve_conflict_detail_keep_mine`/`merge_conflict_detail` don't exist yet.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/app.rs`, add the shared advance-or-return helper and the three resolution methods (near `open_conflict_detail`):

```rust
    /// After resolving a conflict from the detail view: if another conflicted
    /// op remains, stay on the queue with it selected; otherwise behave like
    /// closing the queue (return to whatever view was open before it).
    fn advance_or_return_from_conflict(&mut self) {
        self.conflict_detail = None;
        self.refresh_queue();
        if let View::Queue(q) = &mut self.view {
            match q.ops.iter().position(|o| o.state == "conflicted") {
                Some(idx) => q.cursor = idx,
                None => self.toggle_queue(),
            }
        }
    }

    pub fn resolve_conflict_detail_keep_mine(&mut self) {
        let Some(detail) = &self.conflict_detail else { return };
        let seq = detail.op_seq;
        self.store.lock().unwrap().resolve_keep_mine(seq).ok();
        self.advance_or_return_from_conflict();
    }

    pub fn resolve_conflict_detail_take_theirs(&mut self) {
        let Some(detail) = &self.conflict_detail else { return };
        let seq = detail.op_seq;
        let target = self.store.lock().unwrap().resolve_take_theirs(seq).ok().flatten();
        if let Some(target) = target {
            self.request_refetch(target);
        }
        self.advance_or_return_from_conflict();
    }

    /// Opens $EDITOR with both versions (same conflict-marker document as the
    /// legacy merge flow), now reached only after the user has seen the
    /// side-by-side context in the detail pane. Property conflicts have no
    /// merge action (`can_merge()` is false) and are left untouched.
    pub fn merge_conflict_detail(&mut self, run_editor: impl FnOnce(&str) -> anyhow::Result<String>) {
        let Some(detail) = &self.conflict_detail else { return };
        let crate::ui::conflict_detail::ConflictDetailKind::Block { block_id, .. } = &detail.kind else {
            return;
        };
        let block_id = block_id.clone();
        let doc = format!(
            "<<<<<<< local\n{}\n=======\n{}\n>>>>>>> remote\n",
            detail.local_text, detail.remote_text
        );
        let op_seq = detail.op_seq;
        let remote_edited_time = detail.remote_edited_time.clone();

        self.force_redraw = true;
        let merged = match run_editor(&doc) {
            Ok(m) => m,
            Err(e) => {
                self.notice = Some(format!("editor failed: {e}"));
                return; // leave the detail pane open so the user can retry
            }
        };
        let merged = merged.trim_end_matches('\n').to_string();
        self.store
            .lock()
            .unwrap()
            .resolve_conflict_merge(op_seq, &block_id, &merged, &remote_edited_time)
            .ok();
        self.advance_or_return_from_conflict();
    }
```

Add a new branch in `dispatch_key`, right before the `if km.is("queue", key)` block (so it takes priority as a modal overlay, matching how `app.confirm`/`app.props` are checked earlier):

```rust
    if app.conflict_detail.is_some() {
        match key.code {
            KeyCode::Esc => app.conflict_detail = None,
            KeyCode::Char('p') => app.resolve_conflict_detail_keep_mine(),
            KeyCode::Char('t') => app.resolve_conflict_detail_take_theirs(),
            KeyCode::Char('m') => {
                if app.conflict_detail.as_ref().is_some_and(|d| d.can_merge()) {
                    let editor = crate::editor::editor_command(app.editor_override.as_deref());
                    let mouse = app.mouse;
                    app.merge_conflict_detail(move |initial| {
                        crate::terminal::with_suspended(mouse, || crate::editor::edit_text(&editor, initial))
                    });
                }
            }
            _ => {}
        }
        return;
    }
```

And in the existing queue key-handling block (inside `if let View::Queue(q) = &mut app.view { ... }`), add an `Enter` branch alongside the existing `r`/`p`/`t`/`e` branches:

```rust
        } else if key.code == KeyCode::Enter {
            if let Some(op) = q.selected() {
                if op.state == "conflicted" {
                    let op = op.clone();
                    app.request_conflict_detail(op);
                }
            }
        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

- [ ] **Step 5: Commit**

```
git add crates/notion-tui/src/app.rs
git commit -m "$(cat <<'EOF'
feat(notion-tui): informed resolution from the conflict detail view

keep-mine/take-theirs/merge are now driven from the detail pane opened by
Enter on a conflicted queue row; resolving one conflict returns to the
queue with the next conflict selected, resolving the last returns to the
view open before the queue was entered.
EOF
)"
```

---

### Task 12: e2e verification — markers, detail view, resolution consequences

**Files:**
- Modify: `crates/notion-tui/tests/e2e_conflict.rs`

**Interfaces:**
- Consumes: Tasks 5–11 (markers, detail view, resolution flow) end-to-end against a real `push_once` conflict.

- [ ] **Step 1: Write the extended test**

Extend `crates/notion-tui/tests/e2e_conflict.rs`'s existing `conflicted_push_surfaces_in_queue_and_keep_mine_pushes_through` test. The existing body (unchanged, for context) is:

```rust
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
```

Replace the single `dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('p')));` line (between the `assert!(matches!(app.view, View::Queue(_)));` and the "Second push" comment) with the M10 flow — marker visibility, opening the detail view, and resolving from it:

```rust
    // --- M10: the conflict is legible before the user resolves it ---
    // (`app.view` is already `View::Queue` here — the pre-existing dispatch of
    // Shift-Q above already opened it and the assertion just before confirmed it.)
    assert!(app.sidebar.conflicted_ids.contains("p1"), "refresh_conflicted must mark p1");
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    term.draw(|f| notion_tui::ui::draw(f, &mut app)).unwrap();
    let rendered = format!("{:?}", term.backend().buffer());
    assert!(rendered.contains("⚠ 1 conflicts"), "status bar must show the conflict count:\n{rendered}");

    // Open the detail pane for the conflicted row. request_conflict_detail's async
    // fetch needs app.remote wired to a real client, which this fixture never sets
    // up (mirroring merge_flow.rs's existing convention of driving the message
    // directly instead of through the network call) — so this test does the same,
    // with values matching what the mock server above would actually return.
    let conflicted_op = store
        .lock()
        .unwrap()
        .ops()
        .unwrap()
        .into_iter()
        .find(|o| o.state == "conflicted")
        .unwrap();
    app.open_conflict_detail(notion_tui::app::AppMsg::ConflictDetailReady {
        op_seq: conflicted_op.seq,
        kind: notion_tui::ui::conflict_detail::ConflictDetailKind::Block {
            page_id: "p1".into(),
            block_id: "b1".into(),
            block_type: "paragraph".into(),
            block_payload: "{}".into(),
        },
        target_title: "P".into(),
        field_label: "page body".into(),
        local_text: "local edit".into(),
        remote_text: "x".into(),
        remote_edited_time: "2026-07-06T12:00:00.000Z".into(),
        remote_edited_by: None,
    });
    let detail = app.conflict_detail.as_ref().expect("detail view should be open");
    assert_eq!(detail.local_text, "local edit");
    term.draw(|f| notion_tui::ui::draw(f, &mut app)).unwrap();
    let rendered = format!("{:?}", term.backend().buffer());
    assert!(rendered.contains("keep mine"), "consequence lines must be visible:\n{rendered}");
    assert!(rendered.contains("overwrites the remote edit shown right"), "rendered:\n{rendered}");

    app.resolve_conflict_detail_keep_mine();
    assert!(app.conflict_detail.is_none(), "resolving the only conflict closes the detail pane");
```

Leave the remainder of the existing test (the "Second push" comment onward) exactly as-is: `resolve_conflict_detail_keep_mine` above performs the same store-level `resolve_keep_mine` the old direct `'p'` keypress did, so the pre-existing "second push succeeds, queue drains" assertions still hold unmodified.

- [ ] **Step 2: Run test to verify current behavior, then implement is a no-op (all prior tasks already landed)**

Run: `cargo test -p notion-tui --test e2e_conflict`
Expected: PASS immediately (Tasks 5–11 already implemented everything this test exercises). If it fails, the failure identifies exactly which prior task's behavior doesn't match this end-to-end path — fix the discrepancy in that task's code, not by weakening this test.

- [ ] **Step 3: Run the full suite**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 4: Commit**

```
git add crates/notion-tui/tests/e2e_conflict.rs
git commit -m "$(cat <<'EOF'
test(notion-tui): e2e conflict test covers markers, detail view, and resolution

Verifies status_line's "N conflicts" copy, the sidebar marker, the detail
pane's content, the exact "overwrites the remote edit shown right"
consequence line, and that keep-mine from the detail view closes the pane
exactly like the pre-M10 direct queue shortcut did.
EOF
)"
```

---

### Task 13: Integration sweep

**Files:** none new.

- [ ] **Step 1: Full gates**

Run (from `notion-tui/`): `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green.

- [ ] **Step 2: Cross-task interaction checks**

- `cargo test -p notion-store schema::` — v1→v2 migration + backup, fresh-install v2, forward-compat guard all still pass together.
- `cargo test -p notion-sync` — Task 3's conflict-metadata capture composes with the pre-existing verify-before-resend (M6) and conflict-blocking (existing `push_conflicts.rs`) behavior.
- `cargo test -p notion-tui --test conflict_detail_flow --test e2e_conflict --test queue_flow --test merge_flow` — marker/jump/detail/resolution tasks compose with the pre-existing queue and legacy-merge flows.
- `git -C /Users/rehatbir/Developer/fables status` — confirm only intended files changed; report the full file list.

- [ ] **Step 3: Manual smoke checklist (report, don't fix)**

If a configured token/db exists locally, run `cargo run -p notion-tui`, force a conflict (edit a block locally, edit the same page in the Notion web app, wait for a push cycle), and verify: the sidebar/table/board show the `⚠` marker, `C` jumps to the queue with the conflict selected, `Enter` opens the detail pane with both versions and the diff, and resolving it returns to the previous view. Otherwise skip and note it. Report results; the session owner decides on commits.

---

## Spec coverage

| M10 spec item | Task(s) |
|---|---|
| 1. Conflict location — sidebar marker | Task 5 |
| 1. Conflict location — table/board marker | Tasks 6, 7 |
| 1. Conflict location — status-bar jump action (key + palette "conflicts") | Task 8 |
| 2. Conflict identity — target title/property/block (via M8.3), edit times, remote author | Tasks 1–4, 9 |
| 3. Conflict detail view — side-by-side, Markdown renderer, unified diff | Task 10 |
| 4. Informed resolution — one-line consequences, merge preceded by context | Task 11 |
| 5. Non-blocking flow — next conflict selected / return to previous view | Task 11 |
| Verification bar — e2e test: markers, detail view, resolution consequences | Task 12 |
