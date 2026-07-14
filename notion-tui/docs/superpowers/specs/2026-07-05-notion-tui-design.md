# notion-tui — Design Spec

**Date:** 2026-07-05
**Status:** Approved for implementation planning

## 1. Purpose

A keyboard-first terminal client for Notion, usable as a daily-driver replacement for the Notion app: browse, read, create, and edit pages and databases from the terminal. Mouse support is available everywhere but never required. Built for responsiveness (including on a Raspberry Pi) and for working offline.

## 2. Requirements

- **Use case:** full read + write workhorse.
- **API:** official Notion API only, integration token auth, API version `2025-09-03` (databases contain data sources). Only content shared with the integration is visible; this is accepted.
- **Stack:** Rust. TUI: `ratatui` + `crossterm`. Async: `tokio`. HTTP: `reqwest`. Storage: `rusqlite` (SQLite, bundled). Serialization: `serde`/`serde_json`.
- **Editing model:** hybrid — inline quick edits in the TUI, `$EDITOR` Markdown round-trip for full-page edits.
- **Databases:** table view and kanban board view; full CRUD on rows; edit all common property types (title, rich text, number, select, multi-select, status, date, checkbox, URL, email, phone, people, relation).
- **Sync:** offline-first. SQLite is the UI's source of truth; writes are queued locally and pushed when online; conflicts are detected and surfaced, never silently resolved.
- **V1 extras:** workspace-wide instant search (local FTS index); comments (read and write, page- and block-level).
- **Out of scope for v1:** quick-capture hotkey, multiple workspaces, calendar/gallery/timeline views, inline image rendering, realtime collaboration presence.

## 3. Architecture

Single binary, Cargo workspace with four crates. Chosen over a daemon+client split (unneeded ops complexity for one user; the crate boundary allows evolving to a daemon later) and over a live-first cached client (cannot satisfy offline writes).

### 3.1 `notion-api`

Thin typed client for the official API. Built in-house — community Rust Notion crates are stale/incomplete and we need control over retry and rate budgeting.

- Endpoints: search, pages (get/create/update), blocks (children get/append, update, delete), data sources (query/schema), comments (list/create), users.
- Rate limiting: token-bucket budgeting to Notion's ~3 requests/second average; automatic retry with exponential backoff + jitter on 429 and 5xx, honoring `Retry-After`.
- Pagination handled internally (cursor loops) with an async-stream interface.
- Auth token supplied by caller; no storage concerns in this crate.

### 3.2 `notion-store`

SQLite persistence and query layer. All reads the UI performs go through here; all mutations are transactional.

Tables:
- `pages` — id, parent, title, icon, archived, `last_edited_time` (remote), `local_edited_at`, dirty flag.
- `blocks` — id, page_id, parent_block_id, ordinal, type, payload JSON, `has_children`.
- `data_sources` — id, database_id, schema JSON (property definitions).
- `rows` — page-in-database: id, data_source_id, properties JSON, timestamps, dirty flag.
- `comments` — id, parent (page/block), thread id, author, rich text, created time.
- `pending_ops` — the write queue: seq, op type, target id, payload, base `last_edited_time`, state (`pending`/`inflight`/`failed`/`conflicted`), error text.
- `sync_meta` — high-water marks, last full crawl, schema version.
- FTS5 virtual table over page titles + block plain text, kept in sync by triggers; powers instant search.

### 3.3 `notion-sync`

Background engine, two independent tokio loops, communicating with the UI via a watch/notify channel ("data changed", "sync state changed").

**Puller.** The official API has no changes feed. Strategy: poll the Search endpoint sorted by `last_edited_time` descending; walk results until reaching entries older than the stored high-water mark; re-fetch full block trees only for dirty pages; refresh data-source schemas and query changed rows similarly; fetch comments for changed pages. First run performs a full crawl (progress shown in the status bar). Steady-state cost when nothing changed: one search request per poll interval (default 30s, configurable).

**Pusher.** Drains `pending_ops` in sequence order, FIFO per page so a page's edits serialize correctly. Maps ops to API calls. On success: clear dirty flags, update remote timestamps/ids (temporary local ids for created pages/blocks/rows are rewritten to real ids everywhere). On network failure: pause, flip status to offline, resume on reconnect. On API rejection: mark op `failed` with the error, visible in the queue screen — never silently dropped.

**Conflict policy.** Each op records the target's remote `last_edited_time` at enqueue time (its *base*). Before pushing, the pusher compares the current remote value against the base. If the remote changed, the op becomes `conflicted`: the UI shows a banner and the conflicts screen offers *keep mine* (push anyway), *take theirs* (drop op, accept remote), or *merge* (both versions rendered as Markdown, diffed in `$EDITOR`, result pushed). The puller never overwrites local records that have unpushed edits.

### 3.4 `notion-tui` (binary)

The ratatui application. Core rule: **renders exclusively from `notion-store`; no screen ever awaits the network.** Sync notifications trigger re-query and redraw, so open views live-refresh as data lands.

- Elm-ish architecture: single `AppState`, event loop folding terminal events + sync notifications into state updates, pure view functions per screen.
- Async work (store queries off the hot path, `$EDITOR` subprocess, sync control) via message channels; the render loop never blocks.

## 4. UI/UX

### 4.1 Layout

Collapsible sidebar (workspace tree: pages hierarchy, databases, favorites) + main pane + one-line status bar (sync state `✓ synced · N pending · offline · syncing…`, current path, pending keystrokes). Modals (search, command palette, property form, dialogs, help) float centered.

### 4.2 Views

1. **Page** — block tree rendering: headings, paragraphs, to-dos (toggleable), bulleted/numbered lists, toggle blocks (collapsible), code (syntax-highlighted via `syntect`), quotes, callouts, dividers, simple tables, child pages/databases as navigable links, images as `[image: caption]` placeholders. One block is always the cursor block.
2. **Database table** — scrollable grid; column widths inferred from property types; sort and filter bound to keys; row cursor; `Enter` opens the row as a page.
3. **Database board** — kanban columns grouped by a chosen select/status property; moving a card between columns is an optimistic status write.
4. **Search** — modal fuzzy search over the FTS index; instant results; `Enter` jumps to page/row.
5. **Comments panel** — side panel listing threads for current page or cursor block; reply and new-thread composition inline.
6. **Conflicts/queue screen** — pending, failed, and conflicted ops with resolution actions.

### 4.3 Keyboard model

Vim-flavored, rebindable via config file (plain keys, named keys, and `ctrl+x`
chords) — except a small fixed set: Tab (focus switch), Ctrl+d/Ctrl+u
(half-page scroll), Ctrl+P (search), and Backspace-as-back. (Amended per the
2026-07-12 production-polish design, M8.10.)

- Motion: `j/k` cursor, `h/l` collapse/expand (page) or column switch (board), `g/G` top/bottom, `Ctrl+d/u` half-page.
- Navigation: `Enter` open/follow, `Backspace` or `-` back (browser-style history stack), `Tab` cycle panes, `1` toggle sidebar, `/` or `Ctrl+P` search, `c` comments, `?` help overlay, `:` command palette (rename, move page, change view, group-by, sync now, queue screen, …).
- Inline edits: `Space` toggle to-do, `i` edit cursor block text (inline input widget), `a` add block below, `o` new database row, `dd` delete block/row, `u` undo the most recent local edit this session (removes its op from the queue if still pending; if already pushed, the undo is enqueued as a new inverse edit), `p` property form for cursor row, `J/K` move card across board columns.
- Heavy edits: `e` opens the page (or selected subtree) as Markdown in `$EDITOR`.

### 4.4 `$EDITOR` round-trip

Page blocks → Markdown; on save, old→new Markdown is diffed and translated into per-block operations (update / insert / delete / move) so unchanged blocks keep their IDs and attached comments. Blocks Markdown cannot express (callouts, toggles, synced blocks, embeds, colors) are emitted as protected islands delimited by `<!--notion:block-id-->` comments that survive round-trips untouched unless deliberately deleted (deletion deletes the block, after confirmation).

### 4.5 Mouse (optional, on by default)

Click to move cursor/focus panes, click links and sidebar entries, wheel scroll, click column headers to sort, drag board cards between columns. No mouse-exclusive features.

## 5. Config & auth

- Integration token in the system keyring (`keyring` crate); fallback: `~/.config/notion-tui/config.toml` with 0600 permissions and a startup warning.
- Config (TOML): poll interval, editor override, theme, keybinding overrides, default database views, mouse on/off.
- First-run wizard: prompt for token, validate with a `users/me` call, kick off initial crawl with progress display.

## 6. Error handling & resilience

- Network errors never interrupt interaction: status flips to offline, pusher pauses, puller backs off; reads keep working from SQLite.
- 429/5xx absorbed inside `notion-api` (backoff + `Retry-After`); sync slows rather than fails.
- API rejections of queued writes → visible `failed` ops with reason; retry or discard from the queue screen.
- All store mutations transactional; crash mid-sync cannot corrupt local state. Store schema versioned with migrations.
- Panics in the TUI restore the terminal (raw mode/alternate screen guard) before propagating.

## 7. Testing

- Unit tests per crate.
- Markdown↔blocks converter: property-based round-trip tests (`proptest`) — the highest-risk correctness surface.
- Sync engine: integration tests against a mock Notion server (`wiremock`) with recorded fixtures, covering incremental pull, first crawl, offline queueing, reconnect drain, and all three conflict resolutions.
- Store: query + migration tests on temp databases.
- TUI: snapshot tests via ratatui `TestBackend` (`insta`); one end-to-end smoke test driving the event loop against the mock server.

## 8. Milestones

1. **Read-only core** — api + store + puller + page/table views, search. Usable browser.
2. **Writes** — pending_ops, pusher, inline edits, row CRUD, property form.
3. **Editor round-trip** — Markdown converter + diff-to-ops, `e` flow.
4. **Board view, comments, conflicts UI** — remaining v1 surface.
5. **Polish** — themes, rebinding, first-run wizard, packaging (single static binary, aarch64 + x86_64).
