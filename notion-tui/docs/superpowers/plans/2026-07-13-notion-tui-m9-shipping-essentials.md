# notion-tui M9 — Shipping Essentials: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make notion-tui a credible public artifact: a real CLI surface, a log file a bug report can be filed from, a sync engine that respects rate limits and backs off on failure, crates.io-clean metadata, a docs storefront, and Windows fit-and-finish.

**Architecture:** No new crates. `notion-tui` gains `cli.rs` (hand-rolled flag parser handled before anything else starts) and `logging.rs` (tracing → rotating file in the state dir + panic-hook chaining); `notion-sync` gains capped exponential backoff and takes an `Arc<NotionClient>` so UI and sync share one pacing budget; `notion-sync/puller.rs` gains an hwm overlap window with a stored-`last_edited_time` skip so eventually-consistent search ordering can't lose items; docs (README/CHANGELOG/CONTRIBUTING/demo tape) and Cargo metadata round out the shipping surface.

**Tech Stack:** Existing: ratatui/crossterm, tokio, reqwest(rustls), rusqlite(bundled), keyring, wiremock, TestBackend, tempfile. New deps: `tracing` (already in the lockfile transitively), `tracing-subscriber` (env-filter), `tracing-appender`. No CLI-parsing dependency (see Task 1 rationale). Docs tooling: `vhs` (local-only, not a build dep).

## Global Constraints

- Platforms: Linux x86_64 + aarch64, macOS, Windows — every code task must pass the existing three-OS CI matrix.
- Quality gates after every task, run from `/Users/rehatbir/Developer/fables/notion-tui`: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`.
- Combined API pacing stays ~3 rps: one `NotionClient` (`min_interval` = 334 ms) shared by UI and sync; never construct a second client for app-loop use.
- MSRV policy: `rust-version = "1.85"` in `[workspace.package]` (edition-2024 transitive deps in the current lockfile require ≥ 1.85), empirically verified and, if needed, raised in Task 11; enforced by a dedicated `msrv` CI job using `--locked`.
- Log-file requirement (spec §7 verification bar): rotating daily log at `<data_dir>/notion-tui/logs/notion-tui.YYYY-MM-DD.log` (7 files retained); a bug report must be fileable from that file alone, and `notion-tui --version` must work from a packaged binary on each OS.
- The git root is the monorepo `/Users/rehatbir/Developer/fables`; the workspace lives in `notion-tui/`; CI lives at the monorepo's `.github/workflows/notion-tui-ci.yml`.
- **Baseline:** this plan assumes the M6 working-tree changes and the M7 plan (`2026-07-13-notion-tui-m7-finish-v1-spec.md`) have landed. Task interfaces below consume M7's final shapes: `pull_once(client, store, status_tx)`, `SyncStatus::Syncing { done, total }`, `SyncHandle.notify` + `request_sync_now`, the `pull_cursor`/`pull_max_seen`/`pull_done` checkpoint meta keys, `reconcile_deletions` every 20 cycles, and `Store::meta_delete`. If M8 has also landed, nothing here conflicts (M8 touches modal theming/inputs/wizard wording, none of which these tasks edit — except `config.rs` error text, see Task 2 note).
- Never weaken an existing test to make it pass; if a behavior intentionally changes, update the test to assert the new behavior and say so (Task 6 does this once, explicitly).
- Commit steps below are for the executing agent at the end of each green task; run them exactly as written. If the session owner has asked for no commits, skip commit steps and say so in the report.

## Execution waves (parallelization map)

- **Wave 1 (four parallel tracks, coordinated files):**
  - Track CLI: Task 1 → Task 2 → Task 3 (`crates/notion-tui/src/{cli.rs,config.rs,logging.rs,lib.rs,main.rs}`, `tests/cli.rs`, workspace `Cargo.toml` dep additions). Owns `main.rs` EXCEPT the client-construction block (see Track SYNC).
  - Track SYNC: Task 4 → Task 5 → Task 6 (`crates/notion-sync/src/{lib.rs,puller.rs}`, `crates/notion-sync/tests/*`, `crates/notion-api/tests/pacing.rs`). Task 4 owns ONLY the two client-construction sites in `main.rs` (the `NotionClient::new` line feeding `spawn_sync` and the `remote_client` block) — nothing else in that file.
  - Track DOCS: Task 7 → Task 8 (`CHANGELOG.md`, `CONTRIBUTING.md`, `README.md`, `docs/demo.tape`). Owns `README.md` wholly (Track CLI must not touch it).
  - Track WIN: Task 9 (`crates/notion-tui/src/editor.rs`, `crates/notion-tui/tests/windows.rs`).
- **Wave 2 (serial, after all Wave-1 tracks merge):** Task 10 (tracing instrumentation — touches all four crates) → Task 11 (publishing hygiene — touches every `Cargo.toml` + CI yml) → Task 12 (integration sweep + verification bar).

---

### Task 1: CLI surface — `--version`/`--help` handled before anything starts

Today `main.rs` never reads `std::env::args()`: `notion-tui --version` loads config (or launches the first-run wizard) and enters the alternate screen. Fix by parsing argv as the very first statement of `main`.

**Rationale for hand-rolling:** the workspace has no arg-parsing crate (no `clap`/`pico-args`/`lexopt` in `Cargo.lock`), the surface is exactly five flags, and the codebase deliberately keeps the dep tree small (reqwest is the only heavyweight). A ~40-line parser with unit tests beats a new dependency; revisit only if the flag surface grows past ~10.

**Files:**
- Create: `crates/notion-tui/src/cli.rs`
- Create: `crates/notion-tui/tests/cli.rs`
- Modify: `crates/notion-tui/src/lib.rs` (add `pub mod cli;`)
- Modify: `crates/notion-tui/src/main.rs` (top of `main`, before `config::load`)

**Interfaces:**
- Consumes: nothing.
- Produces (Tasks 2 and 3 consume `CliOptions`):

```rust
// crates/notion-tui/src/cli.rs
#[derive(Debug, Default, PartialEq)]
pub struct CliOptions {
    pub config: Option<std::path::PathBuf>,
    pub db_path: Option<std::path::PathBuf>,
    pub debug: bool,
}

#[derive(Debug, PartialEq)]
pub enum Cli {
    Run(CliOptions),
    Version,
    Help,
}

pub const HELP: &str = /* usage text, see Step 3 */;
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String>;
```

- [ ] **Step 1: Write the failing unit tests**

Create `crates/notion-tui/src/cli.rs` containing only the tests module for now (the parser lands in Step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_vec(args: &[&str]) -> Result<Cli, String> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn version_and_help_flags_short_circuit() {
        assert_eq!(parse_vec(&["--version"]), Ok(Cli::Version));
        assert_eq!(parse_vec(&["-V"]), Ok(Cli::Version));
        assert_eq!(parse_vec(&["--help"]), Ok(Cli::Help));
        assert_eq!(parse_vec(&["-h"]), Ok(Cli::Help));
        // Short-circuit even when combined with other (even invalid) args after it.
        assert_eq!(parse_vec(&["--version", "--bogus"]), Ok(Cli::Version));
    }

    #[test]
    fn no_args_runs_with_defaults() {
        assert_eq!(parse_vec(&[]), Ok(Cli::Run(CliOptions::default())));
    }

    #[test]
    fn value_flags_accept_space_and_equals_forms() {
        let expected = Cli::Run(CliOptions {
            config: Some(PathBuf::from("/tmp/c.toml")),
            db_path: Some(PathBuf::from("/tmp/n.db")),
            debug: true,
        });
        assert_eq!(
            parse_vec(&["--config", "/tmp/c.toml", "--db-path=/tmp/n.db", "--debug"]),
            expected
        );
    }

    #[test]
    fn unknown_flag_is_a_named_error() {
        let err = parse_vec(&["--bogus"]).unwrap_err();
        assert!(err.contains("--bogus"), "error must name the argument: {err}");
    }

    #[test]
    fn missing_value_is_a_named_error() {
        let err = parse_vec(&["--config"]).unwrap_err();
        assert!(err.contains("--config") && err.contains("value"), "{err}");
    }

    #[test]
    fn help_text_mentions_every_flag() {
        for flag in ["--version", "--help", "--config", "--db-path", "--debug"] {
            assert!(HELP.contains(flag), "HELP is missing {flag}");
        }
    }
}
```

Add `pub mod cli;` to `crates/notion-tui/src/lib.rs` (alphabetical: between `app` and `config`).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p notion-tui cli`
Expected: COMPILE FAIL — `parse`, `Cli`, `CliOptions`, `HELP` don't exist.

- [ ] **Step 3: Implement the parser**

Fill in `crates/notion-tui/src/cli.rs` above the tests module:

```rust
use std::path::PathBuf;

/// Printed by `--help` and (to stderr) on argument errors.
pub const HELP: &str = "\
notion-tui — a keyboard-first, offline-first terminal client for Notion

Usage: notion-tui [OPTIONS]

Options:
      --config <PATH>   Read config from PATH instead of the platform default
                        (<config_dir>/notion-tui/config.toml)
      --db-path <PATH>  Use PATH for the local SQLite cache instead of the
                        platform default (<data_dir>/notion-tui/notion.db)
      --debug           Verbose logging to the log file (same as RUST_LOG=debug)
  -h, --help            Print help
  -V, --version         Print version
";

#[derive(Debug, Default, PartialEq)]
pub struct CliOptions {
    pub config: Option<PathBuf>,
    pub db_path: Option<PathBuf>,
    pub debug: bool,
}

#[derive(Debug, PartialEq)]
pub enum Cli {
    Run(CliOptions),
    Version,
    Help,
}

/// Hand-rolled parser: five flags don't justify a dependency (nothing in the
/// tree parses args today). Accepts `--flag value` and `--flag=value`.
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Cli, String> {
    let mut opts = CliOptions::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f.to_string(), Some(v.to_string())),
            None => (arg, None),
        };
        match flag.as_str() {
            "--version" | "-V" => return Ok(Cli::Version),
            "--help" | "-h" => return Ok(Cli::Help),
            "--debug" => opts.debug = true,
            "--config" => {
                let v = inline.or_else(|| it.next()).ok_or("--config requires a value")?;
                opts.config = Some(PathBuf::from(v));
            }
            "--db-path" => {
                let v = inline.or_else(|| it.next()).ok_or("--db-path requires a value")?;
                opts.db_path = Some(PathBuf::from(v));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(Cli::Run(opts))
}
```

Run: `cargo test -p notion-tui cli`
Expected: PASS.

- [ ] **Step 4: Write the failing binary-level integration test**

Create `crates/notion-tui/tests/cli.rs`. `CARGO_BIN_EXE_notion-tui` is provided by cargo for the crate's own integration tests — no `assert_cmd` dependency needed. `Command::output()` nulls stdin, so if the wizard were (incorrectly) reached it would hit EOF and exit non-zero, failing the test.

```rust
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_notion-tui"))
}

#[test]
fn version_prints_and_exits_zero_without_config_token_or_tty() {
    let out = bin().arg("--version").env_remove("NOTION_TOKEN").output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.trim(), format!("notion-tui {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_prints_usage_and_exits_zero() {
    let out = bin().arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Usage: notion-tui"));
    assert!(stdout.contains("--db-path"));
}

#[test]
fn unknown_flag_exits_2_with_a_named_error_on_stderr() {
    let out = bin().arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("--bogus"), "stderr: {stderr}");
    assert!(stderr.contains("Usage: notion-tui"), "errors should include usage");
}
```

- [ ] **Step 5: Run to verify failure**

Run: `cargo test -p notion-tui --test cli`
Expected: FAIL — `--version` currently either launches the wizard (blocks then errors on EOF stdin, wrong output) or enters the TUI; `--bogus` is ignored entirely.

- [ ] **Step 6: Wire `main.rs`**

At the very top of `main()` in `crates/notion-tui/src/main.rs`, before `config::load`:

```rust
    let cli_opts = match notion_tui::cli::parse(std::env::args().skip(1)) {
        Ok(notion_tui::cli::Cli::Version) => {
            println!("notion-tui {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Ok(notion_tui::cli::Cli::Help) => {
            print!("{}", notion_tui::cli::HELP);
            return Ok(());
        }
        Ok(notion_tui::cli::Cli::Run(opts)) => opts,
        Err(msg) => {
            eprint!("error: {msg}\n\n{}", notion_tui::cli::HELP);
            std::process::exit(2);
        }
    };
    let _ = &cli_opts; // consumed by Tasks 2 and 3
```

(The `let _ = &cli_opts;` placeholder keeps clippy's unused-variable lint green until Task 2 consumes it; Task 2 deletes it.)

- [ ] **Step 7: Run to pass + full gate**

Run: `cargo test -p notion-tui --test cli && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

- [ ] **Step 8: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): handle --version/--help/--debug/--config/--db-path before the TUI starts"`

---

### Task 2: `--config` and `--db-path` thread into config loading

**Files:**
- Modify: `crates/notion-tui/src/config.rs` (`load_with_opts`, existing `load`/`load_with_token` become wrappers)
- Modify: `crates/notion-tui/src/cli.rs` (`CliOptions::apply`)
- Modify: `crates/notion-tui/src/main.rs` (pass `cli_opts` through)

**Interfaces:**
- Consumes: Task 1's `CliOptions`.
- Produces:
  - `pub fn load_with_opts(token: Option<String>, config_path: Option<&std::path::Path>) -> anyhow::Result<Config>` — an explicit `config_path` that can't be read is a hard error naming the path (silently ignoring a typo'd `--config` would be worse than no flag); the default path keeps today's exists-filter behavior.
  - `impl CliOptions { pub fn apply(&self, cfg: &mut crate::config::Config) }` — applies `db_path` (CLI beats every config source).
  - Scope note (documented in Task 8's README): `--config` affects *reading* config. The wizard's no-keyring fallback still writes the token to the default path via `store_token` — the keyring is the primary sink and re-pointing the fallback write is not worth threading a path through `wizard::run` for v1.
  - Coordination note: M8.7 ("friendly config errors") also edits `config.rs` error text; if it has landed, keep its improved messages and only add the path-selection logic here.

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module in `crates/notion-tui/src/config.rs`:

```rust
    #[test]
    fn explicit_config_path_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("custom.toml");
        std::fs::write(&p, "token = \"custom-tok\"\ntheme = \"light\"\n").unwrap();
        let cfg = load_with_opts(None, Some(&p)).unwrap();
        // Assert on theme, not token: a real keyring/NOTION_TOKEN on the dev
        // machine legitimately outranks the file token.
        assert_eq!(cfg.theme, "light");
    }

    #[test]
    fn explicit_config_path_that_cannot_be_read_is_a_hard_error() {
        let err = load_with_opts(None, Some(std::path::Path::new("/nonexistent/nt.toml")))
            .unwrap_err()
            .to_string();
        assert!(err.contains("/nonexistent/nt.toml"), "must name the path: {err}");
    }
```

And to `crates/notion-tui/src/cli.rs`'s tests module:

```rust
    #[test]
    fn apply_overrides_db_path_and_nothing_else() {
        let mut cfg = crate::config::from_sources(
            Some("tok".into()),
            None,
            Some("theme = \"light\""),
            PathBuf::from("/default/notion.db"),
        )
        .unwrap();
        CliOptions::default().apply(&mut cfg);
        assert_eq!(cfg.db_path, PathBuf::from("/default/notion.db"));

        let opts = CliOptions { db_path: Some(PathBuf::from("/cli/override.db")), ..Default::default() };
        opts.apply(&mut cfg);
        assert_eq!(cfg.db_path, PathBuf::from("/cli/override.db"));
        assert_eq!(cfg.theme, "light", "apply must not touch other fields");
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p notion-tui config && cargo test -p notion-tui cli`
Expected: COMPILE FAIL — `load_with_opts` and `apply` don't exist.

- [ ] **Step 3: Implement**

In `crates/notion-tui/src/config.rs`, replace the body of `load_with_token` with a delegation and add `load_with_opts` (the file-reading logic moves; `from_sources` is untouched):

```rust
/// Like `load`, but `token` (when given) takes precedence over every other
/// source, and `config_path` (when given, i.e. `--config`) replaces the
/// default config file location. An explicit path that can't be read is a
/// hard error; the default path is simply skipped when absent.
pub fn load_with_opts(token: Option<String>, config_path: Option<&std::path::Path>) -> anyhow::Result<Config> {
    let file = match config_path {
        Some(p) => Some(
            std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("--config {}: {e}", p.display()))?,
        ),
        None => dirs::config_dir()
            .map(|d| d.join("notion-tui/config.toml"))
            .filter(|p| p.exists())
            .and_then(|p| std::fs::read_to_string(p).ok()),
    };
    let default_db = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("notion-tui/notion.db");
    from_sources(
        token.or_else(token_from_keyring),
        std::env::var("NOTION_TOKEN").ok(),
        file.as_deref(),
        default_db,
    )
}

pub fn load_with_token(token: Option<String>) -> anyhow::Result<Config> {
    load_with_opts(token, None)
}
```

(Keep the doc comment currently on `load_with_token` — move it onto `load_with_opts` and merge with the text above.)

In `crates/notion-tui/src/cli.rs`:

```rust
impl CliOptions {
    /// CLI overrides beat every config source.
    pub fn apply(&self, cfg: &mut crate::config::Config) {
        if let Some(db) = &self.db_path {
            cfg.db_path = db.clone();
        }
    }
}
```

In `crates/notion-tui/src/main.rs`, replace the `config::load()` match (and delete Task 1's `let _ = &cli_opts;`):

```rust
    let mut cfg = match config::load_with_opts(None, cli_opts.config.as_deref()) {
        Ok(cfg) => cfg,
        Err(e) if e.to_string().contains("NOTION_TOKEN") => {
            let token = notion_tui::wizard::run().await?;
            config::load_with_opts(Some(token), cli_opts.config.as_deref())?
        }
        Err(e) => return Err(e),
    };
    cli_opts.apply(&mut cfg);
```

- [ ] **Step 4: Run to pass + full gate**

Run: `cargo test -p notion-tui && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (pre-existing config tests exercise `from_sources`, which is unchanged).

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): --config and --db-path override the default locations"`

---

### Task 3: Observability — rotating file log + panic-hook backtrace

**Files:**
- Create: `crates/notion-tui/src/logging.rs`
- Modify: `crates/notion-tui/src/lib.rs` (add `pub mod logging;`)
- Modify: `crates/notion-tui/src/main.rs` (init after CLI parse, before config load)
- Modify: `/Users/rehatbir/Developer/fables/notion-tui/Cargo.toml` (`[workspace.dependencies]`)
- Modify: `crates/notion-tui/Cargo.toml` (deps)

**Interfaces:**
- Consumes: Task 1's `CliOptions.debug`.
- Produces (Task 10 emits events against this; Task 8 documents the path):
  - `pub fn log_dir() -> PathBuf` — `<data_dir>/notion-tui/logs` (next to the default DB; `dirs::data_dir()` = `~/.local/share` on Linux, `~/Library/Application Support` on macOS, `%APPDATA%` on Windows).
  - `pub fn file_subscriber(dir: &Path, filter: EnvFilter) -> anyhow::Result<(impl Subscriber + Send + Sync, WorkerGuard)>` — build-only, unit-testable (the global subscriber can be set once per process, so tests use `tracing::subscriber::with_default`).
  - `pub fn init(debug: bool) -> anyhow::Result<WorkerGuard>` — installs the global subscriber (`RUST_LOG` wins; else `debug` → `debug`, else `info` with hyper/reqwest/rustls capped at `warn`) and chains a panic hook that logs message + `std::backtrace::Backtrace::force_capture()`.
  - Panic-hook ordering (spec: "writes the backtrace to the log **after** restoring the terminal"): `init` runs before `TerminalGuard::enter` (`terminal.rs:18-22`), so the guard's hook wraps ours. On panic: guard hook → `restore()` → our hook → log backtrace → original hook → stderr report.
  - Rotation: `tracing_appender::rolling` daily, prefix `notion-tui`, suffix `log`, `max_log_files(7)` → `notion-tui.YYYY-MM-DD.log`.

- [ ] **Step 1: Add the dependencies**

In `/Users/rehatbir/Developer/fables/notion-tui/Cargo.toml` `[workspace.dependencies]` (after `thiserror`):

```toml
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
```

In `crates/notion-tui/Cargo.toml` `[dependencies]`:

```toml
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
tracing-appender = { workspace = true }
```

Run: `cargo build -p notion-tui`
Expected: compiles (deps resolve; `tracing` was already in the lockfile transitively).

- [ ] **Step 2: Write the failing test**

Create `crates/notion-tui/src/logging.rs` with only the tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_subscriber_writes_events_to_a_rotating_file_in_the_given_dir() {
        let dir = tempfile::tempdir().unwrap();
        let (subscriber, guard) =
            file_subscriber(dir.path(), tracing_subscriber::EnvFilter::new("debug")).unwrap();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("hello-logfile");
        });
        drop(guard); // flush the non-blocking writer

        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "expected exactly one log file: {files:?}");
        let name = files[0].file_name().unwrap().to_string_lossy().to_string();
        assert!(name.starts_with("notion-tui.") && name.ends_with(".log"), "{name}");
        let contents = std::fs::read_to_string(&files[0]).unwrap();
        assert!(contents.contains("hello-logfile"), "log was: {contents}");
    }

    #[test]
    fn log_dir_lives_in_the_state_dir_next_to_the_db() {
        let dir = log_dir();
        assert!(dir.ends_with("notion-tui/logs"), "{dir:?}");
    }
}
```

Add `pub mod logging;` to `crates/notion-tui/src/lib.rs` (between `keymap` and `markdown`).

Run: `cargo test -p notion-tui logging`
Expected: COMPILE FAIL — `file_subscriber`/`log_dir` don't exist.

- [ ] **Step 3: Implement**

Above the tests module in `logging.rs`:

```rust
use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Directory the rotating log files live in: `<data_dir>/notion-tui/logs`,
/// next to the default DB. Documented in the README (Troubleshooting) so bug
/// reports can be filed from the log alone.
pub fn log_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("notion-tui/logs")
}

/// Builds (but does not install) the file-logging subscriber. Split from
/// `init` because the global subscriber can only be set once per process —
/// tests exercise this via `tracing::subscriber::with_default`.
pub fn file_subscriber(
    dir: &Path,
    filter: EnvFilter,
) -> anyhow::Result<(impl tracing::Subscriber + Send + Sync, WorkerGuard)> {
    std::fs::create_dir_all(dir)?;
    let appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("notion-tui")
        .filename_suffix("log")
        .max_log_files(7)
        .build(dir)?;
    let (writer, guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
        .lossy(false)
        .finish(appender);
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .finish();
    Ok((subscriber, guard))
}

/// Installs the global file subscriber and a panic hook that logs the panic
/// message + backtrace. MUST run before `TerminalGuard::enter`: the guard's
/// hook (installed later) wraps this one, so a panic restores the terminal
/// first, then logs, then prints the default stderr report.
pub fn init(debug: bool) -> anyhow::Result<WorkerGuard> {
    let default_directives = if debug {
        "debug"
    } else {
        "info,hyper=warn,hyper_util=warn,reqwest=warn,rustls=warn"
    };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_directives));
    let (subscriber, guard) = file_subscriber(&log_dir(), filter)?;
    tracing::subscriber::set_global_default(subscriber)?;

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        tracing::error!("panic: {info}\nbacktrace:\n{backtrace}");
        prev(info);
    }));

    tracing::info!(version = env!("CARGO_PKG_VERSION"), "notion-tui starting");
    Ok(guard)
}
```

In `crates/notion-tui/src/main.rs`, immediately after the CLI match from Task 1 (before `config::load_with_opts`, so wizard/network problems get logged too):

```rust
    // File logging is best-effort: a read-only home dir must not block the app.
    let _log_guard = match notion_tui::logging::init(cli_opts.debug) {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("warning: file logging disabled: {e}");
            None
        }
    };
```

(`_log_guard` must live until `main` returns — the underscore-prefixed binding does that; do NOT use a bare `_`, which would drop it immediately.)

- [ ] **Step 4: Run to pass + full gate**

Run: `cargo test -p notion-tui logging && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/Cargo.toml notion-tui/Cargo.lock notion-tui/crates/notion-tui && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): rotating file log in the state dir with panic-hook backtraces"`

---

### Task 4: Share one `NotionClient` between UI and sync

Pacing is per-instance: `NotionClient.next_allowed: Mutex<Instant>` (`crates/notion-api/src/client.rs:23`) is advanced by `pace()` (`client.rs:100-107`) with `min_interval` = 334 ms. `main.rs` currently builds TWO clients — one moved into `spawn_sync` (line 19-21) and a separate `remote_client` Arc for interactive fetches (lines 34-42) — so combined traffic can reach ~6 rps. Share one `Arc<NotionClient>`.

**Files:**
- Modify: `crates/notion-sync/src/lib.rs` (`spawn_sync` signature)
- Modify: `crates/notion-tui/src/main.rs` (ONLY the client-construction block — Track CLI owns the rest of the file)
- Modify: `crates/notion-sync/tests/sync_loop.rs` (call sites)
- Create: `crates/notion-api/tests/pacing.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Task 5 edits the same loop next):
  - `pub fn spawn_sync(client: std::sync::Arc<NotionClient>, store: SharedStore, interval: Duration) -> SyncHandle` (was `client: NotionClient`; the `Arc::new(client)` inside the spawned task is deleted).
  - `main.rs` builds exactly one `Arc<NotionClient>`, cloned into `spawn_sync` and into `app::RemoteHandle.client` (already typed `Arc<NotionClient>`, `app.rs:53-56`); the `remote_client` construction and its "second independent pacing budget" comment are deleted.

- [ ] **Step 1: Write the failing (compile-level) test change + pacing regression pin**

In `crates/notion-sync/tests/sync_loop.rs`, wrap both `spawn_sync` call sites (lines 83 and 114) in `Arc::new`:

```rust
    let mut handle = spawn_sync(Arc::new(client), store.clone(), Duration::from_millis(50));
```

(and the same at the second site, which passes `store` by value). If M7's Task 12 tests added more `spawn_sync` call sites in this file, wrap those identically.

Create `crates/notion-api/tests/pacing.rs` — this test PASSES already; it pins the per-instance pacing semantics that make sharing the fix (if pacing ever moved off the instance, this catches it):

```rust
use std::sync::Arc;
use std::time::Duration;

use notion_api::NotionClient;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_shared_client_paces_concurrent_callers_through_one_budget() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "user", "id": "u1", "name": "Bot", "type": "bot"
        })))
        .mount(&server)
        .await;

    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(50), Duration::from_millis(1));
    let client = Arc::new(c);

    let started = std::time::Instant::now();
    let tasks: Vec<_> = (0..4)
        .map(|_| {
            let c = client.clone();
            tokio::spawn(async move { c.get_json("/v1/users/me").await.unwrap() })
        })
        .collect();
    for t in tasks {
        t.await.unwrap();
    }
    // Four requests through one 50 ms-spaced budget: the fourth cannot
    // complete before three spacing intervals have elapsed.
    assert!(
        started.elapsed() >= Duration::from_millis(150),
        "shared pacing must serialize concurrent callers (elapsed {:?})",
        started.elapsed()
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p notion-tui-sync --test sync_loop`
Expected: COMPILE FAIL — `spawn_sync` takes `NotionClient`, not `Arc<NotionClient>`.

Run: `cargo test -p notion-tui-api --test pacing`
Expected: PASS (regression pin, documented above).

- [ ] **Step 3: Implement**

`crates/notion-sync/src/lib.rs` — change the signature and delete the internal re-wrap:

```rust
pub fn spawn_sync(client: Arc<NotionClient>, store: SharedStore, interval: Duration) -> SyncHandle {
```

and inside the `tokio::spawn(async move { ... })`, delete the line `let client = Arc::new(client);` (the moved `client` is already an `Arc`; `one_cycle(&client, ...)` compiles unchanged).

`crates/notion-tui/src/main.rs` — replace the two-client setup. Delete the `remote_client` block (currently lines 34-42, including the "Separate client instance…" comment) and change the construction to:

```rust
    // ONE client shared by sync and interactive fetches: pacing is
    // per-instance, so sharing is what keeps combined traffic at ~3 rps.
    let client = std::sync::Arc::new(notion_api::NotionClient::new(cfg.token.clone()));
    let mut handle = notion_sync::spawn_sync(
        client.clone(),
        store.clone(),
        Duration::from_secs(cfg.poll_interval_secs),
    );
```

and where `app.remote` is set:

```rust
    app.remote = Some(app::RemoteHandle { client, tx: app_tx });
```

- [ ] **Step 4: Run to pass + full gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (only `sync_loop.rs` and `main.rs` construct-and-pass clients to `spawn_sync`).

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-sync): share one NotionClient between UI and sync for combined ~3 rps pacing"`

---

### Task 5: Offline/failure backoff with cap

Today the loop in `spawn_sync` sleeps a fixed `interval` (default 30 s) even while `Offline`/`Failed`, hammering an unreachable network every 30 s forever. Add per-consecutive-failure exponential backoff, capped, reset on the first healthy cycle; the manual `sync now` wake (M7's `notify` select arm) still fires immediately.

**Files:**
- Modify: `crates/notion-sync/src/lib.rs` (`backoff_delay` + loop wiring; inline tests)
- Modify: `crates/notion-sync/tests/sync_loop.rs` (behavioral test)

**Interfaces:**
- Consumes: Task 4's `Arc<NotionClient>` signature; M7's `tokio::select!` sleep/notify arm.
- Produces:

```rust
const BACKOFF_CAP: Duration = Duration::from_secs(300);

/// Delay before the next cycle: the configured interval after a healthy
/// cycle, doubling per consecutive failed cycle, capped at 5 minutes (or the
/// configured interval, whichever is larger — a user-configured 10-minute
/// poll is never shortened by backoff).
fn backoff_delay(base: Duration, consecutive_failures: u32) -> Duration
```

  - Failure detection reads the status the cycle just published: `matches!(*status_tx.borrow(), SyncStatus::Offline | SyncStatus::Failed(_))` — no `one_cycle` signature change.

- [ ] **Step 1: Write the failing unit tests**

Add to the bottom of `crates/notion-sync/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_per_consecutive_failure_and_caps_at_five_minutes() {
        let base = Duration::from_secs(30);
        assert_eq!(backoff_delay(base, 0), Duration::from_secs(30));
        assert_eq!(backoff_delay(base, 1), Duration::from_secs(60));
        assert_eq!(backoff_delay(base, 2), Duration::from_secs(120));
        assert_eq!(backoff_delay(base, 3), Duration::from_secs(240));
        assert_eq!(backoff_delay(base, 4), Duration::from_secs(300));
        assert_eq!(backoff_delay(base, 32), Duration::from_secs(300), "no overflow at huge counts");
    }

    #[test]
    fn backoff_never_shortens_a_long_configured_interval() {
        let base = Duration::from_secs(600);
        assert_eq!(backoff_delay(base, 5), Duration::from_secs(600));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p notion-tui-sync --lib`
Expected: COMPILE FAIL — `backoff_delay` doesn't exist.

- [ ] **Step 3: Implement the pure function**

In `lib.rs`, next to `spawn_sync`:

```rust
const BACKOFF_CAP: Duration = Duration::from_secs(300);

fn backoff_delay(base: Duration, consecutive_failures: u32) -> Duration {
    let cap = BACKOFF_CAP.max(base);
    base.saturating_mul(2u32.saturating_pow(consecutive_failures.min(16))).min(cap)
}
```

Run: `cargo test -p notion-tui-sync --lib`
Expected: PASS.

- [ ] **Step 4: Write the failing behavioral test**

Append to `crates/notion-sync/tests/sync_loop.rs`:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failing_cycles_back_off_instead_of_hammering() {
    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let mut client = NotionClient::with_base_url("t", server.uri());
    client.set_timing(Duration::from_millis(1), Duration::from_millis(1));

    let _handle = spawn_sync(Arc::new(client), store, Duration::from_millis(25));
    tokio::time::sleep(Duration::from_millis(600)).await;

    // Each failed cycle costs exactly MAX_ATTEMPTS(5) search requests
    // (500s on the idempotent search path are client-retried). With backoff
    // (25→50→100→200→400 ms) ~5 cycles fit in 600 ms; a fixed 25 ms interval
    // would run ~24. The bound is deliberately loose for slow CI runners.
    let cycles = server.received_requests().await.unwrap().len() / 5;
    assert!(cycles <= 8, "expected backed-off cycles (~5), got {cycles} — fixed-interval hammering?");
}
```

Run: `cargo test -p notion-tui-sync --test sync_loop failing_cycles`
Expected: FAIL — fixed interval yields ~24 cycles.

- [ ] **Step 5: Wire the loop**

In `spawn_sync`'s spawned task, thread a failure counter around the existing `catch_unwind` + `select!` (M7 shape shown; keep `cycle_count` if M7's Task 13 added it):

```rust
        let mut consecutive_failures: u32 = 0;
        loop {
            let cycle = one_cycle(&client, &store, &status_tx, &data_tx, &pending_tx, cycle_count);
            if let Err(payload) = futures::FutureExt::catch_unwind(std::panic::AssertUnwindSafe(cycle)).await
            {
                let _ = payload;
                status_tx.send_replace(SyncStatus::Failed(
                    "sync engine crashed — restart notion-tui".into(),
                ));
            }
            let failed = matches!(*status_tx.borrow(), SyncStatus::Offline | SyncStatus::Failed(_));
            consecutive_failures = if failed { consecutive_failures.saturating_add(1) } else { 0 };
            cycle_count = cycle_count.wrapping_add(1);
            tokio::select! {
                _ = tokio::time::sleep(backoff_delay(interval, consecutive_failures)) => {}
                _ = notify_loop.notified() => {}
            }
        }
```

(`watch::Sender::borrow` reads the last value sent — the status `one_cycle` just published.)

- [ ] **Step 6: Run to pass + full gate**

Run: `cargo test -p notion-tui-sync && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS — the pre-existing `sync_loop.rs` tests reach `Idle` on the first cycle (no backoff engaged), and `poisoned_store_mutex_does_not_kill_sync` recovers on cycle one.

- [ ] **Step 7: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-sync && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-sync): exponential offline/failure backoff capped at 5 minutes"`

---

### Task 6: hwm overlap window + unchanged-item skip

Notion's search ordering is eventually consistent: an item edited just before the high-water mark can surface *after* newer items, so the current hard cutoff (`edited <= hwm` → stop) can permanently skip it. Fix by scanning 5 minutes past the hwm — and make that overlap cheap by skipping the block/row re-fetch when the stored `last_edited_time` already matches (which also stops `data_version` churn on no-op cycles). Ordering inside the arm changes so the page/data-source row is written LAST — its `last_edited_time` becomes the "fully pulled" commit marker the skip check trusts (a crash between block write and page write re-pulls; the reverse order would skip forever).

**Files:**
- Modify: `crates/notion-sync/src/puller.rs` (`pull_once` post-M7 shape; `minus_secs` helper + inline tests)
- Modify: `crates/notion-sync/tests/pull.rs` (two new tests; one intentional update to `incremental_pull_skips_unchanged`)

**Interfaces:**
- Consumes: M7's `pull_once(client, store, status_tx)` + checkpoint keys; `Store::get_page` (`store.rs:140`), `Store::get_data_source` (`store.rs:218`).
- Produces:
  - `const HWM_OVERLAP_SECS: i64 = 300;`
  - `fn minus_secs(ts: &str, secs: i64) -> String` — pure-std RFC3339 arithmetic for Notion's fixed `YYYY-MM-DDTHH:MM:SS.mmmZ` shape (no chrono/time dependency); returns the input unchanged on any parse surprise (worst case: no overlap, i.e. today's behavior).
  - `updated` now counts items actually (re)written, not items scanned (`SyncStatus::Idle { updated }` becomes truthful; `done` progress still counts every scanned item).

- [ ] **Step 1: Write the failing unit tests for the timestamp helper**

Add to the bottom of `crates/notion-sync/src/puller.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::minus_secs;

    #[test]
    fn subtracts_within_the_same_hour() {
        assert_eq!(minus_secs("2026-07-05T10:00:00.000Z", 300), "2026-07-05T09:55:00.000Z");
    }

    #[test]
    fn rolls_over_midnight_month_and_year() {
        assert_eq!(minus_secs("2026-01-01T00:02:00.000Z", 300), "2025-12-31T23:57:00.000Z");
        assert_eq!(minus_secs("2026-03-01T00:00:30.500Z", 60), "2026-02-28T23:59:30.500Z");
    }

    #[test]
    fn preserves_millis_and_survives_garbage() {
        assert_eq!(minus_secs("2026-07-05T10:00:00.123Z", 1), "2026-07-05T09:59:59.123Z");
        assert_eq!(minus_secs("not-a-timestamp", 300), "not-a-timestamp");
        assert_eq!(minus_secs("", 300), "");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p notion-tui-sync --lib`
Expected: COMPILE FAIL — `minus_secs` doesn't exist. (Puller inline tests compile as part of the lib target.)

- [ ] **Step 3: Implement the helper**

In `puller.rs` (above `pull_once`); the civil-date conversion is Howard Hinnant's standard algorithm:

```rust
/// Search ordering is only eventually consistent: an item edited shortly
/// before the hwm can surface after newer items. Instead of cutting off
/// exactly at the hwm, keep scanning this many seconds past it; the
/// stored-last_edited_time skip keeps the overlap cheap.
const HWM_OVERLAP_SECS: i64 = 300;

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `ts` minus `secs`, for Notion's fixed-shape RFC3339 UTC timestamps
/// ("2026-07-05T10:00:00.000Z"). Pure std. On any parse surprise the input
/// is returned unchanged (worst case: no overlap window, today's behavior).
fn minus_secs(ts: &str, secs: i64) -> String {
    let num = |r: std::ops::Range<usize>| ts.get(r).and_then(|s| s.parse::<i64>().ok());
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(s)) =
        (num(0..4), num(5..7), num(8..10), num(11..13), num(14..16), num(17..19))
    else {
        return ts.to_string();
    };
    let millis = ts.get(20..23).unwrap_or("000");
    let epoch = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - secs;
    let (days, rem) = (epoch.div_euclid(86400), epoch.rem_euclid(86400));
    let (y, mo, d) = civil_from_days(days);
    format!(
        "{y:04}-{mo:02}-{d:02}T{:02}:{:02}:{:02}.{millis}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
```

Run: `cargo test -p notion-tui-sync --lib`
Expected: PASS.

- [ ] **Step 4: Write the failing pull tests**

Append to `crates/notion-sync/tests/pull.rs` (add `use notion_store::PageRec;` to the imports; signatures below use M7's three-arg `pull_once` — declare the watch channel inline as M7's tests do):

```rust
#[tokio::test]
async fn late_arriving_item_within_the_overlap_window_is_still_pulled() {
    // hwm is 10:00; this page's edit (09:58) surfaced late in the eventually-
    // consistent search order. The old hard cutoff dropped it forever.
    let (server, client, store) = pull_fixture_one_page("2026-07-05T09:58:00.000Z").await;
    store.lock().unwrap().meta_set("hwm", "2026-07-05T10:00:00.000Z").unwrap();

    let (status_tx, _rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    let updated = pull_once(&client, &store, &status_tx).await.unwrap();

    assert_eq!(updated, 1, "an item 2 min behind hwm is inside the 5-min overlap window");
    assert_eq!(store.lock().unwrap().get_page("p1").unwrap().unwrap().title, "Newest");
    // hwm must never move backwards.
    assert_eq!(
        store.lock().unwrap().meta_get("hwm").unwrap().as_deref(),
        Some("2026-07-05T10:00:00.000Z")
    );
    let _ = server;
}

#[tokio::test]
async fn unchanged_item_inside_the_overlap_window_is_not_refetched() {
    let server = MockServer::start().await;
    // Search returns the page, but NO children/comments mocks are mounted:
    // any re-fetch attempt would 404 and fail the test.
    Mock::given(method("POST"))
        .and(path("/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(search_body(json!([page_json(
            "p1",
            "Same",
            "2026-07-05T09:58:00.000Z"
        )]))))
        .mount(&server)
        .await;

    let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    store
        .lock()
        .unwrap()
        .upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Same".into(),
            icon: None,
            archived: false,
            last_edited_time: "2026-07-05T09:58:00.000Z".into(),
        })
        .unwrap();
    store.lock().unwrap().meta_set("hwm", "2026-07-05T10:00:00.000Z").unwrap();

    let (status_tx, _rx) = tokio::sync::watch::channel(notion_sync::SyncStatus::Starting);
    let updated = pull_once(&fast_client(server.uri()), &store, &status_tx).await.unwrap();

    assert_eq!(updated, 0, "matching stored last_edited_time must skip the re-fetch");
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "search only");
}
```

**Intentional test update:** `incremental_pull_skips_unchanged` (`pull.rs:110-135`) currently relies on the hard hwm cutoff — its item's `edited` equals the hwm and the page was never stored. Under overlap semantics that item is inside the window, and "skip" now comes from the stored-`last_edited_time` comparison instead. Update the test (same assertions, new arrangement) by seeding the store before pulling, and add this comment above the seeding:

```rust
    // M9: `edited == hwm` is now inside the overlap window; the skip comes
    // from the stored last_edited_time matching, not from the hwm cutoff.
    store
        .lock()
        .unwrap()
        .upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Same".into(),
            icon: None,
            archived: false,
            last_edited_time: "2026-07-05T10:00:00.000Z".into(),
        })
        .unwrap();
```

- [ ] **Step 5: Run to verify failure**

Run: `cargo test -p notion-tui-sync --test pull`
Expected: FAIL — `late_arriving_...` gets `updated == 0` (hard cutoff drops the item); `unchanged_item_...` errors on the unmocked children fetch (or fails the request-count assertion).

- [ ] **Step 6: Implement in `pull_once`**

Three changes to the post-M7 `pull_once` body:

1. Compute the cutoff once, after reading `hwm`, and use it in the early-break:

```rust
    let cutoff = if hwm.is_empty() { String::new() } else { minus_secs(&hwm, HWM_OVERLAP_SECS) };
```

and replace the break condition `if !hwm.is_empty() && edited.as_str() <= hwm.as_str()` with:

```rust
            if !cutoff.is_empty() && edited.as_str() <= cutoff.as_str() {
                finished = true;
                break;
            }
```

(`max_seen` still compares against `hwm`, so the hwm never regresses.)

2. Restructure the `SearchItem::Page(p)` arm — skip check first, page row written last:

```rust
                SearchItem::Page(p) => {
                    let (parent_type, parent_id) = parent_cols(&p.parent);
                    let rec = PageRec {
                        id: p.id.clone(),
                        parent_type,
                        parent_id,
                        title: p.title.clone(),
                        icon: p.icon.clone(),
                        archived: p.archived,
                        last_edited_time: p.last_edited_time.clone(),
                    };
                    let dirty = lock_store(store).is_page_dirty(&p.id).unwrap_or(false);
                    if dirty {
                        // Keep metadata fresh; never clobber locally-dirty blocks.
                        lock_store(store)
                            .upsert_page(&rec)
                            .map_err(|e| SyncError::Store(e.to_string()))?;
                    } else {
                        let unchanged = lock_store(store)
                            .get_page(&p.id)
                            .map_err(|e| SyncError::Store(e.to_string()))?
                            .is_some_and(|stored| stored.last_edited_time == p.last_edited_time);
                        if !unchanged {
                            let flat = client.fetch_block_tree(&p.id).await?;
                            let recs: Vec<BlockRec> = /* identical mapping to today */;
                            lock_store(store)
                                .replace_page_blocks(&p.id, &recs)
                                .map_err(|e| SyncError::Store(e.to_string()))?;
                            let comments = client.list_comments(&p.id).await?;
                            let comment_recs: Vec<notion_store::CommentRec> = /* identical mapping */;
                            lock_store(store)
                                .replace_comments(&p.id, &comment_recs)
                                .map_err(|e| SyncError::Store(e.to_string()))?;
                            // Written LAST: the stored last_edited_time is the
                            // "fully pulled" commit marker the skip check trusts.
                            lock_store(store)
                                .upsert_page(&rec)
                                .map_err(|e| SyncError::Store(e.to_string()))?;
                            updated += 1;
                        }
                    }
                }
```

(The two `/* identical mapping */` vectors are today's `BlockRec`/`CommentRec` construction loops from `puller.rs:61-73` and `puller.rs:78-89`, moved verbatim.)

3. Same shape for `SearchItem::DataSource(d)` — skip check against `get_data_source`, upsert last:

```rust
                SearchItem::DataSource(d) => {
                    let unchanged = lock_store(store)
                        .get_data_source(&d.id)
                        .map_err(|e| SyncError::Store(e.to_string()))?
                        .is_some_and(|stored| stored.last_edited_time == d.last_edited_time);
                    if !unchanged {
                        let ds = client.get_data_source(&d.id).await?;
                        let rows = client.query_data_source_all(&d.id).await?;
                        let recs: Vec<RowRec> = /* identical mapping to today */;
                        lock_store(store)
                            .replace_rows(&d.id, &recs)
                            .map_err(|e| SyncError::Store(e.to_string()))?;
                        lock_store(store)
                            .upsert_data_source(&DataSourceRec {
                                id: ds.meta.id.clone(),
                                database_id: ds.meta.database_id.clone(),
                                title: ds.meta.title.clone(),
                                schema_json: ds.schema.to_string(),
                                last_edited_time: ds.meta.last_edited_time.clone(),
                            })
                            .map_err(|e| SyncError::Store(e.to_string()))?;
                        updated += 1;
                    }
                }
```

Finally, in M7's post-match bookkeeping, delete the blanket `updated += 1;` (it moved into the arms); `done += 1;`, the `SyncStatus::Syncing` send, and the `pull_done` checkpoint write stay as-is. (Side effect: dirty pages now count toward `done` progress instead of being `continue`d past — harmless, and progress reads truer.)

- [ ] **Step 7: Run to pass + full gate**

Run: `cargo test -p notion-tui-sync && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, including `first_crawl_stores_pages_blocks_and_hwm` (no hwm → no cutoff, nothing stored → nothing skipped), `store_write_failure_fails_the_cycle_and_preserves_hwm` (the `get_page` on the dropped table now errors first — still a `SyncError::Store`, hwm still preserved), M7's `interrupted_first_crawl_...` (p1 fully processed before the injected 500, so its commit-marker upsert landed; second pass re-processes only p2, `updated == 1`), and `dirty_pull.rs` (dirty branch behavior unchanged).

- [ ] **Step 8: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-sync && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-sync): hwm overlap window tolerates eventually-consistent search ordering"`

---

### Task 7: CHANGELOG.md + CONTRIBUTING.md

**Files:**
- Create: `/Users/rehatbir/Developer/fables/notion-tui/CHANGELOG.md`
- Create: `/Users/rehatbir/Developer/fables/notion-tui/CONTRIBUTING.md`

**Interfaces:**
- Consumes: nothing.
- Produces: the seed CHANGELOG M11's `release-plz` appends to (Keep a Changelog headings, `## [X.Y.Z] - YYYY-MM-DD` version format — M11 must configure its changelog template to match), and the CONTRIBUTING conventions M11's commit lint enforces.

- [ ] **Step 1: Write CHANGELOG.md**

Historical dates from git: `0.1.0` publish-prep landed 2026-07-08 (`11d2ebe`), `0.1.1` bump 2026-07-12 (`cd086bc`). Create exactly:

```markdown
# Changelog

All notable changes to notion-tui are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Entries from the next release onward are generated automatically from
conventional commit messages by the release pipeline.

## [Unreleased]

### Added
- CLI flags handled before the TUI starts: `--version`, `--help`,
  `--config <path>`, `--db-path <path>`, `--debug`.
- Rotating log file in the state dir (7 daily files); `--debug` / `RUST_LOG`
  raise verbosity; panics log a backtrace after restoring the terminal.
- Command palette: `rename`, `move page`, `group-by`, `sync now`; fuzzy
  (subsequence) matching in palette and search.
- Mouse support: click-to-focus, sidebar/link clicks, header-sort, board
  card drag.
- Table filtering, universal back-history, first-crawl progress counts with
  interruption-safe checkpointing, remote-deletion reconciliation.
- MSRV declared (`rust-version` in Cargo.toml) and enforced in CI.
- Windows: default editor falls back to `notepad` when nothing is configured.

### Changed
- Sync backs off exponentially (capped at 5 min) while offline/failing
  instead of retrying every 30 s.
- UI and sync share one API client so combined traffic respects Notion's
  ~3 requests/second guidance.
- Pull tolerates eventually-consistent search ordering (5-minute overlap
  window past the high-water mark) and skips re-fetching unchanged items.

### Fixed
- Data-safety fixes from the 2026-07-12 audit: HTTP timeouts, no blind retry
  of non-idempotent creates (verify-then-resend), store errors fail the pull
  cycle instead of advancing the high-water mark, view state survives sync
  refreshes, editor failures surfaced, unclosed-fence confirm, schema
  forward-compat guard with pre-migration backup, transactional comment
  replacement, poison-safe store locking.

## [0.1.1] - 2026-07-12

### Fixed
- First-run wizard token persists via the native keychain on macOS/Windows.

## [0.1.0] - 2026-07-08

### Added
- Initial public release: keyboard-first, offline-first Notion TUI —
  sidebar/page/table/board views backed by a local SQLite cache, queued
  offline edits with background sync, `$EDITOR` round-trip Markdown editing,
  comments, conflict queue (keep-mine / take-theirs / merge), first-run
  token wizard, themes, and rebindable keys.
```

(If M8 has landed by execution time, add one `### Added` line under Unreleased: `- Interaction polish: real text-input editing, themed modals, human-readable queue rows, empty states, friendlier config errors.`)

- [ ] **Step 2: Verify**

Run (from `notion-tui/`): `grep -q "keepachangelog" CHANGELOG.md && grep -q "## \[0.1.0\] - 2026-07-08" CHANGELOG.md && echo CHANGELOG-OK`
Expected: `CHANGELOG-OK`

- [ ] **Step 3: Write CONTRIBUTING.md**

Create exactly:

```markdown
# Contributing to notion-tui

## Repo layout

notion-tui lives inside the `fables` monorepo. The Rust workspace root is
`notion-tui/` — run every cargo command from there, not the repo root.

| Crate | Published as | What it is |
|---|---|---|
| `crates/notion-api` | `notion-tui-api` | Notion HTTP client (pacing, retries, timeouts) |
| `crates/notion-store` | `notion-tui-store` | SQLite cache + pending-ops queue (bundled, FTS5, WAL) |
| `crates/notion-sync` | `notion-tui-sync` | Background push/pull engine |
| `crates/notion-tui` | `notion-tui` | The TUI binary |

## Prerequisites

- Rust stable (CI also checks the MSRV declared in `Cargo.toml`'s
  `rust-version`)
- Nothing else: SQLite is bundled, TLS is rustls.

## Quality gates

All three must pass; CI enforces them on Linux, macOS, and Windows:

    cargo test --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    cargo fmt --check

## Testing conventions

- `notion-api` / `notion-sync`: wiremock mock servers — tests never hit the
  real Notion API.
- `notion-tui`: ratatui `TestBackend` flow tests (`tests/*_flow.rs`, e2e
  suites) and insta snapshots.
- `notion-store`: in-memory or temp-file SQLite.
- Every behavior change lands with a test that fails without it.

## Style

- rustfmt with `max_width = 110` (see `rustfmt.toml`).
- Conventional commits, scoped to the crate:
  `feat(notion-tui): ...`, `fix(notion-sync): ...`, `docs: ...`.
  Release automation derives versions and CHANGELOG entries from these, so
  the type prefix matters: `fix` → patch, `feat` → minor, `!` → major.

## Debugging

Run with `--debug` (or `RUST_LOG=trace ...`) and look at the rotating log
file — locations are listed in the README's Troubleshooting section. Attach
the current log file to bug reports.
```

- [ ] **Step 4: Verify + gate**

Run: `grep -q "cargo clippy --workspace --all-targets -- -D warnings" CONTRIBUTING.md && grep -q "max_width = 110" CONTRIBUTING.md && echo CONTRIB-OK`
Expected: `CONTRIB-OK` (docs-only task: no cargo gate needed, but `cargo fmt --check` must still pass trivially).

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/CHANGELOG.md notion-tui/CONTRIBUTING.md && git -C /Users/rehatbir/Developer/fables commit -m "docs(notion-tui): seed CHANGELOG and add CONTRIBUTING"`

---

### Task 8: README storefront — demo GIF, sync explainer, troubleshooting, FAQ

**Files:**
- Modify: `/Users/rehatbir/Developer/fables/notion-tui/README.md`
- Create: `/Users/rehatbir/Developer/fables/notion-tui/docs/demo.tape`
- Create (if renderable): `/Users/rehatbir/Developer/fables/notion-tui/docs/demo.gif`

**Interfaces:**
- Consumes: Task 3's log path (`<data_dir>/notion-tui/logs/notion-tui.YYYY-MM-DD.log`), Task 2's `--config`/`--db-path` semantics, `config.rs` keyring constants (`KEYRING_SERVICE = "notion-tui"`, `KEYRING_USER = "integration-token"`).
- Produces: the README M9.2's "documents the log path for bug reports" requirement lands in; the demo tape M11 can re-render on release.
- Fixes an existing inaccuracy while here: the README says config lives at `~/.config/notion-tui/config.toml` unconditionally, but `dirs::config_dir()` is `~/Library/Application Support` on macOS and `%APPDATA%` on Windows — the file-locations table below is the truth.

- [ ] **Step 1: Write the vhs tape**

Create `docs/demo.tape` exactly:

```tape
# Renders docs/demo.gif. Requires vhs (https://github.com/charmbracelet/vhs)
# and a configured notion-tui (stored token + an already-synced demo
# workspace — run notion-tui once beforehand so the cache is warm).
Output docs/demo.gif
Set FontSize 16
Set Width 1200
Set Height 700
Set TypingSpeed 75ms
Type "notion-tui"
Enter
Sleep 4s
Type "j"
Sleep 500ms
Type "j"
Sleep 500ms
Enter
Sleep 2.5s
Type "?"
Sleep 2.5s
Escape
Type "/"
Sleep 1s
Type "notes"
Sleep 2s
Escape
Type "q"
Sleep 500ms
```

- [ ] **Step 2: Render the GIF (best-effort)**

Run: `command -v vhs && (cd /Users/rehatbir/Developer/fables/notion-tui && vhs docs/demo.tape) || echo "vhs unavailable or no configured workspace — note in report"`
Expected: `docs/demo.gif` exists, OR the skip message. If skipped: still commit the tape, OMIT the `![demo]` image line in Step 3 (a broken image on the crates.io page is worse than none), and flag the manual follow-up in the task report.

- [ ] **Step 3: Rewrite README.md**

Replace the full contents of `README.md` with the following (the Install/Setup/Config/Keys cores are today's text, corrected and re-homed; include the demo image line only if Step 2 produced the GIF):

```markdown
# notion-tui

A keyboard-first terminal client for Notion. Offline-first: everything renders
from a local SQLite cache; edits queue locally and sync in the background.

![notion-tui demo](docs/demo.gif)

## Install

From crates.io:

    cargo install notion-tui

Prebuilt static binaries: grab `notion-tui-<arch>-unknown-linux-musl` from a
release, `chmod +x`, and put it on your `PATH`.

From source:

    cargo install --path crates/notion-tui

## Setup

Run `notion-tui`. On first run it prompts for a Notion internal-integration
token (create one at notion.so/profile/integrations and share the pages you
want with it), validates it, and stores it in your system keyring (or, with a
warning, in the config file with 0600 permissions).

## Usage

    notion-tui [OPTIONS]

    --config <PATH>    read config from PATH
    --db-path <PATH>   use PATH for the local cache instead of the default
    --debug            verbose logging (same as RUST_LOG=debug)
    -h, --help         print help
    -V, --version      print version

Press `?` in the app for the full, live (rebind-aware) key list.

## Config

Config file location per OS is in the table under Troubleshooting.
`--config` overrides where it's read from.

    poll_interval_secs = 30       # background sync interval
    theme = "default"             # default | dark | light
    mouse = true
    editor = "nvim"               # overrides $EDITOR for the `e` flow
    db_path = "/custom/notion.db"

    [keys]                        # rebind any action (see `?` in-app)
    quit = "x"
    search = "f"

## How syncing works

- Everything you see comes from a local SQLite cache — the app never blocks
  on the network.
- Your edits are appended to a local queue and pushed in the background;
  then the puller fetches remote changes. One cycle runs every
  `poll_interval_secs` (default 30 s). While offline or failing, the
  interval backs off exponentially (up to 5 min) and recovers on the next
  successful cycle; `sync now` in the command palette forces a cycle
  immediately.
- All traffic shares one client paced to ~3 requests/second (Notion's
  guidance), with automatic retry for safe requests and
  verify-before-resend for creates, so flaky networks can't duplicate your
  edits.

### Conflicts

If a page or property changed remotely after your queued edit was made,
the edit is parked as **conflicted** instead of overwriting anyone's work.
The queue screen shows each conflict; resolve with **keep mine** (push your
version), **take theirs** (drop yours, refetch), or **merge** (both versions
open in your editor). Nothing is discarded without you choosing.

## Troubleshooting

### Where things live

| | Linux | macOS | Windows |
|---|---|---|---|
| Config | `~/.config/notion-tui/config.toml` | `~/Library/Application Support/notion-tui/config.toml` | `%APPDATA%\notion-tui\config.toml` |
| Cache DB | `~/.local/share/notion-tui/notion.db` | `~/Library/Application Support/notion-tui/notion.db` | `%APPDATA%\notion-tui\notion.db` |
| Logs | `~/.local/share/notion-tui/logs/` | `~/Library/Application Support/notion-tui/logs/` | `%APPDATA%\notion-tui\logs\` |

Log files rotate daily (`notion-tui.YYYY-MM-DD.log`, last 7 kept). **When
filing a bug, attach the current log file** — re-run with `--debug` first if
the problem reproduces. `RUST_LOG` overrides the log filter entirely.

### Keyring issues

The token is stored under service `notion-tui`, user `integration-token`.

- **Linux**: a Secret Service provider (GNOME Keyring, KWallet) must be
  running; on headless boxes there usually isn't one, so the wizard falls
  back to writing the token into the config file (0600) with a warning.
- **Remove a stored token**: macOS
  `security delete-generic-password -s notion-tui -a integration-token`;
  Linux `secret-tool clear service notion-tui username integration-token`;
  Windows: Credential Manager → Windows Credentials → `notion-tui`.

### Reset

- **Re-sync from scratch**: quit, delete the cache DB (table above), restart.
  Unpushed local edits live in that DB — check the queue screen is empty
  first.
- **Start over completely**: delete the DB, the config file, and the keyring
  entry; the first-run wizard reappears.
- **"this database was created by a newer notion-tui"**: you downgraded.
  Upgrade again, or point `--db-path` at a fresh file.

## FAQ

**Why don't I see a page I know exists?** The integration only sees pages
explicitly shared with it (page → ••• → Connections → your integration).

**Does my data go anywhere besides Notion?** No. The app talks only to
`api.notion.com` and stores its cache in the local SQLite file above.

**Why can't I edit relation/people/formula properties?** They're read-only
in v1 — proper pickers are planned. Editing anything else never destroys
fields the app can't faithfully rebuild.

**How fast do remote edits show up?** Within one poll interval (default
30 s) while healthy. The status bar shows sync state; `sync now` forces it.

**Does it work over SSH/tmux?** Yes — anywhere crossterm works. Mouse
support is optional (`mouse = false`).

**Something looks wrong — where do I start?** Run with `--debug`, reproduce,
and read (or attach) the newest file in the logs directory above.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Notable changes land in
[CHANGELOG.md](CHANGELOG.md).

## License

MIT or Apache-2.0, at your option.
```

Note the "Static release binaries" paragraph and `./scripts/build-release.sh` line from today's README are intentionally dropped from Install (the script remains in-tree; M11 retires or re-documents it).

- [ ] **Step 4: Verify local links + referenced files exist**

Run (from `notion-tui/`):

```bash
python3 - <<'EOF'
import re, sys, pathlib
md = pathlib.Path("README.md").read_text()
missing = [t for t in re.findall(r"\]\(((?!https?://|#)[^)]+)\)", md) if not pathlib.Path(t).exists()]
sys.exit(f"missing link targets: {missing}" if missing else 0)
EOF
echo LINKS-OK
```

Expected: `LINKS-OK` (fails if `docs/demo.gif` is referenced but absent, or CONTRIBUTING/CHANGELOG missing — Task 7 must have landed first in this track).

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/README.md notion-tui/docs/demo.tape notion-tui/docs/demo.gif 2>/dev/null; git -C /Users/rehatbir/Developer/fables add notion-tui/README.md notion-tui/docs/demo.tape && git -C /Users/rehatbir/Developer/fables commit -m "docs(notion-tui): README storefront — demo, sync explainer, troubleshooting, FAQ"`

---

### Task 9: Windows fit-and-finish — `notepad` fallback + windows-gated checks

**Files:**
- Modify: `crates/notion-tui/src/editor.rs` (`editor_command` → testable `editor_command_from`)
- Create: `crates/notion-tui/tests/windows.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub fn editor_command(override_: Option<&str>) -> String` (signature unchanged) delegating to `fn editor_command_from(override_: Option<&str>, env_editor: Option<String>, windows: bool) -> String` so the Windows branch is unit-testable on every OS.
  - `tests/windows.rs` — `#![cfg(windows)]`, exercised only by the `windows-latest` CI runner: keyring roundtrip against Credential Manager, `dirs` path resolution under `%APPDATA%`.
  - Terminal behavior on Windows CI needs no new work: the whole TestBackend suite plus Task 1's binary-spawning `tests/cli.rs` already run on `windows-latest` (real process startup, arg handling, stdout).

- [ ] **Step 1: Write the failing unit tests**

Append to `crates/notion-tui/src/editor.rs`'s tests module:

```rust
    #[test]
    fn falls_back_to_notepad_on_windows_and_vi_elsewhere() {
        assert_eq!(editor_command_from(None, None, true), "notepad");
        assert_eq!(editor_command_from(None, None, false), "vi");
    }

    #[test]
    fn config_override_then_env_still_win_on_every_platform() {
        for windows in [true, false] {
            assert_eq!(editor_command_from(Some("nvim"), Some("nano".into()), windows), "nvim");
            assert_eq!(editor_command_from(None, Some("nano".into()), windows), "nano");
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p notion-tui editor`
Expected: COMPILE FAIL — `editor_command_from` doesn't exist.

- [ ] **Step 3: Implement**

Replace `editor_command` in `editor.rs` (currently lines 20-26):

```rust
/// Resolves the editor command: config override, then `$EDITOR`, then the
/// platform default (`notepad` on Windows — always present, and `vi` isn't —
/// `vi` elsewhere).
pub fn editor_command(override_: Option<&str>) -> String {
    editor_command_from(override_, std::env::var("EDITOR").ok(), cfg!(windows))
}

fn editor_command_from(override_: Option<&str>, env_editor: Option<String>, windows: bool) -> String {
    override_
        .map(str::to_string)
        .or(env_editor)
        .unwrap_or_else(|| if windows { "notepad".to_string() } else { "vi".to_string() })
}
```

Run: `cargo test -p notion-tui editor`
Expected: PASS.

- [ ] **Step 4: Add the windows-gated integration checks**

Create `crates/notion-tui/tests/windows.rs`:

```rust
//! Windows-only fit-and-finish checks. The whole file compiles to nothing on
//! other platforms; the windows-latest CI runner executes it for real.
#![cfg(windows)]

#[test]
fn keyring_roundtrip_works_against_credential_manager() {
    // Deliberately NOT the production service name — never touch a real token.
    let entry = keyring::Entry::new("notion-tui-ci-test", "roundtrip").unwrap();
    entry.set_password("s3cret").unwrap();
    assert_eq!(entry.get_password().unwrap(), "s3cret");
    entry.delete_credential().unwrap();
    assert!(entry.get_password().is_err(), "deleted credential must be gone");
}

#[test]
fn state_and_config_dirs_resolve_under_appdata() {
    let data = dirs::data_dir().unwrap();
    let config = dirs::config_dir().unwrap();
    assert!(data.to_string_lossy().contains("AppData"), "{data:?}");
    assert!(config.to_string_lossy().contains("AppData"), "{config:?}");
}
```

(`keyring` and `dirs` are already regular dependencies of `notion-tui`, so integration tests can use them directly. `delete_credential` is the keyring-v3 name; if the pinned minor predates the rename, the compiler on windows CI will say so — use `delete_password` then.)

- [ ] **Step 5: Run to pass + full gate**

Run: `cargo test -p notion-tui && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS locally (windows.rs compiles to an empty test binary on macOS/Linux). The real windows assertions run when CI's `windows-latest` leg goes green — check that on the PR.

- [ ] **Step 6: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): notepad editor fallback on Windows + windows-gated keyring/path checks"`

---

### Task 10: tracing instrumentation across all four crates

Task 3 built the sink; this task makes the other three crates (and the sync loop) emit into it. Level policy: `error` = a cycle/user action failed; `warn` = a request/op failed (may recover); `info` = lifecycle (startup, cycle results with changes, migration, backoff); `debug` = per-request/per-item detail (`--debug` territory); `trace` = unused for now.

**Files:**
- Modify: `crates/notion-api/Cargo.toml` (+`tracing`; dev: +`tracing-subscriber`), `crates/notion-store/Cargo.toml` (+`tracing`), `crates/notion-sync/Cargo.toml` (+`tracing`)
- Modify: `crates/notion-api/src/client.rs`, `crates/notion-sync/src/lib.rs`, `crates/notion-sync/src/puller.rs`, `crates/notion-sync/src/pusher.rs`, `crates/notion-store/src/schema.rs`, `crates/notion-store/src/store.rs`
- Create: `crates/notion-api/tests/tracing_events.rs`

**Interfaces:**
- Consumes: Task 3's subscriber (at runtime); Tasks 4-6's final sync-loop shape.
- Produces: log events only — zero behavior/signature changes. Any test that breaks here indicates a real behavior change and must be treated as a bug in this task.

- [ ] **Step 1: Add the deps**

`crates/notion-api/Cargo.toml`: add `tracing = { workspace = true }` to `[dependencies]` and `tracing-subscriber = { workspace = true }` to `[dev-dependencies]`. `crates/notion-store/Cargo.toml` and `crates/notion-sync/Cargo.toml`: add `tracing = { workspace = true }` to `[dependencies]`.

Run: `cargo build --workspace`
Expected: compiles.

- [ ] **Step 2: Write the failing event test**

Create `crates/notion-api/tests/tracing_events.rs`:

```rust
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use notion_api::NotionClient;
use tracing::instrument::WithSubscriber;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
    type Writer = Buf;
    fn make_writer(&'a self) -> Buf {
        self.clone()
    }
}

#[tokio::test]
async fn failed_requests_emit_a_warn_event_naming_the_path_and_status() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "code": "object_not_found", "message": "nope"
        })))
        .mount(&server)
        .await;

    let buf = Buf::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(buf.clone())
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .finish();

    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    let _err = async { c.get_json("/v1/users/me").await }
        .with_subscriber(subscriber)
        .await
        .unwrap_err();

    let logs = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
    assert!(logs.contains("WARN"), "expected a warn event, logs: {logs}");
    assert!(logs.contains("/v1/users/me"), "event must name the path: {logs}");
    assert!(logs.contains("404"), "event must include the status: {logs}");
}
```

Run: `cargo test -p notion-tui-api --test tracing_events`
Expected: FAIL — no events are emitted yet (empty log buffer).

- [ ] **Step 3: Instrument `client.rs`**

In `request()` (`client.rs:109-171`), add events at each decision point (no control-flow changes):

- 429 branch, before the sleep: `tracing::debug!(path, wait_ms = wait.as_millis() as u64, attempt, "rate limited (429), backing off");`
- 5xx safe-retry branch, before the sleep: `tracing::debug!(path, status = status.as_u16(), attempt, "server error, retrying");`
- Unsafe 5xx early-return, before building `ApiError::Api`: `tracing::warn!(path, status = status.as_u16(), "server error on non-idempotent request (not retried)");`
- Non-success early-return (`!status.is_success()`), before building the error: `tracing::warn!(path, status = status.as_u16(), "notion api error");`
- Send-failure branch (`Err(e)` from `req.send()`), before returning/continuing: `tracing::warn!(path, error = %e, attempt, "request failed to send");`
- After the loop, before `Err(ApiError::RetriesExhausted(...))`: `tracing::warn!(path, "retries exhausted");`

- [ ] **Step 4: Instrument sync + store**

`crates/notion-sync/src/lib.rs` (`one_cycle` + `spawn_sync`):
- push `Ok(n)` arm — change the binding from `Ok(_)` to `Ok(n)` and log when work happened: `if n > 0 { tracing::info!(pushed = n, "push complete"); }`
- push `Err(ApiError::Network(_))` arm: `tracing::info!("offline (push)");`
- push catch-all `Err(e)` arm: `tracing::error!(error = %e, "push failed");`
- pull `Ok(updated)` arm: `if updated > 0 { tracing::info!(updated, "pull complete"); }`
- pull `Err(SyncError::Api(ApiError::Network(_)))` arm: `tracing::info!("offline (pull)");`
- pull catch-all `Err(e)` arm: `tracing::error!(error = %e, "pull failed");`
- reconcile branch (M7 Task 13): when `removed_pages + removed_ds > 0`: `tracing::info!(removed_pages, removed_ds, "reconciled remote deletions");`
- `spawn_sync` loop, after computing `consecutive_failures` (Task 5): `if consecutive_failures > 0 { tracing::info!(consecutive_failures, delay_secs = backoff_delay(interval, consecutive_failures).as_secs(), "sync backing off"); }`
- the `catch_unwind` handler: `tracing::error!("sync cycle panicked");` before the `send_replace`.

`crates/notion-sync/src/puller.rs` — in the `!unchanged` branches from Task 6: `tracing::debug!(page = %p.id, "pulling page");` (page arm) and `tracing::debug!(data_source = %d.id, "pulling data source");` (data-source arm).

`crates/notion-sync/src/pusher.rs` — at the existing terminal `set_op_state` sites in `push_once` (lines ~505, ~520, ~524, ~535 pre-task): conflicted → `tracing::warn!(seq = op.seq, op_type = %op.op_type, "op conflicted");`, failed → `tracing::warn!(seq = op.seq, op_type = %op.op_type, error = %msg_or_e, "op failed");` (use the message variable already in scope at each site).

`crates/notion-store/src/schema.rs` — in `migrate()`, inside the `version < 1` branch before `execute_batch`: `tracing::info!(from = version, to = LATEST_VERSION, "migrating schema");`
`crates/notion-store/src/store.rs` — in `Store::open`, where the pre-migration backup copy happens (M6's backup block): `tracing::info!(backup = %backup.display(), "backing up database before migration");`

- [ ] **Step 5: Run to pass + full gate**

Run: `cargo test -p notion-tui-api --test tracing_events && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS — events without an installed subscriber are no-ops, so no other test changes.

- [ ] **Step 6: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates notion-tui/Cargo.lock && git -C /Users/rehatbir/Developer/fables commit -m "feat: tracing instrumentation across api/store/sync/tui"`

---

### Task 11: Publishing hygiene — metadata, LICENSE copies, crate READMEs, MSRV in CI

Already done by the earlier publish-prep commit (`11d2ebe`): package names, `description`s, workspace `version`/`edition`/`license`/`repository`, `notion-tui`'s `readme`/`keywords`/`categories`, path-deps with `version = "0.1.1"`, and `LICENSE-MIT`/`LICENSE-APACHE` at the workspace root. **The delta:** `rust-version`, subdirectory-aware `repository`, per-crate `documentation`/`keywords`/`categories`/`readme`, LICENSE copies + READMEs inside each crate dir (crates.io packages only files under the crate dir), and an MSRV CI job.

**Files:**
- Modify: `/Users/rehatbir/Developer/fables/notion-tui/Cargo.toml` (`[workspace.package]`)
- Modify: all four `crates/*/Cargo.toml`
- Create: `crates/notion-api/README.md`, `crates/notion-store/README.md`, `crates/notion-sync/README.md`
- Create (copies): `crates/{notion-api,notion-store,notion-sync,notion-tui}/LICENSE-MIT` and `LICENSE-APACHE`
- Modify: `/Users/rehatbir/Developer/fables/.github/workflows/notion-tui-ci.yml`

**Interfaces:**
- Consumes: nothing from other tasks (Wave 2 ordering only avoids Cargo.toml merge noise).
- Produces: `rust-version = "1.85"` (or the empirically-verified value from Step 5) that M11's release pipeline and the `msrv` CI job both reference.

- [ ] **Step 1: Workspace metadata**

In `/Users/rehatbir/Developer/fables/notion-tui/Cargo.toml`, edit `[workspace.package]` to exactly:

```toml
[workspace.package]
version = "0.1.1"
edition = "2021"
license = "MIT OR Apache-2.0"
repository = "https://github.com/MysteryCoder456/fables/tree/main/notion-tui"
rust-version = "1.85"
```

(Subdirectory-aware `repository`: the workspace is not the git root. M11 note: `release-plz` reads this URL for changelog links — verify it tolerates `/tree/` URLs when configuring it.)

- [ ] **Step 2: Per-crate metadata**

Append to each `[package]` section:

`crates/notion-api/Cargo.toml`:
```toml
rust-version.workspace = true
documentation = "https://docs.rs/notion-tui-api"
readme = "README.md"
keywords = ["notion", "api", "client"]
categories = ["api-bindings"]
```

`crates/notion-store/Cargo.toml`:
```toml
rust-version.workspace = true
documentation = "https://docs.rs/notion-tui-store"
readme = "README.md"
keywords = ["notion", "sqlite", "cache", "offline-first"]
categories = ["database", "caching"]
```

`crates/notion-sync/Cargo.toml`:
```toml
rust-version.workspace = true
documentation = "https://docs.rs/notion-tui-sync"
readme = "README.md"
keywords = ["notion", "sync", "offline-first"]
categories = ["asynchronous"]
```

`crates/notion-tui/Cargo.toml` (keeps its existing `readme = "../../README.md"`, `keywords`, `categories`):
```toml
rust-version.workspace = true
documentation = "https://docs.rs/notion-tui"
```

- [ ] **Step 3: LICENSE copies + lib-crate READMEs**

Run:

```bash
cd /Users/rehatbir/Developer/fables/notion-tui
for c in notion-api notion-store notion-sync notion-tui; do
  cp LICENSE-MIT LICENSE-APACHE "crates/$c/"
done
```

Create `crates/notion-api/README.md`:

```markdown
# notion-tui-api

Notion HTTP client used internally by [notion-tui](https://crates.io/crates/notion-tui):
request pacing (~3 rps), timeouts, retry policy split by idempotency
(non-idempotent creates are never blind-retried), and typed endpoints for the
subset of the Notion API notion-tui needs.

This crate exists to be a dependency of `notion-tui` and versions in lockstep
with it; it is not a general-purpose Notion SDK and makes no independent
semver promises.

License: MIT OR Apache-2.0.
```

Create `crates/notion-store/README.md`:

```markdown
# notion-tui-store

Local SQLite cache and pending-operations queue used internally by
[notion-tui](https://crates.io/crates/notion-tui): pages/blocks/rows/comments
storage (bundled SQLite, FTS5 search, WAL), the offline edit queue, conflict
states, and schema migrations with pre-migration backups.

This crate exists to be a dependency of `notion-tui` and versions in lockstep
with it; it makes no independent semver promises.

License: MIT OR Apache-2.0.
```

Create `crates/notion-sync/README.md`:

```markdown
# notion-tui-sync

Background sync engine used internally by
[notion-tui](https://crates.io/crates/notion-tui): push-then-pull cycles over
the local queue, verify-before-resend for ambiguous create failures,
high-water-mark incremental pulls with an eventual-consistency overlap
window, capped exponential backoff, and remote-deletion reconciliation.

This crate exists to be a dependency of `notion-tui` and versions in lockstep
with it; it makes no independent semver promises.

License: MIT OR Apache-2.0.
```

- [ ] **Step 4: Verify packaging picks everything up**

Run (from `notion-tui/`):

```bash
for c in notion-tui-api notion-tui-store notion-tui-sync notion-tui; do
  echo "== $c =="
  cargo package -p "$c" --list --allow-dirty | grep -E "^(LICENSE-MIT|LICENSE-APACHE|README.md|Cargo.toml)$"
done
```

Expected: all four crates list `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`, `Cargo.toml`. (For `notion-tui`, `README.md` appears because cargo copies the out-of-package `readme` path into the package root.)

- [ ] **Step 5: Empirically verify the MSRV claim**

The lockfile is current (mid-2026); edition-2024 transitive deps require ≥ 1.85, hence the starting claim. Verify:

```bash
rustup toolchain install 1.85.0 --profile minimal --no-self-update
cargo +1.85.0 check --workspace --all-targets --locked
```

Expected: clean check. If it fails with "package X requires rustc ≥ 1.Y": set `rust-version` to the smallest of `1.86`, `1.88`, `1.90` that passes (binary-search with the same command), update Step 1's value AND Step 6's CI toolchain to match, and record the final value in the task report.

- [ ] **Step 6: MSRV CI job**

In `/Users/rehatbir/Developer/fables/.github/workflows/notion-tui-ci.yml`, append under `jobs:` (the workflow-level `defaults.run.working-directory: notion-tui` already applies to all jobs):

```yaml
  msrv:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@master
        with:
          toolchain: "1.85"
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: notion-tui
      - name: Check on MSRV
        run: cargo check --workspace --all-targets --locked
```

(Quote the toolchain — bare `1.85` is YAML-float. Keep the version in sync with `rust-version` if Step 5 raised it.)

Run: `python3 -c "import yaml; yaml.safe_load(open('/Users/rehatbir/Developer/fables/.github/workflows/notion-tui-ci.yml')); print('YAML-OK')"`
Expected: `YAML-OK`

- [ ] **Step 7: Full gate**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS (metadata-only changes).

- [ ] **Step 8: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui .github/workflows/notion-tui-ci.yml && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): crates.io publishing metadata, per-crate licenses/readmes, MSRV in CI"`

---

### Task 12: Integration sweep + spec §7 verification bar

**Files:** none new.

- [ ] **Step 1: Full gates**

Run (from `notion-tui/`): `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green.

- [ ] **Step 2: Cross-task interaction checks**

- `cargo test -p notion-tui --test cli` — CLI + logging init order (Tasks 1-3) compose in the real binary.
- `cargo test -p notion-tui-sync` — shared-client signature + backoff + overlap (Tasks 4-6) compose.
- `cargo test -p notion-tui-api --test pacing --test tracing_events` — pacing pin and instrumentation coexist.
- `git -C /Users/rehatbir/Developer/fables status` — confirm only intended files changed; report the full list.

- [ ] **Step 3: Verification bar — packaged binary + log-file drill**

1. `cargo build --release -p notion-tui` then `./target/release/notion-tui --version` — must print `notion-tui 0.1.1` (or current version) and exit 0. (macOS/Linux locally; Windows is covered by CI's `windows-latest` running `tests/cli.rs` against the real binary.)
2. Log-file drill (needs a configured token; skip and note otherwise): run `./target/release/notion-tui --debug --db-path "$(mktemp -d)/drill.db"`, quit with `q` after the first sync tick, then verify the newest `<data_dir>/notion-tui/logs/notion-tui.*.log` contains (a) the `notion-tui starting` line with the version field, (b) at least one sync-cycle event. A maintainer reading only that file should know: version, platform paths, what sync did, and any errors — that is the "bug report from the log alone" bar.
3. Report results; the session owner reviews the branch/PR.

---

## Spec coverage map

| M9 item | Task(s) |
|---|---|
| M9.1 CLI surface (`--version`, `--help`, `--config`, `--db-path`, `--debug` before the TUI) | 1, 2 |
| M9.2 Observability (tracing all four crates, rotating log in state dir, `--debug`/`RUST_LOG`, panic-hook backtrace after terminal restore, README log path) | 3 (sink + panic hook), 10 (events across crates), 8 (README log path) |
| M9.3 Sync engine economics (backoff with cap; one shared client ~3 rps; hwm overlap window) | 5 (backoff), 4 (shared client), 6 (overlap) |
| M9.4 Publishing hygiene (license copies, readme, keywords, categories, documentation, subdir repository, `rust-version` + CI) | 11 |
| M9.5 Docs storefront (demo GIF, troubleshooting, sync & conflict explainer, FAQ, CHANGELOG seed, CONTRIBUTING) | 7 (CHANGELOG/CONTRIBUTING), 8 (README + demo) |
| M9.6 Windows fit-and-finish (`notepad` fallback; paths/keyring/terminal on Windows CI) | 9 (+ Task 1's binary test on windows CI) |
| Spec §7 verification bar (`--version` from packaged binary; bug report from log alone) | 12 |
