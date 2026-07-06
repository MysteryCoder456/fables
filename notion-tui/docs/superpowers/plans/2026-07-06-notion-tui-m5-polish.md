# notion-tui Milestone 5 — Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship-quality v1 (design spec §8 milestone 5): themes, rebindable keys, first-run wizard with keyring token storage, syntax-highlighted code blocks, help overlay + minimal command palette, mouse/editor config, and packaging as static binaries for aarch64 and x86_64.

**Architecture:** Config grows into the full spec §5 surface (theme, keybinding overrides, editor override, mouse toggle) and token resolution gains a keyring layer ahead of env/file. A `Keymap` module translates logical action names to keys so every binding is overridable from `config.toml`. A `Theme` struct is threaded from `App` into the render functions. The first-run wizard runs on plain stdin/stdout *before* the terminal enters raw mode, validates the token with `GET /v1/users/me`, and persists it. Packaging uses musl targets for fully static binaries.

**Tech Stack:** New dependencies: `keyring` (token storage), `syntect` with `default-fancy` features (pure-Rust syntax highlighting — no C regex dependency, keeps musl cross-builds trivial).

**Depends on:** Milestones 3 and 4 merged (the keymap covers `e`, `c`, `Q`, `v` bindings; the palette dispatches to queue/board toggles).

## Global Constraints

- Notion API version `2025-09-03`.
- Token fallback file must be written with `0600` permissions and produce a startup warning when used instead of the keyring (spec §5).
- Packaging target: single static binary, `aarch64-unknown-linux-musl` and `x86_64-unknown-linux-musl` (spec §8).
- Keybindings must be rebindable via config file (spec §4.3); every logical action in the app routes through the `Keymap`.

---

## Design notes (read before starting)

**Keymap scope.** Every key the app binds gets a logical action name. Config overrides look like `[keys] quit = "x"`. Key strings: single characters (`"q"`), or the named keys `"esc"`, `"enter"`, `"space"`, `"tab"`, `"backspace"`. Modifier-carrying bindings (`Ctrl+d`, `Ctrl+u`, `Ctrl+p`) and modal-internal editing keys (typing into inputs, `y`/`n` in confirms) stay fixed — rebinding those would break text entry. The action list: `quit, sidebar, search, undo, edit, comments, queue, board, help, palette, insert, append, new_row, props, delete, sort, up, down, left, right, top, bottom, open, back, toggle, move_card_next, move_card_prev`.

**Theme scope.** A `Theme` carries the handful of styles the UI actually varies: `highlight` (cursor/selection), `status` (status bar), `border` (pane borders), `accent` (titles). Built-ins: `default` (terminal colors, current look), `dark`, `light`. Render functions take `&Theme` as a parameter — no globals.

**Wizard flow.** `config::load()` distinguishes "no token anywhere" from other errors. On no-token, `main` runs the wizard on plain stdio: prompt → validate via `users/me` → offer keyring save, falling back to the config file (0600 + warning). Only then does the TUI start; the initial crawl progress is already visible via the existing status bar (`⟳ syncing…`).

**Static binaries.** `rusqlite` is `bundled` (compiles SQLite from source — works on musl) and `reqwest` uses `rustls` (no OpenSSL) — the workspace was set up for this from M1. Packaging is therefore just: release-profile tuning, the two musl targets, and a build script.

---

### Task 1: config v2 — theme, mouse, editor, keybinding overrides

**Files:**
- Modify: `crates/notion-tui/src/config.rs`, `crates/notion-tui/src/main.rs`, `crates/notion-tui/src/terminal.rs`, `crates/notion-tui/src/editor.rs`

**Interfaces:**
- Consumes: existing `from_sources`.
- Produces: `Config` gains `pub theme: String` (default `"default"`), `pub mouse: bool` (default `true`), `pub editor: Option<String>`, `pub keys: std::collections::HashMap<String, String>` (from the `[keys]` TOML table). `TerminalGuard::enter(mouse: bool)`. `editor::editor_command(override_: Option<&str>) -> String` (override → `$EDITOR` → `vi`).

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `crates/notion-tui/src/config.rs`:
```rust
    #[test]
    fn parses_theme_mouse_editor_and_key_overrides() {
        let cfg = from_sources(
            Some("tok".into()),
            Some(concat!(
                "theme = \"light\"\n",
                "mouse = false\n",
                "editor = \"nano\"\n",
                "[keys]\n",
                "quit = \"x\"\n",
                "search = \"f\"\n",
            )),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.theme, "light");
        assert!(!cfg.mouse);
        assert_eq!(cfg.editor.as_deref(), Some("nano"));
        assert_eq!(cfg.keys.get("quit").map(String::as_str), Some("x"));
        assert_eq!(cfg.keys.len(), 2);
    }

    #[test]
    fn config_defaults_when_fields_absent() {
        let cfg = from_sources(Some("tok".into()), None, PathBuf::from("/tmp/x.db")).unwrap();
        assert_eq!(cfg.theme, "default");
        assert!(cfg.mouse);
        assert!(cfg.editor.is_none());
        assert!(cfg.keys.is_empty());
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui config:: --lib`
Expected: FAIL — `Config` has no `theme` field.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/config.rs` — extend the struct and `from_sources`:
```rust
use std::collections::HashMap;

#[derive(Debug)]
pub struct Config {
    pub token: String,
    pub poll_interval_secs: u64,
    pub db_path: PathBuf,
    pub theme: String,
    pub mouse: bool,
    pub editor: Option<String>,
    pub keys: HashMap<String, String>,
}
```
and in `from_sources`'s `Ok(Config { ... })`:
```rust
        theme: file.get("theme").and_then(|v| v.as_str()).unwrap_or("default").to_string(),
        mouse: file.get("mouse").and_then(|v| v.as_bool()).unwrap_or(true),
        editor: file.get("editor").and_then(|v| v.as_str()).map(str::to_string),
        keys: file
            .get("keys")
            .and_then(|v| v.as_table())
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
```

`crates/notion-tui/src/terminal.rs` — mouse toggle:
```rust
impl TerminalGuard {
    pub fn enter(mouse: bool) -> anyhow::Result<TerminalGuard> {
        enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
        if mouse {
            crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
        }
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        Ok(TerminalGuard)
    }
}
```
(`restore()` already unconditionally disables mouse capture — disabling never-enabled capture is harmless.)

`crates/notion-tui/src/editor.rs` — editor override:
```rust
/// Resolves the editor command: config override, then `$EDITOR`, then `vi`.
pub fn editor_command(override_: Option<&str>) -> String {
    override_
        .map(str::to_string)
        .or_else(|| std::env::var("EDITOR").ok())
        .unwrap_or_else(|| "vi".to_string())
}
```

`crates/notion-tui/src/main.rs` — pass the new knobs through: `TerminalGuard::enter(cfg.mouse)?`, and store `cfg.editor` on the app (add `pub editor_override: Option<String>` to `App`, set after construction: `app.editor_override = cfg.editor.clone();`). Update every `editor_command()` call site (M3's `e` binding, M4's merge arm) to `editor_command(app.editor_override.as_deref())` — in `main.rs`'s merge arm the closure captures the value before the call:
```rust
                Some(m @ app::AppMsg::MergeReady { .. }) => {
                    let editor = notion_tui::editor::editor_command(app.editor_override.as_deref());
                    app.open_merge_editor(m, move |initial| {
                        notion_tui::editor::edit_text(&editor, initial)
                    });
                }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui`
Expected: PASS, including the M3 editor-flow tests (which inject their editor and don't touch `editor_command`).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/config.rs crates/notion-tui/src/main.rs crates/notion-tui/src/terminal.rs crates/notion-tui/src/editor.rs crates/notion-tui/src/app.rs
git commit -m "feat(notion-tui): config for theme, mouse, editor override, and key overrides"
```

---

### Task 2: `Keymap` — rebindable keys

**Files:**
- Create: `crates/notion-tui/src/keymap.rs`
- Modify: `crates/notion-tui/src/lib.rs`, `crates/notion-tui/src/app.rs`, `crates/notion-tui/src/main.rs`
- Test: `crates/notion-tui/tests/keymap_flow.rs` (new)

**Interfaces:**
- Consumes: `Config.keys` (Task 1).
- Produces:
```rust
#[derive(Clone)]
pub struct Keymap { /* HashMap<String, KeyCode> */ }
impl Keymap {
    pub fn new() -> Keymap;                                          // defaults
    pub fn with_overrides(overrides: &HashMap<String, String>) -> Keymap;
    pub fn is(&self, action: &str, key: KeyEvent) -> bool;           // true iff key matches the binding and carries no Ctrl/Alt
    pub fn key_for(&self, action: &str) -> Option<KeyCode>;          // for the help overlay
    pub fn actions() -> &'static [(&'static str, &'static str)];     // (action, description) for help
}
```
`App` gains `pub keymap: Keymap` (default in `App::new`, overridden from config in `main`). `handle_key`/`dispatch_key` literal key checks are replaced by `keymap.is(...)` lookups.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/keymap_flow.rs`:
```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::Store;
use notion_tui::app::{dispatch_key, App};
use notion_tui::keymap::Keymap;

#[test]
fn default_binding_and_override() {
    let km = Keymap::new();
    assert!(km.is("quit", KeyEvent::from(KeyCode::Char('q'))));

    let mut overrides = HashMap::new();
    overrides.insert("quit".to_string(), "x".to_string());
    let km = Keymap::with_overrides(&overrides);
    assert!(km.is("quit", KeyEvent::from(KeyCode::Char('x'))));
    assert!(!km.is("quit", KeyEvent::from(KeyCode::Char('q'))));
    // Named keys parse:
    overrides.insert("open".to_string(), "enter".to_string());
    let km = Keymap::with_overrides(&overrides);
    assert!(km.is("open", KeyEvent::from(KeyCode::Enter)));
}

#[test]
fn rebound_quit_key_quits_the_app() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut app = App::new(store);
    let mut overrides = HashMap::new();
    overrides.insert("quit".to_string(), "x".to_string());
    app.keymap = Keymap::with_overrides(&overrides);

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('q')));
    assert!(!app.should_quit); // old key no longer bound
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('x')));
    assert!(app.should_quit);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test keymap_flow`
Expected: FAIL — module `keymap` does not exist.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/keymap.rs`:
```rust
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const DEFAULTS: &[(&str, &str, KeyCode)] = &[
    ("quit", "quit the app", KeyCode::Char('q')),
    ("sidebar", "toggle sidebar", KeyCode::Char('1')),
    ("search", "search", KeyCode::Char('/')),
    ("undo", "undo last edit", KeyCode::Char('u')),
    ("edit", "edit page in $EDITOR", KeyCode::Char('e')),
    ("comments", "comments panel", KeyCode::Char('c')),
    ("queue", "queue/conflicts screen", KeyCode::Char('Q')),
    ("board", "toggle board view", KeyCode::Char('v')),
    ("help", "help overlay", KeyCode::Char('?')),
    ("palette", "command palette", KeyCode::Char(':')),
    ("insert", "edit block text", KeyCode::Char('i')),
    ("append", "add block below", KeyCode::Char('a')),
    ("new_row", "new database row", KeyCode::Char('o')),
    ("props", "property form", KeyCode::Char('p')),
    ("delete", "delete (press twice)", KeyCode::Char('d')),
    ("sort", "sort by column", KeyCode::Char('s')),
    ("up", "cursor up", KeyCode::Char('k')),
    ("down", "cursor down", KeyCode::Char('j')),
    ("left", "left / collapse", KeyCode::Char('h')),
    ("right", "right / expand", KeyCode::Char('l')),
    ("top", "jump to top", KeyCode::Char('g')),
    ("bottom", "jump to bottom", KeyCode::Char('G')),
    ("open", "open / follow", KeyCode::Enter),
    ("back", "back", KeyCode::Char('-')),
    ("toggle", "toggle to-do", KeyCode::Char(' ')),
    ("move_card_next", "move card right", KeyCode::Char('J')),
    ("move_card_prev", "move card left", KeyCode::Char('K')),
];

fn parse_key(s: &str) -> Option<KeyCode> {
    match s {
        "esc" => Some(KeyCode::Esc),
        "enter" => Some(KeyCode::Enter),
        "space" => Some(KeyCode::Char(' ')),
        "tab" => Some(KeyCode::Tab),
        "backspace" => Some(KeyCode::Backspace),
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(KeyCode::Char(c)),
                _ => None,
            }
        }
    }
}

#[derive(Clone)]
pub struct Keymap {
    map: HashMap<String, KeyCode>,
}

impl Keymap {
    pub fn new() -> Keymap {
        Keymap {
            map: DEFAULTS.iter().map(|(a, _, k)| (a.to_string(), *k)).collect(),
        }
    }

    pub fn with_overrides(overrides: &HashMap<String, String>) -> Keymap {
        let mut km = Keymap::new();
        for (action, key_str) in overrides {
            if let Some(code) = parse_key(key_str) {
                km.map.insert(action.clone(), code);
            }
        }
        km
    }

    /// Matches on the key code, rejecting Ctrl/Alt chords (those bindings are
    /// fixed). SHIFT is allowed through because uppercase Char events carry it.
    pub fn is(&self, action: &str, key: KeyEvent) -> bool {
        if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return false;
        }
        self.map.get(action) == Some(&key.code)
    }

    pub fn key_for(&self, action: &str) -> Option<KeyCode> {
        self.map.get(action).copied()
    }

    pub fn actions() -> &'static [(&'static str, &'static str)] {
        // SAFETY of representation: recomputed slice of the same static data.
        const LIST: &[(&str, &str)] = &{
            // const-friendly projection is awkward; just duplicate the two columns:
            [
                ("quit", "quit the app"), ("sidebar", "toggle sidebar"), ("search", "search"),
                ("undo", "undo last edit"), ("edit", "edit page in $EDITOR"), ("comments", "comments panel"),
                ("queue", "queue/conflicts screen"), ("board", "toggle board view"), ("help", "help overlay"),
                ("palette", "command palette"), ("insert", "edit block text"), ("append", "add block below"),
                ("new_row", "new database row"), ("props", "property form"), ("delete", "delete (press twice)"),
                ("sort", "sort by column"), ("up", "cursor up"), ("down", "cursor down"),
                ("left", "left / collapse"), ("right", "right / expand"), ("top", "jump to top"),
                ("bottom", "jump to bottom"), ("open", "open / follow"), ("back", "back"),
                ("toggle", "toggle to-do"), ("move_card_next", "move card right"), ("move_card_prev", "move card left"),
            ]
        };
        LIST
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_chords_never_match() {
        let km = Keymap::new();
        assert!(!km.is("undo", KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    }
}
```

`crates/notion-tui/src/lib.rs` — add `pub mod keymap;`.

`crates/notion-tui/src/app.rs` — add `pub keymap: crate::keymap::Keymap` to `App` (init `crate::keymap::Keymap::new()`), then convert the key checks. The mechanical rule: every `key.code == KeyCode::Char('x')` / `match key.code { KeyCode::Char('x') => ... }` arm for an action in the table becomes a `km.is("action", key)` check, where `let km = app.keymap.clone();` is taken at the top of `handle_key` and `dispatch_key`. Representative conversions (apply the same pattern to every action key in both functions):
```rust
pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let km = app.keymap.clone();
    if km.is("quit", key) {
        app.should_quit = true;
        return Action::None;
    }
    if key.code == KeyCode::Tab {
        // Tab stays fixed (pane cycling)
        ...
    }
    if km.is("sidebar", key) { ... }
    if km.is("undo", key) {
        app.pending_d = false;
        return Action::Undo;
    }
    ...
    // View arms become if/else chains since match patterns can't call methods:
    if let View::Page(view) = &mut app.view {
        if km.is("down", key) || key.code == KeyCode::Down { view.move_cursor(1); }
        else if km.is("up", key) || key.code == KeyCode::Up { view.move_cursor(-1); }
        else if km.is("top", key) { view.cursor = 0; }
        else if km.is("bottom", key) { view.cursor = view.lines().len().saturating_sub(1); }
        else if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL { view.move_cursor(10); }
        else if key.code == KeyCode::Char('u') && key.modifiers == KeyModifiers::CONTROL { view.move_cursor(-10); }
        else if km.is("left", key) || km.is("right", key) { view.toggle_at_cursor(); }
        else if km.is("toggle", key) {
            if let Some(block_id) = view.todo_block_at_cursor() {
                return Action::ToggleTodo(block_id);
            }
        }
        else if km.is("open", key) { ... }
        else if km.is("back", key) || key.code == KeyCode::Backspace { ... }
    }
    ...
}
```
Ordering constraints to preserve while converting:
- Ctrl+d/Ctrl+u checks must run **before** the `delete`/`undo` single-key checks (the `is` method already rejects CONTROL, so plain `km.is` checks can't accidentally swallow them — but the Ctrl branches themselves stay literal).
- The `dd` state machine keys off `km.is("delete", key)` in place of `key.code == KeyCode::Char('d')`.
- In `dispatch_key`, `/`-search keeps its Ctrl+P alternate as a literal check alongside `km.is("search", key)`.
- The board's `move_card_next`/`move_card_prev` replace the literal `J`/`K` checks; `down`/`up`/`left`/`right` replace `j`/`k`/`h`/`l`.

`crates/notion-tui/src/main.rs` — after constructing the app:
```rust
    app.keymap = notion_tui::keymap::Keymap::with_overrides(&cfg.keys);
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui`
Expected: PASS — the new keymap tests plus every existing key-driven test (they all use default bindings, which the default `Keymap` preserves exactly).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/keymap.rs crates/notion-tui/src/lib.rs crates/notion-tui/src/app.rs crates/notion-tui/src/main.rs crates/notion-tui/tests/keymap_flow.rs
git commit -m "feat(notion-tui): rebindable keymap driven by config overrides"
```

---

### Task 3: themes

**Files:**
- Create: `crates/notion-tui/src/ui/theme.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`, `crates/notion-tui/src/app.rs`, `crates/notion-tui/src/main.rs`, and each render fn in `crates/notion-tui/src/ui/{sidebar,page,table,board,queue,comments}.rs`

**Interfaces:**
- Consumes: `Config.theme` (Task 1).
- Produces:
```rust
#[derive(Clone)]
pub struct Theme {
    pub highlight: Style,  // cursor/selection rows
    pub status: Style,     // status bar
    pub border: Style,     // pane borders
    pub title: Style,      // block titles
}
pub fn named(name: &str) -> Theme;  // "default" | "dark" | "light"; unknown -> default
```
`App.theme: Theme`; every `render(f, area, view, focused)` becomes `render(f, area, view, focused, theme: &Theme)`; selection styling uses `theme.highlight` instead of the hardcoded `Modifier::REVERSED`.

- [ ] **Step 1: Write the failing test**

Inline tests in `crates/notion-tui/src/ui/theme.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn named_themes_differ_and_unknown_falls_back() {
        let dark = named("dark");
        let light = named("light");
        assert_ne!(dark.highlight, light.highlight);
        assert_eq!(named("nope").status, named("default").status);
    }

    #[test]
    fn dark_theme_uses_explicit_colors() {
        let t = named("dark");
        assert_eq!(t.highlight.bg, Some(Color::Indexed(24)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui ui::theme --lib`
Expected: FAIL — module missing.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/ui/theme.rs`:
```rust
use ratatui::style::{Color, Modifier, Style};

#[derive(Clone, Debug)]
pub struct Theme {
    pub highlight: Style,
    pub status: Style,
    pub border: Style,
    pub title: Style,
}

pub fn named(name: &str) -> Theme {
    match name {
        "dark" => Theme {
            highlight: Style::default().bg(Color::Indexed(24)).fg(Color::White),
            status: Style::default().bg(Color::Indexed(236)).fg(Color::Indexed(250)),
            border: Style::default().fg(Color::Indexed(240)),
            title: Style::default().fg(Color::Indexed(75)).add_modifier(Modifier::BOLD),
        },
        "light" => Theme {
            highlight: Style::default().bg(Color::Indexed(153)).fg(Color::Black),
            status: Style::default().bg(Color::Indexed(252)).fg(Color::Indexed(236)),
            border: Style::default().fg(Color::Indexed(248)),
            title: Style::default().fg(Color::Indexed(25)).add_modifier(Modifier::BOLD),
        },
        _ => Theme {
            // Terminal-native: exactly the pre-theme look.
            highlight: Style::default().add_modifier(Modifier::REVERSED),
            status: Style::default().add_modifier(Modifier::REVERSED),
            border: Style::default(),
            title: Style::default(),
        },
    }
}
```

`crates/notion-tui/src/ui/mod.rs` — `pub mod theme;`; `draw()` reads `let theme = &app.theme;`, passes it to every view render call, and the status bar becomes:
```rust
    f.render_widget(
        Paragraph::new(status_line(&app.sync_status, app.pending, app.conflicted)).style(theme.status),
        rows[1],
    );
```

Each view render fn gains a trailing `theme: &Theme` parameter; inside, the three substitutions are mechanical and identical everywhere:
- `Style::default().add_modifier(Modifier::REVERSED)` (cursor/selection) → `theme.highlight`
- `Block::default().borders(Borders::ALL)` → `Block::default().borders(Borders::ALL).border_style(theme.border)`
- `.title(title)` → `.title(Span::styled(title, theme.title))` (import `ratatui::text::Span`)

Example, `page::render`:
```rust
pub fn render(f: &mut Frame, area: Rect, view: &PageView, focused: bool, theme: &Theme) {
    let items: Vec<ListItem> = view
        .lines()
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut item = ListItem::new(Line::from(format!("{}{}", "  ".repeat(l.indent), l.text)));
            if i == view.cursor && focused {
                item = item.style(theme.highlight);
            }
            item
        })
        .collect();
    let title = format!(" {} ", view.page.title);
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(Span::styled(title, theme.title)),
        ),
        area,
    );
}
```
Apply the same three substitutions in `sidebar.rs`, `table.rs`, `board.rs`, `queue.rs`, `comments.rs`. The modal renders (`search`, `input`, `props`, `confirm`) keep terminal-default styling (they float over any theme legibly).

`crates/notion-tui/src/app.rs` — add `pub theme: crate::ui::theme::Theme` (init `crate::ui::theme::named("default")`).
`crates/notion-tui/src/main.rs` — `app.theme = notion_tui::ui::theme::named(&cfg.theme);`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui`
Expected: PASS — theme unit tests plus all existing render tests (which construct `App` with the `default` theme, whose styles are byte-identical to the pre-theme hardcoded ones, so `TestBackend` assertions keep passing).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/ui crates/notion-tui/src/app.rs crates/notion-tui/src/main.rs
git commit -m "feat(notion-tui): named color themes threaded through all views"
```

---

### Task 4: token storage — keyring with file fallback

**Files:**
- Modify: `Cargo.toml` (workspace), `crates/notion-tui/Cargo.toml`, `crates/notion-tui/src/config.rs`

**Interfaces:**
- Consumes: `keyring` crate.
- Produces: `from_sources` gains a leading `keyring_token: Option<String>` parameter (precedence: keyring → env → file); `pub fn token_from_keyring() -> Option<String>`; `pub enum TokenSink { Keyring, File }`; `pub fn store_token(token: &str) -> anyhow::Result<TokenSink>` — tries the keyring, falls back to writing `~/.config/notion-tui/config.toml` with `0600` perms and returns which sink was used so the caller can print the spec-mandated warning.

- [ ] **Step 1: Add the dependency**

`Cargo.toml` workspace `[workspace.dependencies]`:
```toml
keyring = { version = "3", features = ["sync-secret-service", "vendored"] }
```
`crates/notion-tui/Cargo.toml` `[dependencies]`: `keyring = { workspace = true }`.

- [ ] **Step 2: Write the failing test**

Add to `crates/notion-tui/src/config.rs` tests (the keyring itself is not unit-testable headlessly; test the precedence logic and the file-fallback write):
```rust
    #[test]
    fn keyring_token_beats_env_and_file() {
        let cfg = from_sources(
            Some("ring-tok".into()),
            Some("env-tok".into()),
            Some("token = \"file-tok\""),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.token, "ring-tok");
    }

    #[test]
    fn write_token_file_sets_0600() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_token_file(&path, "tok-123").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("token = \"tok-123\""));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
```
(Existing `from_sources` tests gain a leading `None` argument — update all three.) Add `tempfile` to `notion-tui`'s dev-dependencies if Task M3-7 hasn't already promoted it (it promoted it to a full dependency — fine).

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p notion-tui config:: --lib`
Expected: FAIL — `from_sources` arity, `write_token_file` missing.

- [ ] **Step 4: Write minimal implementation**

`crates/notion-tui/src/config.rs`:
```rust
const KEYRING_SERVICE: &str = "notion-tui";
const KEYRING_USER: &str = "integration-token";

pub fn token_from_keyring() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()?
        .get_password()
        .ok()
}

pub enum TokenSink {
    Keyring,
    File,
}

/// Persists the token: keyring preferred; config-file fallback with 0600
/// perms. Returns which sink was used so the caller can warn on File (spec §5).
pub fn store_token(token: &str) -> anyhow::Result<TokenSink> {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        if entry.set_password(token).is_ok() {
            return Ok(TokenSink::Keyring);
        }
    }
    let path = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("no config dir"))?
        .join("notion-tui/config.toml");
    write_token_file(&path, token)?;
    Ok(TokenSink::File)
}

pub fn write_token_file(path: &std::path::Path, token: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut contents = std::fs::read_to_string(path).unwrap_or_default();
    if !contents.contains("token =") {
        contents.push_str(&format!("token = \"{token}\"\n"));
    } else {
        // Replace the existing token line.
        contents = contents
            .lines()
            .map(|l| if l.trim_start().starts_with("token =") { format!("token = \"{token}\"") } else { l.to_string() })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
    }
    std::fs::write(path, contents)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
```
Change `from_sources` signature to `from_sources(keyring_token: Option<String>, env_token: Option<String>, file_contents: Option<&str>, default_db: PathBuf)` with:
```rust
    let token = keyring_token
        .or(env_token)
        .or_else(|| file.get("token").and_then(|v| v.as_str()).map(str::to_string))
        .ok_or_else(|| ...)?;   // unchanged error message
```
and `load()` passes `token_from_keyring()` as the first argument.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p notion-tui config:: --lib`
Expected: PASS (all config tests, old and new).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/notion-tui/Cargo.toml crates/notion-tui/src/config.rs
git commit -m "feat(notion-tui): keyring token storage with 0600 file fallback"
```

---

### Task 5: first-run wizard

**Files:**
- Create: `crates/notion-tui/src/wizard.rs`
- Modify: `crates/notion-tui/src/lib.rs`, `crates/notion-tui/src/main.rs`, `crates/notion-api/src/endpoints.rs`
- Test: `crates/notion-api/tests/users.rs` (new), inline tests in `wizard.rs`

**Interfaces:**
- Consumes: `config::{store_token, TokenSink}` (Task 4).
- Produces: `NotionClient::me() -> Result<String, ApiError>` (bot/user name from `GET /v1/users/me`); `wizard::run_with(input: impl BufRead, output: impl Write, validate: impl FnMut(&str) -> Option<String>) -> anyhow::Result<String>` (loops prompting until `validate` returns the workspace-bot name, returns the accepted token); `wizard::run() -> anyhow::Result<String>` (production wrapper: stdio + a tokio-blocking `me()` validation + `store_token` + warning on `TokenSink::File`).

- [ ] **Step 1: Write the failing tests**

`crates/notion-api/tests/users.rs`:
```rust
use std::time::Duration;

use notion_api::NotionClient;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn me_returns_bot_name() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/users/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "user", "id": "u1", "name": "My Integration", "type": "bot"})))
        .mount(&server).await;
    let mut c = NotionClient::with_base_url("t", server.uri());
    c.set_timing(Duration::from_millis(1), Duration::from_millis(1));
    assert_eq!(c.me().await.unwrap(), "My Integration");
}
```

Inline in `crates/notion-tui/src/wizard.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_first_valid_token() {
        let input = b"secret_good\n" as &[u8];
        let mut output = Vec::new();
        let token = run_with(input, &mut output, |t| {
            (t == "secret_good").then(|| "My Bot".to_string())
        })
        .unwrap();
        assert_eq!(token, "secret_good");
        let printed = String::from_utf8(output).unwrap();
        assert!(printed.contains("My Bot"));
    }

    #[test]
    fn reprompts_on_invalid_token() {
        let input = b"bad\nsecret_good\n" as &[u8];
        let mut output = Vec::new();
        let token = run_with(input, &mut output, |t| {
            (t == "secret_good").then(|| "My Bot".to_string())
        })
        .unwrap();
        assert_eq!(token, "secret_good");
        assert!(String::from_utf8(output).unwrap().contains("invalid"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-api --test users` and `cargo test -p notion-tui wizard --lib`
Expected: FAIL — `me` and module missing.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-api/src/endpoints.rs` — add:
```rust
    /// Validates the token; returns the integration's bot name.
    pub async fn me(&self) -> Result<String, ApiError> {
        let v = self.get_json("/v1/users/me").await?;
        Ok(v["name"].as_str().unwrap_or_default().to_string())
    }
```

`crates/notion-tui/src/wizard.rs`:
```rust
use std::io::{BufRead, Write};

/// Testable core: prompts for a token until `validate` accepts one (returning
/// the integration name), echoing progress to `output`. Returns the token.
pub fn run_with(
    input: impl BufRead,
    mut output: impl Write,
    mut validate: impl FnMut(&str) -> Option<String>,
) -> anyhow::Result<String> {
    writeln!(output, "notion-tui first-run setup")?;
    writeln!(output, "Create an internal integration at https://www.notion.so/profile/integrations")?;
    writeln!(output, "and share the pages you want with it, then paste the token below.")?;
    for line in input.lines() {
        let token = line?.trim().to_string();
        if token.is_empty() {
            continue;
        }
        write!(output, "validating… ")?;
        match validate(&token) {
            Some(name) => {
                writeln!(output, "ok — connected as \"{name}\"")?;
                return Ok(token);
            }
            None => {
                writeln!(output, "invalid token, try again:")?;
            }
        }
    }
    anyhow::bail!("stdin closed before a valid token was provided")
}

/// Production wrapper: stdio prompt, validation via a live `users/me` call,
/// then persistence (keyring preferred, 0600 file fallback with a warning).
pub async fn run() -> anyhow::Result<String> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let token = tokio::task::block_in_place(|| {
        run_with(stdin.lock(), stdout.lock(), |t| {
            let client = notion_api::NotionClient::new(t.to_string());
            tokio::runtime::Handle::current()
                .block_on(client.me())
                .ok()
                .filter(|n| !n.is_empty())
        })
    })?;
    match crate::config::store_token(&token)? {
        crate::config::TokenSink::Keyring => println!("token saved to system keyring"),
        crate::config::TokenSink::File => {
            println!("warning: no keyring available — token saved to ~/.config/notion-tui/config.toml (0600)");
        }
    }
    Ok(token)
}
```

`crates/notion-tui/src/lib.rs` — add `pub mod wizard;`.

`crates/notion-tui/src/main.rs` — run the wizard when no token is configured (before `TerminalGuard::enter`):
```rust
    let cfg = match config::load() {
        Ok(cfg) => cfg,
        Err(e) if e.to_string().contains("NOTION_TOKEN") => {
            let token = notion_tui::wizard::run().await?;
            let mut cfg = config::load()?; // reload: token now persisted
            cfg.token = token;
            cfg
        }
        Err(e) => return Err(e),
    };
```
Note: `#[tokio::main]` defaults to the multi-thread runtime, which `block_in_place` requires — no change needed.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p notion-api --test users && cargo test -p notion-tui wizard --lib`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/notion-api/src/endpoints.rs crates/notion-api/tests/users.rs crates/notion-tui/src/wizard.rs crates/notion-tui/src/lib.rs crates/notion-tui/src/main.rs
git commit -m "feat(notion-tui): first-run wizard with token validation and persistence"
```

---

### Task 6: syntax-highlighted code blocks

**Files:**
- Modify: `Cargo.toml` (workspace), `crates/notion-tui/Cargo.toml`, `crates/notion-tui/src/ui/page.rs`

**Interfaces:**
- Consumes: `syntect` (`SyntaxSet`, `Theme`, `highlight::HighlightLines`), the code-block payload's `language` field (already stored by the puller).
- Produces: `BlockLine` gains `pub spans: Option<ratatui::text::Line<'static>>` — pre-styled content for code lines; `page::render` uses it when present. Code blocks with multiple lines render one `BlockLine` per source line (replacing the current single `│ {text}` line).

- [ ] **Step 1: Add the dependency**

Workspace `[workspace.dependencies]`:
```toml
syntect = { version = "5", default-features = false, features = ["default-fancy"] }
once_cell = "1"
```
`crates/notion-tui/Cargo.toml`: `syntect = { workspace = true }`, `once_cell = { workspace = true }`.

- [ ] **Step 2: Write the failing test**

Add to the existing tests module in `crates/notion-tui/src/ui/page.rs`:
```rust
    #[test]
    fn code_block_renders_per_line_with_syntax_styling() {
        let v = PageView::new(
            page(),
            vec![rec("c", None, 0, "code", "let x = 1;\nlet y = 2;", r#"{"language": "rust"}"#)],
        );
        let lines = v.lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].text.contains("let x = 1;"));
        let spans = lines[0].spans.as_ref().expect("code lines carry styled spans");
        // At least one span has an explicit foreground color (i.e. real highlighting happened).
        assert!(spans.spans.iter().any(|s| s.style.fg.is_some()));
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p notion-tui ui::page --lib`
Expected: FAIL — no `spans` field.

- [ ] **Step 4: Write minimal implementation**

`crates/notion-tui/src/ui/page.rs`:
```rust
use once_cell::sync::Lazy;
use ratatui::style::Color;
use ratatui::text::Span;
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::SyntaxSet;

static SYNTAXES: Lazy<SyntaxSet> = Lazy::new(SyntaxSet::load_defaults_newlines);
static THEMES: Lazy<ThemeSet> = Lazy::new(ThemeSet::load_defaults);

fn highlight_code_line(line: &str, language: &str) -> Line<'static> {
    let syntax = SYNTAXES
        .find_syntax_by_token(language)
        .unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let theme = &THEMES.themes["base16-ocean.dark"];
    let mut h = HighlightLines::new(syntax, theme);
    let spans: Vec<Span<'static>> = h
        .highlight_line(line, &SYNTAXES)
        .unwrap_or_default()
        .into_iter()
        .map(|(style, text)| {
            Span::styled(
                text.to_string(),
                ratatui::style::Style::default().fg(Color::Rgb(
                    style.foreground.r,
                    style.foreground.g,
                    style.foreground.b,
                )),
            )
        })
        .collect();
    Line::from(spans)
}
```
Extend `BlockLine`:
```rust
pub struct BlockLine {
    pub block_id: String,
    pub text: String,
    pub indent: usize,
    pub link_page_id: Option<String>,
    pub spans: Option<Line<'static>>,
}
```
In `push_children`, replace the `"code"` arm: instead of pushing one line, push one `BlockLine` per source line and `continue` past the shared push:
```rust
            if b.block_type == "code" {
                let language = payload["language"].as_str().unwrap_or("").to_string();
                for src_line in b.plain_text.lines() {
                    out.push(BlockLine {
                        block_id: b.id.clone(),
                        text: format!("│ {src_line}"),
                        indent,
                        link_page_id: None,
                        spans: Some(highlight_code_line(src_line, &language)),
                    });
                }
                continue;
            }
```
Every other arm sets `spans: None` in its pushed `BlockLine`. In `render`, prefer the styled line:
```rust
            let content = match &l.spans {
                Some(styled) => {
                    let mut spans = vec![Span::raw(format!("{}│ ", "  ".repeat(l.indent)))];
                    spans.extend(styled.spans.iter().cloned());
                    Line::from(spans)
                }
                None => Line::from(format!("{}{}", "  ".repeat(l.indent), l.text)),
            };
            let mut item = ListItem::new(content);
```
(Cursor operations — `block_id_at_cursor`, `dd`, etc. — keep working: each code line carries the same `block_id`, so acting on any line of a code block targets the whole block, which is correct since Notion code blocks are atomic.)

Note for M3 interaction: `markdown::render`'s `Unit` builds from `BlockRec`, not `BlockLine` — unaffected.

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test -p notion-tui`
Expected: PASS (including existing page tests — the `renders_prefixes_and_numbering` test doesn't cover code; if any existing assertion indexed past a code block, adjust it for per-line expansion).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/notion-tui/Cargo.toml crates/notion-tui/src/ui/page.rs
git commit -m "feat(notion-tui): syntect-highlighted code blocks in page view"
```

---

### Task 7: help overlay (`?`) and command palette (`:`)

**Files:**
- Create: `crates/notion-tui/src/ui/help.rs`, `crates/notion-tui/src/ui/palette.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs`, `crates/notion-tui/src/app.rs`
- Test: `crates/notion-tui/tests/help_palette_flow.rs` (new)

**Interfaces:**
- Consumes: `Keymap::{actions, key_for}` (Task 2), `App::{toggle_queue}` (M4), the board `v`-toggle logic (M4).
- Produces: `App` gains `pub help_open: bool` and `pub palette: Option<PaletteState>`. Help: `?` toggles a centered overlay listing every action, its description, and its current binding; any key closes it. Palette: `:` opens a filter-as-you-type list of commands `help`, `queue`, `board`, `table`, `quit`; `Enter` runs the selected command; `Esc` closes. `pub struct PaletteState { pub input: String, pub cursor: usize }` with `pub fn matches(&self) -> Vec<&'static str>` and `on_key(&mut self, key) -> PaletteAction`; `pub enum PaletteAction { None, Changed, Run(&'static str), Close }`.

- [ ] **Step 1: Write the failing test**

`crates/notion-tui/tests/help_palette_flow.rs`:
```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::Store;
use notion_tui::app::{dispatch_key, App, Focus, View};

fn app() -> App {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut a = App::new(store);
    a.focus = Focus::Main;
    a
}

fn key(c: char) -> KeyEvent { KeyEvent::from(KeyCode::Char(c)) }

#[test]
fn question_mark_toggles_help_and_any_key_closes() {
    let mut a = app();
    dispatch_key(&mut a, key('?'));
    assert!(a.help_open);
    dispatch_key(&mut a, key('j'));
    assert!(!a.help_open);
}

#[test]
fn palette_filters_and_runs_queue_command() {
    let mut a = app();
    dispatch_key(&mut a, key(':'));
    assert!(a.palette.is_some());
    dispatch_key(&mut a, key('q'));
    dispatch_key(&mut a, key('u'));
    dispatch_key(&mut a, KeyEvent::from(KeyCode::Enter));
    assert!(a.palette.is_none());
    assert!(matches!(a.view, View::Queue(_)));
}

#[test]
fn palette_esc_closes_without_running() {
    let mut a = app();
    dispatch_key(&mut a, key(':'));
    dispatch_key(&mut a, KeyEvent::from(KeyCode::Esc));
    assert!(a.palette.is_none());
    assert!(matches!(a.view, View::Empty));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p notion-tui --test help_palette_flow`
Expected: FAIL — no `help_open` field.

- [ ] **Step 3: Write minimal implementation**

`crates/notion-tui/src/ui/help.rs`:
```rust
use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use ratatui::Frame;

use crate::keymap::Keymap;

fn key_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        other => format!("{other:?}"),
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

pub fn render(f: &mut Frame, keymap: &Keymap) {
    let area = f.area();
    let popup = centered_rect((area.width * 3 / 4).clamp(40, 70), (area.height * 3 / 4).clamp(10, 32), area);
    f.render_widget(Clear, popup);
    let items: Vec<ListItem> = Keymap::actions()
        .iter()
        .map(|(action, desc)| {
            let key = keymap.key_for(action).map(key_label).unwrap_or_default();
            ListItem::new(format!("{key:>10}  {desc}"))
        })
        .collect();
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" help (any key to close) ")),
        popup,
    );
}
```

`crates/notion-tui/src/ui/palette.rs`:
```rust
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

pub const COMMANDS: &[&str] = &["help", "queue", "board", "table", "quit"];

pub struct PaletteState {
    pub input: String,
    pub cursor: usize,
}

pub enum PaletteAction {
    None,
    Changed,
    Run(&'static str),
    Close,
}

impl PaletteState {
    pub fn new() -> PaletteState {
        PaletteState { input: String::new(), cursor: 0 }
    }

    pub fn matches(&self) -> Vec<&'static str> {
        COMMANDS.iter().copied().filter(|c| c.contains(self.input.as_str())).collect()
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PaletteAction {
        match key.code {
            KeyCode::Esc => PaletteAction::Close,
            KeyCode::Enter => match self.matches().get(self.cursor) {
                Some(cmd) => PaletteAction::Run(cmd),
                None => PaletteAction::Close,
            },
            KeyCode::Down => {
                let n = self.matches().len();
                if n > 0 { self.cursor = (self.cursor + 1).min(n - 1); }
                PaletteAction::None
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                PaletteAction::None
            }
            KeyCode::Backspace => {
                self.input.pop();
                self.cursor = 0;
                PaletteAction::Changed
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.cursor = 0;
                PaletteAction::Changed
            }
            _ => PaletteAction::None,
        }
    }
}

impl Default for PaletteState {
    fn default() -> Self { Self::new() }
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

pub fn render(f: &mut Frame, state: &PaletteState) {
    let area = f.area();
    let popup = centered_rect((area.width / 2).clamp(24, 50), 12, area);
    f.render_widget(Clear, popup);
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);
    f.render_widget(
        Paragraph::new(state.input.as_str()).block(Block::default().borders(Borders::ALL).title(" : ")),
        inner[0],
    );
    let items: Vec<ListItem> = state
        .matches()
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut item = ListItem::new(*c);
            if i == state.cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    f.render_widget(List::new(items).block(Block::default().borders(Borders::ALL)), inner[1]);
}
```

`crates/notion-tui/src/ui/mod.rs` — `pub mod help; pub mod palette;`; render at the end of `draw()` (topmost overlays):
```rust
    if let Some(palette_state) = &app.palette {
        palette::render(f, palette_state);
    }
    if app.help_open {
        help::render(f, &app.keymap);
    }
```

`crates/notion-tui/src/app.rs` — fields `pub help_open: bool` (init `false`), `pub palette: Option<crate::ui::palette::PaletteState>` (init `None`). At the very top of `dispatch_key` (before even the confirm modal — help/palette are the topmost layers):
```rust
    if app.help_open {
        app.help_open = false;
        return;
    }
    if app.palette.is_some() {
        let action = app.palette.as_mut().unwrap().on_key(key);
        match action {
            crate::ui::palette::PaletteAction::None | crate::ui::palette::PaletteAction::Changed => {}
            crate::ui::palette::PaletteAction::Close => app.palette = None,
            crate::ui::palette::PaletteAction::Run(cmd) => {
                app.palette = None;
                app.run_command(cmd);
            }
        }
        return;
    }
```
Openers (with the other global keys, using the keymap):
```rust
    if km.is("help", key) {
        app.help_open = true;
        return;
    }
    if km.is("palette", key) {
        app.palette = Some(crate::ui::palette::PaletteState::new());
        return;
    }
```
And the command runner on `impl App`:
```rust
    pub fn run_command(&mut self, cmd: &str) {
        match cmd {
            "help" => self.help_open = true,
            "queue" => {
                if !matches!(self.view, View::Queue(_)) {
                    self.toggle_queue();
                }
            }
            "board" | "table" => {
                // Reuse the v-toggle: only switches when the current view is the other kind.
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
```
(This requires extracting M4's inline `v`-toggle body from `dispatch_key` into `pub fn toggle_board(&mut self)` on `App` and calling it from both places — do that as part of this step.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p notion-tui --test help_palette_flow`
Expected: PASS (3 tests). Run `cargo test -p notion-tui` for regressions.

- [ ] **Step 5: Commit**

```bash
git add crates/notion-tui/src/ui/help.rs crates/notion-tui/src/ui/palette.rs crates/notion-tui/src/ui/mod.rs crates/notion-tui/src/app.rs crates/notion-tui/tests/help_palette_flow.rs
git commit -m "feat(notion-tui): help overlay and command palette"
```

---

### Task 8: packaging — release profile, static musl builds, install docs

**Files:**
- Modify: `Cargo.toml` (workspace)
- Create: `scripts/build-release.sh`, `README.md` (or extend if one exists — check first)

**Interfaces:**
- Consumes: the whole workspace.
- Produces: tuned release binaries; a repeatable script emitting `dist/notion-tui-{aarch64,x86_64}-unknown-linux-musl`.

- [ ] **Step 1: Release profile**

Append to the workspace `Cargo.toml`:
```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = true
```

- [ ] **Step 2: Build script**

`scripts/build-release.sh`:
```bash
#!/usr/bin/env bash
# Builds static release binaries for both v1 targets (spec §8).
# Native target builds directly; the foreign target uses `cross` if installed.
set -euo pipefail
cd "$(dirname "$0")/.."

TARGETS=(aarch64-unknown-linux-musl x86_64-unknown-linux-musl)
HOST_ARCH=$(uname -m)
mkdir -p dist

for target in "${TARGETS[@]}"; do
    if [[ "$target" == "$HOST_ARCH"* ]]; then
        rustup target add "$target"
        cargo build --release --target "$target" -p notion-tui
    elif command -v cross >/dev/null; then
        cross build --release --target "$target" -p notion-tui
    else
        echo "skipping $target (install 'cross' for foreign-arch builds: cargo install cross)"
        continue
    fi
    cp "target/$target/release/notion-tui" "dist/notion-tui-$target"
    echo "built dist/notion-tui-$target"
done
```
Then: `chmod +x scripts/build-release.sh`.

- [ ] **Step 3: Install/README section**

Create `README.md` (check for an existing one first; if present, append the sections instead):
```markdown
# notion-tui

A keyboard-first terminal client for Notion. Offline-first: everything renders
from a local SQLite cache; edits queue locally and sync in the background.

## Install

Prebuilt static binaries: grab `notion-tui-<arch>-unknown-linux-musl` from a
release, `chmod +x`, and put it on your `PATH`.

From source:

    cargo install --path crates/notion-tui

Static release binaries for both supported targets:

    ./scripts/build-release.sh   # emits dist/notion-tui-{aarch64,x86_64}-unknown-linux-musl

## Setup

Run `notion-tui`. On first run it prompts for a Notion internal-integration
token (create one at notion.so/profile/integrations and share pages with it),
validates it, and stores it in your system keyring (or, with a warning, in
`~/.config/notion-tui/config.toml` with 0600 permissions).

## Config (`~/.config/notion-tui/config.toml`)

    poll_interval_secs = 30       # background sync interval
    theme = "default"             # default | dark | light
    mouse = true
    editor = "nvim"               # overrides $EDITOR for the `e` flow
    db_path = "/custom/notion.db"

    [keys]                        # rebind any action (see `?` in-app for the list)
    quit = "x"
    search = "f"

## Keys

Press `?` in the app for the full, live (rebind-aware) list.
```

- [ ] **Step 4: Verify**

Run: `cargo build --release -p notion-tui`
Expected: builds clean; `ls -lh target/release/notion-tui` shows a stripped binary.
Run: `./scripts/build-release.sh`
Expected: at minimum the native-arch musl binary lands in `dist/` (`file dist/notion-tui-*` reports "statically linked"); the foreign arch is skipped with a message unless `cross` is installed.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml scripts/build-release.sh README.md
git commit -m "chore: release profile, static musl build script, install docs"
```

---

### Task 9: final sweep — full suite + workspace-wide manual pass

**Files:** none new.

- [ ] **Step 1: Full test suite**

Run: `cargo test`
Expected: PASS across all four crates.

- [ ] **Step 2: Clippy + fmt gate**

Run: `cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: clean. Fix anything surfaced (commit fixes as `chore: clippy/fmt cleanup`).

- [ ] **Step 3: Manual acceptance pass against the real workspace**

- Delete (or move aside) the local DB and config; run `notion-tui` — wizard prompts, bad token re-prompts, good token connects and saves; initial crawl progress shows in the status bar.
- `theme = "dark"` and a `[keys] quit = "x"` override both take effect after restart; `?` shows `x` as the quit binding.
- A page with a fenced code block renders highlighted.
- `:` palette runs `queue` and `board`.
- `./scripts/build-release.sh` artifacts run on the Pi (`dist/notion-tui-aarch64-unknown-linux-musl`).

- [ ] **Step 4: Commit any fixes, then finish the branch**

Use superpowers:finishing-a-development-branch.

---

## Verification (final)

All spec §5 items (keyring + 0600 fallback + warning, config surface, first-run wizard), §4.3 rebindability, §4.2 syntax highlighting, and §8 packaging targets are covered by Tasks 1–8; §6's panic-safe terminal restore and store migrations already exist from M1. After this milestone the v1 spec surface is complete.
