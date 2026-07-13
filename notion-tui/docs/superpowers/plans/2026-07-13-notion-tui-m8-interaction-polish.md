# notion-tui M8 — Interaction Polish: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove the "first hour of real use" annoyances: real cursor-based text editing everywhere, fully themed modals, a human-readable op queue, empty-state hints, consistent motion keys, honest wizard/config errors, layout niceties, dd-confirm consistency, and complete help.

**Architecture:** One new reusable widget module (`ui/textline.rs`, a grapheme-safe editable line adopted by every text input) and one new helper module (`describe.rs`, turning `OpRec`s into human-readable descriptions — M10 consumes it). Everything else is surgical edits to existing modules: theme threading through the six modal render fns, per-view empty-state branches, two new `handle_key` motion arms, a `Result`-based wizard validator, config/keymap warning plumbing, table width/ellipsis + page soft-wrap rendering, two new `ConfirmKind` variants, and a chord-aware `Keymap`.

**Tech Stack:** Existing: ratatui 0.29/crossterm, tokio, rusqlite, insta + TestBackend, proptest, wiremock. New deps: `unicode-segmentation = "1"` and `unicode-width = "0.2"` (both already in Cargo.lock as transitive ratatui deps; promoted to direct).

## Global Constraints

- The git root is the monorepo `/Users/rehatbir/Developer/fables`; the workspace lives in `notion-tui/`. All `cargo` commands run from `/Users/rehatbir/Developer/fables/notion-tui`.
- Quality gates must stay green after every task: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`.
- **Baseline is the current working tree** (it contains uncommitted M6 work: `ConfirmKind`, `read_only_notice`, `summarize_applied`, `parse_markdown_checked` all already exist — build on them, do not re-create them).
- **M7 is executing concurrently** (`docs/superpowers/plans/2026-07-13-notion-tui-m7-finish-v1-spec.md`). Do NOT touch app.rs's `Action` enum variants, `PickerPurpose`, `SyncStatus`, or the `TableView`/`BoardView` fields M7 adds (`filter`, group-by override). Steps that name an M7 surface (`ui/picker.rs`, the table-filter input, the rename input) apply only if M7 has landed in your tree; otherwise record them as follow-ups in your task report.
- Commits: stage ONLY the files your task touched (`git add <explicit paths>`), never `git add -A` or `git commit -a` — the tree carries unrelated in-flight milestone work. If a file you must stage contains unrelated uncommitted hunks (app.rs is the likely case), stop and ask the session owner instead of committing.
- Never weaken an existing test to make it pass; when a behavior intentionally changes (dd-confirm, code-label line counts, `InputState.value` → `value()`), update the test to assert the new behavior and say so in the task report.
- **Exact user-facing copy (spec-mandated where quoted in the spec):**
  - Empty states: sidebar `first sync in progress…` · search `no results` · comments `no comments yet — press n` · page `empty page — press a to add a block` · table `no rows — press o to add one` · queue `queue is empty — local edits appear here until synced`
  - Wizard: 401 → `invalid token, try again:` · network → `can't reach Notion — check your connection`
  - Fixed keys (stay fixed, spec §5): `Tab`, `Ctrl+d`, `Ctrl+u`, `Ctrl+P` (Backspace-as-back and Esc are documented as fixed in help but are not collision-checked chords).
- **Pinned for M10** (consumed verbatim by the M10 plan):
  ```rust
  // crates/notion-tui/src/describe.rs
  pub struct OpDescription {
      pub summary: String,              // e.g. "edit ¶ in 'Meeting Notes'"
      pub target_title: Option<String>, // page/row title when resolvable
      pub detail: Option<String>,       // property name (row ops) / block-text snippet (block ops) / comment body snippet
  }
  pub fn describe_op(op: &notion_store::OpRec, store: &notion_store::Store) -> OpDescription
  ```

## Execution waves (parallelization map)

- **Wave 1 (six parallel tracks, disjoint files):**
  - Track TEXT: Task 1 → Task 2 → Task 3 (`ui/textline.rs`, then the modal files, then the same modal files again for theming — strictly sequential within track; Task 2 also owns `app.rs::refresh_search` only)
  - Track QUEUE: Task 4 (`describe.rs`, `notion-store/src/store.rs::get_row`, `ui/queue.rs`, `app.rs::toggle_queue`/`refresh_queue` only)
  - Track WIZARD: Task 5 (`wizard.rs`)
  - Track CONFIG: Task 6 (`config.rs`, `keymap.rs::with_overrides_checked`, `main.rs` startup-warning lines only)
  - Track TABLE: Task 7 (`ui/table.rs` render/width code only)
  - Track PAGE: Task 8 (`ui/page.rs`)
- **Wave 2 (after all of Wave 1):**
  - Task 9 (empty states — touches sidebar/page/table/search/comments/queue render fns, so it must follow Tasks 3, 4, 7, 8)
  - Task 10 (motion keys — owns `handle_key`'s Table/Board arms in app.rs)
- **Wave 3 (after Wave 2):**
  - Task 11 (dd-confirm — owns app.rs's `Action::Delete*` dispatch arms + confirm resolution match + `ui/confirm.rs`; sequential after Task 10 because both edit app.rs)
  - Task 12 (help completeness + chord keymap — `keymap.rs` after Task 6, `ui/help.rs` after Task 3, spec doc amendment)
- **Wave 4 (serial):** Task 13 (integration sweep).

---

### Task 1: `TextLine` — grapheme-safe editable line widget

**Files:**
- Create: `crates/notion-tui/src/ui/textline.rs`
- Modify: `crates/notion-tui/src/ui/mod.rs:1-13` (register module)
- Modify: `/Users/rehatbir/Developer/fables/notion-tui/Cargo.toml` (`[workspace.dependencies]`), `crates/notion-tui/Cargo.toml` (add the two unicode crates)
- Test: inline in `textline.rs` (unit + proptest — this is the spec §7 "grapheme-editing property tests" deliverable)

**Interfaces:**
- Consumes: nothing.
- Produces (Task 2 and M7's picker adopt exactly this):
```rust
pub struct TextLine { /* text: String, cursor: byte offset, private */ }
pub enum TextLineEvent { Ignored, Moved, Edited }
impl TextLine {
    pub fn new(initial: impl Into<String>) -> TextLine;   // cursor at end
    pub fn text(&self) -> &str;
    pub fn cursor(&self) -> usize;                         // byte offset, always a grapheme boundary
    pub fn cursor_cols(&self) -> u16;                      // display columns before the cursor (for Frame::set_cursor_position)
    pub fn insert(&mut self, c: char);
    pub fn backspace(&mut self) -> bool;                   // removes one grapheme; false at start
    pub fn delete(&mut self) -> bool;                      // removes grapheme after cursor; false at end
    pub fn left(&mut self); pub fn right(&mut self); pub fn home(&mut self); pub fn end(&mut self);
    pub fn on_key(&mut self, key: KeyEvent) -> TextLineEvent; // Char/Backspace/Delete/Left/Right/Home/End; ignores Ctrl/Alt chords
}
impl std::ops::Deref for TextLine { type Target = str; /* -> text */ }
```
  The `Deref<Target = str>` impl lets read-only call sites (`.contains(...)`, `&state.input` into a `&str` parameter) compile unchanged — this is the friction-minimizer for M7's palette `matches()` rewrite.

- [ ] **Step 1: Add the dependencies**

In `/Users/rehatbir/Developer/fables/notion-tui/Cargo.toml` `[workspace.dependencies]` add:

```toml
unicode-segmentation = "1"
unicode-width = "0.2"
```

In `crates/notion-tui/Cargo.toml` `[dependencies]` add:

```toml
unicode-segmentation = { workspace = true }
unicode-width = { workspace = true }
```

Run: `cargo check -p notion-tui` — expect clean (both crates are already in Cargo.lock via ratatui).

- [ ] **Step 2: Write the failing tests**

Create `crates/notion-tui/src/ui/textline.rs` with the tests first (module registered in Step 3 so they compile):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent};
    use unicode_segmentation::UnicodeSegmentation;

    #[test]
    fn backspace_removes_a_whole_multiscalar_grapheme() {
        // Regional-indicator flag: two scalars, one grapheme.
        let mut l = TextLine::new("hi🇨🇦");
        assert!(l.backspace());
        assert_eq!(l.text(), "hi");
        // ZWJ family emoji: many scalars, one grapheme.
        let mut l = TextLine::new("a👩‍👩‍👧‍👦");
        assert!(l.backspace());
        assert_eq!(l.text(), "a");
    }

    #[test]
    fn left_right_move_by_grapheme_and_clamp() {
        let mut l = TextLine::new("x👋y");
        l.left(); // before 'y'
        l.left(); // before the emoji
        assert_eq!(&l.text()[l.cursor()..], "👋y");
        l.right();
        assert_eq!(&l.text()[l.cursor()..], "y");
        l.right();
        l.right(); // clamped at end
        assert_eq!(l.cursor(), l.text().len());
    }

    #[test]
    fn home_end_delete_and_mid_insert() {
        let mut l = TextLine::new("First");
        l.home();
        assert!(l.delete()); // "irst"
        l.right();
        l.insert('X'); // "iXrst"
        assert_eq!(l.text(), "iXrst");
        l.end();
        assert!(!l.delete()); // nothing after the cursor
    }

    #[test]
    fn on_key_reports_edits_vs_moves_and_ignores_ctrl_chords() {
        use crossterm::event::KeyModifiers;
        let mut l = TextLine::new("");
        assert!(matches!(l.on_key(KeyEvent::from(KeyCode::Char('a'))), TextLineEvent::Edited));
        assert!(matches!(l.on_key(KeyEvent::from(KeyCode::Left)), TextLineEvent::Moved));
        assert!(matches!(l.on_key(KeyEvent::from(KeyCode::Backspace)), TextLineEvent::Edited));
        assert!(matches!(l.on_key(KeyEvent::from(KeyCode::Backspace)), TextLineEvent::Moved)); // empty: nothing removed
        assert!(matches!(
            l.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL)),
            TextLineEvent::Ignored
        ));
    }

    #[test]
    fn cursor_cols_is_display_width_not_bytes() {
        let mut l = TextLine::new("日本"); // 6 bytes, 4 display columns
        assert_eq!(l.cursor_cols(), 4);
        l.left();
        assert_eq!(l.cursor_cols(), 2);
    }

    proptest::proptest! {
        /// The spec §7 grapheme-editing property test: any sequence of edits
        /// leaves the cursor on a grapheme boundary and the text valid.
        #[test]
        fn editing_never_splits_graphemes(s in "\\PC{0,20}", ops in proptest::collection::vec(0..7usize, 0..60)) {
            let mut l = TextLine::new(s);
            for op in ops {
                match op {
                    0 => l.left(),
                    1 => l.right(),
                    2 => { l.backspace(); }
                    3 => { l.delete(); }
                    4 => l.home(),
                    5 => l.end(),
                    _ => l.insert('é'),
                }
                let c = l.cursor();
                proptest::prop_assert!(
                    c == l.text().len() || l.text().grapheme_indices(true).any(|(i, _)| i == c),
                    "cursor {c} not on a grapheme boundary of {:?}", l.text()
                );
            }
        }

        #[test]
        fn backspace_to_empty_never_panics(s in "\\PC{0,20}") {
            let mut l = TextLine::new(s);
            while l.backspace() {}
            proptest::prop_assert!(l.text().is_empty());
        }
    }
}
```

- [ ] **Step 3: Register the module and run tests to verify they fail**

Add `pub mod textline;` to `crates/notion-tui/src/ui/mod.rs`'s module list (alphabetical, after `pub mod theme;`).

Run: `cargo test -p notion-tui textline`
Expected: FAIL to compile — `TextLine` doesn't exist yet.

- [ ] **Step 4: Implement**

Above the tests in `textline.rs`:

```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A single-line editable text buffer with a movable cursor. All editing is
/// grapheme-cluster-safe: Backspace/Delete/Left/Right operate on whole
/// graphemes, so multi-scalar emoji never end up half-deleted.
#[derive(Clone, Debug, Default)]
pub struct TextLine {
    text: String,
    /// Byte offset of the cursor within `text`; always on a grapheme boundary.
    cursor: usize,
}

/// What a key did to the line — lets callers distinguish "the text changed"
/// (search must re-query) from "only the cursor moved".
pub enum TextLineEvent {
    Ignored,
    Moved,
    Edited,
}

impl TextLine {
    pub fn new(initial: impl Into<String>) -> TextLine {
        let text = initial.into();
        let cursor = text.len();
        TextLine { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Display width (terminal columns) of the text before the cursor — what
    /// render fns add to the input area's x to place the terminal cursor.
    pub fn cursor_cols(&self) -> u16 {
        self.text[..self.cursor].width() as u16
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor].grapheme_indices(true).last().map(|(i, _)| i)
    }

    fn next_boundary(&self) -> Option<usize> {
        self.text[self.cursor..].graphemes(true).next().map(|g| self.cursor + g.len())
    }

    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    pub fn backspace(&mut self) -> bool {
        match self.prev_boundary() {
            Some(start) => {
                self.text.replace_range(start..self.cursor, "");
                self.cursor = start;
                true
            }
            None => false,
        }
    }

    pub fn delete(&mut self) -> bool {
        match self.next_boundary() {
            Some(end) => {
                self.text.replace_range(self.cursor..end, "");
                true
            }
            None => false,
        }
    }

    pub fn left(&mut self) {
        if let Some(i) = self.prev_boundary() {
            self.cursor = i;
        }
    }

    pub fn right(&mut self) {
        if let Some(i) = self.next_boundary() {
            self.cursor = i;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.text.len();
    }

    pub fn on_key(&mut self, key: KeyEvent) -> TextLineEvent {
        if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return TextLineEvent::Ignored;
        }
        match key.code {
            KeyCode::Char(c) => {
                self.insert(c);
                TextLineEvent::Edited
            }
            KeyCode::Backspace => {
                if self.backspace() {
                    TextLineEvent::Edited
                } else {
                    TextLineEvent::Moved
                }
            }
            KeyCode::Delete => {
                if self.delete() {
                    TextLineEvent::Edited
                } else {
                    TextLineEvent::Moved
                }
            }
            KeyCode::Left => {
                self.left();
                TextLineEvent::Moved
            }
            KeyCode::Right => {
                self.right();
                TextLineEvent::Moved
            }
            KeyCode::Home => {
                self.home();
                TextLineEvent::Moved
            }
            KeyCode::End => {
                self.end();
                TextLineEvent::Moved
            }
            _ => TextLineEvent::Ignored,
        }
    }
}

impl std::ops::Deref for TextLine {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p notion-tui textline && cargo clippy -p notion-tui --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS.

- [ ] **Step 6: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/Cargo.toml notion-tui/Cargo.lock notion-tui/crates/notion-tui/Cargo.toml notion-tui/crates/notion-tui/src/ui/textline.rs notion-tui/crates/notion-tui/src/ui/mod.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): grapheme-safe TextLine editing widget"`
(Only if `git status` shows no unrelated staged hunks in these files; otherwise report and skip.)

---

### Task 2: adopt `TextLine` in every text input + visible terminal cursor

**Files:**
- Modify: `crates/notion-tui/src/ui/input.rs` (whole `InputState`), `crates/notion-tui/src/ui/search.rs:8-62,79-109`, `crates/notion-tui/src/ui/palette.rs:9-110`, `crates/notion-tui/src/ui/props.rs:21-36,95-134,417-433` (the `Editor::Text` arm only)
- Modify: `crates/notion-tui/src/app.rs:683-693` (`refresh_search` ONLY — Track QUEUE and Wave 2/3 own the rest of app.rs)
- Test: update inline tests in the four ui files, update `crates/notion-tui/tests/input_flow.rs:72,94` and `crates/notion-tui/tests/table_edit_flow.rs:60`; create `crates/notion-tui/tests/text_editing_flow.rs`

**Interfaces:**
- Consumes: Task 1's `TextLine`/`TextLineEvent`.
- Produces:
  - `InputState { pub title: String, pub line: TextLine }` — `new(title, initial)` keeps its exact signature (M7's rename/filter inputs construct through it and inherit everything); the old `pub value: String` field is replaced by `pub fn value(&self) -> &str`.
  - `SearchState.input: TextLine` (field name kept; `Deref` keeps `.is_empty()`-style reads working).
  - `PaletteState.input: TextLine` (same).
  - `props::Editor::Text { buffer: TextLine, error: Option<String> }`.
  - Every one of these modals calls `f.set_cursor_position(...)` while open, so the terminal cursor is visible at the edit point.

- [ ] **Step 1: Write the failing flow tests**

Create `crates/notion-tui/tests/text_editing_flow.rs` (fixture copied from `input_flow.rs:8-58`):

```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::{BlockRec, PageRec, Store};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui;
use notion_tui::ui::page::PageView;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn store_with_block(text: &str) -> notion_sync::SharedStore {
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
            plain_text: text.into(),
            has_children: false,
        }],
    )
    .unwrap();
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

#[test]
fn home_delete_right_insert_edits_mid_string() {
    let store = store_with_block("First");
    let mut app = app_on_page(store.clone());
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('i'))); // prefill "First", cursor at end
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Home));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Delete)); // "irst"
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Right)); // after 'i'
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('X'))); // "iXrst"
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Enter));
    let blocks = store.lock().unwrap().page_blocks("p1").unwrap();
    assert_eq!(blocks[0].plain_text, "iXrst");
}

#[test]
fn backspace_removes_a_whole_flag_emoji_in_the_input_modal() {
    let store = store_with_block("x");
    let mut app = app_on_page(store.clone());
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('a'))); // empty "new block" input
    for c in "hi".chars() {
        dispatch_key(&mut app, KeyEvent::from(KeyCode::Char(c)));
    }
    // Terminals deliver 🇨🇦 as two scalar-value key events:
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('🇨')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('🇦')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Backspace)); // must remove the whole flag
    assert_eq!(app.input.as_ref().unwrap().value(), "hi");
}

#[test]
fn input_modal_shows_the_terminal_cursor_at_the_edit_point() {
    let store = store_with_block("First");
    let mut app = app_on_page(store);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('i')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Home));
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    // input popup: width (60*2/3).clamp(20,70)=40 → x=10; height 3 → y=(12-3)/2=4.
    // Cursor at Home ⇒ one cell inside the border: (11, 5).
    let pos = term.get_cursor_position().unwrap();
    assert_eq!((pos.x, pos.y), (11, 5));
}

#[test]
fn search_left_arrow_does_not_refire_the_query() {
    let store = store_with_block("hello world");
    let mut app = app_on_page(store);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('/')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('h')));
    let hits = app.search.as_ref().unwrap().results.len();
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Left)); // movement only
    assert_eq!(app.search.as_ref().unwrap().results.len(), hits);
    assert_eq!(app.search.as_ref().unwrap().input.text(), "h");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test text_editing_flow`
Expected: FAIL — `value()`/`.text()` don't exist; Home/Delete are eaten by the `_ => InputAction::None` arm; `get_cursor_position` returns (0,0) because no frame sets a cursor.

- [ ] **Step 3: Rewrite `InputState` over `TextLine`**

`crates/notion-tui/src/ui/input.rs` — replace the struct, `on_key`, and `render`:

```rust
use crate::ui::textline::{TextLine, TextLineEvent};

pub struct InputState {
    pub title: String,
    pub line: TextLine,
}

impl InputState {
    pub fn new(title: impl Into<String>, initial: impl Into<String>) -> InputState {
        InputState {
            title: title.into(),
            line: TextLine::new(initial),
        }
    }

    pub fn value(&self) -> &str {
        self.line.text()
    }

    pub fn on_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Esc => InputAction::Cancel,
            KeyCode::Enter => InputAction::Submit(self.line.text().to_string()),
            _ => match self.line.on_key(key) {
                TextLineEvent::Edited => InputAction::Changed,
                TextLineEvent::Moved | TextLineEvent::Ignored => InputAction::None,
            },
        }
    }
}
```

In `render`, after the `Paragraph` widget:

```rust
    f.render_widget(
        Paragraph::new(state.line.text()).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", state.title)),
        ),
        popup,
    );
    f.set_cursor_position(ratatui::layout::Position::new(
        popup.x + 1 + state.line.cursor_cols().min(popup.width.saturating_sub(2)),
        popup.y + 1,
    ));
```

Update the inline tests: `s.value` → `s.value()` (two asserts), and extend `typing_backspace_and_submit` with a `Left`+`Char` mid-edit assertion:

```rust
        s.on_key(KeyEvent::from(KeyCode::Left));
        s.on_key(KeyEvent::from(KeyCode::Char('a')));
        assert_eq!(s.value(), "ah"); // typed before the final grapheme
```

(Adjust the final Submit assertion accordingly: expect `"ah"`.)

- [ ] **Step 4: Convert `SearchState` and `PaletteState`**

`search.rs`: field becomes `pub input: TextLine` (init `TextLine::new("")` — keep `SearchState::new()`); `on_key` keeps the Esc/Enter/Down/Up arms and replaces the Backspace/Char arms with a single default:

```rust
            _ => match self.input.on_key(key) {
                crate::ui::textline::TextLineEvent::Edited => SearchAction::QueryChanged,
                _ => SearchAction::None,
            },
```

`render`: `Paragraph::new(state.input.text())`, then

```rust
    f.set_cursor_position(ratatui::layout::Position::new(
        inner[0].x + 1 + state.input.cursor_cols().min(inner[0].width.saturating_sub(2)),
        inner[0].y + 1,
    ));
```

Inline tests: `assert_eq!(s.input, "a")` → `assert_eq!(s.input.text(), "a")` (both occurrences).

`palette.rs`: same conversion. `matches()` keeps compiling via `Deref` (`self.input.as_str()` → `self.input.text()` for clarity); the Backspace/Char arms collapse to:

```rust
            _ => match self.input.on_key(key) {
                crate::ui::textline::TextLineEvent::Edited => {
                    self.cursor = 0;
                    PaletteAction::Changed
                }
                _ => PaletteAction::None,
            },
```

`render`: `Paragraph::new(state.input.text())` + the same `set_cursor_position` on `inner[0]`.

`app.rs:683-693` `refresh_search`: `Some(s) => s.input.text().to_string(),`.

- [ ] **Step 5: Convert the props text editor**

`props.rs`: `Editor::Text { buffer: TextLine, error: Option<String> }`. Construction site (`on_key` Enter arm, ~line 261):

```rust
                                Editor::Text {
                                    buffer: crate::ui::textline::TextLine::new(field.value_text.clone()),
                                    error: None,
                                }
```

`on_key`'s `Editor::Text` arm: keep Esc and Enter (Enter validates `validate_input(&field.prop_type, buffer.text())` and commits `buffer.text().to_string()`); replace the Backspace/Char arms with:

```rust
                    _ => {
                        if matches!(buffer.on_key(key), crate::ui::textline::TextLineEvent::Edited) {
                            *error = None;
                        }
                        PropsAction::None
                    }
```

`render`'s `Editor::Text` arm: build the paragraph from `buffer.text()` and set the cursor on the buffer row:

```rust
            Editor::Text { buffer, error } => {
                let text = match error {
                    Some(msg) => format!("{}\n✗ {msg}", buffer.text()),
                    None => buffer.text().to_string(),
                };
                f.render_widget(
                    Paragraph::new(text).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" {} ", field.name)),
                    ),
                    popup,
                );
                f.set_cursor_position(ratatui::layout::Position::new(
                    popup.x + 1 + buffer.cursor_cols().min(popup.width.saturating_sub(2)),
                    popup.y + 1,
                ));
            }
```

- [ ] **Step 6: Fix the remaining call sites and run everything**

`tests/input_flow.rs:72,94` and `tests/table_edit_flow.rs:60`: `.value` → `.value()`.
Then grep for stragglers: `grep -rn "\.input\.push\|\.input\.pop\|\.value\b" crates/notion-tui/src crates/notion-tui/tests` — must return nothing (except `value_text`).

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS, including all pre-existing input/search/palette/props flow tests (they type via `on_key`, which still accepts Char/Backspace).

- [ ] **Step 7: M7 integration checklist (do now if M7 has landed; else report as follow-up)**

- [ ] `crates/notion-tui/src/ui/picker.rs` (M7 Task 3): switch `PickerState.input` from `String` to `TextLine` exactly as PaletteState above (default `on_key` arm → `input.on_key`; `matches()` reads `self.input.text()`; `render` gains the same `set_cursor_position` on its input row).
- [ ] M7's table-filter input (`InputPurpose::FilterTable`) and rename input (`InputPurpose::RenamePage`) arrive through `InputState` and inherit TextLine behavior automatically — verify by opening each once, no code needed.
- [ ] M7's palette `matches()` rewrite calls `crate::fuzzy::subsequence_rank(&self.input, ...)`; with `TextLine` this still compiles via `Deref` if the parameter is `&str` — if not, change to `self.input.text()`. Whichever milestone lands second makes this one-line adaptation.

- [ ] **Step 8: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/ui/input.rs notion-tui/crates/notion-tui/src/ui/search.rs notion-tui/crates/notion-tui/src/ui/palette.rs notion-tui/crates/notion-tui/src/ui/props.rs notion-tui/crates/notion-tui/tests/text_editing_flow.rs notion-tui/crates/notion-tui/tests/input_flow.rs notion-tui/crates/notion-tui/tests/table_edit_flow.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): cursor-based grapheme-safe editing in all text inputs"`
(app.rs's one-line `refresh_search` change: stage only if app.rs carries no unrelated uncommitted hunks; otherwise report.)

---

### Task 3: theme the modals

**Files:**
- Modify: `crates/notion-tui/src/ui/theme.rs` (add `popup_block` helper), `crates/notion-tui/src/ui/search.rs`, `input.rs`, `props.rs`, `confirm.rs`, `palette.rs`, `help.rs` (render signatures), `crates/notion-tui/src/ui/mod.rs:107-124` (call sites)
- Test: `crates/notion-tui/tests/themed_modals.rs` (new — the spec §7 "themed modals" render-smoke deliverable)

**Interfaces:**
- Consumes: Task 2's converted render fns (same files — sequential within Track TEXT).
- Produces (M7's picker follows the same pattern):
```rust
// ui/theme.rs
pub fn popup_block(title: String, theme: &Theme) -> ratatui::widgets::Block<'static>;
// new render signatures:
pub fn search::render(f: &mut Frame, state: &mut SearchState, theme: &Theme);
pub fn input::render(f: &mut Frame, state: &InputState, theme: &Theme);
pub fn props::render(f: &mut Frame, state: &PropsState, theme: &Theme);
pub fn confirm::render(f: &mut Frame, state: &ConfirmState, theme: &Theme);
pub fn palette::render(f: &mut Frame, state: &mut PaletteState, theme: &Theme);
pub fn help::render(f: &mut Frame, keymap: &Keymap, theme: &Theme);
```

- [ ] **Step 1: Write the failing test**

Create `crates/notion-tui/tests/themed_modals.rs`:

```rust
use std::sync::{Arc, Mutex};

use notion_tui::app::App;
use notion_tui::ui;
use ratatui::style::Color;
use ratatui::{backend::TestBackend, Terminal};

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

/// The dark theme's border color (theme.rs: Color::Indexed(240)) must appear
/// in the frame whenever a modal is open — proving the modal took the theme.
fn assert_dark_border(app: &mut App, which: &str) {
    app.theme = notion_tui::ui::theme::named("dark");
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(60, 20);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    let themed = term
        .backend()
        .buffer()
        .content
        .iter()
        .any(|c| c.style().fg == Some(Color::Indexed(240)));
    assert!(themed, "{which} modal ignored the dark theme's border color");
}

#[test]
fn search_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true; // sidebar draws its own themed border; hide it so only the modal can pass
    app.search = Some(notion_tui::ui::search::SearchState::new());
    assert_dark_border(&mut app, "search");
}

#[test]
fn input_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.input = Some(notion_tui::ui::input::InputState::new("t", ""));
    assert_dark_border(&mut app, "input");
}

#[test]
fn confirm_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.confirm = Some(notion_tui::ui::confirm::ConfirmState {
        message: "sure?".into(),
        ids: vec![],
        kind: notion_tui::ui::confirm::ConfirmKind::DeleteProtected,
    });
    assert_dark_border(&mut app, "confirm");
}

#[test]
fn palette_and_help_are_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.palette = Some(notion_tui::ui::palette::PaletteState::new());
    assert_dark_border(&mut app, "palette");
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.help_open = true;
    assert_dark_border(&mut app, "help");
}

#[test]
fn props_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.props = Some(notion_tui::ui::props::PropsState::new(
        "r1".into(),
        vec![notion_tui::ui::props::PropField {
            name: "Name".into(),
            prop_type: "title".into(),
            value_text: "x".into(),
            options: vec![],
        }],
    ));
    assert_dark_border(&mut app, "props");
}
```

Note: `View::Empty` renders an unstyled `Block` and the status bar uses `theme.status` (Indexed(236)/(250), not 240) — so with the sidebar hidden, only a themed modal border can produce Indexed(240).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test themed_modals`
Expected: FAIL to compile at first (render signatures unchanged compile, but the assertion fails: modals draw `Style::default()` borders). If it compiles, every test FAILs on the assert.

- [ ] **Step 3: Implement**

`theme.rs` — add:

```rust
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};

/// The one popup chrome every modal shares: themed border + themed title.
pub fn popup_block(title: String, theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Span::styled(title, theme.title))
}
```

Then in each modal:
- `input.rs`: `pub fn render(f: &mut Frame, state: &InputState, theme: &Theme)`; block becomes `crate::ui::theme::popup_block(format!(" {} ", state.title), theme)`.
- `search.rs`: signature gains `theme: &Theme`; input block `popup_block(" search ".into(), theme)`; results `List` block `popup_block(String::new(), theme)` and `.highlight_style(theme.highlight)` replaces `Modifier::REVERSED`.
- `palette.rs`: same — `popup_block(" : ".into(), theme)` twice, `.highlight_style(theme.highlight)`.
- `confirm.rs`: signature gains `theme`; block `popup_block(" confirm ".into(), theme)`.
- `props.rs`: signature gains `theme`; all four `Block::default()...` sites become `popup_block(...)`; the three `Style::default().add_modifier(Modifier::REVERSED)` item styles become `theme.highlight`.
- `help.rs`: `pub fn render(f: &mut Frame, keymap: &Keymap, theme: &Theme)`; block `popup_block(" help (any key to close) ".into(), theme)`.
- `ui/mod.rs:107-124`: append `, &theme` to all six calls.

The `"default"` theme is all `Style::default()` + `REVERSED` highlight, so existing insta snapshots in `tests/snapshots/` must not change.

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS, including the render_smoke insta snapshots (unchanged under the default theme). If a snapshot diff appears, the default-theme fallback regressed — fix, don't re-accept snapshots.

- [ ] **Step 5: M7 integration checklist**

- [ ] If `crates/notion-tui/src/ui/picker.rs` (M7 Task 3) exists in your tree: give `picker::render` the same `theme: &Theme` parameter and `popup_block`/`theme.highlight` treatment, and pass `&theme` at its `ui/mod.rs` call site. Otherwise record as follow-up.

- [ ] **Step 6: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/ui/theme.rs notion-tui/crates/notion-tui/src/ui/input.rs notion-tui/crates/notion-tui/src/ui/search.rs notion-tui/crates/notion-tui/src/ui/palette.rs notion-tui/crates/notion-tui/src/ui/props.rs notion-tui/crates/notion-tui/src/ui/confirm.rs notion-tui/crates/notion-tui/src/ui/help.rs notion-tui/crates/notion-tui/src/ui/mod.rs notion-tui/crates/notion-tui/tests/themed_modals.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): thread Theme through every modal"`

---

### Task 4: `describe_op` — human-readable queue rows (M10 contract)

**Files:**
- Create: `crates/notion-tui/src/describe.rs`
- Modify: `crates/notion-tui/src/lib.rs` (add `pub mod describe;`), `crates/notion-store/src/store.rs` (add `get_row` next to `rows()` at :254), `crates/notion-tui/src/ui/queue.rs`, `crates/notion-tui/src/app.rs:480-514` (`toggle_queue` + `refresh_queue` ONLY)
- Test: inline in `describe.rs`; extend `crates/notion-tui/tests/queue_flow.rs`

**Interfaces:**
- Consumes: `notion_store::{OpRec, Store}` (real names verified: `OpRec { seq, op_type, target_id, payload, base_edited_time, state, error }`); op payloads as written by `store.rs` (`update_block`/`append_block`/`delete_block`/`reorder_block` carry `page_id`; `create_row` carries `data_source_id` + `properties`; `update_row` carries `properties` patch; `create_comment` carries `parent_id`/`body`).
- **Produces — FINAL SIGNATURE, consumed verbatim by the M10 plan:**
```rust
// crates/notion-tui/src/describe.rs
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
  Also: `Store::get_row(&self, id: &str) -> anyhow::Result<Option<RowRec>>` (includes archived rows — delete_row ops must still resolve their title) and `QueueView::new(ops: Vec<OpRec>, summaries: Vec<String>)`.
  Unknown op types (M7's `rename_page`/`move_page`, anything future) degrade to `"{op_type with spaces} on '{title-or-short-id}'"` — never panic, never print a full UUID.

- [ ] **Step 1: Write the failing unit tests**

In `crates/notion-tui/src/describe.rs` (module registered in Step 2):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{BlockRec, OpRec, PageRec, Store};
    use serde_json::json;

    fn store_with_page() -> Store {
        let mut s = Store::open_in_memory().unwrap();
        s.upsert_page(&PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Meeting Notes".into(),
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
                plain_text: "agenda item one".into(),
                has_children: false,
            }],
        )
        .unwrap();
        s
    }

    #[test]
    fn update_block_op_names_page_and_snippets_block_text() {
        let mut s = store_with_page();
        s.edit_update_block_text("b1", "agenda item one v2").unwrap();
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "edit ¶ in 'Meeting Notes'");
        assert_eq!(d.target_title.as_deref(), Some("Meeting Notes"));
        assert!(d.detail.as_deref().unwrap().starts_with("agenda item one"));
    }

    #[test]
    fn update_row_op_names_property_and_row_title() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn()
            .execute(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived, dirty)
                 VALUES ('r1', 'ds1', ?1, 't1', 0, 0)",
                [json!({
                    "Name": {"type": "title", "title": [{"plain_text": "Q3 Launch"}]},
                    "Status": {"type": "status", "status": {"name": "Todo"}}
                })
                .to_string()],
            )
            .unwrap();
        s.edit_update_row("r1", json!({"Status": {"type": "status", "status": {"name": "Doing"}}}))
            .unwrap();
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "set Status on 'Q3 Launch'");
        assert_eq!(d.detail.as_deref(), Some("Status"));
    }

    #[test]
    fn create_comment_op_names_the_parent_page() {
        let mut s = store_with_page();
        s.edit_add_comment("p1", "page", None, "looks good to me").unwrap();
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "comment on 'Meeting Notes'");
        assert_eq!(d.detail.as_deref(), Some("looks good to me"));
    }

    #[test]
    fn unknown_op_type_falls_back_to_short_id_never_full_uuid() {
        let s = Store::open_in_memory().unwrap();
        let op = OpRec {
            seq: 1,
            op_type: "rename_page".into(),
            target_id: "0123456789abcdef0123".into(),
            payload: "{}".into(),
            base_edited_time: None,
            state: "pending".into(),
            error: None,
        };
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "rename page on '01234567…'");
        assert!(d.target_title.is_none());
    }

    #[test]
    fn delete_row_op_resolves_title_of_archived_row() {
        let mut s = Store::open_in_memory().unwrap();
        s.conn()
            .execute(
                "INSERT INTO rows (id, data_source_id, properties, last_edited_time, archived, dirty)
                 VALUES ('r1', 'ds1', ?1, 't1', 0, 0)",
                [json!({"Name": {"type": "title", "title": [{"plain_text": "Q3 Launch"}]}}).to_string()],
            )
            .unwrap();
        s.edit_delete_row("r1").unwrap(); // archives the row
        let op = s.ops().unwrap().pop().unwrap();
        let d = describe_op(&op, &s);
        assert_eq!(d.summary, "delete row 'Q3 Launch'");
    }
}
```

And in `tests/queue_flow.rs` (fixture `store_with_conflicted_op` already builds page "P" with an update_block op):

```rust
#[test]
fn queue_rows_show_titles_not_uuids() {
    use notion_tui::ui;
    use ratatui::{backend::TestBackend, Terminal};

    let (store, _) = store_with_conflicted_op();
    let mut app = App::new(store);
    app.focus = Focus::Main;
    dispatch_key(&mut app, shift_q());
    let backend = TestBackend::new(80, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let content: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    assert!(content.contains("edit ¶ in 'P'"), "queue row not humanized:\n{content}");
}
```

- [ ] **Step 2: Register modules and run tests to verify they fail**

Add `pub mod describe;` to `crates/notion-tui/src/lib.rs` (after `pub mod config;`).

Run: `cargo test -p notion-tui describe && cargo test -p notion-tui --test queue_flow`
Expected: FAIL to compile — `describe_op`, `Store::get_row`, `QueueView::new` two-arg all missing.

- [ ] **Step 3: Implement `Store::get_row`**

In `crates/notion-store/src/store.rs`, directly after `rows()` (:254-271):

```rust
    /// Single-row lookup by id, including archived rows (queue descriptions of
    /// delete_row ops must still resolve the row's title).
    pub fn get_row(&self, id: &str) -> anyhow::Result<Option<RowRec>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, data_source_id, properties, last_edited_time, archived
             FROM rows WHERE id = ?1",
        )?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(RowRec {
                id: r.get(0)?,
                data_source_id: r.get(1)?,
                properties: r.get(2)?,
                last_edited_time: r.get(3)?,
                archived: r.get::<_, i64>(4)? != 0,
            }),
            None => None,
        })
    }
```

- [ ] **Step 4: Implement `describe.rs`**

```rust
use notion_store::{OpRec, Store};
use serde_json::Value;

/// A human-readable rendering of a pending op. `summary` is what the queue
/// shows; the structured fields exist for M10's conflict detail view.
pub struct OpDescription {
    /// One-line human summary, e.g. "edit ¶ in 'Meeting Notes'" or "set Status on 'Q3 Launch'".
    pub summary: String,
    /// Title of the page / row / comment parent the op targets, when resolvable from the store.
    pub target_title: Option<String>,
    /// The property name (row ops), block-text snippet (block ops), or comment-body snippet.
    pub detail: Option<String>,
}

fn short_id(id: &str) -> String {
    let mut s: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        s.push('…');
    }
    s
}

fn snippet(text: &str) -> Option<String> {
    if text.is_empty() {
        return None;
    }
    let mut s: String = text.chars().take(24).collect();
    if text.chars().count() > 24 {
        s.push('…');
    }
    Some(s)
}

fn title_from_props(props: &Value) -> Option<String> {
    props
        .as_object()?
        .values()
        .find(|p| p["type"] == "title")
        .map(crate::ui::table::cell_text)
        .filter(|t| !t.is_empty())
}

fn row_title(store: &Store, row_id: &str) -> Option<String> {
    let row = store.get_row(row_id).ok().flatten()?;
    let props: Value = serde_json::from_str(&row.properties).ok()?;
    title_from_props(&props)
}

fn block_snippet(store: &Store, page_id: &str, block_id: &str) -> Option<String> {
    let text = store
        .page_blocks(page_id)
        .ok()?
        .into_iter()
        .find(|b| b.id == block_id)?
        .plain_text;
    snippet(&text)
}

pub fn describe_op(op: &OpRec, store: &Store) -> OpDescription {
    let payload: Value = serde_json::from_str(&op.payload).unwrap_or_default();
    match op.op_type.as_str() {
        "update_block" | "append_block" | "delete_block" | "reorder_block" => {
            let verb = match op.op_type.as_str() {
                "update_block" => "edit",
                "append_block" => "add",
                "delete_block" => "delete",
                _ => "move",
            };
            let page_id = payload["page_id"].as_str().unwrap_or("");
            let target_title = store.get_page(page_id).ok().flatten().map(|p| p.title);
            let shown = target_title.clone().unwrap_or_else(|| short_id(page_id));
            OpDescription {
                summary: format!("{verb} ¶ in '{shown}'"),
                target_title,
                detail: block_snippet(store, page_id, &op.target_id),
            }
        }
        "create_row" => {
            let target_title = title_from_props(&payload["properties"]);
            let ds_id = payload["data_source_id"].as_str().unwrap_or("");
            let ds_title = store
                .get_data_source(ds_id)
                .ok()
                .flatten()
                .map(|d| d.title)
                .unwrap_or_else(|| short_id(ds_id));
            let shown = target_title.clone().unwrap_or_else(|| "untitled".into());
            OpDescription {
                summary: format!("add row '{shown}' to '{ds_title}'"),
                target_title,
                detail: None,
            }
        }
        "update_row" => {
            let target_title = row_title(store, &op.target_id);
            let prop = payload["properties"]
                .as_object()
                .and_then(|m| m.keys().next().cloned());
            let shown = target_title.clone().unwrap_or_else(|| short_id(&op.target_id));
            let summary = match &prop {
                Some(p) => format!("set {p} on '{shown}'"),
                None => format!("edit '{shown}'"),
            };
            OpDescription {
                summary,
                target_title,
                detail: prop,
            }
        }
        "delete_row" | "restore_row" => {
            let verb = if op.op_type == "delete_row" { "delete" } else { "restore" };
            let target_title = row_title(store, &op.target_id);
            let shown = target_title.clone().unwrap_or_else(|| short_id(&op.target_id));
            OpDescription {
                summary: format!("{verb} row '{shown}'"),
                target_title,
                detail: None,
            }
        }
        "create_comment" => {
            let parent_id = payload["parent_id"].as_str().unwrap_or("");
            let target_title = store
                .get_page(parent_id)
                .ok()
                .flatten()
                .map(|p| p.title)
                .or_else(|| row_title(store, parent_id));
            let shown = target_title.clone().unwrap_or_else(|| short_id(parent_id));
            OpDescription {
                summary: format!("comment on '{shown}'"),
                target_title,
                detail: payload["body"].as_str().and_then(snippet),
            }
        }
        other => {
            // M7 adds rename_page/move_page; future ops land here too.
            let target_title = store.get_page(&op.target_id).ok().flatten().map(|p| p.title);
            let shown = target_title.clone().unwrap_or_else(|| short_id(&op.target_id));
            OpDescription {
                summary: format!("{} on '{shown}'", other.replace('_', " ")),
                target_title,
                detail: None,
            }
        }
    }
}
```

- [ ] **Step 5: Wire the queue**

`ui/queue.rs`:

```rust
pub struct QueueView {
    pub ops: Vec<OpRec>,
    /// Parallel to `ops`: human summaries from `crate::describe::describe_op`.
    pub summaries: Vec<String>,
    pub cursor: usize,
    pub list_state: ListState,
}

impl QueueView {
    pub fn new(ops: Vec<OpRec>, summaries: Vec<String>) -> QueueView {
        QueueView {
            ops,
            summaries,
            cursor: 0,
            list_state: ListState::default(),
        }
    }
    // move_cursor / selected unchanged
}
```

Render item body (:37-47) becomes:

```rust
    let items: Vec<ListItem> = view
        .ops
        .iter()
        .enumerate()
        .map(|(i, op)| {
            let err = op.error.as_deref().unwrap_or("");
            let summary = view
                .summaries
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("{} {}", op.op_type, op.target_id));
            ListItem::new(format!("#{} {} [{}] {}", op.seq, summary, op.state, err))
        })
        .collect();
```

`app.rs` `toggle_queue` (:480-491) — the non-queue arm:

```rust
            other => {
                self.queue_return = Some(Box::new(other));
                let guard = self.store.lock().unwrap();
                let ops = guard.ops().unwrap_or_default();
                let summaries: Vec<String> =
                    ops.iter().map(|o| crate::describe::describe_op(o, &guard).summary).collect();
                drop(guard);
                self.view = View::Queue(crate::ui::queue::QueueView::new(ops, summaries));
            }
```

`refresh_queue` (:505-514): same guard/ops/summaries dance, then `QueueView::new(ops, summaries)` with the preserved-cursor clamp as today.

- [ ] **Step 6: Run tests**

Run: `cargo test -p notion-tui-store && cargo test -p notion-tui && cargo test --workspace`
Expected: PASS (queue_flow's pre-existing tests read `q.ops`, which is unchanged).

- [ ] **Step 7: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/describe.rs notion-tui/crates/notion-tui/src/lib.rs notion-tui/crates/notion-tui/src/ui/queue.rs notion-tui/crates/notion-tui/tests/queue_flow.rs notion-tui/crates/notion-store/src/store.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): human-readable queue via describe_op (M10 contract)"`
(app.rs: same unrelated-hunks caveat as always.)

---

### Task 5: wizard honesty — 401 vs network failure

**Files:**
- Modify: `crates/notion-tui/src/wizard.rs` (whole file is small)
- Test: inline in `wizard.rs`

**Interfaces:**
- Consumes: `notion_api::ApiError` variants (`Network(reqwest::Error)`, `Api { status, .. }`, `RetriesExhausted(String)` — verified in `crates/notion-api/src/error.rs`).
- Produces:
```rust
pub enum TokenError {
    /// Notion answered and rejected the token (401/403) or returned no identity.
    Invalid,
    /// Notion could not be reached at all (connect/timeout/DNS or retries exhausted).
    Network(String),
}
pub fn run_with(
    input: impl BufRead,
    mut output: impl Write,
    mut validate: impl FnMut(&str) -> Result<String, TokenError>,
) -> anyhow::Result<String>;
```

- [ ] **Step 1: Write the failing test**

Append to `wizard.rs`'s tests module:

```rust
    #[test]
    fn network_failure_message_differs_from_invalid_token() {
        let input = b"tok1\ntok2\n" as &[u8];
        let mut output = Vec::new();
        let mut calls = 0;
        let token = run_with(input, &mut output, |_| {
            calls += 1;
            if calls == 1 {
                Err(TokenError::Network("dns error".into()))
            } else {
                Ok("My Bot".to_string())
            }
        })
        .unwrap();
        assert_eq!(token, "tok2");
        let printed = String::from_utf8(output).unwrap();
        assert!(printed.contains("can't reach Notion — check your connection"));
        assert!(printed.contains("dns error"));
        assert!(!printed.contains("invalid token"), "network failure must not blame the token");
    }
```

And update the two existing tests' closures to the new signature:
- `accepts_first_valid_token`: `|t| if t == "secret_good" { Ok("My Bot".to_string()) } else { Err(TokenError::Invalid) }`
- `reprompts_on_invalid_token`: same closure; its `contains("invalid")` assertion stays.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui wizard`
Expected: FAIL to compile — `TokenError` missing, closures return the wrong type.

- [ ] **Step 3: Implement**

```rust
/// Why a pasted token was not accepted — the wizard must not tell a user with
/// a broken network that their token is wrong (spec M8.6).
pub enum TokenError {
    /// Notion answered and rejected the token (401/403) or returned no identity.
    Invalid,
    /// Notion could not be reached at all (connect/timeout/DNS or retries exhausted).
    Network(String),
}
```

In `run_with`, the match becomes:

```rust
        match validate(&token) {
            Ok(name) => {
                writeln!(output, "ok — connected as \"{name}\"")?;
                return Ok(token);
            }
            Err(TokenError::Invalid) => {
                writeln!(output, "invalid token, try again:")?;
            }
            Err(TokenError::Network(detail)) => {
                writeln!(
                    output,
                    "can't reach Notion — check your connection ({detail}); try again:"
                )?;
            }
        }
```

In `run()`, the production closure:

```rust
        run_with(stdin.lock(), stdout.lock(), |t| {
            let client = notion_api::NotionClient::new(t.to_string());
            match tokio::runtime::Handle::current().block_on(client.me()) {
                Ok(name) if !name.is_empty() => Ok(name),
                Ok(_) => Err(TokenError::Invalid),
                Err(notion_api::ApiError::Network(e)) => Err(TokenError::Network(e.to_string())),
                Err(notion_api::ApiError::RetriesExhausted(what)) => Err(TokenError::Network(what)),
                Err(_) => Err(TokenError::Invalid), // 401/403/etc — Notion answered and said no
            }
        })
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui wizard && cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/wizard.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): wizard distinguishes invalid token from network failure"`

---

### Task 6: friendly config errors — file/line in TOML failures, warnings for bad theme/keys

**Files:**
- Modify: `crates/notion-tui/src/config.rs` (`Config` struct, `from_sources`, `load_with_token`), `crates/notion-tui/src/keymap.rs` (`with_overrides_checked`), `crates/notion-tui/src/main.rs:26-31` (startup warning surfacing)
- Test: inline in `config.rs` and `keymap.rs`

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Task 12 extends `with_overrides_checked` with fixed-chord collision warnings):
  - `Config` gains `pub warnings: Vec<String>` (non-fatal config problems).
  - `pub fn Keymap::with_overrides_checked(overrides: &HashMap<String, String>) -> (Keymap, Vec<String>)`; `with_overrides` becomes a thin wrapper that discards warnings.
  - `pub const KNOWN_THEMES: &[&str] = &["default", "dark", "light"];` in `config.rs`.
  - `main.rs` joins config + keymap warnings into `app.notice` at startup.

- [ ] **Step 1: Write the failing tests**

Append to `config.rs`'s tests module:

```rust
    #[test]
    fn toml_parse_error_reports_the_line() {
        let err = from_sources(
            None,
            Some("tok".into()),
            Some("theme = [oops"),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.to_lowercase().contains("line"), "no line info in: {msg}");
    }

    #[test]
    fn unknown_theme_warns_instead_of_silently_defaulting() {
        let cfg = from_sources(
            None,
            Some("tok".into()),
            Some("theme = \"solarized\""),
            PathBuf::from("/tmp/x.db"),
        )
        .unwrap();
        assert_eq!(cfg.warnings.len(), 1);
        assert!(cfg.warnings[0].contains("solarized"));
        assert!(cfg.warnings[0].contains("unknown theme"));
    }

    #[test]
    fn known_theme_produces_no_warning() {
        let cfg = from_sources(None, Some("tok".into()), Some("theme = \"dark\""), PathBuf::from("/tmp/x.db"))
            .unwrap();
        assert!(cfg.warnings.is_empty());
    }
```

Append to `keymap.rs`'s tests module:

```rust
    #[test]
    fn bad_overrides_warn_and_keep_defaults() {
        let mut o = HashMap::new();
        o.insert("quit".to_string(), "ctrl+shift+q".to_string()); // unparseable
        o.insert("nosuch".to_string(), "x".to_string()); // unknown action
        let (km, warnings) = Keymap::with_overrides_checked(&o);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("nosuch") && w.contains("unknown action")));
        assert!(warnings.iter().any(|w| w.contains("ctrl+shift+q")));
        // the bad override must NOT clobber the default:
        assert!(km.is("quit", KeyEvent::from(KeyCode::Char('q'))));
    }
```

(`use std::collections::HashMap;` is already imported at the top of keymap.rs.)

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui config && cargo test -p notion-tui keymap`
Expected: FAIL — `warnings` field and `with_overrides_checked` missing. (`toml_parse_error_reports_the_line` may already pass — toml's Display includes line/column; keep it as a regression pin.)

- [ ] **Step 3: Implement config side**

`config.rs`:

```rust
pub const KNOWN_THEMES: &[&str] = &["default", "dark", "light"];
```

`Config` gains `pub warnings: Vec<String>,`. In `from_sources`, before building `Config`:

```rust
    let mut warnings = Vec::new();
    let theme = file
        .get("theme")
        .and_then(|v| v.as_str())
        .unwrap_or("default")
        .to_string();
    if !KNOWN_THEMES.contains(&theme.as_str()) {
        warnings.push(format!("unknown theme \"{theme}\" — using default"));
    }
```

…and the struct literal uses `theme,` and `warnings,`. In `load_with_token`, name the file on any error (parse errors keep toml's own line/column text as the cause):

```rust
pub fn load_with_token(token: Option<String>) -> anyhow::Result<Config> {
    let path = dirs::config_dir().map(|d| d.join("notion-tui/config.toml"));
    let file = path
        .as_ref()
        .filter(|p| p.exists())
        .and_then(|p| std::fs::read_to_string(p).ok());
    let default_db = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("notion-tui/notion.db");
    let file_existed = file.is_some();
    from_sources(
        token.or_else(token_from_keyring),
        std::env::var("NOTION_TOKEN").ok(),
        file.as_deref(),
        default_db,
    )
    .map_err(|e| match (&path, file_existed) {
        (Some(p), true) => e.context(format!("in config file {}", p.display())),
        _ => e,
    })
}
```

- [ ] **Step 4: Implement keymap side**

`keymap.rs`:

```rust
    pub fn with_overrides_checked(overrides: &HashMap<String, String>) -> (Keymap, Vec<String>) {
        let mut km = Keymap::new();
        let mut warnings = Vec::new();
        for (action, key_str) in overrides {
            if !DEFAULTS.iter().any(|(a, _, _)| a == action) {
                warnings.push(format!("keys.{action}: unknown action"));
                continue;
            }
            match parse_key(key_str) {
                Some(code) => {
                    km.map.insert(action.clone(), code);
                }
                None => warnings.push(format!("keys.{action}: can't parse key \"{key_str}\"")),
            }
        }
        (km, warnings)
    }

    pub fn with_overrides(overrides: &HashMap<String, String>) -> Keymap {
        Self::with_overrides_checked(overrides).0
    }
```

`main.rs` — replace line 29 (`app.keymap = ...`) with:

```rust
    let (keymap, key_warnings) = notion_tui::keymap::Keymap::with_overrides_checked(&cfg.keys);
    app.keymap = keymap;
    let startup_warnings: Vec<String> = cfg.warnings.iter().cloned().chain(key_warnings).collect();
    if !startup_warnings.is_empty() {
        app.notice = Some(startup_warnings.join(" · "));
    }
```

(The notice clears on the first keypress by design — it is a nudge, not a log; M9's tracing gets the durable copy.)

- [ ] **Step 5: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/config.rs notion-tui/crates/notion-tui/src/keymap.rs notion-tui/crates/notion-tui/src/main.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): friendly config errors — file/line context and warnings"`

---

### Task 7: type-aware table column widths with ellipsis

**Files:**
- Modify: `crates/notion-tui/src/ui/table.rs:195-255` (`render`) plus two new free functions
- Test: inline in `table.rs`

**Interfaces:**
- Consumes: nothing from other tasks (`unicode-segmentation`/`unicode-width` land in Task 1, which is Wave-1-parallel — if Task 1 hasn't merged yet, add the two dependency lines from Task 1 Step 1 yourself; they'll merge cleanly).
- Produces:
```rust
// ui/table.rs
fn column_constraint(col: &Column, is_title: bool) -> Constraint; // checkbox 6, number 10, date 12, url/email 28, default 14, title Min(20)
pub fn truncate_ellipsis(s: &str, max_cols: u16) -> String;       // display-width-aware, grapheme-safe, appends '…'
```
  **M7 coordination:** if M7 Task 8 (table filtering) has landed, `render` iterates `view.visible()` instead of `view.rows` — apply the truncation inside whichever row loop exists; do not add/remove `TableView` fields.

- [ ] **Step 1: Write the failing tests**

Append to `table.rs`'s tests module:

```rust
    #[test]
    fn truncate_ellipsis_is_width_aware_and_grapheme_safe() {
        assert_eq!(truncate_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_ellipsis("hello world", 6), "hello…");
        // CJK chars are 2 columns wide: 日本 = 4 cols, +… = 5.
        assert_eq!(truncate_ellipsis("日本語テスト", 5), "日本…");
        // A budget of 3 can't fit the first 2-wide char plus…: just the char that fits.
        assert_eq!(truncate_ellipsis("日本語", 3), "日…");
    }

    #[test]
    fn column_widths_are_type_aware() {
        let cb = Column { name: "Done".into(), prop_type: "checkbox".into() };
        let url = Column { name: "Link".into(), prop_type: "url".into() };
        match column_constraint(&cb, false) {
            Constraint::Length(n) => assert!(n <= 8, "checkbox must be narrow, got {n}"),
            other => panic!("expected Length, got {other:?}"),
        }
        match column_constraint(&url, false) {
            Constraint::Length(n) => assert!(n >= 24, "url must be wide, got {n}"),
            other => panic!("expected Length, got {other:?}"),
        }
        assert!(matches!(
            column_constraint(&Column { name: "Name".into(), prop_type: "title".into() }, true),
            Constraint::Min(20)
        ));
    }

    #[test]
    fn render_clips_long_cells_with_ellipsis() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut v = TableView::new(
            ds(),
            vec![row("r1", "a task with a very long name that cannot fit", false, "High")],
        );
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(46, 8); // narrow: title column gets squeezed
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut v, true, &theme)).unwrap();
        let content: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(content.contains('…'), "long cell should be ellipsized:\n{content}");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui table`
Expected: FAIL to compile — `column_constraint`/`truncate_ellipsis` missing.

- [ ] **Step 3: Implement**

In `table.rs` (above `render`):

```rust
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Column width by property type (spec M8.8: checkbox narrow, URL wide).
fn column_constraint(col: &Column, is_title: bool) -> Constraint {
    if is_title {
        return Constraint::Min(20);
    }
    match col.prop_type.as_str() {
        "checkbox" => Constraint::Length(6),
        "number" => Constraint::Length(10),
        "date" => Constraint::Length(12),
        "url" | "email" => Constraint::Length(28),
        _ => Constraint::Length(14),
    }
}

/// Truncates to at most `max_cols` display columns, appending '…' when
/// anything was cut. Grapheme-safe: never slices inside an emoji/CJK char.
pub fn truncate_ellipsis(s: &str, max_cols: u16) -> String {
    if (s.width() as u16) <= max_cols {
        return s.to_string();
    }
    let budget = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut used: u16 = 0;
    for g in s.graphemes(true) {
        let gw = g.width() as u16;
        if used + gw > budget {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out.push('…');
    out
}
```

In `render`, replace the widths block (:224-235) and the row-building block (:216-223):

```rust
    let widths: Vec<Constraint> = view
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| column_constraint(c, i == 0))
        .collect();
    // Concrete per-column budgets for ellipsis (ratatui clips silently otherwise):
    // fixed columns take their Length; the title column gets the leftover.
    let fixed_sum: u16 = widths
        .iter()
        .map(|w| if let Constraint::Length(n) = w { *n } else { 0 })
        .sum();
    let spacing = view.columns.len().saturating_sub(1) as u16; // Table's default column_spacing = 1
    let title_budget = area
        .width
        .saturating_sub(2) // borders
        .saturating_sub(spacing)
        .saturating_sub(fixed_sum)
        .max(20);
    let budgets: Vec<u16> = widths
        .iter()
        .map(|w| if let Constraint::Length(n) = w { *n } else { title_budget })
        .collect();
    let rows: Vec<TRow> = view
        .rows
        .iter()
        .map(|r| {
            let cells: Vec<String> = view
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| truncate_ellipsis(&view.cell(r, c), budgets[i]))
                .collect();
            TRow::new(cells)
        })
        .collect();
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS. Existing table tests use short cells (untouched by truncation). If an insta snapshot in `tests/snapshots/` shifts because a column narrowed (e.g. a checkbox column), inspect the diff: a *narrower checkbox / wider url* change is this task's intended behavior — accept it with `cargo insta review` and say so in the report; anything else is a bug.

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/ui/table.rs notion-tui/crates/notion-tui/tests/snapshots && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): type-aware table column widths with ellipsis"`

---

### Task 8: page soft-wrap + code-block language label

**Files:**
- Modify: `crates/notion-tui/src/ui/page.rs:114-126` (code branch of `lines()`), `:185-218` (`render`)
- Test: inline in `page.rs` (including one intentional update to `code_block_renders_per_line_with_syntax_styling`)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces:
  - `PageView::lines()` emits an extra label line (`text: "╭─ {language}"`, `spans: None`) before a code block's lines when the block has a non-empty language.
  - `render` soft-wraps non-code lines to the pane width via a private `fn wrap_words(text: &str, width: usize) -> Vec<String>`; code lines (`spans: Some`) still render single-line.

- [ ] **Step 1: Write the failing tests**

Append to `page.rs`'s tests module:

```rust
    #[test]
    fn code_block_gets_language_label_line() {
        let v = PageView::new(
            page(),
            vec![rec("c", None, 0, "code", "let x = 1;", r#"{"language": "rust"}"#)],
        );
        let lines = v.lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].text, "╭─ rust");
        assert!(lines[0].spans.is_none());
        assert!(lines[1].text.contains("let x = 1;"));
    }

    #[test]
    fn code_block_without_language_has_no_label() {
        let v = PageView::new(page(), vec![rec("c", None, 0, "code", "plain", "{}")]);
        assert_eq!(v.lines().len(), 1);
        assert!(v.lines()[0].text.contains("plain"));
    }

    #[test]
    fn wrap_words_splits_at_width() {
        assert_eq!(wrap_words("aaa bbb ccc", 7), vec!["aaa bbb", "ccc"]);
        assert_eq!(wrap_words("short", 10), vec!["short"]);
        assert_eq!(wrap_words("", 10), vec![""]);
    }

    #[test]
    fn render_soft_wraps_long_paragraphs() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let long = "alpha bravo charlie delta echo foxtrot golf hotel india juliet";
        let mut v = PageView::new(page(), vec![rec("b", None, 0, "paragraph", long, "{}")]);
        v.cursor = 0;
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(24, 12);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut v, true, &theme)).unwrap();
        let content: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        // At 24 cols the tail words only appear if the line wrapped instead of clipping.
        assert!(content.contains("juliet"), "tail of long line should wrap into view:\n{content}");
    }
```

Also update the existing `code_block_renders_per_line_with_syntax_styling` test (its fixture has `language: rust`, so it now produces 3 lines — intentional behavior change):

```rust
        let lines = v.lines();
        assert_eq!(lines.len(), 3); // label + 2 code lines
        assert_eq!(lines[0].text, "╭─ rust");
        assert!(lines[1].text.contains("let x = 1;"));
        let spans = lines[1].spans.as_ref().expect("code lines carry styled spans");
        assert!(spans.spans.iter().any(|s| s.style.fg.is_some()));
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui page`
Expected: FAIL — no label line, `wrap_words` missing, long line clipped.

- [ ] **Step 3: Implement**

In `lines()`'s code branch (:114-126), insert the label push before the per-line loop:

```rust
                _ if b.block_type == "code" => {
                    let language = payload["language"].as_str().unwrap_or("").to_string();
                    if !language.is_empty() {
                        out.push(BlockLine {
                            block_id: b.id.clone(),
                            text: format!("╭─ {language}"),
                            indent,
                            link_page_id: None,
                            spans: None,
                        });
                    }
                    for src_line in b.plain_text.lines() {
                        // ... existing per-line push, unchanged ...
```

Add the wrap helper (free function above `render`):

```rust
/// Greedy word-wrap: splits on spaces, never breaks a word, always returns at
/// least one (possibly empty) segment. Width is in chars — close enough for
/// prose; code lines bypass wrapping entirely.
fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let cur = lines.last_mut().unwrap();
        if cur.is_empty() {
            *cur = word.to_string();
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(word.to_string());
        }
    }
    lines
}
```

In `render` (:186-200), the `None` spans arm becomes a multi-line item:

```rust
    let wrap_width = area.width.saturating_sub(2) as usize; // borders
    let items: Vec<ListItem> = view
        .lines()
        .iter()
        .map(|l| {
            match &l.spans {
                Some(styled) => {
                    let mut spans = vec![Span::raw(format!("{}│ ", "  ".repeat(l.indent)))];
                    spans.extend(styled.spans.iter().cloned());
                    ListItem::new(Line::from(spans))
                }
                None => {
                    let indent = "  ".repeat(l.indent);
                    let avail = wrap_width.saturating_sub(indent.chars().count()).max(10);
                    let text = ratatui::text::Text::from(
                        wrap_words(&l.text, avail)
                            .into_iter()
                            .map(|seg| Line::from(format!("{indent}{seg}")))
                            .collect::<Vec<Line>>(),
                    );
                    ListItem::new(text)
                }
            }
        })
        .collect();
```

(Cursor semantics unchanged: one `BlockLine` = one list item; a wrapped item highlights and scrolls as a unit.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS. `render_smoke`'s page snapshot uses short lines and no code blocks — must be byte-identical. Editor/markdown roundtrip tests are unaffected (`markdown/render.rs` is a different renderer and untouched).

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/ui/page.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): soft-wrap page lines and label code-block language"`

---

### Task 9: empty states for every view

**Files:**
- Modify: `crates/notion-tui/src/ui/sidebar.rs` (`render`), `page.rs` (`render`), `table.rs` (`render`), `search.rs` (`render`), `comments.rs` (`render`), `queue.rs` (`render`)
- Test: `crates/notion-tui/tests/empty_states.rs` (new — spec §7's "render smoke tests extended to empty states")

**Interfaces:**
- Consumes: Tasks 3/4/7/8 (same render fns — Wave 2). All six render fns already take `theme` after Task 3.
- Produces: when a view's list is empty, a single dim hint line renders inside the same themed block. Exact copy (Global Constraints): sidebar `first sync in progress…`, page `empty page — press a to add a block`, table `no rows — press o to add one`, search `no results` (only when the query is non-empty), comments `no comments yet — press n`, queue `queue is empty — local edits appear here until synced`.
- **M7 coordination:** if M7's table filter has landed, the table's emptiness check uses the filtered display list (`view.visible().is_empty()`), and an empty *filtered* result should hint `no rows match the filter` instead — implement that variant only if `FilterState` exists in your tree.

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-tui/tests/empty_states.rs`:

```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_tui::app::{App, Focus, View};
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

fn draw(app: &mut App) -> String {
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(70, 16);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    term.backend().buffer().content.iter().map(|c| c.symbol()).collect()
}

#[test]
fn empty_sidebar_hints_first_sync() {
    let mut app = App::new(test_store());
    assert!(draw(&mut app).contains("first sync in progress…"));
}

#[test]
fn empty_page_hints_add_block() {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.view = View::Page(notion_tui::ui::page::PageView::new(
        notion_store::PageRec {
            id: "p1".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "Empty".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        },
        vec![],
    ));
    assert!(draw(&mut app).contains("empty page — press a to add a block"));
}

#[test]
fn empty_table_hints_new_row() {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.view = View::Table(notion_tui::ui::table::TableView::new(
        notion_store::DataSourceRec {
            id: "ds".into(),
            database_id: "db".into(),
            title: "Tasks".into(),
            schema_json: r#"{"Name": {"type": "title"}}"#.into(),
            last_edited_time: "t".into(),
        },
        vec![],
    ));
    assert!(draw(&mut app).contains("no rows — press o to add one"));
}

#[test]
fn search_with_query_and_no_hits_says_no_results() {
    let mut app = App::new(test_store());
    let mut search = notion_tui::ui::search::SearchState::new();
    search.on_key(KeyEvent::from(KeyCode::Char('z'))); // non-empty query, no hits
    app.search = Some(search);
    assert!(draw(&mut app).contains("no results"));
}

#[test]
fn search_with_empty_query_stays_quiet() {
    let mut app = App::new(test_store());
    app.search = Some(notion_tui::ui::search::SearchState::new());
    assert!(!draw(&mut app).contains("no results"));
}

#[test]
fn empty_comments_hint_pressing_n() {
    let mut app = App::new(test_store());
    app.comments = Some(notion_tui::ui::comments::CommentsState {
        parent_id: "p1".into(),
        parent_kind: "page".into(),
        items: vec![],
        cursor: 0,
        list_state: ratatui::widgets::ListState::default(),
    });
    assert!(draw(&mut app).contains("no comments yet — press n"));
}

#[test]
fn empty_queue_explains_itself() {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.view = View::Queue(notion_tui::ui::queue::QueueView::new(vec![], vec![]));
    assert!(draw(&mut app).contains("queue is empty"));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test empty_states`
Expected: FAIL — all views render blank interiors today.

- [ ] **Step 3: Implement**

Same pattern in each render fn — build the block first, early-return a dim `Paragraph` when the list is empty. Reference implementation for `sidebar.rs` (restructure `render` so the block is a local):

```rust
pub fn render(f: &mut Frame, area: Rect, state: &mut SidebarState, focused: bool, theme: &Theme) {
    let title = if focused { " notion ● " } else { " notion " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Span::styled(title, theme.title));
    let visible = state.visible();
    if visible.is_empty() {
        f.render_widget(
            ratatui::widgets::Paragraph::new("first sync in progress…")
                .style(ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM))
                .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = visible
        .iter()
        .map(|v| { /* existing item building, unchanged */ })
        .collect();
    // ... existing render_stateful_widget with `block`, unchanged ...
}
```

Apply the identical early-return shape with each view's copy string:
- `page.rs`: `if view.lines().is_empty()` → `"empty page — press a to add a block"` (block already a local via Task 3? No — page keeps its own block; hoist it the same way).
- `table.rs`: `if view.rows.is_empty()` (or `view.visible().is_empty()` post-M7) → `"no rows — press o to add one"`.
- `queue.rs`: `if view.ops.is_empty()` → `"queue is empty — local edits appear here until synced"`.
- `comments.rs`: `if state.items.is_empty()` → `"no comments yet — press n"`.
- `search.rs`: this one is inside the popup — when `!state.input.text().is_empty() && state.results.is_empty()`, render the results area (`inner[1]`) as the dim Paragraph instead of the List (keep the input row + cursor rendering untouched):

```rust
    if !state.input.text().is_empty() && state.results.is_empty() {
        f.render_widget(
            Paragraph::new("no results")
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(crate::ui::theme::popup_block(String::new(), theme)),
            inner[1],
        );
    } else {
        // existing List rendering
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS. `render_smoke::renders_frame_with_status_bar` draws an empty sidebar — its insta snapshot WILL change (it now shows the hint). Review with `cargo insta review`, accept only that hint-line diff, and state so in the report.

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/ui/sidebar.rs notion-tui/crates/notion-tui/src/ui/page.rs notion-tui/crates/notion-tui/src/ui/table.rs notion-tui/crates/notion-tui/src/ui/search.rs notion-tui/crates/notion-tui/src/ui/comments.rs notion-tui/crates/notion-tui/src/ui/queue.rs notion-tui/crates/notion-tui/tests/empty_states.rs notion-tui/crates/notion-tui/tests/snapshots && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): one-line empty-state hints in every view"`

---

### Task 10: consistent motion keys — Ctrl+d/u and g/G in Table and Board

**Files:**
- Modify: `crates/notion-tui/src/app.rs:877-899` (Table arm of `handle_key`), `:900-932` (Board arm)
- Test: `crates/notion-tui/tests/motion_flow.rs` (new)

**Interfaces:**
- Consumes: existing `TableView::move_cursor`, `BoardView::move_cursor_card`/`cards_in` (verified in ui/table.rs:183-188, ui/board.rs:78-92).
- Produces: `Ctrl+d`/`Ctrl+u` scroll ±10 in Table and Board (matching the Page arm at app.rs:855-858); `g`/`G` (the `top`/`bottom` keymap actions) jump in Board (Table already has them at :882-885).
- **M7 coordination:** if M7's `apply_action`/mouse refactor has landed and moved these arms, add the same branches wherever the Table/Board key handling now lives — the behavior contract is the test file, not the line numbers.

- [ ] **Step 1: Write the failing tests**

Create `crates/notion-tui/tests/motion_flow.rs`:

```rust
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_tui::app::{dispatch_key, App, Focus, View};
use notion_tui::ui::board::BoardView;
use notion_tui::ui::table::TableView;
use serde_json::json;

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn ds(schema: serde_json::Value) -> notion_store::DataSourceRec {
    notion_store::DataSourceRec {
        id: "ds".into(),
        database_id: "db".into(),
        title: "Tasks".into(),
        schema_json: schema.to_string(),
        last_edited_time: "t".into(),
    }
}

fn row(i: usize, status: &str) -> notion_store::RowRec {
    notion_store::RowRec {
        id: format!("r{i}"),
        data_source_id: "ds".into(),
        properties: json!({
            "Name": {"type": "title", "title": [{"plain_text": format!("task {i}")}]},
            "Status": {"type": "status", "status": {"name": status}}
        })
        .to_string(),
        last_edited_time: "t".into(),
        archived: false,
    }
}

fn table_app(n: usize) -> App {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    let rows = (0..n).map(|i| row(i, "Todo")).collect();
    app.view = View::Table(TableView::new(ds(json!({"Name": {"type": "title"}})), rows));
    app
}

fn board_app(n: usize) -> App {
    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    let schema = json!({
        "Name": {"type": "title"},
        "Status": {"type": "status", "status": {"options": [{"name": "Todo"}, {"name": "Done"}]}}
    });
    let rows = (0..n).map(|i| row(i, "Todo")).collect();
    app.view = View::Board(BoardView::new(ds(schema), rows));
    app
}

#[test]
fn ctrl_d_u_page_through_table() {
    let mut app = table_app(30);
    dispatch_key(&mut app, ctrl('d'));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 10);
    dispatch_key(&mut app, ctrl('u'));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 0);
}

#[test]
fn ctrl_d_u_page_through_board_column() {
    let mut app = board_app(30);
    dispatch_key(&mut app, ctrl('d'));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 10);
    dispatch_key(&mut app, ctrl('u'));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 0);
}

#[test]
fn g_and_shift_g_jump_in_board() {
    let mut app = board_app(25);
    dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 24);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('g')));
    let View::Board(v) = &app.view else { panic!() };
    assert_eq!(v.card, 0);
}

#[test]
fn g_and_shift_g_still_jump_in_table() {
    let mut app = table_app(25); // regression pin — table already had top/bottom
    dispatch_key(&mut app, KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 24);
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('g')));
    let View::Table(v) = &app.view else { panic!() };
    assert_eq!(v.cursor, 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test motion_flow`
Expected: `ctrl_d_u_page_through_table`, `ctrl_d_u_page_through_board_column`, `g_and_shift_g_jump_in_board` FAIL (cursor stays 0); the table g/G regression pin passes.

- [ ] **Step 3: Implement**

Table arm (`app.rs:877-899`) — after the `bottom` branch, mirroring the Page arm exactly:

```rust
            } else if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor(10);
            } else if key.code == KeyCode::Char('u') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor(-10);
```

Board arm (`app.rs:900-932`) — after the `up` branch:

```rust
            } else if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor_card(10);
            } else if key.code == KeyCode::Char('u') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor_card(-10);
            } else if km.is("top", key) {
                view.card = 0;
            } else if km.is("bottom", key) {
                view.card = view.cards_in(view.col).len().saturating_sub(1);
```

(`move_card_next`/`move_card_prev` are `J`/`K` — no clash with `G`; the board's `up`/`down` branches already guard `modifiers == NONE` so Ctrl chords fall through correctly.)

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS, including `board_flow.rs` (J/K card moves untouched).

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/app.rs notion-tui/crates/notion-tui/tests/motion_flow.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): Ctrl+d/u and g/G motion in table and board views"`
(app.rs unrelated-hunks caveat applies.)

---

### Task 11: `dd` gets the confirm treatment

**Files:**
- Modify: `crates/notion-tui/src/ui/confirm.rs:12-19` (`ConfirmKind`), `crates/notion-tui/src/app.rs` (the `Action::DeleteBlock`/`Action::DeleteRow` dispatch arms at :1234-1235 and the confirm-resolution match at :981-986, plus one new private method)
- Test: update `crates/notion-tui/tests/edit_flow.rs:63-90` and `crates/notion-tui/tests/table_edit_flow.rs:79-81` (intentional behavior change), add new cases in `edit_flow.rs`

**Interfaces:**
- Consumes: Task 10 merged (same file, app.rs — sequential in Wave 3). M6's `ConfirmState { message, ids, kind }` and the `y`/`n` resolution plumbing.
- Produces:
  - `ConfirmKind` gains `DeleteBlock` and `DeleteRow` variants (existing `DeleteProtected`/`ApplyDespiteWarnings` untouched).
  - `dd` opens a confirm instead of deleting immediately; `y` deletes and sets `app.notice = "deleted — press u to undo (this session)"`; `n`/Esc cancels with no store change.
- **M7 coordination:** the `Action` enum itself is untouched (M7 constraint) — only the *handling* of `DeleteBlock`/`DeleteRow` changes. If M7's `apply_action` refactor has landed, make the same change inside `apply_action`.

- [ ] **Step 1: Update/write the failing tests**

Rewrite `edit_flow.rs::dd_deletes_block_under_cursor` (behavior intentionally changes — say so in the report):

```rust
#[test]
fn dd_asks_for_confirmation_then_y_deletes() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    assert!(app.confirm.is_some(), "dd must confirm before deleting");
    assert_eq!(store.lock().unwrap().page_blocks("p1").unwrap().len(), 1, "not deleted yet");

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));
    let s = store.lock().unwrap();
    assert!(s.page_blocks("p1").unwrap().is_empty());
    assert_eq!(s.ops().unwrap()[0].op_type, "delete_block");
    drop(s);
    assert!(app.notice.as_deref().unwrap_or("").contains("undo"));
}

#[test]
fn dd_then_n_keeps_the_block() {
    let store = store_with_todo();
    let mut app = app_on_todo_page(store.clone());

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('d')));
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('n')));

    assert!(app.confirm.is_none());
    assert_eq!(store.lock().unwrap().page_blocks("p1").unwrap().len(), 1);
    assert!(store.lock().unwrap().ops().unwrap().is_empty());
}
```

(`single_d_does_not_delete` stays as-is — arming behavior unchanged.)
In `table_edit_flow.rs`, after the second `Char('d')` (:81), insert `dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('y')));` and keep the existing assertions.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui --test edit_flow --test table_edit_flow`
Expected: `dd_asks_for_confirmation_then_y_deletes` FAILs (no confirm — block already deleted); `dd_then_n_keeps_the_block` FAILs (block gone).

- [ ] **Step 3: Implement**

`confirm.rs` — extend the enum:

```rust
pub enum ConfirmKind {
    /// Delete protected blocks whose marker lines were removed in the editor.
    DeleteProtected,
    /// Apply an edited document whose parse produced warnings (e.g. an
    /// unclosed code fence swallowing the rest of the page).
    ApplyDespiteWarnings,
    /// `dd` on a block in the page view.
    DeleteBlock,
    /// `dd` on a row in table/board views.
    DeleteRow,
}
```

`app.rs` — replace the two dispatch arms (:1234-1235):

```rust
        Action::DeleteBlock(id) => {
            app.confirm = Some(crate::ui::confirm::ConfirmState {
                message: "delete this block?".into(),
                ids: vec![id],
                kind: crate::ui::confirm::ConfirmKind::DeleteBlock,
            });
        }
        Action::DeleteRow(id) => {
            app.confirm = Some(crate::ui::confirm::ConfirmState {
                message: "delete this row?".into(),
                ids: vec![id],
                kind: crate::ui::confirm::ConfirmKind::DeleteRow,
            });
        }
```

Extend the confirm-resolution match (:981-986):

```rust
            crate::ui::confirm::ConfirmKind::DeleteBlock | crate::ui::confirm::ConfirmKind::DeleteRow => {
                app.confirm_delete_target(confirmed)
            }
```

New private method on `App` (next to `confirm_delete_protected`):

```rust
    /// Resolves a `dd` confirm: `y` deletes (block or row per the confirm's
    /// kind) and points at the session-only undo; `n` drops it untouched.
    fn confirm_delete_target(&mut self, confirmed: bool) {
        let Some(state) = self.confirm.take() else { return };
        if !confirmed {
            return;
        }
        if let Some(id) = state.ids.first() {
            match state.kind {
                crate::ui::confirm::ConfirmKind::DeleteBlock => self.delete_block(id),
                crate::ui::confirm::ConfirmKind::DeleteRow => self.delete_row(id),
                _ => {}
            }
        }
        self.notice = Some("deleted — press u to undo (this session)".into());
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace`
Expected: PASS. Sweep for other tests that press `dd` (`grep -rn "Char('d')" crates/notion-tui/tests/` — only edit_flow.rs and table_edit_flow.rs at baseline) and add the `y` keystroke where needed.

- [ ] **Step 5: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/ui/confirm.rs notion-tui/crates/notion-tui/src/app.rs notion-tui/crates/notion-tui/tests/edit_flow.rs notion-tui/crates/notion-tui/tests/table_edit_flow.rs && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): dd confirms before deleting, consistent with protected blocks"`

---

### Task 12: help completeness — fixed keys listed, `ctrl+x` bindings, spec amendment

**Files:**
- Modify: `crates/notion-tui/src/keymap.rs` (chord-typed map, `parse_key`, `is`, `key_for`, `FIXED_CHORDS`/`FIXED_KEYS_HELP`, collision warnings), `crates/notion-tui/src/ui/help.rs` (`key_label`, fixed-keys section, popup height), `docs/superpowers/specs/2026-07-05-notion-tui-design.md:82` (amend "fully rebindable")
- Test: inline in `keymap.rs`, extend `crates/notion-tui/tests/help_palette_flow.rs`

**Interfaces:**
- Consumes: Task 6's `with_overrides_checked` (extended here), Task 3's `help::render(f, keymap, theme)` signature.
- Produces:
```rust
// keymap.rs
pub type Chord = (KeyCode, KeyModifiers);                    // modifiers ∈ {NONE, CONTROL}
fn parse_key(s: &str) -> Option<Chord>;                      // now also "ctrl+x"
pub const FIXED_CHORDS: &[Chord];                            // Tab, Ctrl+d, Ctrl+u, Ctrl+p — override collisions warn + skip
pub const FIXED_KEYS_HELP: &[(&str, &str)];                  // rows for the help overlay (incl. backspace/esc docs)
impl Keymap { pub fn key_for(&self, action: &str) -> Option<Chord>; /* is() honors the chord's CONTROL bit */ }
```
  Undo's help description becomes `"undo last edit (session-only)"` in BOTH `DEFAULTS` and `Keymap::actions()` (M8.9's documentation half).

- [ ] **Step 1: Write the failing tests**

Append to `keymap.rs`'s tests module:

```rust
    #[test]
    fn ctrl_syntax_binds_a_control_chord() {
        let mut o = HashMap::new();
        o.insert("undo".to_string(), "ctrl+z".to_string());
        let (km, warnings) = Keymap::with_overrides_checked(&o);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(km.is("undo", KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
        assert!(!km.is("undo", KeyEvent::from(KeyCode::Char('z')))); // plain z is not the chord
        assert!(!km.is("undo", KeyEvent::from(KeyCode::Char('u')))); // old default replaced
    }

    #[test]
    fn fixed_chords_cannot_be_claimed() {
        for reserved in ["ctrl+d", "ctrl+u", "ctrl+p", "tab"] {
            let mut o = HashMap::new();
            o.insert("quit".to_string(), reserved.to_string());
            let (km, warnings) = Keymap::with_overrides_checked(&o);
            assert_eq!(warnings.len(), 1, "{reserved} must warn");
            assert!(warnings[0].contains("fixed"), "{}", warnings[0]);
            assert!(km.is("quit", KeyEvent::from(KeyCode::Char('q'))), "default must survive");
        }
    }

    #[test]
    fn plain_bindings_still_reject_ctrl_chords() {
        let km = Keymap::new();
        assert!(!km.is("down", KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)));
    }
```

Append to `tests/help_palette_flow.rs`:

```rust
#[test]
fn help_overlay_lists_fixed_keys_and_session_only_undo() {
    use notion_tui::ui;
    use ratatui::{backend::TestBackend, Terminal};

    let store = std::sync::Arc::new(std::sync::Mutex::new(
        notion_store::Store::open_in_memory().unwrap(),
    ));
    let mut app = notion_tui::app::App::new(store);
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    app.help_open = true;
    let backend = TestBackend::new(80, 50);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let content: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
    for needle in ["fixed keys", "ctrl+d", "ctrl+p", "backspace", "session-only"] {
        assert!(content.contains(needle), "help missing {needle:?}:\n{content}");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p notion-tui keymap && cargo test -p notion-tui --test help_palette_flow`
Expected: FAIL — `parse_key` rejects `ctrl+z`; help shows neither fixed keys nor session-only.

- [ ] **Step 3: Implement the chord keymap**

`keymap.rs`:

```rust
pub type Chord = (KeyCode, KeyModifiers);

/// Chords the app hard-codes (spec §5 descope: fixed keys stay fixed).
/// `with_overrides_checked` refuses to bind actions onto these.
pub const FIXED_CHORDS: &[Chord] = &[
    (KeyCode::Tab, KeyModifiers::NONE),
    (KeyCode::Char('d'), KeyModifiers::CONTROL),
    (KeyCode::Char('u'), KeyModifiers::CONTROL),
    (KeyCode::Char('p'), KeyModifiers::CONTROL),
];

/// Rows for the help overlay's fixed-keys section (documentation superset of
/// FIXED_CHORDS: backspace/esc are hard-coded in handlers, not collision-checked).
pub const FIXED_KEYS_HELP: &[(&str, &str)] = &[
    ("tab", "switch sidebar/main focus (fixed)"),
    ("ctrl+d", "half-page down (fixed)"),
    ("ctrl+u", "half-page up (fixed)"),
    ("ctrl+p", "search (fixed)"),
    ("backspace", "back to previous page (fixed alias of 'back')"),
    ("esc", "close modal / cancel (fixed)"),
];

fn parse_key(s: &str) -> Option<Chord> {
    if let Some(rest) = s.strip_prefix("ctrl+") {
        let mut chars = rest.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => Some((KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL)),
            _ => None,
        };
    }
    let code = match s {
        "esc" => KeyCode::Esc,
        "enter" => KeyCode::Enter,
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    };
    Some((code, KeyModifiers::NONE))
}
```

- `map` becomes `HashMap<String, Chord>`; `Keymap::new()` maps DEFAULTS to `(*k, KeyModifiers::NONE)`.
- `is()`:

```rust
    /// Matches code + the binding's CONTROL bit. ALT always rejects; SHIFT
    /// rides along because uppercase Char events carry it.
    pub fn is(&self, action: &str, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        let Some((code, mods)) = self.map.get(action) else {
            return false;
        };
        key.modifiers.contains(KeyModifiers::CONTROL) == mods.contains(KeyModifiers::CONTROL)
            && *code == key.code
    }
```

- `key_for` returns `Option<Chord>` (`self.map.get(action).copied()`).
- In `with_overrides_checked`'s `Some(chord)` arm, before inserting:

```rust
                Some(chord) => {
                    if FIXED_CHORDS.contains(&chord) {
                        warnings.push(format!(
                            "keys.{action}: \"{key_str}\" is a fixed key and can't be rebound"
                        ));
                        continue;
                    }
                    km.map.insert(action.clone(), chord);
                }
```

- `DEFAULTS` and `actions()`: change the undo description to `"undo last edit (session-only)"` in both lists.

`ui/help.rs`:

```rust
fn key_label((code, mods): (KeyCode, crossterm::event::KeyModifiers)) -> String {
    let base = match code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        other => format!("{other:?}"),
    };
    if mods.contains(crossterm::event::KeyModifiers::CONTROL) {
        format!("ctrl+{base}")
    } else {
        base
    }
}
```

(`board_hints` compiles unchanged — `key_for` now yields a chord and `key_label` takes one; its existing exact-string test still passes because board keys are plain.)

In `render`, bump the height clamp so the longer list fits — `(area.height * 3 / 4).clamp(10, 40)` — and append the fixed-keys section after the action items:

```rust
    let mut items: Vec<ListItem> = Keymap::actions()
        .iter()
        .map(|(action, desc)| {
            let key = keymap.key_for(action).map(key_label).unwrap_or_default();
            ListItem::new(format!("{key:>10}  {desc}"))
        })
        .collect();
    items.push(ListItem::new(""));
    items.push(ListItem::new("fixed keys (not rebindable):"));
    for (key, desc) in crate::keymap::FIXED_KEYS_HELP {
        items.push(ListItem::new(format!("{key:>10}  {desc}")));
    }
```

- [ ] **Step 4: Amend the design spec (M8.10's descope paperwork)**

In `docs/superpowers/specs/2026-07-05-notion-tui-design.md:82`, replace:

```
Vim-flavored, fully rebindable via config file.
```

with:

```
Vim-flavored, rebindable via config file (plain keys, named keys, and `ctrl+x`
chords) — except a small fixed set: Tab (focus switch), Ctrl+d/Ctrl+u
(half-page scroll), Ctrl+P (search), and Backspace-as-back. (Amended per the
2026-07-12 production-polish design, M8.10.)
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p notion-tui && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS, including `keymap_flow.rs` (plain overrides unchanged) and `ctrl_chords_never_match` — that test asserts default `undo` doesn't fire on Ctrl+u, which still holds because the default binding's CONTROL bit is NONE.

- [ ] **Step 6: Commit**

Run: `git -C /Users/rehatbir/Developer/fables add notion-tui/crates/notion-tui/src/keymap.rs notion-tui/crates/notion-tui/src/ui/help.rs notion-tui/crates/notion-tui/tests/help_palette_flow.rs notion-tui/docs/superpowers/specs/2026-07-05-notion-tui-design.md && git -C /Users/rehatbir/Developer/fables commit -m "feat(notion-tui): help lists fixed keys; keymap accepts ctrl+x chords"`

---

### Task 13: integration sweep

**Files:** none new.

- [ ] **Step 1: Full gates**

Run (from `notion-tui/`): `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green.

- [ ] **Step 2: Cross-task interaction checks**

- `cargo test -p notion-tui --test text_editing_flow --test themed_modals --test empty_states --test motion_flow` — the four new suites compose.
- `cargo test -p notion-tui --test edit_flow --test table_edit_flow --test queue_flow` — dd-confirm + humanized queue coexist.
- Open the help overlay path mentally against Task 3+12: `help::render(f, keymap, theme)` — one signature, both features present.
- If M7 landed mid-flight: re-run its `palette_command_flow`/`table_filter_flow` suites and complete every "M7 coordination" checklist item left as a follow-up (Task 2 Step 7, Task 3 Step 5, Task 7, Task 9, Task 10, Task 11 notes).
- `git -C /Users/rehatbir/Developer/fables status` — confirm only intended files changed; report the full list.

- [ ] **Step 3: Manual smoke checklist (report, don't fix)**

If a configured token/db exists locally, run `cargo run -p notion-tui` briefly and verify: cursor visible and movable (Left/Home) in `/` search and `i` edit; dark theme (`theme = "dark"`) colors every modal; `Q` queue shows titles, not UUIDs; an empty workspace shows "first sync in progress…"; `dd` asks y/n; `?` lists fixed keys. Otherwise skip and note it.

---

## Spec coverage

| M8 item | Tasks |
|---|---|
| 1. Real text editing (Left/Right/Home/End/Delete, visible cursor, grapheme-safe) | 1, 2 |
| 2. Theme the modals | 3 |
| 3. Human-readable queue (`describe_op` — M10 contract) | 4 |
| 4. Empty states | 9 |
| 5. Consistent motion keys (Ctrl+d/u, g/G in Table/Board) | 10 |
| 6. Wizard honesty (401 vs network) | 5 |
| 7. Friendly config errors (file/line, theme + key warnings) | 6 |
| 8. Layout niceties (column widths + ellipsis; soft-wrap; code language label) | 7, 8 |
| 9. Destructive-action consistency (`dd` confirm; session-only undo documented) | 11 (confirm), 12 (help doc) |
| 10. Help completeness (fixed keys listed; spec amended; `ctrl+x` parse_key) | 12 |
| Verification bar (§7): empty-state + themed-modal render smoke, grapheme property tests | 9, 3, 1 |

## Design decisions and resolved ambiguities

- **M8.9 chose the confirm, not the undo toast** — the spec offers either; the controller pinned extending M6's `ConfirmKind` pattern, and the confirmed path *also* gets an undo-pointing notice.
- **`describe_op` lives in notion-tui (not notion-store)** because summaries reuse `ui::table::cell_text` for title extraction; the store only gains the missing `get_row` primitive.
- **Search shows "no results" only for a non-empty query** — an untyped search box staying quiet reads better than a premature "no results"; the spec string is used verbatim once typing starts.
- **`TextLine` derefs to `str`** to keep M7's concurrent palette/search reads compiling with at most a one-line change on whichever side lands second.
- **Fixed-chord collision checking covers exactly Tab/Ctrl+d/Ctrl+u/Ctrl+p** (the spec §5 list); Backspace-as-back and Esc are documented as fixed in help but stay bindable targets since modal handlers consume them before the keymap.
- **Soft-wrap is per-list-item** (a wrapped block highlights/scrolls as one unit) — cursor arithmetic (`lines()` indices) is untouched, which keeps every existing page-view flow test valid.
- **Startup config warnings go to `app.notice`** (cleared on first keypress) — a durable log home arrives with M9's tracing; duplicating that infrastructure now would be YAGNI.
- **Insta snapshots:** exactly two intentional snapshot churn points — Task 9 (empty sidebar hint appears in `renders_frame_with_status_bar`) and possibly Task 7 (column widths). Executors must review, not blanket-accept.

## Assumptions about other milestones

- M7's plan (same directory, 2026-07-13) is executing concurrently; this plan never touches `Action` variants, `PickerPurpose`, `SyncStatus`, or M7's `TableView.filter`/board group-by fields, and names M7's three new input surfaces (generic `Picker` popup, table-filter input, rename input) in Tasks 2/3 integration checklists. M7's rename/filter inputs inherit TextLine + theming for free via `InputState`.
- M10 will consume `describe_op` exactly as pinned in Task 4's Interfaces block and may extend `OpDescription` with timing fields — the struct is non-exhaustive by convention (M10 adds fields, doesn't rename these three).
