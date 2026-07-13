# notion-tui M6 — Trust & Data Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate every data-corrupting, data-losing, or silently-failing behavior found in the 2026-07-12 audits, gated by a three-OS CI pipeline.

**Architecture:** No new crates. `notion-api` gains timeouts and a per-request retry policy; `notion-sync` gains error propagation (no hwm advance on failure), inflight-verification for ambiguous creates, and poison/crash surfacing; `notion-store` gains a schema forward-compat guard, pre-migration backup, and a comments transaction; `notion-tui` gains property-edit guards, view-state preservation across sync refreshes, and surfaced editor errors/summaries/fence warnings.

**Tech Stack:** Existing: ratatui/crossterm, tokio, reqwest(rustls), rusqlite(bundled), wiremock, tempfile. New: none (CI uses dtolnay/rust-toolchain + Swatinem/rust-cache actions).

## Global Constraints

- Notion API version `2025-09-03` (unchanged).
- **Do NOT `git commit` or `git push` anywhere in this plan.** Leave all changes in the working tree; the session owner reviews and commits. (This intentionally overrides the usual per-task commit steps.)
- The git root is the monorepo `/Users/rehatbir/Developer/fables`; the workspace lives in `notion-tui/`. All `cargo` commands run from `/Users/rehatbir/Developer/fables/notion-tui`.
- Never weaken an existing test to make it pass; if a behavior intentionally changes, update the test to assert the new behavior and say so.
- All quality gates must stay green after every task: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.

## Execution waves (parallelization map)

- **Wave 0 (serial, must land first):** Task 1 (rustfmt + CI) — it may reformat every file; nothing else may run concurrently.
- **Wave 1 (five parallel tracks, disjoint files):**
  - Track API: Task 2 (`crates/notion-api`)
  - Track SYNC: Task 3 → Task 4 → Task 5 (`crates/notion-sync`, sequential within track)
  - Track STORE: Task 6 → Task 7 (`crates/notion-store`, sequential within track)
  - Track PROPS: Task 8 (`crates/notion-tui/src/ui/props.rs` + app.rs props-commit arm — coordinate with Track APP below: Task 8 owns ONLY the `PropsAction::Commit` match arm in app.rs)
  - Track APP: Task 9 → Task 10 → Task 11 (`crates/notion-tui/src/app.rs`, `markdown/parse.rs`, sequential within track)
- **Wave 2 (serial):** Task 12 (integration sweep).

Interfaces between tracks are pinned in each task's **Interfaces** block; implement exactly those signatures so parallel work composes.

---

### Task 1: rustfmt.toml + three-OS CI gate

**Files:**
- Create: `/Users/rehatbir/Developer/fables/notion-tui/rustfmt.toml`
- Create: `/Users/rehatbir/Developer/fables/.github/workflows/notion-tui-ci.yml`

**Interfaces:**
- Consumes: nothing.
- Produces: a passing `cargo fmt --check` and the CI workflow every later milestone relies on.

- [ ] **Step 1: Establish a rustfmt config that matches the existing style**

`cargo fmt --check` currently fails with 389 hunks because the code is formatted wider than rustfmt's default 100. Create `notion-tui/rustfmt.toml`:

```toml
max_width = 110
```

Run: `cargo fmt --check` (from `notion-tui/`). If it still fails, try `max_width = 120`. If neither width gets the failure count to zero, keep `max_width = 110` and run `cargo fmt` once — a one-time reformat is acceptable ONLY in this task (nothing else is in flight in Wave 0).

- [ ] **Step 2: Verify fmt gate**

Run: `cargo fmt --check && echo FMT-OK`
Expected: `FMT-OK`

- [ ] **Step 3: Write the CI workflow**

Create `/Users/rehatbir/Developer/fables/.github/workflows/notion-tui-ci.yml`:

```yaml
name: notion-tui CI

on:
  push:
    branches: [main]
    paths: ["notion-tui/**", ".github/workflows/notion-tui-ci.yml"]
  pull_request:
    paths: ["notion-tui/**", ".github/workflows/notion-tui-ci.yml"]

defaults:
  run:
    working-directory: notion-tui

jobs:
  test:
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: notion-tui
      - name: Test
        run: cargo test --workspace
      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Format
        run: cargo fmt --check
```

- [ ] **Step 4: Validate the workflow file parses**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('/Users/rehatbir/Developer/fables/.github/workflows/notion-tui-ci.yml')); print('YAML-OK')"`
Expected: `YAML-OK`

- [ ] **Step 5: Full local gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all pass. Do NOT commit (global constraint).

---

### Task 2: notion-api — HTTP timeouts + per-request retry policy

**Files:**
- Modify: `crates/notion-api/src/client.rs`
- Modify: `crates/notion-api/src/endpoints.rs` (call-site policy choices)
- Test: `crates/notion-api/tests/retry.rs` (extend), `crates/notion-api/tests/timeout.rs` (new)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Task 4 in Track SYNC depends on these exact semantics):
  - `NotionClient::with_base_url` builds reqwest with `.timeout(Duration::from_secs(30)).connect_timeout(Duration::from_secs(10))`.
  - Private `enum Idempotency { Safe, Unsafe }`; `request()` gains an `idempotency: Idempotency` parameter.
  - Public wrappers keep existing signatures: `get_json`/`delete_json`/`patch_json` are `Safe`; `post_json` is `Unsafe`; new `pub async fn post_json_idempotent(&self, path: &str, body: &Value) -> Result<Value, ApiError>` is `Safe` (for read-only POSTs: search, data-source query).
  - **Unsafe policy:** 429 is still retried (the server rejected the request — nothing was applied). Connection errors and 5xx are NOT retried: return the error on first occurrence so the caller (pusher) can verify-then-resend.
  - `endpoints.rs`: `search_page` and `query_data_source_*` switch to `post_json_idempotent`; `append_children`, `create_page`, `create_comment*` stay on `post_json`. **Note:** `append_children` is a PATCH today — change it to call a new private unsafe-PATCH path: add `pub(crate) async fn patch_json_unsafe(...)` mirroring `post_json` policy, and use it for `append_children` only.

- [ ] **Step 1: Write the failing tests**

Append to `crates/notion-api/tests/retry.rs`:

```rust
#[tokio::test]
async fn non_idempotent_post_is_not_retried_on_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(500))
        .expect(1) // exactly one attempt — no blind retry
        .mount(&server).await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let err = c.post_json("/v1/pages", &serde_json::json!({})).await.unwrap_err();
    assert!(matches!(err, ApiError::Api { status: 500, .. }));
}

#[tokio::test]
async fn non_idempotent_post_still_retries_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(1)
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/pages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "p1"})))
        .mount(&server).await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let v = c.post_json("/v1/pages", &serde_json::json!({})).await.unwrap();
    assert_eq!(v["id"], "p1");
}

#[tokio::test]
async fn idempotent_post_wrapper_retries_500() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"results": []})))
        .mount(&server).await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    assert!(c.post_json_idempotent("/v1/search", &serde_json::json!({})).await.is_ok());
}
```

Create `crates/notion-api/tests/timeout.rs`:

```rust
use std::time::Duration;
use notion_api::NotionClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn stalled_response_times_out_instead_of_hanging() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(120)))
        .mount(&server).await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c.set_request_timeout(Duration::from_millis(200)); // test hook, mirrors set_timing
    let started = std::time::Instant::now();
    let res = c.get_json("/v1/users/me").await;
    assert!(res.is_err());
    assert!(started.elapsed() < Duration::from_secs(10));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-api --test retry --test timeout`
Expected: FAIL — `post_json_idempotent`/`set_request_timeout` missing; `non_idempotent_post_is_not_retried_on_500` fails because the 500 is retried 5 times (wiremock `.expect(1)` violated).

- [ ] **Step 3: Implement**

`crates/notion-api/src/client.rs` — key changes:

```rust
#[derive(Clone, Copy, PartialEq)]
enum Idempotency {
    Safe,
    Unsafe,
}

pub struct NotionClient {
    http: reqwest::Client,
    // ... existing fields unchanged ...
    request_timeout: Duration,
}

impl NotionClient {
    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        let request_timeout = Duration::from_secs(30);
        Self {
            http: reqwest::Client::builder()
                .timeout(request_timeout)
                .connect_timeout(Duration::from_secs(10))
                .build()
                .expect("reqwest client"),
            // ... existing fields ...
            request_timeout,
        }
    }

    /// Test hook: shrink the per-request timeout (rebuilds the inner client).
    pub fn set_request_timeout(&mut self, timeout: Duration) {
        self.request_timeout = timeout;
        self.http = reqwest::Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout)
            .build()
            .expect("reqwest client");
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::POST, path, Some(body), Idempotency::Unsafe).await
    }

    pub async fn post_json_idempotent(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::POST, path, Some(body), Idempotency::Safe).await
    }

    pub(crate) async fn patch_json_unsafe(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::PATCH, path, Some(body), Idempotency::Unsafe).await
    }
    // get_json/patch_json/delete_json call request(..., Idempotency::Safe).
}
```

In `request()`, the two retry sites change (429 handling is untouched — always retried):

```rust
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    if idempotency == Idempotency::Unsafe || attempt + 1 == MAX_ATTEMPTS {
                        return Err(e.into());
                    }
                    tokio::time::sleep(self.backoff_base * 2u32.pow(attempt)).await;
                    continue;
                }
            };
            // ... 429 branch unchanged ...
            if status.is_server_error() {
                if idempotency == Idempotency::Unsafe {
                    let v: Value = resp.json().await.unwrap_or_default();
                    return Err(ApiError::Api {
                        status: status.as_u16(),
                        code: v["code"].as_str().unwrap_or("unknown").to_string(),
                        message: v["message"].as_str().unwrap_or("").to_string(),
                    });
                }
                tokio::time::sleep(self.backoff_base * 2u32.pow(attempt)).await;
                continue;
            }
```

`crates/notion-api/src/endpoints.rs`: switch `search_page` and the data-source query calls to `post_json_idempotent`; switch `append_children` to `patch_json_unsafe`. Everything else keeps its current wrapper.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-api`
Expected: PASS, including all pre-existing retry tests (they exercise GET/search paths, which remain Safe).

- [ ] **Step 5: Cross-crate check**

Run: `cargo test --workspace`
Expected: PASS. If a notion-sync test asserted 5xx-retry behavior on creates, update that test to assert the new fail-fast semantics (documenting the intentional change).

---

### Task 3: notion-sync — puller propagates store errors; hwm never advances past failures

**Files:**
- Modify: `crates/notion-sync/src/puller.rs`, `crates/notion-sync/src/lib.rs`
- Test: `crates/notion-sync/tests/pull.rs` (extend)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Tasks 4 and 5 build on this):
```rust
#[derive(Debug)]
pub enum SyncError {
    Api(notion_api::ApiError),
    Store(String),
}
impl std::fmt::Display for SyncError { /* "api: {e}" / "store: {msg}" */ }
```
  - `pull_once(...) -> Result<u32, SyncError>` (was `Result<u32, ApiError>`).
  - `push_once` keeps returning `Result<u32, ApiError>` (Task 4 preserves that).
  - `spawn_sync`'s pull match arms become `Err(SyncError::Api(ApiError::Network(_))) => Offline`, any other `Err(e) => Failed(e.to_string())`.

- [ ] **Step 1: Write the failing test**

Append to `crates/notion-sync/tests/pull.rs` (follow the existing test setup in that file for mock-server fixtures — reuse its helper that mounts a search result and block tree):

```rust
#[tokio::test]
async fn store_write_failure_fails_the_cycle_and_preserves_hwm() {
    // Arrange a normal single-page pull fixture (copy the arrangement from the
    // simplest passing pull test in this file), then poison the store by
    // dropping the pages table so upsert_page must fail.
    let (server, client, store) = pull_fixture_one_page("2026-01-02T00:00:00.000Z").await;
    store.lock().unwrap().conn().execute_batch("DROP TABLE pages;").unwrap();

    let res = notion_sync::pull_once(&client, &store).await;
    assert!(res.is_err(), "store failure must fail the pull cycle");
    // hwm must NOT have advanced past the failed item.
    let hwm = store.lock().unwrap().meta_get("hwm").unwrap().unwrap_or_default();
    assert_eq!(hwm, "");
    let _ = server;
}
```

If `pull.rs` has no shared fixture helper, extract one (`pull_fixture_one_page`) from an existing test as part of this step — do not duplicate 40 lines of mock setup.
Note: `meta_get`/`meta_set` live in `sync_meta`, not `pages`, so dropping `pages` breaks only the upsert.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-sync --test pull store_write_failure`
Expected: FAIL — `pull_once` currently returns `Ok` (every store error is `.ok()`-swallowed).

- [ ] **Step 3: Implement**

`crates/notion-sync/src/lib.rs` — add `SyncError` (exact shape from Interfaces) and re-export it. Update the pull arm of `spawn_sync`:

```rust
            match pull_once(&client, &store).await {
                Ok(updated) => { /* unchanged */ }
                Err(SyncError::Api(ApiError::Network(_))) => {
                    status_tx.send_replace(SyncStatus::Offline);
                }
                Err(e) => {
                    status_tx.send_replace(SyncStatus::Failed(e.to_string()));
                }
            }
```

`crates/notion-sync/src/puller.rs` — every `store.lock().unwrap().X(...).ok()` becomes `.map_err(|e| SyncError::Store(e.to_string()))?`; every `client.Y(...).await?` becomes `.await.map_err(SyncError::Api)?` (or add `impl From<ApiError> for SyncError` and keep `?`). The `meta_set("hwm", ...)` at the end also propagates. Because errors now return early, the hwm write is only reached after a fully clean pass — which is exactly the required semantics.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-sync && cargo test --workspace`
Expected: PASS. `dirty_pull.rs`/`sync_loop.rs` compile against the new signature (they match on `Ok`; adjust any explicit `ApiError` matches to `SyncError::Api`).

---

### Task 4: notion-sync — verify-before-resend for ambiguous create failures

**Files:**
- Modify: `crates/notion-sync/src/pusher.rs`
- Test: `crates/notion-sync/tests/push_verify.rs` (new)

**Interfaces:**
- Consumes: Task 2's fail-fast policy (creates are NOT blind-retried by the client) and the existing `Store::set_op_state(seq, state, error)` / op `state` column (`pending`/`inflight`/`failed`/`conflicted`).
- Produces: pusher semantics used by M10's queue UI:
  - Before sending a create-type op (`append_block`, `create_row`, `create_comment`), the pusher sets its state to `inflight`.
  - On success: op deleted (unchanged). On definite API rejection (4xx): `failed` (unchanged). On ambiguous failure (network error / 5xx / timeout): the op STAYS `inflight` and the pass aborts as today.
  - When `push_once` encounters an op already in state `inflight` (a previous pass died ambiguously), it VERIFIES before resending:
    - `append_block`: fetch the container's children (`client.fetch_block_children(container_id)` — use the existing children-listing endpoint that `fetch_block_tree` builds on); if a block exists whose `plain_text` equals the op's `text` and whose id is not already in the store, adopt it (`rewrite_block_id(temp_id, real_id)`, delete op) instead of re-appending.
    - `create_comment`: `client.list_comments(parent_id)`; adopt by matching `body`.
    - `create_row`: `client.query_data_source_all(data_source_id)`; adopt by matching the title property's plain text against the op payload's title.
    - If no remote match: resend (set `inflight`, send as normal).

- [ ] **Step 1: Write the failing test**

Create `crates/notion-sync/tests/push_verify.rs` (model the mock scaffolding on `push.rs`, which already builds a store with a queued `append_block` op and a wiremock server):

```rust
// Scenario 1: the append reached Notion but the response was lost.
// A prior pass left the op `inflight`. The children listing already
// contains the block. push_once must adopt the remote id and NOT
// re-append (the append mock has .expect(0)).
#[tokio::test]
async fn inflight_append_is_adopted_not_resent_when_found_remotely() { /* see push.rs scaffolding */ }

// Scenario 2: op is `inflight` but the children listing does NOT contain
// the block. push_once must resend (append mock .expect(1)) and succeed.
#[tokio::test]
async fn inflight_append_is_resent_when_not_found_remotely() { /* ... */ }

// Scenario 3: a fresh `pending` create moves through `inflight`: mock the
// append endpoint to return 500 (fail-fast per Task 2). After push_once,
// the op state must be "inflight" (not "failed", not deleted), because a
// 5xx on a create is ambiguous.
#[tokio::test]
async fn ambiguous_create_failure_leaves_op_inflight() { /* ... */ }
```

Write all three bodies concretely by copying `push.rs`'s fixture pattern (store + `edit_append_block` + mock server). Assertions go through `store.lock().unwrap().ops()` and wiremock `.expect(n)`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-sync --test push_verify`
Expected: FAIL — no inflight handling exists; 5xx currently marks the op `failed` (or was retried before Task 2).

- [ ] **Step 3: Implement**

In `crates/notion-sync/src/pusher.rs`:

1. In `push_once`'s loop, before `push_one`, for op types `append_block` / `create_row` / `create_comment`:
   - If `op.state == "inflight"`: run the matching `verify_*` function; on adoption, count as pushed and continue to the next op.
   - Set `inflight` before sending: `store.lock().unwrap().set_op_state(op.seq, "inflight", None).ok();`
2. Ambiguity classification for creates:

```rust
fn is_ambiguous(e: &ApiError) -> bool {
    matches!(e, ApiError::Network(_) | ApiError::RetriesExhausted(_))
        || matches!(e, ApiError::Api { status, .. } if *status >= 500)
}
```

   In the `match push_one(...)` arms, for create-type ops: `Err(e) if is_ambiguous(&e)` → leave state `inflight`, and if it was a network error, abort the pass (`return Err(...)`) as today; for a 5xx, block the target and continue (state stays `inflight` for next pass). Non-create ops keep today's behavior exactly (update/delete are idempotent — Task 2 still blind-retries them safely at the client layer).
3. `verify_append_block(client, store, op) -> Result<bool, ApiError>`: parse payload (`container` = `parent_id` or `page_id`, `text`), list the container's children remotely, find a block with `plain_text == text` whose id doesn't exist locally (`store` lookup), then `rewrite_block_id(&op.target_id, &real_id)` + `delete_op(op.seq)` + the same `clear_page_dirty` dance as `push_append_block`. Return `Ok(true)` on adoption.
4. `verify_create_comment` / `verify_create_row`: same shape via `list_comments` / `query_data_source_all` matching body / title as pinned in Interfaces.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-sync && cargo test --workspace`
Expected: PASS, including the pre-existing `push.rs`/`push_conflicts.rs` suites (fresh ops still succeed on the happy path; their state transitions now pass through `inflight`, which those tests don't observe).

---

### Task 5: notion-sync — poison recovery + sync-task crash surfacing

**Files:**
- Modify: `crates/notion-sync/src/lib.rs`, `crates/notion-sync/src/puller.rs`, `crates/notion-sync/src/pusher.rs`
- Test: `crates/notion-sync/tests/sync_loop.rs` (extend)

**Interfaces:**
- Consumes: Task 3's `SyncError`.
- Produces:
  - `pub(crate) fn lock_store(store: &SharedStore) -> std::sync::MutexGuard<'_, notion_store::Store>` in `lib.rs` — recovers from poisoning via `unwrap_or_else(|p| p.into_inner())` (SQLite transactionality makes the data safe even if a holder panicked mid-logical-operation).
  - `spawn_sync` monitors its own task: if the loop task panics, status becomes `SyncStatus::Failed("sync engine crashed — restart notion-tui")`.

- [ ] **Step 1: Write the failing test**

Append to `crates/notion-sync/tests/sync_loop.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn poisoned_store_mutex_does_not_kill_sync() {
    // Fixture: same as the existing sync_loop test (mock server with an
    // empty search result, in-memory store, spawn_sync with a short interval).
    let (server, client, store) = empty_workspace_fixture().await;

    // Poison the mutex from a scratch thread.
    let poisoner = store.clone();
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.lock().unwrap();
        panic!("poison");
    })
    .join();
    assert!(store.lock().is_err(), "mutex must actually be poisoned");

    let mut handle = notion_sync::spawn_sync_shared(client, store, std::time::Duration::from_millis(50));
    // The loop must still reach Idle despite the poisoned mutex.
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            handle.status.changed().await.unwrap();
            if matches!(*handle.status.borrow(), notion_sync::SyncStatus::Idle { .. }) {
                break;
            }
        }
    })
    .await
    .expect("sync must recover from a poisoned mutex");
    let _ = server;
}
```

Check `sync_loop.rs` first: if `spawn_sync` takes `NotionClient` by value and the fixture builds one, reuse its exact construction; `spawn_sync_shared` above is only needed if the existing fixture can't be reused — prefer the existing entry point.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-sync --test sync_loop poisoned`
Expected: FAIL — today the first `store.lock().unwrap()` in the loop panics and the status never reaches `Idle` (timeout).

- [ ] **Step 3: Implement**

`lib.rs`:

```rust
pub(crate) fn lock_store(store: &SharedStore) -> std::sync::MutexGuard<'_, notion_store::Store> {
    store.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
```

Replace every `store.lock().unwrap()` in `lib.rs`, `puller.rs`, `pusher.rs` with `lock_store(store)` (grep: `lock().unwrap()`). This removes the systematic panic source (poisoning).

Then guard against *unexpected* panics inside a cycle: extract the per-cycle body (push + pull + status/pending sends) into `async fn one_cycle(client: &Arc<NotionClient>, store: &SharedStore, status_tx: &watch::Sender<SyncStatus>, data_tx: &watch::Sender<u64>, pending_tx: &watch::Sender<u32>)`, and wrap each iteration with `catch_unwind` (note: `watch::Sender` is not `Clone`, which is why the sends stay inside the one spawned task):

```rust
    tokio::spawn(async move {
        let client = Arc::new(client);
        loop {
            let cycle = one_cycle(&client, &store, &status_tx, &data_tx, &pending_tx);
            if let Err(payload) = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(cycle)).await {
                let _ = payload;
                status_tx.send_replace(SyncStatus::Failed("sync engine crashed — restart notion-tui".into()));
            }
            tokio::time::sleep(interval).await;
        }
    });
```

This needs the `futures` crate (workspace already depends on it transitively via reqwest; add `futures = "0.3"` to `[workspace.dependencies]` and `crates/notion-sync/Cargo.toml` explicitly).

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-sync && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

---

### Task 6: notion-store — schema forward-compat guard + pre-migration backup

**Files:**
- Modify: `crates/notion-store/src/schema.rs`, `crates/notion-store/src/store.rs` (the `Store::open` path)
- Test: inline tests in `schema.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `schema::LATEST_VERSION: i64` (currently `1`).
  - `migrate(conn)` errors with a message containing `"newer"` and both versions when `user_version > LATEST_VERSION`.
  - `Store::open(path)` copies the DB file to `<path>.bak-v<version>` before running migrations when `0 < user_version < LATEST_VERSION` (no-op today since LATEST is 1; the machinery must exist and be tested via a simulated old version).

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/notion-store/src/schema.rs`:

```rust
    #[test]
    fn refuses_db_from_newer_app_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            let s = Store::open(&path).unwrap();
            s.conn().pragma_update(None, "user_version", 99).unwrap();
        }
        let err = Store::open(&path).unwrap_err().to_string();
        assert!(err.contains("newer"), "unhelpful error: {err}");
        assert!(err.contains("99"));
    }

    #[test]
    fn backs_up_db_before_upgrading_from_older_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            let s = Store::open(&path).unwrap();
            // Simulate a database created by an older build.
            s.conn().pragma_update(None, "user_version", 0).unwrap();
        }
        // Note: user_version 0 on an already-initialized DB would re-run
        // SCHEMA_V1 and fail on existing tables; the backup must happen
        // BEFORE migrate is attempted, so assert on the backup file even
        // if open() then errors.
        let _ = Store::open(&path);
        assert!(
            dir.path().join("n.db.bak-v0").exists(),
            "backup file must exist before migration runs"
        );
    }
```

(The second test intentionally exercises only the backup mechanism; when `LATEST_VERSION` grows past 1 the real upgrade path will cover it end-to-end.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-store schema`
Expected: FAIL — no guard, no backup.

- [ ] **Step 3: Implement**

`schema.rs`:

```rust
pub const LATEST_VERSION: i64 = 1;

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
    Ok(())
}
```

In `Store::open(path)` (find it in `store.rs` — it opens the connection then calls `migrate`): after opening the connection and reading `user_version`, and before calling `migrate`:

```rust
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let file_exists_with_data = version > 0 || /* file preexisted: */ path.as_ref().exists();
        if file_exists_with_data && version < schema::LATEST_VERSION {
            let backup = path.as_ref().with_extension(format!("db.bak-v{version}"));
            // Ignore copy errors only if the source doesn't exist yet (fresh DB).
            if path.as_ref().exists() {
                std::fs::copy(path.as_ref(), &backup)?;
            }
        }
```

Careful with the fresh-DB case: a brand-new file (version 0, just created by SQLite on open) also matches `version < LATEST`. Distinguish by checking whether the `pages` table exists (`SELECT count(*) FROM sqlite_master WHERE name='pages'`) — only back up when the DB already has our schema objects. Implement that check; do not back up truly fresh DBs.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-store && cargo test --workspace`
Expected: PASS (fresh-DB open paths in every other test must not produce `.bak` files — the `sqlite_master` check guarantees it).

---

### Task 7: notion-store — transactional `replace_comments`

**Files:**
- Modify: `crates/notion-store/src/store.rs:587-603`
- Test: `crates/notion-store/tests/comments.rs` (extend)

**Interfaces:**
- Consumes: nothing.
- Produces: same public signature; delete+insert now atomic.

- [ ] **Step 1: Write the failing test**

Append to `crates/notion-store/tests/comments.rs`:

```rust
#[test]
fn replace_comments_is_atomic_on_mid_batch_failure() {
    let s = Store::open_in_memory().unwrap();
    s.replace_comments(
        "p1",
        &[CommentRec {
            id: "c1".into(), parent_id: "p1".into(), parent_kind: "page".into(),
            thread_id: Some("t1".into()), author: "a".into(), body: "original".into(),
            created_time: "2026-01-01T00:00:00.000Z".into(),
        }],
    )
    .unwrap();

    // A batch with a duplicate id in itself will violate the PK on the second
    // insert — after the failure, the ORIGINAL comment must still be there.
    let dup = CommentRec {
        id: "c2".into(), parent_id: "p1".into(), parent_kind: "page".into(),
        thread_id: None, author: "a".into(), body: "new".into(),
        created_time: "2026-01-02T00:00:00.000Z".into(),
    };
    let bad_batch = vec![dup.clone(), CommentRec { body: "conflict".into(), ..dup }];
    // INSERT OR REPLACE can't fail on a duplicate id — switch the statement to
    // plain INSERT as part of this task (replace-semantics come from the
    // preceding DELETE), which makes mid-batch failure representable.
    assert!(s.replace_comments("p1", &bad_batch).is_err());

    let after = s.comments_for("p1").unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].body, "original", "failed batch must not destroy prior comments");
}
```

(Requires `CommentRec: Clone` — derive it if missing.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-store --test comments replace_comments_is_atomic`
Expected: FAIL — the DELETE commits immediately today, so `after` is the partial new batch, not the original.

- [ ] **Step 3: Implement**

```rust
    pub fn replace_comments(&self, parent_id: &str, recs: &[CommentRec]) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM comments WHERE parent_id = ?1 AND id NOT LIKE 'tmp-%'", [parent_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO comments (id, parent_id, parent_kind, thread_id, author, body, created_time)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for c in recs {
                stmt.execute(rusqlite::params![
                    c.id, c.parent_id, c.parent_kind, c.thread_id, c.author, c.body, c.created_time
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
```

Keep `INSERT` (not `INSERT OR REPLACE`): the DELETE already cleared synced rows; a collision with a kept `tmp-%` row cannot happen (remote ids are UUIDs). If any existing test relied on `OR REPLACE` upsert semantics, surface that in your report rather than silently restoring it.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-store && cargo test --workspace`
Expected: PASS.

---

### Task 8: notion-tui — property-edit guards (relation/people/date-end)

**Files:**
- Modify: `crates/notion-tui/src/ui/props.rs`, `crates/notion-tui/src/app.rs` (ONLY the `PropsAction::Commit` handling arm — Track APP owns the rest of app.rs)
- Test: `crates/notion-tui/tests/property_form_flow.rs` (extend), inline tests in `props.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `build_property_value(prop_type: &str, text: &str, existing: Option<&serde_json::Value>) -> Option<serde_json::Value>` — **signature change**: returns `None` for any type it cannot faithfully build (`relation`, `people`, `files`, `formula`, `rollup`, `created_time`, `created_by`, `last_edited_time`, `last_edited_by`, `unique_id`, and any unknown type). `date` merges `end` (and `time_zone`) from `existing` when present.
  - `PropsState::on_key`: `Enter` on a read-only field (same type list) does NOT open an editor; it returns `PropsAction::None` and sets a new `pub read_only_notice: Option<String>` on `PropsState` ("relation properties are read-only (v1)") which `props::render` shows in the footer line.
  - The `PropsAction::Commit` arm in app.rs treats a `None` from `build_property_value` as a no-op plus `self.notice = Some(format!("{prop_type} properties can't be edited in notion-tui yet"))`.

- [ ] **Step 1: Write the failing tests**

Inline in `props.rs` tests module:

```rust
    #[test]
    fn refuses_to_build_values_for_unsupported_types() {
        for t in ["relation", "people", "files", "formula", "rollup", "created_time", "totally_new_type"] {
            assert!(build_property_value(t, "anything", None).is_none(), "{t} must refuse");
        }
    }

    #[test]
    fn date_edit_preserves_existing_end_and_timezone() {
        let existing = serde_json::json!({"type": "date", "date": {"start": "2026-01-01", "end": "2026-02-01", "time_zone": "America/Toronto"}});
        let v = build_property_value("date", "2026-01-15", Some(&existing)).unwrap();
        assert_eq!(v["date"]["start"], "2026-01-15");
        assert_eq!(v["date"]["end"], "2026-02-01");
        assert_eq!(v["date"]["time_zone"], "America/Toronto");
    }

    #[test]
    fn enter_on_relation_field_does_not_open_editor() {
        let mut st = props_state_with_field("Linked", "relation", "2 linked");
        let act = st.on_key(crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Enter));
        assert!(matches!(act, PropsAction::None));
        assert!(st.editor.is_none());
        assert!(st.read_only_notice.as_deref().unwrap_or("").contains("read-only"));
    }
```

(Add a small `props_state_with_field(name, prop_type, value_text)` test helper next to the existing test helpers in props.rs; if none exist, build `PropsState` literally.)

In `tests/property_form_flow.rs`, add an end-to-end guard: drive the app exactly like the existing commit test in that file but on a `relation` field, then assert `store.ops()` gained **no** op and `app.notice` mentions "can't be edited".

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui props && cargo test -p notion-tui --test property_form_flow`
Expected: FAIL — `build_property_value` has the old 2-arg signature and returns `Null` for relation.

- [ ] **Step 3: Implement**

`props.rs`:

```rust
const READ_ONLY_TYPES: &[&str] = &[
    "relation", "people", "files", "formula", "rollup", "created_time",
    "created_by", "last_edited_time", "last_edited_by", "unique_id",
];

pub fn build_property_value(prop_type: &str, text: &str, existing: Option<&Value>) -> Option<Value> {
    if READ_ONLY_TYPES.contains(&prop_type) {
        return None;
    }
    Some(match prop_type {
        // ... every existing supported arm, unchanged, except:
        "date" => {
            let mut date = json!({"start": text});
            if let Some(old) = existing.and_then(|e| e.get("date")) {
                for k in ["end", "time_zone"] {
                    if let Some(v) = old.get(k) {
                        if !v.is_null() {
                            date[k] = v.clone();
                        }
                    }
                }
            }
            json!({"type": "date", "date": date})
        }
        _ => return None,   // unknown types refuse instead of Value::Null
    })
}
```

`PropsState`: add `pub read_only_notice: Option<String>` (init `None` at every construction site). In `on_key`'s `Enter` arm, before opening any editor:

```rust
                    if READ_ONLY_TYPES.contains(&field.prop_type.as_str()) {
                        self.read_only_notice =
                            Some(format!("{} properties are read-only (v1)", field.prop_type));
                        return PropsAction::None;
                    }
```

Clear `read_only_notice` on any cursor movement. In `props::render`, when `read_only_notice` is set, render it as the last line inside the popup (reuse the style of the existing select-hint line).

`app.rs` `PropsAction::Commit` arm: locate where it calls `build_property_value(...)` (search `build_property_value`); it must fetch the row's current property JSON for `existing` (the row is available where fields were built — `store` lookup by `row_id` + prop name) and handle `None`:

```rust
                let existing = current_property_value(&self.store, &row_id, &prop_name);
                match crate::ui::props::build_property_value(&prop_type, &text, existing.as_ref()) {
                    Some(value) => { /* existing edit_update_row path, unchanged */ }
                    None => {
                        self.notice =
                            Some(format!("{prop_type} properties can't be edited in notion-tui yet"));
                    }
                }
```

Add the small `current_property_value` helper next to the commit arm (parse the row's `properties` JSON, take `[prop_name]`).

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS. Existing checkbox/select/date commit tests keep passing (their `existing` is either absent or has no `end`).

---

### Task 9: notion-tui — view-state preservation across sync refreshes

**Files:**
- Modify: `crates/notion-tui/src/app.rs` (`refresh_current_view`, `open_page`, `open_table`)
- Test: `crates/notion-tui/tests/refresh_state_flow.rs` (new)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `open_page`/`open_table` keep their signatures (fresh navigation still resets state).
  - `refresh_current_view` preserves, for `View::Page`: `cursor` (clamped to the new `lines()` length) and `collapsed_toggles` (carried over as-is; stale ids are harmless in a `HashSet`). For `View::Table`: `cursor` is re-pointed at the previously selected row id when that row still exists (fall back to clamped index), and `sort`/`sort_col` are carried over.

- [ ] **Step 1: Write the failing test**

Create `crates/notion-tui/tests/refresh_state_flow.rs` (use the store-seeding helpers pattern from `render_smoke.rs`/`table_edit_flow.rs` — seed pages/blocks/rows through the public `Store` API):

```rust
use std::sync::{Arc, Mutex};
use notion_store::Store;
use notion_tui::app::{App, View};

fn app_with_page_of_blocks(n: usize) -> App {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    // seed: one page "p1" with n paragraph blocks b0..bn (copy the exact
    // seeding calls used by render_smoke.rs)
    /* ... */
    let mut app = App::new(store);
    app.open_page("p1");
    app
}

#[test]
fn page_refresh_preserves_cursor_and_collapsed_toggles() {
    let mut app = app_with_page_of_blocks(10);
    if let View::Page(v) = &mut app.view {
        v.cursor = 7;
        v.collapsed_toggles.insert("b3".into());
    }
    app.refresh_current_view();
    let View::Page(v) = &app.view else { panic!("still a page view") };
    assert_eq!(v.cursor, 7);
    assert!(v.collapsed_toggles.contains("b3"));
}

#[test]
fn page_refresh_clamps_cursor_when_blocks_shrink() {
    let mut app = app_with_page_of_blocks(10);
    if let View::Page(v) = &mut app.view {
        v.cursor = 9;
    }
    // delete blocks b5..b9 in the store, then refresh
    /* store delete calls */
    app.refresh_current_view();
    let View::Page(v) = &app.view else { panic!() };
    assert!(v.cursor < v.lines().len().max(1));
}

#[test]
fn table_refresh_follows_selected_row_and_keeps_sort() {
    // seed a data source with rows r1..r5; open table; sort by col 1 desc;
    // select r4; refresh; assert selected_row_id() == Some("r4") and
    // sort == previous sort.
    /* ... */
}

#[test]
fn fresh_navigation_still_resets_state() {
    let mut app = app_with_page_of_blocks(10);
    if let View::Page(v) = &mut app.view { v.cursor = 7; }
    app.open_page("p1"); // explicit re-open, not a refresh
    let View::Page(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 0);
}
```

Fill the elided seeding using the concrete helper calls found in the existing tests (do not invent store APIs — copy from `render_smoke.rs`).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test refresh_state_flow`
Expected: FAIL — cursor comes back 0, toggles empty.

- [ ] **Step 3: Implement**

In `refresh_current_view` (`app.rs:582`), replace the Page and Table arms (Board arm already does this — mirror its shape):

```rust
            View::Page(v) => {
                let (id, cursor, collapsed) = (v.page.id.clone(), v.cursor, v.collapsed_toggles.clone());
                self.open_page(&id);
                if let View::Page(nv) = &mut self.view {
                    nv.cursor = cursor.min(nv.lines().len().saturating_sub(1));
                    nv.collapsed_toggles = collapsed;
                }
            }
            View::Table(v) => {
                let (id, cursor, sort, sort_col, selected) =
                    (v.ds.id.clone(), v.cursor, v.sort, v.sort_col, v.selected_row_id());
                self.open_table(&id);
                if let View::Table(nv) = &mut self.view {
                    nv.sort = sort;
                    nv.sort_col = sort_col;
                    // Re-apply the sort to the fresh rows the same way toggle_sort
                    // does (extract its sorting body into a private `fn apply_sort`
                    // on TableView if needed, and call it from both places).
                    nv.apply_sort();
                    nv.cursor = selected
                        .and_then(|sid| nv.rows_iter_position(&sid))
                        .unwrap_or_else(|| cursor.min(nv.row_count().saturating_sub(1)));
                }
            }
```

Check `ui/table.rs` for the real names: if there is no `apply_sort`/`rows_iter_position`/`row_count`, add them as thin methods over the existing fields (`apply_sort` = the sort body currently inside `toggle_sort`; `rows_iter_position(id)` = position of the row with that id in current display order; `row_count` = displayed rows length). Keep `open_page`'s reset-to-zero behavior untouched — preservation lives only in `refresh_current_view`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS, including `board_flow.rs` (Board arm untouched).

---

### Task 10: notion-tui — surfaced $EDITOR failures and post-edit summaries

**Files:**
- Modify: `crates/notion-tui/src/app.rs` (`edit_in_editor` at ~216-248, `open_merge_editor` at ~500-520)
- Test: `crates/notion-tui/tests/editor_flow.rs` (extend)

**Interfaces:**
- Consumes: Task 9 must already be merged in this track (sequential; both edit app.rs).
- Produces: every editor-flow outcome lands in `app.notice`:
  - launch/exit failure → `editor failed: {err}`
  - apply failure → `edit failed: {err}`
  - success → `edited: {updated} updated · {inserted} added · {deleted} deleted · {reordered} moved` (omit zero-count segments; if all are zero: `no changes`).

- [ ] **Step 1: Write the failing tests**

Append to `crates/notion-tui/tests/editor_flow.rs` (this file already drives `edit_in_editor` with injected closures):

```rust
#[test]
fn editor_failure_is_surfaced_in_notice() {
    let mut app = /* existing page-view fixture from this file */;
    app.edit_in_editor(|_initial| anyhow::bail!("editor exited with signal 9"));
    assert!(app.notice.as_deref().unwrap_or("").contains("editor failed"));
    assert!(app.notice.as_deref().unwrap().contains("signal 9"));
}

#[test]
fn successful_edit_reports_a_change_summary() {
    let mut app = /* fixture with a page containing 2 paragraphs */;
    app.edit_in_editor(|initial| Ok(format!("{initial}\nbrand new line")));
    let n = app.notice.as_deref().unwrap_or("");
    assert!(n.contains("1 added"), "notice was: {n}");
}

#[test]
fn no_op_edit_reports_no_changes() {
    let mut app = /* same fixture */;
    app.edit_in_editor(|initial| Ok(initial.to_string()));
    assert_eq!(app.notice.as_deref(), Some("no changes"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test editor_flow`
Expected: FAIL — notice stays `None` on all three paths today.

- [ ] **Step 3: Implement**

In `edit_in_editor`:

```rust
        let edited = match run_editor(&md) {
            Ok(text) => text,
            Err(e) => {
                self.notice = Some(format!("editor failed: {e}"));
                return;
            }
        };
        // ... apply_edited_markdown ...
        match applied {
            Ok(result) if !result.protected_missing.is_empty() => { /* unchanged confirm path */ }
            Ok(result) => {
                self.notice = Some(summarize_applied(&result));
                self.refresh_current_view();
            }
            Err(e) => {
                self.notice = Some(format!("edit failed: {e}"));
            }
        }
```

Add (free function in app.rs, unit-testable):

```rust
fn summarize_applied(a: &crate::markdown::Applied) -> String {
    let mut parts = Vec::new();
    if a.updated > 0 { parts.push(format!("{} updated", a.updated)); }
    if a.inserted > 0 { parts.push(format!("{} added", a.inserted)); }
    if a.deleted > 0 { parts.push(format!("{} deleted", a.deleted)); }
    if a.reordered > 0 { parts.push(format!("{} moved", a.reordered)); }
    if parts.is_empty() { "no changes".to_string() } else { format!("edited: {}", parts.join(" · ")) }
}
```

(Export `Applied` from `crate::markdown` if it isn't already re-exported — check `markdown/mod.rs`.) Apply the same `Err` treatment to `open_merge_editor`'s `let Ok(merged) = ... else { return }` (~app.rs:508): replace the silent `else` with `self.notice = Some(format!("editor failed: {e}")); return;` via a `match`. The confirm-protected path keeps its existing behavior (summary emitted after `confirm_delete_protected` completes is nice-to-have; skip it — YAGNI).

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS.

---

### Task 11: notion-tui — unclosed-fence detection with confirm gate

**Files:**
- Modify: `crates/notion-tui/src/markdown/parse.rs`, `crates/notion-tui/src/markdown/mod.rs`, `crates/notion-tui/src/app.rs`, `crates/notion-tui/src/ui/confirm.rs`
- Test: inline in `parse.rs`, extend `crates/notion-tui/tests/editor_flow.rs`

**Interfaces:**
- Consumes: Tasks 9–10 merged (same files, sequential track).
- Produces:
  - `parse.rs`: `pub fn parse_markdown_checked(md: &str) -> (Vec<ParsedLine>, Vec<ParseWarning>)` with `pub enum ParseWarning { UnclosedFence { line: usize } }`; `parse_markdown` becomes a thin wrapper discarding warnings (all existing callers unaffected).
  - `ui/confirm.rs`: `ConfirmState` gains `pub kind: ConfirmKind` with `pub enum ConfirmKind { DeleteProtected, ApplyDespiteWarnings }` (existing construction sites set `DeleteProtected`).
  - app.rs: `edit_in_editor` checks warnings BEFORE applying; on warnings it stores `pending_editor_text: Option<(String, Vec<Unit>, String)>` (page_id, units, edited text) and opens a confirm `"unclosed code fence at line N — everything after it becomes one code block. Apply anyway?"`; `y` applies (then the normal summary/protected logic runs), `n` discards with notice `edit discarded`.

- [ ] **Step 1: Write the failing tests**

Inline in `parse.rs`:

```rust
#[cfg(test)]
mod fence_tests {
    use super::*;

    #[test]
    fn detects_unclosed_fence_with_line_number() {
        let md = "hello\n```rust\nlet x = 1;";
        let (lines, warnings) = parse_markdown_checked(md);
        assert_eq!(lines.len(), 2); // paragraph + code block
        assert!(matches!(warnings[..], [ParseWarning::UnclosedFence { line: 2 }]));
    }

    #[test]
    fn closed_fence_produces_no_warning() {
        let (_, warnings) = parse_markdown_checked("```\ncode\n```\nafter");
        assert!(warnings.is_empty());
    }
}
```

In `tests/editor_flow.rs`:

```rust
#[test]
fn unclosed_fence_asks_for_confirmation_and_discard_keeps_page_intact() {
    let mut app = /* 2-paragraph page fixture */;
    app.edit_in_editor(|initial| Ok(format!("{initial}\n```\ntrailing")));
    assert!(app.confirm.is_some(), "must ask before applying a fence-swallowing edit");
    // Say no:
    /* drive the confirm 'n' key the same way the protected-block test does */
    assert_eq!(app.notice.as_deref(), Some("edit discarded"));
    // Store unchanged: no pending ops were enqueued.
    assert_eq!(app_store_ops_len(&app), 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui fence && cargo test -p notion-tui --test editor_flow unclosed`
Expected: FAIL — `parse_markdown_checked` missing.

- [ ] **Step 3: Implement**

`parse.rs` — restructure the fence branch to track termination and line numbers:

```rust
pub enum ParseWarning {
    UnclosedFence { line: usize },
}

pub fn parse_markdown_checked(md: &str) -> (Vec<ParsedLine>, Vec<ParseWarning>) {
    // same body as today's parse_markdown, but iterate with an explicit
    // line counter; in the fence branch, if the inner loop exhausts the
    // iterator without seeing the closing ``` push
    // ParseWarning::UnclosedFence { line: fence_start_line }.
}

pub fn parse_markdown(md: &str) -> Vec<ParsedLine> {
    parse_markdown_checked(md).0
}
```

`app.rs` `edit_in_editor`: after obtaining `edited`, call `parse_markdown_checked(&edited)`; if warnings are non-empty, set the confirm (kind `ApplyDespiteWarnings`, message built from the first warning) and stash `(page_id, units, edited)` in a new `pub pending_editor_text: Option<(String, Vec<crate::markdown::Unit>, String)>` field; return without applying. In the confirm-resolution key handling (find where `confirm_delete_protected` is dispatched on `y`/`n`), branch on `state.kind`: `ApplyDespiteWarnings` + `y` → take `pending_editor_text`, run the apply + summary logic (extract the post-editor half of `edit_in_editor` into a private `fn apply_editor_result(&mut self, page_id: String, units: Vec<Unit>, edited: String)` so both paths share it); `n` → drop it, `self.notice = Some("edit discarded".into())`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

---

### Task 12: integration sweep (after all tracks merge)

**Files:** none new.

- [ ] **Step 1: Full gates**

Run (from `notion-tui/`): `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green.

- [ ] **Step 2: Cross-task interaction checks**

- `cargo test -p notion-sync` — Task 2's fail-fast client + Task 4's inflight verification compose (push_verify tests exercise both).
- `cargo test -p notion-tui --test editor_flow --test refresh_state_flow --test property_form_flow` — the three app.rs tasks compose.
- `git -C /Users/rehatbir/Developer/fables status` — confirm only intended files changed; report the full file list.

- [ ] **Step 3: Manual smoke checklist (report, don't fix)**

If a configured token/db exists locally, run `cargo run -p notion-tui` briefly and verify: app starts, sync status cycles, a page opens, cursor survives a poll tick. Otherwise skip and note it. Report results; the session owner decides on commits.
