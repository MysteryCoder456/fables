# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`notion-tui` is a keyboard-first, offline-first terminal client for Notion. Everything renders from a local SQLite cache; edits queue locally and reconcile with the Notion API (version `2025-09-03`) in the background.

## Commands

This directory is the Cargo workspace root. Run all commands from here.

- Build: `cargo build` (release binaries via `./scripts/build-release.sh`)
- Test (all): `cargo test --workspace`
- Test one crate: `cargo test -p notion-tui-sync` (package names: `notion-tui-api`, `notion-tui-store`, `notion-tui-sync`, `notion-tui`)
- Test one function: `cargo test -p notion-tui-sync mid_batch_failure` (substring match on test name)
- One integration-test file: `cargo test -p notion-tui --test pull`
- Lint (must be clean): `cargo clippy --workspace --all-targets -- -D warnings`
- Format check: `cargo fmt --check`
- Run the app: `cargo run -p notion-tui`

The three gates that must pass before work is done: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`.

Snapshot tests use `insta`. `cargo-insta` is not assumed present — accept a new snapshot manually with `mv path/to/foo.snap.new path/to/foo.snap` after verifying the diff.

## Architecture

Four crates, strict dependency direction `notion-tui → notion-tui-sync → {notion-tui-store, notion-tui-api}`:

- **`notion-api`** — thin async Notion REST client (`reqwest`). `client.rs`/`endpoints.rs`/`types.rs`/`error.rs`. `ApiError::Api { status, .. }` carries HTTP status (e.g. 404 = page gone); network/retry failures are distinct variants.
- **`notion-store`** — the source of truth: a `rusqlite` SQLite cache (`schema.rs` = tables `pages`, `blocks`, `data_sources`, `rows`, `comments`, `pending_ops`, `sync_meta`, and an `fts5` virtual table). All reads and all local mutations go through `store.rs`.
- **`notion-sync`** — the background reconciler. `puller.rs` (pull_once + reconcile_deletions), `pusher.rs` (push_once), `lib.rs` (`spawn_sync` → `SyncHandle`, one_cycle).
- **`notion-tui`** — the ratatui UI. `app.rs` = state + logic; `ui/` = per-view widgets; `main.rs` = the async event loop.

### Offline-first edit flow (the core invariant)

Local edits are **optimistic**: a `Store::edit_*` method mutates the cache immediately, appends an op to `pending_ops`, and returns an `EditReceipt`. In the TUI every mutating path funnels through `apply_action` (`app.rs`) and pushes the receipt onto `App::undo_stack`, so `undo` is uniform across edit types. `pending_ops` is a durable outbox — nothing is lost if the process dies before sync.

The background sync loop (`one_cycle`) runs **push-then-pull** every `poll_interval_secs`:
1. **Push** drains `pending_ops`. Notion assigns real IDs to newly-created blocks/rows/comments, so the pusher rewrites the local temp id to the server id everywhere (`rewrite_block_id`/`rewrite_row_id`/`rewrite_comment_id`). Op `target_id` and payload `page_id` are the join keys used to attribute an op to a page.
2. **Pull** refreshes the cache. It is checkpointed and high-water-mark-filtered, so a resumed first crawl does not double-count and steady-state pulls skip unchanged pages. Because an hwm-filtered pull never sees a remote *deletion*, a full-workspace `reconcile_deletions` crawl runs every Nth cycle (and once on startup) to prune pages/data-sources that vanished server-side.

### Data-safety guards (do not remove)

Deleting local rows is dangerous because it can destroy unpushed edits. Two methods enforce the same guard: `prune_missing` and `forget_page` (used when the server 404s / archives a page) both **skip any page with pending ops** (direct or block-level, via a `pending_ops` join). `forget_page` returns `bool` — `false` means "kept because dirty". A page is only forgotten on a genuine 404/archived signal; offline / non-404 errors must never forget. `App::breadcrumb` walks `parent_id` upward with a visited-set + depth cap because a corrupt store could otherwise cycle forever (it runs every draw). `start_move_page` excludes the moved page's full descendant subtree so a move can't create a parent cycle.

### TUI event loop

`main.rs` runs a `tokio::select!` over three sources: crossterm `EventStream` (keys/mouse), an `mpsc` channel of `AppMsg` (async results — `Refreshed`, `PageGone`, `MergeReady`, …), and the `SyncHandle` watches (`status`/`pending`/`data_version`) that drive the status bar. Keys resolve through `keymap.rs` (user-rebindable) to an `Action`, applied via `dispatch_key`/`apply_action`.

Modal input uses two "purpose" enums so one widget serves many callers: `InputPurpose` (what a text-input submit means — rename, new row, filter, comment, …) and `PickerPurpose` (what a picker selection means — move page, group-by). The active `View` (`Page` / `Table` / `Board` / …) determines rendering and which actions are legal. Mouse is on by default but never *required*: every mouse action has a keyboard twin. Table filtering switches the view to a filtered subset — cursor/row lookups must use the visible rows, not the unfiltered backing store.
