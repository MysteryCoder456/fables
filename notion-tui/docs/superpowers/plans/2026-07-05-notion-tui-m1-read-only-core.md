# notion-tui Milestone 1 (Read-Only Core) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A usable read-only Notion browser in the terminal: background sync pulls the workspace into SQLite; the TUI renders sidebar, pages, database tables, and instant search purely from the local store.

**Architecture:** Cargo workspace with four crates. `notion-api` is a thin typed client for the official Notion API (rate-limited, retrying). `notion-store` owns the SQLite schema (incl. FTS5 search index). `notion-sync` runs a background *puller* (full crawl, then incremental via search high-water mark). `notion-tui` is the ratatui binary that renders exclusively from the store and refreshes when sync notifies it.

**Tech Stack:** Rust stable (≥1.80), tokio, reqwest (rustls), serde/serde_json, rusqlite (bundled SQLite + FTS5), ratatui 0.29 + crossterm 0.28, wiremock + insta for tests.

**Spec:** `notion-tui/docs/superpowers/specs/2026-07-05-notion-tui-design.md` (this plan implements Milestone 1 only).

## Global Constraints

- All paths below are relative to `notion-tui/` inside the `fables` repo (`/home/pi/Developer/fables/notion-tui`). Run cargo commands from `notion-tui/`.
- Notion API version header is exactly `2025-09-03` on every request.
- Rate budget: minimum 334 ms between API requests (~3 req/s); retry max 5 attempts; honor `Retry-After` on 429.
- The TUI **never** performs network I/O; it reads only from `notion-store`.
- Milestone 1 is read-only: no `pending_ops` writes, no editing keys. The schema still includes the write-path tables (created empty) so later milestones don't need migrations.
- Read-only within M1 also means: puller may overwrite local rows freely (no dirty-flag checks yet — those arrive with writes in M2).
- Syntax highlighting (`syntect`) is deferred to Milestone 5 polish; M1 renders code blocks with a plain bordered style.
- Commit after every task (steps include the commands). Commit messages end with:
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`
- Workspace dependency versions (put in root `Cargo.toml [workspace.dependencies]`, crates reference with `{ workspace = true }`): tokio `1` (features `full`), serde `1` (derive), serde_json `1`, thiserror `2`, anyhow `1`, reqwest `0.12` (default-features off; features `json`, `rustls-tls`), rusqlite `0.32` (features `bundled`), ratatui `0.29`, crossterm `0.28` (features `event-stream`), futures `0.3`, dirs `5`, toml `0.8`, wiremock `0.6` (dev), insta `1` (dev), tempfile `3` (dev).

## File Structure

```
notion-tui/
├── Cargo.toml                        # workspace root
├── docs/superpowers/{specs,plans}/
└── crates/
    ├── notion-api/
    │   ├── Cargo.toml
    │   ├── src/lib.rs                # re-exports
    │   ├── src/error.rs              # ApiError
    │   ├── src/client.rs             # NotionClient: pacing, retry, request()
    │   ├── src/types.rs              # SearchItem, PageMeta, Block, rich_text_plain…
    │   ├── src/endpoints.rs          # search / blocks / data sources / pages
    │   └── tests/                    # wiremock integration tests
    ├── notion-store/
    │   ├── Cargo.toml
    │   ├── src/lib.rs
    │   ├── src/schema.rs             # SQL schema v1 + migrate()
    │   └── src/store.rs              # Store: records, upserts, queries, FTS search
    ├── notion-sync/
    │   ├── Cargo.toml
    │   ├── src/lib.rs                # SyncStatus, SyncHandle, spawn_puller
    │   └── src/puller.rs             # pull_once + poll loop
    └── notion-tui/
        ├── Cargo.toml
        ├── src/main.rs               # wire-up: config → store → client → sync → UI loop
        ├── src/config.rs             # token/db-path/poll-interval loading
        ├── src/terminal.rs           # TerminalGuard (panic-safe raw mode)
        ├── src/app.rs                # App state + handle_key dispatch
        ├── src/ui/mod.rs             # draw() layout: sidebar | main + status bar
        ├── src/ui/sidebar.rs         # workspace tree widget + state
        ├── src/ui/page.rs            # block-tree page view
        ├── src/ui/table.rs           # database table view
        ├── src/ui/search.rs          # search modal
        └── tests/e2e.rs              # wiremock → pull → render smoke test
```

---

### Task 1: Workspace scaffolding + `notion-api` client skeleton

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/notion-api/Cargo.toml`
- Create: `crates/notion-api/src/lib.rs`
- Create: `crates/notion-api/src/error.rs`
- Create: `crates/notion-api/src/client.rs`
- Test: `crates/notion-api/tests/client_headers.rs`

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `NotionClient::new(token) -> NotionClient`, `NotionClient::with_base_url(token, base_url)` (tests point this at wiremock), `client.get_json(path: &str) -> Result<serde_json::Value, ApiError>`, `client.post_json(path, &Value) -> Result<Value, ApiError>` (both `pub` so endpoint fns in Task 3/4 and tests can use them), `ApiError { Network, Api { status, code, message }, RetriesExhausted }`, `pub const NOTION_VERSION: &str = "2025-09-03"`.

- [ ] **Step 1: Create the workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = ["crates/notion-api"]

[workspace.dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
anyhow = "1"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }
rusqlite = { version = "0.32", features = ["bundled"] }
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
futures = "0.3"
dirs = "5"
toml = "0.8"
wiremock = "0.6"
insta = "1"
tempfile = "3"
```

- [ ] **Step 2: Create `crates/notion-api/Cargo.toml`**

```toml
[package]
name = "notion-api"
version = "0.1.0"
edition = "2021"

[dependencies]
reqwest = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }

[dev-dependencies]
wiremock = { workspace = true }
```

- [ ] **Step 3: Write the failing test**

`crates/notion-api/tests/client_headers.rs`:

```rust
use notion_api::NotionClient;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn sends_auth_and_version_headers() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .and(header("Authorization", "Bearer test-token"))
        .and(header("Notion-Version", "2025-09-03"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"object": "user"})))
        .expect(1)
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("test-token", server.uri());
    let v = client.get_json("/v1/users/me").await.unwrap();
    assert_eq!(v["object"], "user");
}

#[tokio::test]
async fn api_error_is_parsed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            serde_json::json!({"object": "error", "code": "object_not_found", "message": "Not found"}),
        ))
        .mount(&server)
        .await;

    let client = NotionClient::with_base_url("t", server.uri());
    let err = client.get_json("/v1/users/me").await.unwrap_err();
    match err {
        notion_api::ApiError::Api { status, code, message } => {
            assert_eq!(status, 404);
            assert_eq!(code, "object_not_found");
            assert_eq!(message, "Not found");
        }
        other => panic!("expected Api error, got {other:?}"),
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test -p notion-api`
Expected: compile error — `NotionClient` not found.

- [ ] **Step 5: Write the implementation**

`crates/notion-api/src/error.rs`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),
    #[error("notion api error {status} ({code}): {message}")]
    Api { status: u16, code: String, message: String },
    #[error("retries exhausted for {0}")]
    RetriesExhausted(String),
}
```

`crates/notion-api/src/client.rs` (retry/pacing arrive in Task 2 — keep this minimal):

```rust
use crate::error::ApiError;
use serde_json::Value;

pub const NOTION_VERSION: &str = "2025-09-03";

pub struct NotionClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl NotionClient {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base_url(token, "https://api.notion.com")
    }

    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
        }
    }

    pub async fn get_json(&self, path: &str) -> Result<Value, ApiError> {
        self.request(reqwest::Method::GET, path, None).await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ApiError> {
        self.request(reqwest::Method::POST, path, Some(body)).await
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        let mut req = self
            .http
            .request(method, format!("{}{}", self.base_url, path))
            .bearer_auth(&self.token)
            .header("Notion-Version", NOTION_VERSION);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            let v: Value = resp.json().await.unwrap_or_default();
            return Err(ApiError::Api {
                status: status.as_u16(),
                code: v["code"].as_str().unwrap_or("unknown").to_string(),
                message: v["message"].as_str().unwrap_or("").to_string(),
            });
        }
        Ok(resp.json().await?)
    }
}
```

`crates/notion-api/src/lib.rs`:

```rust
mod client;
mod error;

pub use client::{NotionClient, NOTION_VERSION};
pub use error::ApiError;
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p notion-api`
Expected: 2 passed.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(notion-tui): workspace scaffolding + notion-api client skeleton

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: `notion-api` rate limiting + retry/backoff

**Files:**
- Modify: `crates/notion-api/src/client.rs`
- Test: `crates/notion-api/tests/retry.rs`

**Interfaces:**
- Consumes: `NotionClient` internals from Task 1.
- Produces: same public API; new `client.set_timing(min_interval: Duration, backoff_base: Duration)` (test hook — production defaults 334 ms / 250 ms). Behavior contract: 429 retried honoring `Retry-After` seconds; 5xx and connect errors retried with backoff `backoff_base * 2^attempt`; ≥5 failed attempts → `ApiError::RetriesExhausted`; 4xx (non-429) fails immediately with `ApiError::Api`.

- [ ] **Step 1: Write the failing tests**

`crates/notion-api/tests/retry.rs`:

```rust
use std::time::{Duration, Instant};

use notion_api::NotionClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(50), Duration::from_millis(10));
    c
}

#[tokio::test]
async fn retries_429_honoring_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
        .up_to_n_times(1)
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server).await;

    let v = fast_client(server.uri()).get_json("/v1/x").await.unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn retries_5xx_then_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(502))
        .up_to_n_times(2)
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"ok": true})))
        .mount(&server).await;

    let v = fast_client(server.uri()).get_json("/v1/x").await.unwrap();
    assert_eq!(v["ok"], true);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server).await;

    let err = fast_client(server.uri()).get_json("/v1/x").await.unwrap_err();
    assert!(matches!(err, notion_api::ApiError::RetriesExhausted(_)));
    assert_eq!(server.received_requests().await.unwrap().len(), 5);
}

#[tokio::test]
async fn paces_consecutive_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/x"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server).await;

    let c = fast_client(server.uri()); // min_interval = 50ms
    let start = Instant::now();
    c.get_json("/v1/x").await.unwrap();
    c.get_json("/v1/x").await.unwrap();
    c.get_json("/v1/x").await.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(100), "3 calls must span >= 2 intervals");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-api --test retry`
Expected: compile error — `set_timing` not found.

- [ ] **Step 3: Implement pacing + retry in `client.rs`**

Replace the struct and `request` with:

```rust
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

const MAX_ATTEMPTS: u32 = 5;

pub struct NotionClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
    min_interval: Duration,
    backoff_base: Duration,
    next_allowed: Mutex<Instant>,
}

impl NotionClient {
    pub fn with_base_url(token: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token: token.into(),
            min_interval: Duration::from_millis(334),
            backoff_base: Duration::from_millis(250),
            next_allowed: Mutex::new(Instant::now()),
        }
    }

    /// Test hook: shrink pacing/backoff so retry tests run fast.
    pub fn set_timing(&mut self, min_interval: Duration, backoff_base: Duration) {
        self.min_interval = min_interval;
        self.backoff_base = backoff_base;
    }

    async fn pace(&self) {
        let mut next = self.next_allowed.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep_until(*next).await;
        }
        *next = Instant::now().max(*next) + self.min_interval;
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, ApiError> {
        for attempt in 0..MAX_ATTEMPTS {
            self.pace().await;
            let mut req = self
                .http
                .request(method.clone(), format!("{}{}", self.base_url, path))
                .bearer_auth(&self.token)
                .header("Notion-Version", NOTION_VERSION);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    if attempt + 1 == MAX_ATTEMPTS {
                        return Err(e.into());
                    }
                    tokio::time::sleep(self.backoff_base * 2u32.pow(attempt)).await;
                    continue;
                }
            };
            let status = resp.status();
            if status.as_u16() == 429 {
                let wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(Duration::from_secs)
                    .unwrap_or(self.backoff_base * 2u32.pow(attempt));
                tokio::time::sleep(wait).await;
                continue;
            }
            if status.is_server_error() {
                tokio::time::sleep(self.backoff_base * 2u32.pow(attempt)).await;
                continue;
            }
            if !status.is_success() {
                let v: Value = resp.json().await.unwrap_or_default();
                return Err(ApiError::Api {
                    status: status.as_u16(),
                    code: v["code"].as_str().unwrap_or("unknown").to_string(),
                    message: v["message"].as_str().unwrap_or("").to_string(),
                });
            }
            return Ok(resp.json().await?);
        }
        Err(ApiError::RetriesExhausted(path.to_string()))
    }
}
```

(`new`, `get_json`, `post_json` stay as in Task 1.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p notion-api`
Expected: all 6 tests pass (2 from Task 1 + 4 new).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(notion-api): request pacing, 429/5xx retry with backoff

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: `notion-api` search endpoint + shared rich-text helpers

**Files:**
- Create: `crates/notion-api/src/types.rs`
- Create: `crates/notion-api/src/endpoints.rs`
- Modify: `crates/notion-api/src/lib.rs`
- Test: `crates/notion-api/tests/search.rs`

**Interfaces:**
- Consumes: `NotionClient::post_json` from Task 1/2.
- Produces (used by Task 7 puller):
  - `pub fn rich_text_plain(v: &serde_json::Value) -> String` — concatenates `plain_text` of a rich-text array; empty string if not an array.
  - `pub enum ParentRef { Workspace, Page(String), DataSource(String), Database(String), Block(String), Unknown }` with `ParentRef::parse(&Value) -> ParentRef`.
  - `pub struct PageMeta { pub id: String, pub parent: ParentRef, pub title: String, pub icon: Option<String>, pub archived: bool, pub last_edited_time: String }`
  - `pub struct DataSourceMeta { pub id: String, pub database_id: String, pub title: String, pub last_edited_time: String }`
  - `pub enum SearchItem { Page(PageMeta), DataSource(DataSourceMeta), Other }`
  - `pub struct SearchPage { pub items: Vec<SearchItem>, pub next_cursor: Option<String> }`
  - `client.search_page(cursor: Option<&str>) -> Result<SearchPage, ApiError>` — POST `/v1/search`, sorted by `last_edited_time` descending.

- [ ] **Step 1: Write the failing tests**

`crates/notion-api/tests/search.rs`:

```rust
use notion_api::{NotionClient, SearchItem};
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn page_fixture(id: &str, title: &str, edited: &str) -> serde_json::Value {
    json!({
        "object": "page",
        "id": id,
        "archived": false,
        "last_edited_time": edited,
        "parent": {"type": "workspace", "workspace": true},
        "icon": {"type": "emoji", "emoji": "📄"},
        "properties": {
            "Name": {"id": "title", "type": "title",
                     "title": [{"type": "text", "plain_text": title}]}
        }
    })
}

#[tokio::test]
async fn search_paginates_and_parses() {
    let server = MockServer::start().await;
    // First page: sorted request, no cursor.
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(body_partial_json(json!({
            "sort": {"timestamp": "last_edited_time", "direction": "descending"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [page_fixture("p1", "First", "2026-07-05T10:00:00.000Z")],
            "has_more": true,
            "next_cursor": "c2"
        })))
        .up_to_n_times(1)
        .mount(&server).await;
    // Second page: with cursor c2, contains a data source.
    Mock::given(method("POST")).and(path("/v1/search"))
        .and(body_partial_json(json!({"start_cursor": "c2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{
                "object": "data_source",
                "id": "ds1",
                "last_edited_time": "2026-07-04T09:00:00.000Z",
                "parent": {"type": "database_id", "database_id": "db1"},
                "title": [{"type": "text", "plain_text": "Tasks"}]
            }],
            "has_more": false,
            "next_cursor": null
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());

    let page1 = client.search_page(None).await.unwrap();
    assert_eq!(page1.next_cursor.as_deref(), Some("c2"));
    match &page1.items[0] {
        SearchItem::Page(p) => {
            assert_eq!(p.id, "p1");
            assert_eq!(p.title, "First");
            assert_eq!(p.icon.as_deref(), Some("📄"));
            assert_eq!(p.last_edited_time, "2026-07-05T10:00:00.000Z");
        }
        other => panic!("expected page, got {other:?}"),
    }

    let page2 = client.search_page(Some("c2")).await.unwrap();
    assert!(page2.next_cursor.is_none());
    match &page2.items[0] {
        SearchItem::DataSource(ds) => {
            assert_eq!(ds.id, "ds1");
            assert_eq!(ds.database_id, "db1");
            assert_eq!(ds.title, "Tasks");
        }
        other => panic!("expected data source, got {other:?}"),
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-api --test search`
Expected: compile error — `search_page` / `SearchItem` not found.

- [ ] **Step 3: Implement `types.rs`**

```rust
use serde_json::Value;

/// Concatenate the `plain_text` of every element of a rich-text array.
pub fn rich_text_plain(v: &Value) -> String {
    v.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t["plain_text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentRef {
    Workspace,
    Page(String),
    DataSource(String),
    Database(String),
    Block(String),
    Unknown,
}

impl ParentRef {
    pub fn parse(v: &Value) -> ParentRef {
        let s = |key: &str| v[key].as_str().unwrap_or_default().to_string();
        match v["type"].as_str() {
            Some("workspace") => ParentRef::Workspace,
            Some("page_id") => ParentRef::Page(s("page_id")),
            Some("data_source_id") => ParentRef::DataSource(s("data_source_id")),
            Some("database_id") => ParentRef::Database(s("database_id")),
            Some("block_id") => ParentRef::Block(s("block_id")),
            _ => ParentRef::Unknown,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PageMeta {
    pub id: String,
    pub parent: ParentRef,
    pub title: String,
    pub icon: Option<String>,
    pub archived: bool,
    pub last_edited_time: String,
}

impl PageMeta {
    pub fn parse(v: &Value) -> PageMeta {
        // Title lives in whichever property has type == "title".
        let title = v["properties"]
            .as_object()
            .and_then(|props| {
                props
                    .values()
                    .find(|p| p["type"] == "title")
                    .map(|p| rich_text_plain(&p["title"]))
            })
            .unwrap_or_default();
        PageMeta {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            parent: ParentRef::parse(&v["parent"]),
            title,
            icon: v["icon"]["emoji"].as_str().map(str::to_string),
            archived: v["archived"].as_bool().unwrap_or(false),
            last_edited_time: v["last_edited_time"].as_str().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataSourceMeta {
    pub id: String,
    pub database_id: String,
    pub title: String,
    pub last_edited_time: String,
}

impl DataSourceMeta {
    pub fn parse(v: &Value) -> DataSourceMeta {
        let database_id = match ParentRef::parse(&v["parent"]) {
            ParentRef::Database(id) => id,
            _ => String::new(),
        };
        DataSourceMeta {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            database_id,
            title: rich_text_plain(&v["title"]),
            last_edited_time: v["last_edited_time"].as_str().unwrap_or_default().to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum SearchItem {
    Page(PageMeta),
    DataSource(DataSourceMeta),
    Other,
}

#[derive(Debug, Clone)]
pub struct SearchPage {
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<String>,
}
```

- [ ] **Step 4: Implement `endpoints.rs` (search only for now)**

```rust
use crate::client::NotionClient;
use crate::error::ApiError;
use crate::types::{DataSourceMeta, PageMeta, SearchItem, SearchPage};
use serde_json::{json, Value};

impl NotionClient {
    /// One page of workspace search, sorted by last_edited_time descending.
    pub async fn search_page(&self, cursor: Option<&str>) -> Result<SearchPage, ApiError> {
        let mut body = json!({
            "sort": {"timestamp": "last_edited_time", "direction": "descending"},
            "page_size": 100
        });
        if let Some(c) = cursor {
            body["start_cursor"] = json!(c);
        }
        let v = self.post_json("/v1/search", &body).await?;
        Ok(SearchPage {
            items: v["results"]
                .as_array()
                .map(|arr| arr.iter().map(parse_search_item).collect())
                .unwrap_or_default(),
            next_cursor: v["next_cursor"].as_str().map(str::to_string),
        })
    }
}

fn parse_search_item(v: &Value) -> SearchItem {
    match v["object"].as_str() {
        Some("page") => SearchItem::Page(PageMeta::parse(v)),
        Some("data_source") => SearchItem::DataSource(DataSourceMeta::parse(v)),
        _ => SearchItem::Other,
    }
}
```

Update `lib.rs`:

```rust
mod client;
mod endpoints;
mod error;
mod types;

pub use client::{NotionClient, NOTION_VERSION};
pub use error::ApiError;
pub use types::{
    rich_text_plain, DataSourceMeta, PageMeta, ParentRef, SearchItem, SearchPage,
};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p notion-api`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-api): search endpoint, page/data-source parsing

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: `notion-api` block trees, data-source schema + rows

**Files:**
- Modify: `crates/notion-api/src/types.rs`
- Modify: `crates/notion-api/src/endpoints.rs`
- Modify: `crates/notion-api/src/lib.rs`
- Test: `crates/notion-api/tests/blocks.rs`

**Interfaces:**
- Consumes: `get_json`/`post_json`, `rich_text_plain`, `PageMeta` from earlier tasks.
- Produces (used by Task 7 puller):
  - `pub struct Block { pub id: String, pub block_type: String, pub payload: serde_json::Value, pub plain_text: String, pub has_children: bool }` — `payload` is the type-specific object (e.g. the value under `"paragraph"`); `plain_text` extracted from `payload["rich_text"]`, or `payload["title"]` for `child_page`/`child_database`.
  - `pub struct FlatBlock { pub block: Block, pub parent_block_id: Option<String>, pub ordinal: i64 }` — `parent_block_id == None` means direct child of the page.
  - `client.fetch_block_tree(root_page_id: &str) -> Result<Vec<FlatBlock>, ApiError>` — breadth-first fetch of `/v1/blocks/{id}/children` (paginated), recursing into `has_children` blocks (but NOT into `child_page`/`child_database` — those are separate pages).
  - `pub struct DataSource { pub meta: DataSourceMeta, pub schema: serde_json::Value }` (`schema` = the `properties` map).
  - `client.get_data_source(id: &str) -> Result<DataSource, ApiError>` — GET `/v1/data_sources/{id}`.
  - `pub struct Row { pub id: String, pub properties: serde_json::Value, pub last_edited_time: String, pub archived: bool }`
  - `client.query_data_source_all(id: &str) -> Result<Vec<Row>, ApiError>` — POST `/v1/data_sources/{id}/query`, following cursors.

- [ ] **Step 1: Write the failing tests**

`crates/notion-api/tests/blocks.rs`:

```rust
use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn fetches_nested_block_tree() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/blocks/page1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"object": "block", "id": "b1", "type": "heading_1", "has_children": false,
                 "heading_1": {"rich_text": [{"plain_text": "Title"}]}},
                {"object": "block", "id": "b2", "type": "toggle", "has_children": true,
                 "toggle": {"rich_text": [{"plain_text": "More"}]}},
                {"object": "block", "id": "b3", "type": "child_page", "has_children": true,
                 "child_page": {"title": "Sub page"}}
            ],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/b2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [
                {"object": "block", "id": "b21", "type": "paragraph", "has_children": false,
                 "paragraph": {"rich_text": [{"plain_text": "hidden text"}]}}
            ],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());
    let flat = client.fetch_block_tree("page1").await.unwrap();

    assert_eq!(flat.len(), 4);
    assert_eq!(flat[0].block.id, "b1");
    assert_eq!(flat[0].block.plain_text, "Title");
    assert_eq!(flat[0].parent_block_id, None);
    assert_eq!(flat[0].ordinal, 0);

    let b21 = flat.iter().find(|f| f.block.id == "b21").unwrap();
    assert_eq!(b21.parent_block_id.as_deref(), Some("b2"));
    assert_eq!(b21.block.plain_text, "hidden text");

    // child_page has_children but must NOT be recursed into
    assert!(flat.iter().all(|f| f.parent_block_id.as_deref() != Some("b3")));
    let b3 = flat.iter().find(|f| f.block.id == "b3").unwrap();
    assert_eq!(b3.block.plain_text, "Sub page");
}

#[tokio::test]
async fn queries_data_source_with_pagination() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/data_sources/ds1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "data_source", "id": "ds1",
            "parent": {"type": "database_id", "database_id": "db1"},
            "last_edited_time": "2026-07-01T00:00:00.000Z",
            "title": [{"plain_text": "Tasks"}],
            "properties": {"Name": {"id": "title", "type": "title"},
                           "Done": {"id": "d1", "type": "checkbox"}}
        })))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/data_sources/ds1/query"))
        .and(body_partial_json(json!({"start_cursor": "c2"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r2", "archived": false,
                         "last_edited_time": "2026-07-01T00:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r1", "archived": false,
                         "last_edited_time": "2026-07-02T00:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {}}],
            "has_more": true, "next_cursor": "c2"
        })))
        .mount(&server).await;

    let client = NotionClient::with_base_url("t", server.uri());

    let ds = client.get_data_source("ds1").await.unwrap();
    assert_eq!(ds.meta.title, "Tasks");
    assert_eq!(ds.schema["Done"]["type"], "checkbox");

    let rows = client.query_data_source_all("ds1").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "r1");
    assert_eq!(rows[1].id, "r2");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-api --test blocks`
Expected: compile error — `fetch_block_tree` not found.

- [ ] **Step 3: Add types to `types.rs`**

```rust
#[derive(Debug, Clone)]
pub struct Block {
    pub id: String,
    pub block_type: String,
    pub payload: Value,
    pub plain_text: String,
    pub has_children: bool,
}

impl Block {
    pub fn parse(v: &Value) -> Block {
        let block_type = v["type"].as_str().unwrap_or("unsupported").to_string();
        let payload = v[&block_type].clone();
        let plain_text = match block_type.as_str() {
            "child_page" | "child_database" => {
                payload["title"].as_str().unwrap_or_default().to_string()
            }
            _ => rich_text_plain(&payload["rich_text"]),
        };
        Block {
            id: v["id"].as_str().unwrap_or_default().to_string(),
            block_type,
            payload,
            plain_text,
            has_children: v["has_children"].as_bool().unwrap_or(false),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FlatBlock {
    pub block: Block,
    pub parent_block_id: Option<String>,
    pub ordinal: i64,
}

#[derive(Debug, Clone)]
pub struct DataSource {
    pub meta: DataSourceMeta,
    pub schema: Value,
}

#[derive(Debug, Clone)]
pub struct Row {
    pub id: String,
    pub properties: Value,
    pub last_edited_time: String,
    pub archived: bool,
}
```

- [ ] **Step 4: Add endpoints to `endpoints.rs`**

```rust
use crate::types::{Block, DataSource, FlatBlock, Row};
use std::collections::VecDeque;

impl NotionClient {
    pub async fn fetch_block_tree(&self, root_page_id: &str) -> Result<Vec<FlatBlock>, ApiError> {
        let mut out = Vec::new();
        // (container id to list children of, parent_block_id recorded on those children)
        let mut queue: VecDeque<(String, Option<String>)> = VecDeque::new();
        queue.push_back((root_page_id.to_string(), None));

        while let Some((container, parent_block_id)) = queue.pop_front() {
            let mut cursor: Option<String> = None;
            let mut ordinal: i64 = 0;
            loop {
                let mut path = format!("/v1/blocks/{container}/children?page_size=100");
                if let Some(c) = &cursor {
                    path.push_str(&format!("&start_cursor={c}"));
                }
                let v = self.get_json(&path).await?;
                for item in v["results"].as_array().into_iter().flatten() {
                    let block = Block::parse(item);
                    let recurse = block.has_children
                        && block.block_type != "child_page"
                        && block.block_type != "child_database";
                    if recurse {
                        queue.push_back((block.id.clone(), Some(block.id.clone())));
                    }
                    out.push(FlatBlock {
                        block,
                        parent_block_id: parent_block_id.clone(),
                        ordinal,
                    });
                    ordinal += 1;
                }
                cursor = v["next_cursor"].as_str().map(str::to_string);
                if cursor.is_none() {
                    break;
                }
            }
        }
        Ok(out)
    }

    pub async fn get_data_source(&self, id: &str) -> Result<DataSource, ApiError> {
        let v = self.get_json(&format!("/v1/data_sources/{id}")).await?;
        Ok(DataSource {
            meta: crate::types::DataSourceMeta::parse(&v),
            schema: v["properties"].clone(),
        })
    }

    pub async fn query_data_source_all(&self, id: &str) -> Result<Vec<Row>, ApiError> {
        let mut rows = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut body = serde_json::json!({"page_size": 100});
            if let Some(c) = &cursor {
                body["start_cursor"] = serde_json::json!(c);
            }
            let v = self.post_json(&format!("/v1/data_sources/{id}/query"), &body).await?;
            for item in v["results"].as_array().into_iter().flatten() {
                rows.push(Row {
                    id: item["id"].as_str().unwrap_or_default().to_string(),
                    properties: item["properties"].clone(),
                    last_edited_time: item["last_edited_time"]
                        .as_str().unwrap_or_default().to_string(),
                    archived: item["archived"].as_bool().unwrap_or(false),
                });
            }
            cursor = v["next_cursor"].as_str().map(str::to_string);
            if cursor.is_none() {
                return Ok(rows);
            }
        }
    }
}
```

Add to `lib.rs` exports: `Block, DataSource, FlatBlock, Row`.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p notion-api`
Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-api): block tree fetch, data source schema + row query

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: `notion-store` schema, migrations, FTS5

**Files:**
- Create: `crates/notion-store/Cargo.toml`
- Create: `crates/notion-store/src/lib.rs`
- Create: `crates/notion-store/src/schema.rs`
- Create: `crates/notion-store/src/store.rs`
- Modify: `Cargo.toml` (add workspace member)
- Test: inline `#[cfg(test)]` in `schema.rs`

**Interfaces:**
- Consumes: nothing from other crates (store is standalone; sync maps API types → records).
- Produces: `Store::open(path: &Path) -> anyhow::Result<Store>`, `Store::open_in_memory() -> anyhow::Result<Store>`. Schema v1 tables: `pages`, `blocks`, `data_sources`, `rows`, `comments`, `pending_ops`, `sync_meta`, FTS5 table `fts(content, page_id, block_id)` kept in sync by triggers on `blocks` and `pages`. `PRAGMA user_version` tracks schema version.

- [ ] **Step 1: Create `crates/notion-store/Cargo.toml`, add member**

```toml
[package]
name = "notion-store"
version = "0.1.0"
edition = "2021"

[dependencies]
rusqlite = { workspace = true }
anyhow = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
```

Root `Cargo.toml`: `members = ["crates/notion-api", "crates/notion-store"]`.

- [ ] **Step 2: Write the failing tests (inline in `schema.rs`)**

```rust
#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn migrates_fresh_db_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("n.db");
        {
            let s = Store::open(&path).unwrap();
            let v: i64 = s.conn().pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
            assert_eq!(v, 1);
        }
        // Re-open: must not fail or re-run migrations.
        let s = Store::open(&path).unwrap();
        let n: i64 = s.conn()
            .query_row("SELECT count(*) FROM sqlite_master WHERE name = 'pages'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn fts_triggers_index_blocks_and_pages() {
        let s = Store::open_in_memory().unwrap();
        s.conn().execute(
            "INSERT INTO pages (id, parent_type, parent_id, title, last_edited_time)
             VALUES ('p1', 'workspace', NULL, 'Meeting notes', 't1')", []).unwrap();
        s.conn().execute(
            "INSERT INTO blocks (id, page_id, parent_block_id, ordinal, block_type, payload, plain_text)
             VALUES ('b1', 'p1', NULL, 0, 'paragraph', '{}', 'quarterly roadmap discussion')", []).unwrap();

        let hit: String = s.conn()
            .query_row("SELECT page_id FROM fts WHERE fts MATCH 'roadmap'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(hit, "p1");
        let title_hit: String = s.conn()
            .query_row("SELECT page_id FROM fts WHERE fts MATCH 'meeting'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(title_hit, "p1");

        // Update re-indexes; delete removes.
        s.conn().execute("UPDATE blocks SET plain_text = 'budget review' WHERE id = 'b1'", []).unwrap();
        let n: i64 = s.conn()
            .query_row("SELECT count(*) FROM fts WHERE fts MATCH 'roadmap'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
        s.conn().execute("DELETE FROM blocks WHERE id = 'b1'", []).unwrap();
        let n: i64 = s.conn()
            .query_row("SELECT count(*) FROM fts WHERE fts MATCH 'budget'", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p notion-store`
Expected: compile error — `Store` not found.

- [ ] **Step 4: Implement `schema.rs`, minimal `store.rs`, `lib.rs`**

`src/schema.rs`:

```rust
use rusqlite::Connection;

const SCHEMA_V1: &str = r#"
CREATE TABLE pages (
    id TEXT PRIMARY KEY,
    parent_type TEXT NOT NULL,
    parent_id TEXT,
    title TEXT NOT NULL DEFAULT '',
    icon TEXT,
    archived INTEGER NOT NULL DEFAULT 0,
    last_edited_time TEXT NOT NULL,
    local_edited_at TEXT,
    dirty INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE blocks (
    id TEXT PRIMARY KEY,
    page_id TEXT NOT NULL,
    parent_block_id TEXT,
    ordinal INTEGER NOT NULL,
    block_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    plain_text TEXT NOT NULL DEFAULT '',
    has_children INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_blocks_page ON blocks(page_id, parent_block_id, ordinal);
CREATE TABLE data_sources (
    id TEXT PRIMARY KEY,
    database_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    schema_json TEXT NOT NULL,
    last_edited_time TEXT NOT NULL DEFAULT ''
);
CREATE TABLE rows (
    id TEXT PRIMARY KEY,
    data_source_id TEXT NOT NULL,
    properties TEXT NOT NULL,
    last_edited_time TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    dirty INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_rows_ds ON rows(data_source_id);
CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    parent_id TEXT NOT NULL,
    parent_kind TEXT NOT NULL,
    thread_id TEXT,
    author TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    created_time TEXT NOT NULL DEFAULT ''
);
CREATE TABLE pending_ops (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    op_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    base_edited_time TEXT,
    state TEXT NOT NULL DEFAULT 'pending',
    error TEXT
);
CREATE TABLE sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);

CREATE VIRTUAL TABLE fts USING fts5(content, page_id UNINDEXED, block_id UNINDEXED);

CREATE TRIGGER blocks_fts_ai AFTER INSERT ON blocks BEGIN
    INSERT INTO fts(content, page_id, block_id) VALUES (new.plain_text, new.page_id, new.id);
END;
CREATE TRIGGER blocks_fts_ad AFTER DELETE ON blocks BEGIN
    DELETE FROM fts WHERE block_id = old.id;
END;
CREATE TRIGGER blocks_fts_au AFTER UPDATE ON blocks BEGIN
    DELETE FROM fts WHERE block_id = old.id;
    INSERT INTO fts(content, page_id, block_id) VALUES (new.plain_text, new.page_id, new.id);
END;
CREATE TRIGGER pages_fts_ai AFTER INSERT ON pages BEGIN
    INSERT INTO fts(content, page_id, block_id) VALUES (new.title, new.id, NULL);
END;
CREATE TRIGGER pages_fts_ad AFTER DELETE ON pages BEGIN
    DELETE FROM fts WHERE page_id = old.id AND block_id IS NULL;
END;
CREATE TRIGGER pages_fts_au AFTER UPDATE OF title ON pages BEGIN
    DELETE FROM fts WHERE page_id = old.id AND block_id IS NULL;
    INSERT INTO fts(content, page_id, block_id) VALUES (new.title, new.id, NULL);
END;
"#;

pub fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        conn.pragma_update(None, "user_version", 1)?;
    }
    Ok(())
}
```

`src/store.rs` (grows in Task 6):

```rust
use rusqlite::Connection;
use std::path::Path;

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        crate::schema::migrate(&conn)?;
        Ok(Store { conn })
    }

    pub fn open_in_memory() -> anyhow::Result<Store> {
        let conn = Connection::open_in_memory()?;
        crate::schema::migrate(&conn)?;
        Ok(Store { conn })
    }

    /// Raw connection access (tests and internal use).
    pub fn conn(&self) -> &Connection {
        &self.conn
    }
}
```

`src/lib.rs`:

```rust
mod schema;
mod store;

pub use store::Store;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p notion-store`
Expected: 2 passed. (This also proves the bundled SQLite has FTS5 — if the `CREATE VIRTUAL TABLE` fails, switch the rusqlite feature to `bundled-full` before proceeding.)

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-store): schema v1 with FTS5 index and triggers

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: `notion-store` records, upserts, queries, search

**Files:**
- Modify: `crates/notion-store/src/store.rs`
- Modify: `crates/notion-store/src/lib.rs`
- Test: `crates/notion-store/tests/store_api.rs`

**Interfaces:**
- Consumes: `Store` from Task 5.
- Produces (used by Tasks 7, 9–12):

```rust
pub struct PageRec { pub id: String, pub parent_type: String, pub parent_id: Option<String>,
    pub title: String, pub icon: Option<String>, pub archived: bool, pub last_edited_time: String }
pub struct BlockRec { pub id: String, pub page_id: String, pub parent_block_id: Option<String>,
    pub ordinal: i64, pub block_type: String, pub payload: String, pub plain_text: String,
    pub has_children: bool }
pub struct DataSourceRec { pub id: String, pub database_id: String, pub title: String,
    pub schema_json: String, pub last_edited_time: String }
pub struct RowRec { pub id: String, pub data_source_id: String, pub properties: String,
    pub last_edited_time: String, pub archived: bool }
pub enum NodeKind { Page, DataSource }
pub struct TreeNode { pub id: String, pub title: String, pub parent_id: Option<String>,
    pub kind: NodeKind }
pub struct SearchHit { pub page_id: String, pub title: String, pub snippet: String }

impl Store {
    pub fn upsert_page(&self, p: &PageRec) -> anyhow::Result<()>;
    pub fn replace_page_blocks(&mut self, page_id: &str, blocks: &[BlockRec]) -> anyhow::Result<()>; // one tx: delete old, insert new
    pub fn upsert_data_source(&self, ds: &DataSourceRec) -> anyhow::Result<()>;
    pub fn replace_rows(&mut self, data_source_id: &str, rows: &[RowRec]) -> anyhow::Result<()>;
    pub fn get_page(&self, id: &str) -> anyhow::Result<Option<PageRec>>;
    pub fn page_blocks(&self, page_id: &str) -> anyhow::Result<Vec<BlockRec>>; // ORDER BY parent_block_id NULLS FIRST, ordinal
    pub fn sidebar_nodes(&self) -> anyhow::Result<Vec<TreeNode>>; // non-archived pages NOT in a data source + all data sources
    pub fn get_data_source(&self, id: &str) -> anyhow::Result<Option<DataSourceRec>>;
    pub fn rows(&self, data_source_id: &str) -> anyhow::Result<Vec<RowRec>>;
    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>>; // FTS, max 50, best-rank first
    pub fn meta_get(&self, key: &str) -> anyhow::Result<Option<String>>;
    pub fn meta_set(&self, key: &str, value: &str) -> anyhow::Result<()>;
}
```

Sidebar rule: a page appears in the sidebar tree when `parent_type IN ('workspace','page_id')`; rows (`parent_type = 'data_source_id'`) are reachable only through their data source. `TreeNode.parent_id` for a data source is its `database_id`'s parent page if the database is a `child_database` block — M1 simplification: data sources are listed as top-level siblings under a synthetic "Databases" section by the sidebar UI (Task 9), so here `parent_id = None` for data sources.

- [ ] **Step 1: Write the failing tests**

`crates/notion-store/tests/store_api.rs`:

```rust
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec, Store};

fn page(id: &str, parent_type: &str, parent_id: Option<&str>, title: &str) -> PageRec {
    PageRec {
        id: id.into(), parent_type: parent_type.into(),
        parent_id: parent_id.map(Into::into), title: title.into(),
        icon: None, archived: false, last_edited_time: "t1".into(),
    }
}

fn block(id: &str, page_id: &str, ordinal: i64, text: &str) -> BlockRec {
    BlockRec {
        id: id.into(), page_id: page_id.into(), parent_block_id: None, ordinal,
        block_type: "paragraph".into(), payload: "{}".into(),
        plain_text: text.into(), has_children: false,
    }
}

#[test]
fn page_roundtrip_and_upsert_overwrites() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None, "Old title")).unwrap();
    s.upsert_page(&page("p1", "workspace", None, "New title")).unwrap();
    let got = s.get_page("p1").unwrap().unwrap();
    assert_eq!(got.title, "New title");
}

#[test]
fn replace_page_blocks_swaps_content() {
    let mut s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None, "P")).unwrap();
    s.replace_page_blocks("p1", &[block("b1", "p1", 0, "one")]).unwrap();
    s.replace_page_blocks("p1", &[block("b2", "p1", 0, "two")]).unwrap();
    let blocks = s.page_blocks("p1").unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].id, "b2");
    // FTS reflects the replacement.
    assert!(s.search("one").unwrap().is_empty());
    assert_eq!(s.search("two").unwrap()[0].page_id, "p1");
}

#[test]
fn sidebar_excludes_rows_and_archived() {
    let s = Store::open_in_memory().unwrap();
    s.upsert_page(&page("p1", "workspace", None, "Top")).unwrap();
    s.upsert_page(&page("p2", "page_id", Some("p1"), "Child")).unwrap();
    s.upsert_page(&page("r1", "data_source_id", Some("ds1"), "Row page")).unwrap();
    let mut archived = page("p3", "workspace", None, "Gone");
    archived.archived = true;
    s.upsert_page(&archived).unwrap();
    s.upsert_data_source(&DataSourceRec {
        id: "ds1".into(), database_id: "db1".into(), title: "Tasks".into(),
        schema_json: "{}".into(), last_edited_time: "t1".into(),
    }).unwrap();

    let nodes = s.sidebar_nodes().unwrap();
    let ids: Vec<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    assert!(ids.contains(&"p1") && ids.contains(&"p2") && ids.contains(&"ds1"));
    assert!(!ids.contains(&"r1") && !ids.contains(&"p3"));
}

#[test]
fn rows_and_meta_roundtrip() {
    let mut s = Store::open_in_memory().unwrap();
    s.replace_rows("ds1", &[RowRec {
        id: "r1".into(), data_source_id: "ds1".into(),
        properties: r#"{"Name":{"type":"title","title":[{"plain_text":"Buy milk"}]}}"#.into(),
        last_edited_time: "t1".into(), archived: false,
    }]).unwrap();
    assert_eq!(s.rows("ds1").unwrap().len(), 1);

    assert_eq!(s.meta_get("hwm").unwrap(), None);
    s.meta_set("hwm", "2026-07-05").unwrap();
    s.meta_set("hwm", "2026-07-06").unwrap();
    assert_eq!(s.meta_get("hwm").unwrap().as_deref(), Some("2026-07-06"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-store --test store_api`
Expected: compile error — records not found.

- [ ] **Step 3: Implement records + methods in `store.rs`**

```rust
#[derive(Debug, Clone)]
pub struct PageRec {
    pub id: String,
    pub parent_type: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub icon: Option<String>,
    pub archived: bool,
    pub last_edited_time: String,
}

#[derive(Debug, Clone)]
pub struct BlockRec {
    pub id: String,
    pub page_id: String,
    pub parent_block_id: Option<String>,
    pub ordinal: i64,
    pub block_type: String,
    pub payload: String,
    pub plain_text: String,
    pub has_children: bool,
}

#[derive(Debug, Clone)]
pub struct DataSourceRec {
    pub id: String,
    pub database_id: String,
    pub title: String,
    pub schema_json: String,
    pub last_edited_time: String,
}

#[derive(Debug, Clone)]
pub struct RowRec {
    pub id: String,
    pub data_source_id: String,
    pub properties: String,
    pub last_edited_time: String,
    pub archived: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind { Page, DataSource }

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub id: String,
    pub title: String,
    pub parent_id: Option<String>,
    pub kind: NodeKind,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub page_id: String,
    pub title: String,
    pub snippet: String,
}

impl Store {
    pub fn upsert_page(&self, p: &PageRec) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO pages (id, parent_type, parent_id, title, icon, archived, last_edited_time)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET parent_type=?2, parent_id=?3, title=?4, icon=?5,
                                           archived=?6, last_edited_time=?7",
            rusqlite::params![p.id, p.parent_type, p.parent_id, p.title, p.icon,
                              p.archived as i64, p.last_edited_time],
        )?;
        Ok(())
    }

    pub fn replace_page_blocks(&mut self, page_id: &str, blocks: &[BlockRec]) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM blocks WHERE page_id = ?1", [page_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO blocks (id, page_id, parent_block_id, ordinal, block_type,
                                     payload, plain_text, has_children)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for b in blocks {
                stmt.execute(rusqlite::params![
                    b.id, b.page_id, b.parent_block_id, b.ordinal, b.block_type,
                    b.payload, b.plain_text, b.has_children as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn upsert_data_source(&self, ds: &DataSourceRec) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO data_sources (id, database_id, title, schema_json, last_edited_time)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET database_id=?2, title=?3, schema_json=?4,
                                           last_edited_time=?5",
            rusqlite::params![ds.id, ds.database_id, ds.title, ds.schema_json, ds.last_edited_time],
        )?;
        Ok(())
    }

    pub fn replace_rows(&mut self, data_source_id: &str, rows: &[RowRec]) -> anyhow::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM rows WHERE data_source_id = ?1", [data_source_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for r in rows {
                stmt.execute(rusqlite::params![
                    r.id, r.data_source_id, r.properties, r.last_edited_time, r.archived as i64
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn get_page(&self, id: &str) -> anyhow::Result<Option<PageRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, parent_type, parent_id, title, icon, archived, last_edited_time
             FROM pages WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(PageRec {
                id: r.get(0)?, parent_type: r.get(1)?, parent_id: r.get(2)?,
                title: r.get(3)?, icon: r.get(4)?,
                archived: r.get::<_, i64>(5)? != 0, last_edited_time: r.get(6)?,
            }),
            None => None,
        })
    }

    pub fn page_blocks(&self, page_id: &str) -> anyhow::Result<Vec<BlockRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, page_id, parent_block_id, ordinal, block_type, payload, plain_text, has_children
             FROM blocks WHERE page_id = ?1
             ORDER BY parent_block_id IS NOT NULL, parent_block_id, ordinal",
        )?;
        let out = stmt.query_map([page_id], |r| {
            Ok(BlockRec {
                id: r.get(0)?, page_id: r.get(1)?, parent_block_id: r.get(2)?,
                ordinal: r.get(3)?, block_type: r.get(4)?, payload: r.get(5)?,
                plain_text: r.get(6)?, has_children: r.get::<_, i64>(7)? != 0,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn sidebar_nodes(&self) -> anyhow::Result<Vec<TreeNode>> {
        let mut out = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT id, title, parent_id FROM pages
             WHERE archived = 0 AND parent_type IN ('workspace', 'page_id')
             ORDER BY title COLLATE NOCASE",
        )?;
        let pages = stmt.query_map([], |r| {
            Ok(TreeNode { id: r.get(0)?, title: r.get(1)?, parent_id: r.get(2)?, kind: NodeKind::Page })
        })?;
        for n in pages { out.push(n?); }
        let mut stmt = self.conn.prepare(
            "SELECT id, title FROM data_sources ORDER BY title COLLATE NOCASE",
        )?;
        let sources = stmt.query_map([], |r| {
            Ok(TreeNode { id: r.get(0)?, title: r.get(1)?, parent_id: None, kind: NodeKind::DataSource })
        })?;
        for n in sources { out.push(n?); }
        Ok(out)
    }

    pub fn get_data_source(&self, id: &str) -> anyhow::Result<Option<DataSourceRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, database_id, title, schema_json, last_edited_time
             FROM data_sources WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(DataSourceRec {
                id: r.get(0)?, database_id: r.get(1)?, title: r.get(2)?,
                schema_json: r.get(3)?, last_edited_time: r.get(4)?,
            }),
            None => None,
        })
    }

    pub fn rows(&self, data_source_id: &str) -> anyhow::Result<Vec<RowRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, data_source_id, properties, last_edited_time, archived
             FROM rows WHERE data_source_id = ?1 AND archived = 0",
        )?;
        let out = stmt.query_map([data_source_id], |r| {
            Ok(RowRec {
                id: r.get(0)?, data_source_id: r.get(1)?, properties: r.get(2)?,
                last_edited_time: r.get(3)?, archived: r.get::<_, i64>(4)? != 0,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn search(&self, query: &str) -> anyhow::Result<Vec<SearchHit>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        // Quote the query so user input can't hit FTS syntax errors; suffix * for prefix match.
        let fts_query = format!("\"{}\"*", query.replace('"', " "));
        let mut stmt = self.conn.prepare(
            "SELECT f.page_id, COALESCE(p.title, ''), snippet(fts, 0, '', '', '…', 12)
             FROM fts f JOIN pages p ON p.id = f.page_id
             WHERE fts MATCH ?1 ORDER BY rank LIMIT 50",
        )?;
        let out = stmt.query_map([&fts_query], |r| {
            Ok(SearchHit { page_id: r.get(0)?, title: r.get(1)?, snippet: r.get(2)? })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(out)
    }

    pub fn meta_get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM sync_meta WHERE key = ?1")?;
        let mut rows = stmt.query([key])?;
        Ok(match rows.next()? { Some(r) => Some(r.get(0)?), None => None })
    }

    pub fn meta_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO sync_meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }
}
```

`lib.rs` exports: `pub use store::{BlockRec, DataSourceRec, NodeKind, PageRec, RowRec, SearchHit, Store, TreeNode};`

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p notion-store`
Expected: all pass (Task 5's + 4 new).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(notion-store): records, upserts, sidebar/page/row queries, FTS search

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 7: `notion-sync` puller (first crawl + incremental)

**Files:**
- Create: `crates/notion-sync/Cargo.toml`
- Create: `crates/notion-sync/src/lib.rs`
- Create: `crates/notion-sync/src/puller.rs`
- Modify: `Cargo.toml` (add workspace member)
- Test: `crates/notion-sync/tests/pull.rs`

**Interfaces:**
- Consumes: `NotionClient` + `search_page`/`fetch_block_tree`/`get_data_source`/`query_data_source_all`, `SearchItem`, `ParentRef`, `FlatBlock` (notion-api); `Store` + records + `meta_get`/`meta_set` (notion-store).
- Produces (used by Task 12 wire-up and Task 8 status bar):

```rust
pub type SharedStore = std::sync::Arc<std::sync::Mutex<notion_store::Store>>;

#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    Starting,
    Syncing { done: u32 },
    Idle { updated: u32 },
    Offline,
    Failed(String),
}

pub struct SyncHandle {
    pub status: tokio::sync::watch::Receiver<SyncStatus>,
    pub data_version: tokio::sync::watch::Receiver<u64>, // bumped when pull changed data
}

pub fn spawn_puller(client: notion_api::NotionClient, store: SharedStore,
                    interval: std::time::Duration) -> SyncHandle;

/// One pull cycle; returns number of updated items. Public for tests and wire-up.
pub async fn pull_once(client: &notion_api::NotionClient, store: &SharedStore)
    -> Result<u32, notion_api::ApiError>;
```

High-water mark: `sync_meta["hwm"]` holds the largest `last_edited_time` (ISO-8601 strings compare lexicographically). `pull_once` walks search results (sorted descending) and stops at the first item with `last_edited_time <= hwm`. Page items → `upsert_page` + `fetch_block_tree` → `replace_page_blocks`. DataSource items → `get_data_source` → `upsert_data_source` + `query_data_source_all` → `replace_rows`. Mapping `ParentRef` → (`parent_type`, `parent_id`): `Workspace → ("workspace", None)`, `Page(id) → ("page_id", Some(id))`, `DataSource(id) → ("data_source_id", Some(id))`, `Database(id) → ("database_id", Some(id))`, `Block(id) → ("page_id", Some(id))` (attach to nearest page semantics deferred), `Unknown → ("unknown", None)`.

- [ ] **Step 1: Create `crates/notion-sync/Cargo.toml`, add member**

```toml
[package]
name = "notion-sync"
version = "0.1.0"
edition = "2021"

[dependencies]
notion-api = { path = "../notion-api" }
notion-store = { path = "../notion-store" }
tokio = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
wiremock = { workspace = true }
serde_json = { workspace = true }
```

Root members: add `"crates/notion-sync"`.

- [ ] **Step 2: Write the failing tests**

`crates/notion-sync/tests/pull.rs`:

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::{pull_once, SharedStore};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn search_body(results: serde_json::Value) -> serde_json::Value {
    json!({"results": results, "has_more": false, "next_cursor": null})
}

fn page_json(id: &str, title: &str, edited: &str) -> serde_json::Value {
    json!({"object": "page", "id": id, "archived": false, "last_edited_time": edited,
           "parent": {"type": "workspace", "workspace": true},
           "properties": {"Name": {"type": "title", "title": [{"plain_text": title}]}}})
}

fn empty_children() -> serde_json::Value {
    json!({"results": [], "has_more": false, "next_cursor": null})
}

fn fast_client(uri: String) -> NotionClient {
    let mut c = NotionClient::with_base_url("t", uri);
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    c
}

#[tokio::test]
async fn first_crawl_stores_pages_blocks_and_hwm() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            page_json("p1", "Newest", "2026-07-05T10:00:00.000Z"),
            page_json("p2", "Older", "2026-07-04T10:00:00.000Z"),
        ]))))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph",
                         "has_children": false,
                         "paragraph": {"rich_text": [{"plain_text": "hello world"}]}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p2/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(empty_children()))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let client = fast_client(server.uri());

    let updated = pull_once(&client, &store).await.unwrap();
    assert_eq!(updated, 2);

    let s = store.lock().unwrap();
    assert_eq!(s.get_page("p1").unwrap().unwrap().title, "Newest");
    assert_eq!(s.page_blocks("p1").unwrap().len(), 1);
    assert_eq!(s.meta_get("hwm").unwrap().as_deref(), Some("2026-07-05T10:00:00.000Z"));
    assert_eq!(s.search("hello").unwrap()[0].page_id, "p1");
}

#[tokio::test]
async fn incremental_pull_skips_unchanged() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            page_json("p1", "Same", "2026-07-05T10:00:00.000Z"),
        ]))))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store.lock().unwrap().meta_set("hwm", "2026-07-05T10:00:00.000Z").unwrap();

    let updated = pull_once(&fast_client(server.uri()), &store).await.unwrap();
    assert_eq!(updated, 0);
    // Only the search request went out — no block fetches.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn data_source_pull_stores_schema_and_rows() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([
            {"object": "data_source", "id": "ds1",
             "last_edited_time": "2026-07-05T10:00:00.000Z",
             "parent": {"type": "database_id", "database_id": "db1"},
             "title": [{"plain_text": "Tasks"}]}
        ]))))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/data_sources/ds1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "data_source", "id": "ds1",
            "parent": {"type": "database_id", "database_id": "db1"},
            "last_edited_time": "2026-07-05T10:00:00.000Z",
            "title": [{"plain_text": "Tasks"}],
            "properties": {"Name": {"type": "title"}}
        })))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/data_sources/ds1/query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "r1", "archived": false,
                         "last_edited_time": "2026-07-05T09:00:00.000Z",
                         "parent": {"type": "data_source_id", "data_source_id": "ds1"},
                         "properties": {"Name": {"type": "title",
                                                 "title": [{"plain_text": "Buy milk"}]}}}],
            "has_more": false, "next_cursor": null
        })))
        .mount(&server).await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    pull_once(&fast_client(server.uri()), &store).await.unwrap();

    let s = store.lock().unwrap();
    assert_eq!(s.get_data_source("ds1").unwrap().unwrap().title, "Tasks");
    assert_eq!(s.rows("ds1").unwrap().len(), 1);
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p notion-sync`
Expected: compile error — crate/functions not found.

- [ ] **Step 4: Implement `puller.rs` + `lib.rs`**

`src/puller.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use notion_api::{ApiError, NotionClient, ParentRef, SearchItem};
use notion_store::{BlockRec, DataSourceRec, PageRec, RowRec};
use tokio::sync::watch;

use crate::{SharedStore, SyncHandle, SyncStatus};

fn parent_cols(p: &ParentRef) -> (String, Option<String>) {
    match p {
        ParentRef::Workspace => ("workspace".into(), None),
        ParentRef::Page(id) => ("page_id".into(), Some(id.clone())),
        ParentRef::DataSource(id) => ("data_source_id".into(), Some(id.clone())),
        ParentRef::Database(id) => ("database_id".into(), Some(id.clone())),
        ParentRef::Block(id) => ("page_id".into(), Some(id.clone())),
        ParentRef::Unknown => ("unknown".into(), None),
    }
}

pub async fn pull_once(client: &NotionClient, store: &SharedStore) -> Result<u32, ApiError> {
    let hwm = store.lock().unwrap().meta_get("hwm").ok().flatten().unwrap_or_default();
    let mut max_seen = hwm.clone();
    let mut updated: u32 = 0;
    let mut cursor: Option<String> = None;
    let mut done = false;

    while !done {
        let page = client.search_page(cursor.as_deref()).await?;
        for item in &page.items {
            let edited = match item {
                SearchItem::Page(p) => p.last_edited_time.clone(),
                SearchItem::DataSource(d) => d.last_edited_time.clone(),
                SearchItem::Other => continue,
            };
            if !hwm.is_empty() && edited.as_str() <= hwm.as_str() {
                done = true;
                break;
            }
            if edited > max_seen {
                max_seen = edited.clone();
            }
            match item {
                SearchItem::Page(p) => {
                    let (parent_type, parent_id) = parent_cols(&p.parent);
                    store.lock().unwrap().upsert_page(&PageRec {
                        id: p.id.clone(), parent_type, parent_id,
                        title: p.title.clone(), icon: p.icon.clone(),
                        archived: p.archived, last_edited_time: p.last_edited_time.clone(),
                    }).ok();
                    let flat = client.fetch_block_tree(&p.id).await?;
                    let recs: Vec<BlockRec> = flat.iter().map(|f| BlockRec {
                        id: f.block.id.clone(), page_id: p.id.clone(),
                        parent_block_id: f.parent_block_id.clone(), ordinal: f.ordinal,
                        block_type: f.block.block_type.clone(),
                        payload: f.block.payload.to_string(),
                        plain_text: f.block.plain_text.clone(),
                        has_children: f.block.has_children,
                    }).collect();
                    store.lock().unwrap().replace_page_blocks(&p.id, &recs).ok();
                    updated += 1;
                }
                SearchItem::DataSource(d) => {
                    let ds = client.get_data_source(&d.id).await?;
                    store.lock().unwrap().upsert_data_source(&DataSourceRec {
                        id: ds.meta.id.clone(), database_id: ds.meta.database_id.clone(),
                        title: ds.meta.title.clone(), schema_json: ds.schema.to_string(),
                        last_edited_time: ds.meta.last_edited_time.clone(),
                    }).ok();
                    let rows = client.query_data_source_all(&d.id).await?;
                    let recs: Vec<RowRec> = rows.iter().map(|r| RowRec {
                        id: r.id.clone(), data_source_id: d.id.clone(),
                        properties: r.properties.to_string(),
                        last_edited_time: r.last_edited_time.clone(),
                        archived: r.archived,
                    }).collect();
                    store.lock().unwrap().replace_rows(&d.id, &recs).ok();
                    updated += 1;
                }
                SearchItem::Other => {}
            }
        }
        cursor = page.next_cursor.clone();
        if cursor.is_none() {
            done = true;
        }
    }

    if max_seen > hwm {
        store.lock().unwrap().meta_set("hwm", &max_seen).ok();
    }
    Ok(updated)
}

pub fn spawn_puller(client: NotionClient, store: SharedStore, interval: Duration) -> SyncHandle {
    let (status_tx, status_rx) = watch::channel(SyncStatus::Starting);
    let (data_tx, data_rx) = watch::channel(0u64);
    tokio::spawn(async move {
        let client = Arc::new(client);
        loop {
            status_tx.send_replace(SyncStatus::Syncing { done: 0 });
            match pull_once(&client, &store).await {
                Ok(updated) => {
                    if updated > 0 {
                        data_tx.send_modify(|v| *v += 1);
                    }
                    status_tx.send_replace(SyncStatus::Idle { updated });
                }
                Err(ApiError::Network(_)) => {
                    status_tx.send_replace(SyncStatus::Offline);
                }
                Err(e) => {
                    status_tx.send_replace(SyncStatus::Failed(e.to_string()));
                }
            }
            tokio::time::sleep(interval).await;
        }
    });
    SyncHandle { status: status_rx, data_version: data_rx }
}
```

`src/lib.rs`:

```rust
mod puller;

use std::sync::{Arc, Mutex};

pub type SharedStore = Arc<Mutex<notion_store::Store>>;

#[derive(Debug, Clone, PartialEq)]
pub enum SyncStatus {
    Starting,
    Syncing { done: u32 },
    Idle { updated: u32 },
    Offline,
    Failed(String),
}

pub struct SyncHandle {
    pub status: tokio::sync::watch::Receiver<SyncStatus>,
    pub data_version: tokio::sync::watch::Receiver<u64>,
}

pub use puller::{pull_once, spawn_puller};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p notion-sync`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-sync): puller with first crawl, incremental hwm sync, status channels

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 8: `notion-tui` crate — config, terminal guard, app skeleton with status bar

**Files:**
- Create: `crates/notion-tui/Cargo.toml`
- Create: `crates/notion-tui/src/main.rs` (stub — real wire-up in Task 12)
- Create: `crates/notion-tui/src/config.rs`
- Create: `crates/notion-tui/src/terminal.rs`
- Create: `crates/notion-tui/src/app.rs`
- Create: `crates/notion-tui/src/ui/mod.rs`
- Modify: `Cargo.toml` (add workspace member)
- Test: inline `#[cfg(test)]` in `config.rs` and `crates/notion-tui/tests/render_smoke.rs`

**Interfaces:**
- Consumes: `SyncStatus` (notion-sync), `SharedStore` (notion-sync), ratatui.
- Produces (extended by Tasks 9–12):

```rust
// config.rs
pub struct Config { pub token: String, pub poll_interval_secs: u64, pub db_path: std::path::PathBuf }
pub fn load() -> anyhow::Result<Config>; // env NOTION_TOKEN > file token; sensible defaults
pub fn from_sources(env_token: Option<String>, file_contents: Option<&str>,
                    default_db: std::path::PathBuf) -> anyhow::Result<Config>; // pure, testable

// terminal.rs
pub struct TerminalGuard; // enter() enables raw mode + alt screen + mouse capture;
                          // Drop + panic hook restore the terminal

// app.rs
pub enum Focus { Sidebar, Main }
pub struct App {
    pub focus: Focus,
    pub sync_status: notion_sync::SyncStatus,
    pub should_quit: bool,
    // Task 9+: sidebar, view, history, search
}
impl App { pub fn new() -> App; }
pub fn handle_key(app: &mut App, key: crossterm::event::KeyEvent);

// ui/mod.rs
pub fn draw(f: &mut ratatui::Frame, app: &App);           // layout + status bar
pub fn status_line(status: &notion_sync::SyncStatus) -> String;
```

`status_line` mapping: `Starting → "starting…"`, `Syncing{..} → "⟳ syncing…"`, `Idle{updated: 0} → "✓ synced"`, `Idle{updated: n} → "✓ synced (n updated)"`, `Offline → "⚠ offline"`, `Failed(msg) → "✗ sync failed: {msg}"`.

- [ ] **Step 1: Create `crates/notion-tui/Cargo.toml`, add member**

```toml
[package]
name = "notion-tui"
version = "0.1.0"
edition = "2021"

[dependencies]
notion-api = { path = "../notion-api" }
notion-store = { path = "../notion-store" }
notion-sync = { path = "../notion-sync" }
ratatui = { workspace = true }
crossterm = { workspace = true }
tokio = { workspace = true }
futures = { workspace = true }
anyhow = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
toml = { workspace = true }
dirs = { workspace = true }

[dev-dependencies]
insta = { workspace = true }
wiremock = { workspace = true }
tempfile = { workspace = true }
```

Root members: add `"crates/notion-tui"`.

- [ ] **Step 2: Write the failing tests**

Inline in `config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn env_token_beats_file() {
        let cfg = from_sources(
            Some("env-tok".into()),
            Some("token = \"file-tok\"\npoll_interval_secs = 10"),
            PathBuf::from("/tmp/x.db"),
        ).unwrap();
        assert_eq!(cfg.token, "env-tok");
        assert_eq!(cfg.poll_interval_secs, 10);
    }

    #[test]
    fn file_token_used_when_no_env() {
        let cfg = from_sources(None, Some("token = \"file-tok\""), PathBuf::from("/tmp/x.db")).unwrap();
        assert_eq!(cfg.token, "file-tok");
        assert_eq!(cfg.poll_interval_secs, 30); // default
    }

    #[test]
    fn missing_token_is_actionable_error() {
        let err = from_sources(None, None, PathBuf::from("/tmp/x.db")).unwrap_err();
        assert!(err.to_string().contains("NOTION_TOKEN"));
    }
}
```

`crates/notion-tui/tests/render_smoke.rs`:

```rust
use notion_tui::app::App;
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn renders_frame_with_status_bar() {
    let mut app = App::new();
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(term.backend());
}

#[test]
fn q_quits() {
    use crossterm::event::{KeyCode, KeyEvent};
    let mut app = App::new();
    notion_tui::app::handle_key(&mut app, KeyEvent::from(KeyCode::Char('q')));
    assert!(app.should_quit);
}
```

Note: the binary crate must also expose a library for tests — add `src/lib.rs`:

```rust
pub mod app;
pub mod config;
pub mod terminal;
pub mod ui;
```

(`main.rs` stub: `fn main() { println!("wired in task 12"); }`)

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p notion-tui`
Expected: compile error — modules missing.

- [ ] **Step 4: Implement**

`src/config.rs`:

```rust
use std::path::PathBuf;

pub struct Config {
    pub token: String,
    pub poll_interval_secs: u64,
    pub db_path: PathBuf,
}

pub fn load() -> anyhow::Result<Config> {
    let file = dirs::config_dir()
        .map(|d| d.join("notion-tui/config.toml"))
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok());
    let default_db = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("notion-tui/notion.db");
    from_sources(std::env::var("NOTION_TOKEN").ok(), file.as_deref(), default_db)
}

pub fn from_sources(
    env_token: Option<String>,
    file_contents: Option<&str>,
    default_db: PathBuf,
) -> anyhow::Result<Config> {
    let file: toml::Value = file_contents
        .map(|s| s.parse())
        .transpose()?
        .unwrap_or(toml::Value::Table(Default::default()));
    let token = env_token
        .or_else(|| file.get("token").and_then(|v| v.as_str()).map(str::to_string))
        .ok_or_else(|| anyhow::anyhow!(
            "no Notion token found: set the NOTION_TOKEN environment variable \
             or put token = \"...\" in ~/.config/notion-tui/config.toml"
        ))?;
    Ok(Config {
        token,
        poll_interval_secs: file
            .get("poll_interval_secs").and_then(|v| v.as_integer()).unwrap_or(30) as u64,
        db_path: file
            .get("db_path").and_then(|v| v.as_str()).map(PathBuf::from)
            .unwrap_or(default_db),
    })
}
```

`src/terminal.rs`:

```rust
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

pub struct TerminalGuard;

fn restore() {
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

impl TerminalGuard {
    pub fn enter() -> anyhow::Result<TerminalGuard> {
        enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}
```

`src/app.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent};
use notion_sync::SyncStatus;

pub enum Focus {
    Sidebar,
    Main,
}

pub struct App {
    pub focus: Focus,
    pub sync_status: SyncStatus,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> App {
        App {
            focus: Focus::Sidebar,
            sync_status: SyncStatus::Starting,
            should_quit: false,
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if let KeyCode::Char('q') = key.code {
        app.should_quit = true;
    }
}
```

`src/ui/mod.rs`:

```rust
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use notion_sync::SyncStatus;

pub fn status_line(status: &SyncStatus) -> String {
    match status {
        SyncStatus::Starting => "starting…".into(),
        SyncStatus::Syncing { .. } => "⟳ syncing…".into(),
        SyncStatus::Idle { updated: 0 } => "✓ synced".into(),
        SyncStatus::Idle { updated } => format!("✓ synced ({updated} updated)"),
        SyncStatus::Offline => "⚠ offline".into(),
        SyncStatus::Failed(msg) => format!("✗ sync failed: {msg}"),
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(1)])
        .split(rows[0]);

    f.render_widget(Block::default().borders(Borders::ALL).title(" notion "), cols[0]);
    f.render_widget(Block::default().borders(Borders::ALL), cols[1]);
    f.render_widget(
        Paragraph::new(status_line(&app.sync_status))
            .style(Style::default().add_modifier(Modifier::REVERSED)),
        rows[1],
    );
}
```

- [ ] **Step 5: Run tests; review + accept the snapshot**

Run: `cargo test -p notion-tui`
The snapshot test fails on first run (no stored snapshot). Inspect `crates/notion-tui/snapshots/*.snap.new` — verify the frame shows two bordered panes and a `✓ synced` status line — then run `cargo insta accept` (or move the `.snap.new` file) and re-run: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-tui): config loading, terminal guard, app skeleton + status bar

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 9: Sidebar tree + focus/navigation

**Files:**
- Create: `crates/notion-tui/src/ui/sidebar.rs`
- Modify: `crates/notion-tui/src/app.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`
- Test: inline `#[cfg(test)]` in `ui/sidebar.rs` + snapshot in `tests/render_smoke.rs`

**Interfaces:**
- Consumes: `TreeNode`, `NodeKind` (notion-store); `App`, `Focus` from Task 8.
- Produces:

```rust
// ui/sidebar.rs
pub struct SidebarState {
    pub nodes: Vec<notion_store::TreeNode>,   // raw, from store.sidebar_nodes()
    pub collapsed: std::collections::HashSet<String>,
    pub cursor: usize,                        // index into visible()
    pub hidden: bool,                         // toggled with '1'
}
pub struct VisibleNode<'a> { pub node: &'a notion_store::TreeNode, pub depth: usize }
impl SidebarState {
    pub fn new(nodes: Vec<notion_store::TreeNode>) -> SidebarState;
    pub fn visible(&self) -> Vec<VisibleNode>;   // DFS of pages (children under parents,
                                                 // skipping collapsed), then data sources
                                                 // under a "── databases ──" header
    pub fn move_cursor(&mut self, delta: isize);
    pub fn toggle_collapse(&mut self);           // collapse/expand node at cursor
    pub fn selected(&self) -> Option<&notion_store::TreeNode>;
}
pub fn render(f: &mut Frame, area: Rect, state: &SidebarState, focused: bool);

// app.rs additions
pub enum Action { None, OpenNode(notion_store::TreeNode) }
pub fn handle_key(app: &mut App, key: KeyEvent) -> Action;  // signature change!
```

Key handling with `Focus::Sidebar`: `j`/`k` move cursor, `h`/`l` collapse/expand, `Enter` returns `Action::OpenNode(selected)` and moves focus to Main, `1` toggles `hidden`, `Tab` toggles focus, `q` quits. Task 10/11 handle `OpenNode` (page → page view, data source → table view); in this task `main`-side handling is a no-op.

- [ ] **Step 1: Write the failing tests (inline in `sidebar.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{NodeKind, TreeNode};

    fn nodes() -> Vec<TreeNode> {
        vec![
            TreeNode { id: "a".into(), title: "Alpha".into(), parent_id: None, kind: NodeKind::Page },
            TreeNode { id: "a1".into(), title: "Alpha child".into(),
                       parent_id: Some("a".into()), kind: NodeKind::Page },
            TreeNode { id: "b".into(), title: "Beta".into(), parent_id: None, kind: NodeKind::Page },
            TreeNode { id: "ds".into(), title: "Tasks".into(), parent_id: None,
                       kind: NodeKind::DataSource },
        ]
    }

    #[test]
    fn visible_nests_children_and_lists_data_sources_last() {
        let s = SidebarState::new(nodes());
        let v = s.visible();
        let titles: Vec<(&str, usize)> =
            v.iter().map(|n| (n.node.title.as_str(), n.depth)).collect();
        assert_eq!(titles, vec![
            ("Alpha", 0), ("Alpha child", 1), ("Beta", 0), ("Tasks", 0),
        ]);
    }

    #[test]
    fn collapse_hides_children() {
        let mut s = SidebarState::new(nodes());
        s.cursor = 0; // Alpha
        s.toggle_collapse();
        let titles: Vec<&str> = s.visible().iter().map(|n| n.node.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Beta", "Tasks"]);
    }

    #[test]
    fn cursor_clamps() {
        let mut s = SidebarState::new(nodes());
        s.move_cursor(-5);
        assert_eq!(s.cursor, 0);
        s.move_cursor(100);
        assert_eq!(s.cursor, s.visible().len() - 1);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui sidebar`
Expected: compile error.

- [ ] **Step 3: Implement `sidebar.rs`**

```rust
use std::collections::HashSet;

use notion_store::{NodeKind, TreeNode};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct VisibleNode<'a> {
    pub node: &'a TreeNode,
    pub depth: usize,
}

pub struct SidebarState {
    pub nodes: Vec<TreeNode>,
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub hidden: bool,
}

impl SidebarState {
    pub fn new(nodes: Vec<TreeNode>) -> SidebarState {
        SidebarState { nodes, collapsed: HashSet::new(), cursor: 0, hidden: false }
    }

    pub fn visible(&self) -> Vec<VisibleNode> {
        let mut out = Vec::new();
        // Pages: DFS from roots (parent_id None or parent not present as a page node).
        let page_ids: HashSet<&str> = self.nodes.iter()
            .filter(|n| n.kind == NodeKind::Page).map(|n| n.id.as_str()).collect();
        let roots = self.nodes.iter().filter(|n| {
            n.kind == NodeKind::Page
                && n.parent_id.as_deref().map_or(true, |p| !page_ids.contains(p))
        });
        for root in roots {
            self.push_subtree(root, 0, &mut out);
        }
        for ds in self.nodes.iter().filter(|n| n.kind == NodeKind::DataSource) {
            out.push(VisibleNode { node: ds, depth: 0 });
        }
        out
    }

    fn push_subtree<'a>(&'a self, node: &'a TreeNode, depth: usize, out: &mut Vec<VisibleNode<'a>>) {
        out.push(VisibleNode { node, depth });
        if self.collapsed.contains(&node.id) {
            return;
        }
        for child in self.nodes.iter().filter(|n| {
            n.kind == NodeKind::Page && n.parent_id.as_deref() == Some(node.id.as_str())
        }) {
            self.push_subtree(child, depth + 1, out);
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 { return; }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn toggle_collapse(&mut self) {
        if let Some(node) = self.selected() {
            let id = node.id.clone();
            if !self.collapsed.remove(&id) {
                self.collapsed.insert(id);
            }
        }
    }

    pub fn selected(&self) -> Option<&TreeNode> {
        self.visible().get(self.cursor).map(|v| v.node)
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &SidebarState, focused: bool) {
    let items: Vec<ListItem> = state.visible().iter().enumerate().map(|(i, v)| {
        let marker = match v.node.kind {
            NodeKind::Page => "▸ ",
            NodeKind::DataSource => "▦ ",
        };
        let line = format!("{}{}{}", "  ".repeat(v.depth), marker, v.node.title);
        let mut item = ListItem::new(Line::from(line));
        if i == state.cursor && focused {
            item = item.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        item
    }).collect();
    let title = if focused { " notion ● " } else { " notion " };
    f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(title)), area);
}
```

- [ ] **Step 4: Wire into `app.rs` and `ui/mod.rs`**

`app.rs` — extend `App` and `handle_key`:

```rust
use crate::ui::sidebar::SidebarState;
use notion_store::TreeNode;

pub enum Action {
    None,
    OpenNode(TreeNode),
}

pub struct App {
    pub focus: Focus,
    pub sync_status: SyncStatus,
    pub should_quit: bool,
    pub sidebar: SidebarState,
}

impl App {
    pub fn new() -> App {
        App {
            focus: Focus::Sidebar,
            sync_status: SyncStatus::Starting,
            should_quit: false,
            sidebar: SidebarState::new(Vec::new()),
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => { app.should_quit = true; return Action::None; }
        KeyCode::Tab => {
            app.focus = match app.focus { Focus::Sidebar => Focus::Main, Focus::Main => Focus::Sidebar };
            return Action::None;
        }
        KeyCode::Char('1') => { app.sidebar.hidden = !app.sidebar.hidden; return Action::None; }
        _ => {}
    }
    if matches!(app.focus, Focus::Sidebar) {
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => app.sidebar.move_cursor(1),
            KeyCode::Char('k') | KeyCode::Up => app.sidebar.move_cursor(-1),
            KeyCode::Char('h') | KeyCode::Char('l') => app.sidebar.toggle_collapse(),
            KeyCode::Enter => {
                if let Some(node) = app.sidebar.selected().cloned() {
                    app.focus = Focus::Main;
                    return Action::OpenNode(node);
                }
            }
            _ => {}
        }
    }
    Action::None
}
```

`ui/mod.rs` — `pub mod sidebar;`, and in `draw` render the sidebar into the left column (skip the column entirely when `app.sidebar.hidden`, giving the main pane full width):

```rust
let cols = if app.sidebar.hidden {
    vec![rows[0]]
} else {
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(1)])
        .split(rows[0])
        .to_vec()
};
if !app.sidebar.hidden {
    sidebar::render(f, cols[0], &app.sidebar, matches!(app.focus, crate::app::Focus::Sidebar));
}
let main_area = *cols.last().unwrap();
f.render_widget(Block::default().borders(Borders::ALL), main_area);
```

Add a snapshot test to `tests/render_smoke.rs` seeding `app.sidebar = SidebarState::new(vec![…two pages, one data source…])` and snapshotting the frame.

- [ ] **Step 5: Run tests, accept new snapshot after review**

Run: `cargo test -p notion-tui` then `cargo insta accept` after verifying the sidebar shows the tree with cursor highlight.
Expected: all pass on re-run.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-tui): sidebar tree with collapse, cursor, focus handling

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 10: Page view (block rendering, cursor, history)

**Files:**
- Create: `crates/notion-tui/src/ui/page.rs`
- Modify: `crates/notion-tui/src/app.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`
- Test: inline `#[cfg(test)]` in `ui/page.rs` + snapshot in `tests/render_smoke.rs`

**Interfaces:**
- Consumes: `BlockRec`, `PageRec`, `Store::page_blocks`/`get_page` (notion-store); `App`, `Action` from Task 9.
- Produces:

```rust
// ui/page.rs
pub struct PageView {
    pub page: notion_store::PageRec,
    pub blocks: Vec<notion_store::BlockRec>,   // as returned by page_blocks()
    pub cursor: usize,                          // index into lines()
    pub scroll: u16,
    pub collapsed_toggles: std::collections::HashSet<String>,
}
pub struct BlockLine { pub block_id: String, pub text: String, pub indent: usize,
                       pub link_page_id: Option<String> }
impl PageView {
    pub fn new(page: notion_store::PageRec, blocks: Vec<notion_store::BlockRec>) -> PageView;
    pub fn lines(&self) -> Vec<BlockLine>;      // tree-ordered, skips children of collapsed toggles
    pub fn move_cursor(&mut self, delta: isize);
    pub fn toggle_at_cursor(&mut self);         // collapse/expand toggle blocks
    pub fn link_at_cursor(&self) -> Option<String>;
}
pub fn render(f: &mut Frame, area: Rect, view: &PageView, focused: bool);

// app.rs additions
pub enum View { Empty, Page(crate::ui::page::PageView) }   // View::Table added in Task 11
pub struct App { …, pub view: View, pub history: Vec<String> }  // history of page ids
```

Rendering per block type (text = `plain_text` from the record; payload JSON parsed only where noted):
`heading_1/2/3` → bold text with `# `/`## `/`### ` prefix; `paragraph` → plain; `to_do` → `[x] ` / `[ ] ` prefix (checked = `payload.checked == true`); `bulleted_list_item` → `• `; `numbered_list_item` → `n. ` (n = 1-based position among *contiguous* numbered siblings); `toggle` → `▾ ` (expanded) / `▸ ` (collapsed); `code` → text prefixed `│ ` per line; `quote` → `┃ `; `callout` → `💡 `; `divider` → `────────`; `child_page`/`child_database` → `→ {title}` with `link_page_id = Some(block.id)` (Notion child_page block id == the child page's id). Unknown types → `⍰ {plain_text}`. Children of a block render at `indent = parent indent + 1` (2 spaces per level).

Key handling with `Focus::Main` and `View::Page`: `j/k` cursor, `g`/`G` first/last line, `Ctrl+d`/`Ctrl+u` cursor ±10, `h`/`l`/`Space` toggle collapse on toggle blocks, `Enter` on a line with `link_page_id` → `Action::OpenNode` equivalent (push current page id to `history`, open target), `Backspace`/`-` → pop `history`, reopen previous page.

- [ ] **Step 1: Write the failing tests (inline in `page.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{BlockRec, PageRec};

    fn rec(id: &str, parent: Option<&str>, ord: i64, ty: &str, text: &str, payload: &str) -> BlockRec {
        BlockRec { id: id.into(), page_id: "p".into(),
                   parent_block_id: parent.map(Into::into), ordinal: ord,
                   block_type: ty.into(), payload: payload.into(),
                   plain_text: text.into(), has_children: false }
    }

    fn page() -> PageRec {
        PageRec { id: "p".into(), parent_type: "workspace".into(), parent_id: None,
                  title: "T".into(), icon: None, archived: false, last_edited_time: "t".into() }
    }

    #[test]
    fn renders_prefixes_and_numbering() {
        let v = PageView::new(page(), vec![
            rec("h", None, 0, "heading_1", "Title", "{}"),
            rec("t1", None, 1, "to_do", "done thing", r#"{"checked": true}"#),
            rec("n1", None, 2, "numbered_list_item", "first", "{}"),
            rec("n2", None, 3, "numbered_list_item", "second", "{}"),
            rec("d", None, 4, "divider", "", "{}"),
        ]);
        let texts: Vec<String> = v.lines().iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts[0], "# Title");
        assert_eq!(texts[1], "[x] done thing");
        assert_eq!(texts[2], "1. first");
        assert_eq!(texts[3], "2. second");
        assert!(texts[4].starts_with("────"));
    }

    #[test]
    fn toggle_collapse_hides_children() {
        let mut v = PageView::new(page(), vec![
            rec("tg", None, 0, "toggle", "More", "{}"),
            rec("c1", Some("tg"), 0, "paragraph", "hidden", "{}"),
        ]);
        assert_eq!(v.lines().len(), 2);
        assert_eq!(v.lines()[1].indent, 1);
        v.cursor = 0;
        v.toggle_at_cursor();
        assert_eq!(v.lines().len(), 1);
        assert!(v.lines()[0].text.starts_with("▸"));
    }

    #[test]
    fn child_page_is_a_link() {
        let v = PageView::new(page(), vec![
            rec("cp", None, 0, "child_page", "Sub page", "{}"),
        ]);
        assert_eq!(v.lines()[0].link_page_id.as_deref(), Some("cp"));
        assert_eq!(v.lines()[0].text, "→ Sub page");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui page`
Expected: compile error.

- [ ] **Step 3: Implement `page.rs`**

```rust
use std::collections::HashSet;

use notion_store::{BlockRec, PageRec};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct BlockLine {
    pub block_id: String,
    pub text: String,
    pub indent: usize,
    pub link_page_id: Option<String>,
}

pub struct PageView {
    pub page: PageRec,
    pub blocks: Vec<BlockRec>,
    pub cursor: usize,
    pub scroll: u16,
    pub collapsed_toggles: HashSet<String>,
}

impl PageView {
    pub fn new(page: PageRec, blocks: Vec<BlockRec>) -> PageView {
        PageView { page, blocks, cursor: 0, scroll: 0, collapsed_toggles: HashSet::new() }
    }

    pub fn lines(&self) -> Vec<BlockLine> {
        let mut out = Vec::new();
        self.push_children(None, 0, &mut out);
        out
    }

    fn push_children(&self, parent: Option<&str>, indent: usize, out: &mut Vec<BlockLine>) {
        let mut numbered = 0usize;
        let siblings: Vec<&BlockRec> = self.blocks.iter()
            .filter(|b| b.parent_block_id.as_deref() == parent)
            .collect();
        for b in siblings {
            if b.block_type == "numbered_list_item" { numbered += 1; } else { numbered = 0; }
            let payload: serde_json::Value =
                serde_json::from_str(&b.payload).unwrap_or_default();
            let collapsed = self.collapsed_toggles.contains(&b.id);
            let (text, link) = match b.block_type.as_str() {
                "heading_1" => (format!("# {}", b.plain_text), None),
                "heading_2" => (format!("## {}", b.plain_text), None),
                "heading_3" => (format!("### {}", b.plain_text), None),
                "to_do" => {
                    let mark = if payload["checked"].as_bool().unwrap_or(false) { "x" } else { " " };
                    (format!("[{mark}] {}", b.plain_text), None)
                }
                "bulleted_list_item" => (format!("• {}", b.plain_text), None),
                "numbered_list_item" => (format!("{numbered}. {}", b.plain_text), None),
                "toggle" => {
                    let arrow = if collapsed { "▸" } else { "▾" };
                    (format!("{arrow} {}", b.plain_text), None)
                }
                "code" => (format!("│ {}", b.plain_text), None),
                "quote" => (format!("┃ {}", b.plain_text), None),
                "callout" => (format!("💡 {}", b.plain_text), None),
                "divider" => ("────────".to_string(), None),
                "child_page" | "child_database" => {
                    (format!("→ {}", b.plain_text), Some(b.id.clone()))
                }
                "paragraph" => (b.plain_text.clone(), None),
                _ => (format!("⍰ {}", b.plain_text), None),
            };
            out.push(BlockLine { block_id: b.id.clone(), text, indent, link_page_id: link });
            if !(b.block_type == "toggle" && collapsed) {
                self.push_children(Some(&b.id), indent + 1, out);
            }
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.lines().len();
        if len == 0 { return; }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn toggle_at_cursor(&mut self) {
        if let Some(line) = self.lines().get(self.cursor) {
            let id = line.block_id.clone();
            let is_toggle = self.blocks.iter()
                .any(|b| b.id == id && b.block_type == "toggle");
            if is_toggle && !self.collapsed_toggles.remove(&id) {
                self.collapsed_toggles.insert(id);
            }
        }
    }

    pub fn link_at_cursor(&self) -> Option<String> {
        self.lines().get(self.cursor).and_then(|l| l.link_page_id.clone())
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &PageView, focused: bool) {
    let items: Vec<ListItem> = view.lines().iter().enumerate().map(|(i, l)| {
        let mut item = ListItem::new(Line::from(
            format!("{}{}", "  ".repeat(l.indent), l.text),
        ));
        if i == view.cursor && focused {
            item = item.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        item
    }).collect();
    let title = format!(" {} ", view.page.title);
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}
```

- [ ] **Step 4: Wire into `app.rs` (View enum, history, key handling) + `ui/mod.rs`**

`app.rs`:

```rust
use crate::ui::page::PageView;

pub enum View {
    Empty,
    Page(PageView),
}

pub struct App {
    pub focus: Focus,
    pub sync_status: SyncStatus,
    pub should_quit: bool,
    pub sidebar: SidebarState,
    pub view: View,
    pub history: Vec<String>,   // page ids we navigated away from
}

pub enum Action {
    None,
    OpenNode(TreeNode),
    OpenPage(String),   // by page id (child-page links, history back)
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    // …global keys as in Task 9…
    if matches!(app.focus, Focus::Main) {
        if let View::Page(view) = &mut app.view {
            match (key.code, key.modifiers) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => view.move_cursor(1),
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => view.move_cursor(-1),
                (KeyCode::Char('g'), _) => view.cursor = 0,
                (KeyCode::Char('G'), _) => view.cursor = view.lines().len().saturating_sub(1),
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => view.move_cursor(10),
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => view.move_cursor(-10),
                (KeyCode::Char('h'), _) | (KeyCode::Char('l'), _) | (KeyCode::Char(' '), _) =>
                    view.toggle_at_cursor(),
                (KeyCode::Enter, _) => {
                    if let Some(target) = view.link_at_cursor() {
                        let from = view.page.id.clone();
                        app.history.push(from);
                        return Action::OpenPage(target);
                    }
                }
                (KeyCode::Backspace, _) | (KeyCode::Char('-'), _) => {
                    if let Some(prev) = app.history.pop() {
                        return Action::OpenPage(prev);
                    }
                }
                _ => {}
            }
        }
    }
    Action::None
}
```

`ui/mod.rs`: `pub mod page;` and in `draw`, render `View::Page` via `page::render(f, main_area, view, matches!(app.focus, Focus::Main))`; `View::Empty` keeps the bare bordered block.

Add a snapshot test rendering a `PageView` with a heading, todo, toggle+child, and child_page link.

- [ ] **Step 5: Run tests, accept snapshot after review**

Run: `cargo test -p notion-tui` then `cargo insta accept` after verifying prefixes/indentation look right.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-tui): page view with block rendering, toggles, links, history

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 11: Database table view

**Files:**
- Create: `crates/notion-tui/src/ui/table.rs`
- Modify: `crates/notion-tui/src/app.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`
- Test: inline `#[cfg(test)]` in `ui/table.rs`

**Interfaces:**
- Consumes: `DataSourceRec`, `RowRec` (notion-store); `App`, `View`, `Action` from Task 10.
- Produces:

```rust
// ui/table.rs
pub struct TableView {
    pub ds: notion_store::DataSourceRec,
    pub columns: Vec<Column>,        // title property first, rest alphabetical
    pub rows: Vec<notion_store::RowRec>,
    pub cursor: usize,               // row index
    pub sort: Option<(usize, bool)>, // (column index, ascending)
}
pub struct Column { pub name: String, pub prop_type: String }
impl TableView {
    pub fn new(ds: notion_store::DataSourceRec, rows: Vec<notion_store::RowRec>) -> TableView;
    pub fn cell(&self, row: &notion_store::RowRec, col: &Column) -> String;
    pub fn toggle_sort(&mut self, col_idx: usize);   // asc → desc → asc…, re-sorts rows
    pub fn move_cursor(&mut self, delta: isize);
    pub fn selected_row_id(&self) -> Option<String>;
}
pub fn cell_text(prop: &serde_json::Value) -> String;  // pure property→string
pub fn render(f: &mut Frame, area: Rect, view: &TableView, focused: bool);

// app.rs: add View::Table(TableView)
```

`cell_text` by `prop["type"]`: `title`/`rich_text` → `rich_text_plain` of the array; `number` → number as string; `select`/`status` → `prop[type]["name"]`; `multi_select` → names joined `", "`; `date` → `prop.date.start`; `checkbox` → `☑`/`☐`; `url`/`email`/`phone_number` → raw string; `people` → count `👤 n`; anything else → `""`. (`rich_text_plain` here is a local copy in `table.rs` operating on `serde_json::Value` — three lines — to avoid the TUI crate depending on notion-api just for this.)

Keys with `Focus::Main` + `View::Table`: `j/k` row cursor, `g/G` first/last, `h/l` move the *sort column* selection left/right (highlight in header), `s` toggle sort on the selected column, `Enter` → `Action::OpenPage(row_id)` (rows are pages; blocks were crawled by the puller).

- [ ] **Step 1: Write the failing tests (inline in `table.rs`)** — cover: `cell_text` for title, number, select, multi_select, checkbox, date; `new()` puts title column first; `toggle_sort` sorts ascending then flips.

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
                "Done": {"type": "checkbox"},
                "Prio": {"type": "select"}
            }).to_string(),
            last_edited_time: "t".into(),
        }
    }

    fn row(id: &str, name: &str, done: bool, prio: &str) -> RowRec {
        RowRec {
            id: id.into(), data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": name}]},
                "Done": {"type": "checkbox", "checkbox": done},
                "Prio": {"type": "select", "select": {"name": prio}}
            }).to_string(),
            last_edited_time: "t".into(), archived: false,
        }
    }

    #[test]
    fn title_column_first_and_cells_render() {
        let v = TableView::new(ds(), vec![row("r1", "Buy milk", true, "High")]);
        assert_eq!(v.columns[0].name, "Name");
        assert_eq!(v.cell(&v.rows[0], &v.columns[0]), "Buy milk");
        let done_col = v.columns.iter().position(|c| c.name == "Done").unwrap();
        assert_eq!(v.cell(&v.rows[0], &v.columns[done_col]), "☑");
    }

    #[test]
    fn sort_toggles_direction() {
        let mut v = TableView::new(ds(), vec![
            row("r1", "b task", false, "Low"),
            row("r2", "a task", false, "High"),
        ]);
        v.toggle_sort(0);
        assert_eq!(v.selected_row_id().as_deref(), Some("r2")); // "a task" first
        v.toggle_sort(0);
        assert_eq!(v.selected_row_id().as_deref(), Some("r1")); // descending
    }

    #[test]
    fn cell_text_variants() {
        assert_eq!(cell_text(&json!({"type": "number", "number": 42})), "42");
        assert_eq!(cell_text(&json!({"type": "multi_select",
            "multi_select": [{"name": "a"}, {"name": "b"}]})), "a, b");
        assert_eq!(cell_text(&json!({"type": "date", "date": {"start": "2026-07-05"}})), "2026-07-05");
        assert_eq!(cell_text(&json!({"type": "checkbox", "checkbox": false})), "☐");
    }
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p notion-tui table` → compile error.

- [ ] **Step 3: Implement `table.rs`**

```rust
use notion_store::{DataSourceRec, RowRec};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Row as TRow, Table};
use ratatui::Frame;
use serde_json::Value;

pub struct Column {
    pub name: String,
    pub prop_type: String,
}

pub struct TableView {
    pub ds: DataSourceRec,
    pub columns: Vec<Column>,
    pub rows: Vec<RowRec>,
    pub cursor: usize,
    pub sort: Option<(usize, bool)>,
    pub sort_col: usize, // header selection moved with h/l
}

fn rich_text_plain(v: &Value) -> String {
    v.as_array().map(|a| a.iter()
        .filter_map(|t| t["plain_text"].as_str()).collect::<Vec<_>>().join(""))
        .unwrap_or_default()
}

pub fn cell_text(prop: &Value) -> String {
    match prop["type"].as_str().unwrap_or("") {
        "title" => rich_text_plain(&prop["title"]),
        "rich_text" => rich_text_plain(&prop["rich_text"]),
        "number" => prop["number"].as_f64()
            .map(|n| if n.fract() == 0.0 { format!("{}", n as i64) } else { n.to_string() })
            .unwrap_or_default(),
        "select" => prop["select"]["name"].as_str().unwrap_or("").into(),
        "status" => prop["status"]["name"].as_str().unwrap_or("").into(),
        "multi_select" => prop["multi_select"].as_array().map(|a| a.iter()
            .filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default(),
        "date" => prop["date"]["start"].as_str().unwrap_or("").into(),
        "checkbox" => if prop["checkbox"].as_bool().unwrap_or(false) { "☑".into() } else { "☐".into() },
        "url" => prop["url"].as_str().unwrap_or("").into(),
        "email" => prop["email"].as_str().unwrap_or("").into(),
        "phone_number" => prop["phone_number"].as_str().unwrap_or("").into(),
        "people" => prop["people"].as_array()
            .map(|a| format!("👤 {}", a.len())).unwrap_or_default(),
        _ => String::new(),
    }
}

impl TableView {
    pub fn new(ds: DataSourceRec, rows: Vec<RowRec>) -> TableView {
        let schema: Value = serde_json::from_str(&ds.schema_json).unwrap_or_default();
        let mut columns: Vec<Column> = schema.as_object().map(|m| m.iter()
            .map(|(name, def)| Column {
                name: name.clone(),
                prop_type: def["type"].as_str().unwrap_or("").into(),
            }).collect()).unwrap_or_default();
        columns.sort_by(|a, b| {
            let a_title = a.prop_type == "title";
            let b_title = b.prop_type == "title";
            b_title.cmp(&a_title).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        TableView { ds, columns, rows, cursor: 0, sort: None, sort_col: 0 }
    }

    pub fn cell(&self, row: &RowRec, col: &Column) -> String {
        let props: Value = serde_json::from_str(&row.properties).unwrap_or_default();
        cell_text(&props[&col.name])
    }

    pub fn toggle_sort(&mut self, col_idx: usize) {
        let asc = match self.sort {
            Some((c, asc)) if c == col_idx => !asc,
            _ => true,
        };
        self.sort = Some((col_idx, asc));
        let col_name = self.columns[col_idx].name.clone();
        self.rows.sort_by_cached_key(|r| {
            let props: Value = serde_json::from_str(&r.properties).unwrap_or_default();
            cell_text(&props[&col_name]).to_lowercase()
        });
        if !asc { self.rows.reverse(); }
        self.cursor = 0;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() { return; }
        self.cursor = (self.cursor as isize + delta)
            .clamp(0, self.rows.len() as isize - 1) as usize;
    }

    pub fn selected_row_id(&self) -> Option<String> {
        self.rows.get(self.cursor).map(|r| r.id.clone())
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &TableView, focused: bool) {
    let header = TRow::new(view.columns.iter().enumerate().map(|(i, c)| {
        let mut name = c.name.clone();
        if let Some((sc, asc)) = view.sort {
            if sc == i { name.push_str(if asc { " ▲" } else { " ▼" }); }
        }
        if i == view.sort_col { format!("[{name}]") } else { name }
    }).collect::<Vec<_>>())
        .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<TRow> = view.rows.iter().enumerate().map(|(i, r)| {
        let cells: Vec<String> = view.columns.iter().map(|c| view.cell(r, c)).collect();
        let mut row = TRow::new(cells);
        if i == view.cursor && focused {
            row = row.style(Style::default().add_modifier(Modifier::REVERSED));
        }
        row
    }).collect();
    let widths: Vec<Constraint> = view.columns.iter().enumerate()
        .map(|(i, _)| if i == 0 { Constraint::Min(20) } else { Constraint::Length(14) })
        .collect();
    f.render_widget(
        Table::new(rows, widths).header(header)
            .block(Block::default().borders(Borders::ALL).title(format!(" {} ", view.ds.title))),
        area,
    );
}
```

- [ ] **Step 4: Wire `View::Table` into `app.rs` + `ui/mod.rs`** — add the variant, key handling (`j/k/g/G` cursor, `h/l` move `sort_col` within bounds, `s` → `toggle_sort(sort_col)`, `Enter` → `Action::OpenPage(selected_row_id)` pushing history like page links), and the `draw` arm calling `table::render`.

- [ ] **Step 5: Run tests** — `cargo test -p notion-tui` → all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-tui): database table view with sorting and row navigation

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 12: Search modal, main() wire-up, end-to-end smoke test

**Files:**
- Create: `crates/notion-tui/src/ui/search.rs`
- Modify: `crates/notion-tui/src/app.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`
- Modify: `crates/notion-tui/src/main.rs`
- Test: `crates/notion-tui/tests/e2e.rs` + inline tests in `search.rs`

**Interfaces:**
- Consumes: everything above.
- Produces:

```rust
// ui/search.rs
pub struct SearchState { pub input: String, pub results: Vec<notion_store::SearchHit>, pub cursor: usize }
impl SearchState {
    pub fn new() -> SearchState;
    pub fn on_key(&mut self, key: KeyEvent) -> SearchAction; // typing edits input, j/k|↑↓ move,
                                                             // Enter selects, Esc closes
}
pub enum SearchAction { None, QueryChanged, Open(String /* page_id */), Close }
pub fn render(f: &mut Frame, state: &SearchState); // centered modal over everything

// app.rs
pub struct App { …, pub search: Option<SearchState> }
// '/' or Ctrl+P opens search (any focus); while open, all keys go to SearchState::on_key;
// QueryChanged → app.refresh_search(store), Open(id) → open page + close, Close → close.

// app.rs — store-aware helpers (App now holds store: notion_sync::SharedStore)
impl App {
    pub fn new(store: notion_sync::SharedStore) -> App;
    pub fn refresh_sidebar(&mut self);                   // store.sidebar_nodes()
    pub fn open_node(&mut self, node: &notion_store::TreeNode);  // page → PageView, ds → TableView
    pub fn open_page(&mut self, page_id: &str);          // get_page + page_blocks → PageView;
                                                         // if the id is a data source, open table
    pub fn refresh_search(&mut self);
    pub fn refresh_current_view(&mut self);              // re-query store after data_version bump
}
```

`main()` flow: `config::load()` → `Store::open(db_path)` wrapped in `Arc<Mutex<_>>` → `NotionClient::new(token)` → `spawn_puller(client, store.clone(), Duration::from_secs(poll_interval_secs))` → `TerminalGuard::enter()` → ratatui `Terminal` on crossterm backend → event loop with `tokio::select!` over `crossterm::event::EventStream` (keys + mouse scroll ignored for M1 except wheel = `move_cursor(±3)`), `handle.data_version.changed()` (→ `app.refresh_sidebar(); app.refresh_current_view()`), `handle.status.changed()` (→ `app.sync_status = …`); draw after every event; exit when `app.should_quit`.

- [ ] **Step 1: Write failing tests** — inline in `search.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};

    #[test]
    fn typing_updates_query_and_esc_closes() {
        let mut s = SearchState::new();
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Char('a'))), SearchAction::QueryChanged));
        assert_eq!(s.input, "a");
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Backspace)), SearchAction::QueryChanged));
        assert_eq!(s.input, "");
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Esc)), SearchAction::Close));
    }

    #[test]
    fn enter_opens_selected() {
        let mut s = SearchState::new();
        s.results = vec![notion_store::SearchHit {
            page_id: "p9".into(), title: "T".into(), snippet: "…".into() }];
        match s.on_key(KeyEvent::from(KeyCode::Enter)) {
            SearchAction::Open(id) => assert_eq!(id, "p9"),
            other => panic!("expected Open, got {other:?}"),
        }
    }
}
```

`crates/notion-tui/tests/e2e.rs` (the milestone acceptance test):

```rust
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use notion_store::Store;
use notion_sync::pull_once;
use notion_tui::app::{self, App};
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn crawl_then_browse_smoke() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "page", "id": "p1", "archived": false,
                "last_edited_time": "2026-07-05T10:00:00.000Z",
                "parent": {"type": "workspace", "workspace": true},
                "properties": {"Name": {"type": "title",
                                        "title": [{"plain_text": "Hello Page"}]}}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/v1/blocks/p1/children"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"object": "block", "id": "b1", "type": "paragraph",
                "has_children": false,
                "paragraph": {"rich_text": [{"plain_text": "hello world"}]}}],
            "has_more": false, "next_cursor": null})))
        .mount(&server).await;

    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    pull_once(&client, &store).await.unwrap();

    let mut app = App::new(store);
    app.refresh_sidebar();
    let node = app.sidebar.selected().cloned().unwrap();
    app.open_node(&node);

    let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
    term.draw(|f| ui::draw(f, &app)).unwrap();
    let rendered = format!("{:?}", term.backend().buffer());
    assert!(rendered.contains("Hello Page"));
    assert!(rendered.contains("hello world"));
}
```

- [ ] **Step 2: Run to verify failure** — `cargo test -p notion-tui --test e2e` → compile errors (App::new signature, open_node missing).

- [ ] **Step 3: Implement** — `search.rs` (SearchState + centered modal via `ratatui::layout::Rect` math + `Clear` widget), the `App` store-aware refactor (`App::new(store)`, `refresh_sidebar`, `open_node`, `open_page`, `refresh_search`, `refresh_current_view` — each a short store query + state swap), key routing (`/` and `Ctrl+P` open search; when `app.search.is_some()` all keys route to `on_key`), and `main()`:

```rust
// main.rs
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{Event, EventStream, MouseEventKind};
use futures::StreamExt;
use notion_tui::{app, config, terminal::TerminalGuard, ui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::load()?;
    let store = Arc::new(Mutex::new(notion_store::Store::open(&cfg.db_path)?));
    let client = notion_api::NotionClient::new(cfg.token.clone());
    let mut handle = notion_sync::spawn_puller(
        client, store.clone(), Duration::from_secs(cfg.poll_interval_secs));

    let _guard = TerminalGuard::enter()?;
    let mut term = ratatui::Terminal::new(
        ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;

    let mut app = app::App::new(store);
    app.refresh_sidebar();
    let mut events = EventStream::new();

    while !app.should_quit {
        term.draw(|f| ui::draw(f, &app))?;
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(key))) => app::dispatch_key(&mut app, key),
                Some(Ok(Event::Mouse(m))) => match m.kind {
                    MouseEventKind::ScrollDown => app::scroll(&mut app, 3),
                    MouseEventKind::ScrollUp => app::scroll(&mut app, -3),
                    _ => {}
                },
                Some(Ok(_)) => {}
                _ => break,
            },
            _ = handle.data_version.changed() => {
                app.refresh_sidebar();
                app.refresh_current_view();
            }
            _ = handle.status.changed() => {
                app.sync_status = handle.status.borrow().clone();
            }
        }
    }
    Ok(())
}
```

(`app::dispatch_key` wraps `handle_key` and applies the returned `Action` against the store: `OpenNode` → `open_node`, `OpenPage` → `open_page`. `app::scroll` forwards to the focused view's `move_cursor`.)

- [ ] **Step 4: Run the full suite** — `cargo test` (workspace root) → everything passes.

- [ ] **Step 5: Manual verification against real Notion** — create an internal integration at notion.so/my-integrations, share a page with it, then:

```bash
NOTION_TOKEN=ntn_… cargo run -p notion-tui
```

Expected: status bar shows syncing → synced; shared pages appear in the sidebar; Enter opens a page; `/` finds text from page bodies; `q` quits and the terminal is restored. (On a fresh large workspace the first crawl takes a while — rate-limited at 3 req/s.)

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(notion-tui): search modal, main event loop, e2e smoke test

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

## Self-Review Notes

- **Spec coverage (M1 scope):** API client w/ rate limits (T1–4), store + FTS (T5–6), puller w/ hwm incremental (T7), TUI shell + status bar (T8), sidebar (T9), page view w/ history (T10), table view (T11), search + wire-up + E2E (T12). Deferred per Global Constraints: syntect highlighting, mouse beyond wheel-scroll (full mouse targets M5 polish; wheel scroll included), first-run wizard/keyring (M5).
- **Type consistency:** `SharedStore = Arc<Mutex<Store>>` used by sync + tui; `set_timing` test hook defined in T2, used in T7/T12 tests; `Action`/`View` enums grow monotonically (T9 → T10 → T11) — implementers of later tasks must use the final signatures shown in their own task's Interfaces block.
- **Known simplifications (deliberate, documented):** sidebar lists data sources flat; `ParentRef::Block` mapped to `page_id`; numbered-list numbering resets on any non-numbered sibling; search over titles + block text only (row properties not FTS-indexed until M2).




