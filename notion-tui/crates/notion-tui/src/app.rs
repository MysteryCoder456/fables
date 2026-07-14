use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
pub use notion_store::NodeKind;
use notion_store::TreeNode;
use notion_sync::{SharedStore, SyncStatus};

use crate::ui::board::BoardView;
use crate::ui::input::{InputAction, InputState};
use crate::ui::page::PageView;
use crate::ui::props::{build_fields, build_property_value, PropsAction, PropsState};
use crate::ui::search::{SearchAction, SearchState};
use crate::ui::sidebar::SidebarState;
use crate::ui::table::TableView;

pub enum Focus {
    Sidebar,
    Main,
}

pub enum View {
    Empty,
    Page(PageView),
    Table(TableView),
    Board(BoardView),
    Queue(crate::ui::queue::QueueView),
}

pub enum Action {
    None,
    OpenNode(TreeNode),
    OpenPage(String),
    /// Returned only by the back/backspace key: `open_page` without pushing
    /// history (the entry was already popped by the caller).
    Back(String),
    ToggleTodo(String),
    DeleteBlock(String),
    DeleteRow(String),
    MoveCard {
        row_id: String,
        prop_name: String,
        prop_type: String,
        value: String,
    },
    Undo,
}

pub enum AppMsg {
    Refreshed,
    MergeReady {
        op_seq: i64,
        block_id: String,
        local_text: String,
        remote_text: String,
        remote_edited_time: String,
    },
    PageGone(String),
}

pub struct RemoteHandle {
    pub client: std::sync::Arc<notion_api::NotionClient>,
    pub tx: tokio::sync::mpsc::UnboundedSender<AppMsg>,
}

pub enum InputPurpose {
    EditBlockText {
        block_id: String,
    },
    InsertBlockAfter {
        page_id: String,
        after_block_id: Option<String>,
    },
    NewRow {
        data_source_id: String,
        title_prop_name: String,
    },
    NewComment {
        parent_id: String,
        parent_kind: String,
        thread_id: Option<String>,
    },
    RenamePage {
        page_id: String,
    },
    RenameRow {
        row_id: String,
        title_prop_name: String,
    },
    FilterTable {
        data_source_id: String,
        col: Option<usize>,
    },
}

pub enum PickerPurpose {
    MovePage { page_id: String },
    GroupBy { data_source_id: String },
}

pub struct App {
    pub focus: Focus,
    pub sync_status: SyncStatus,
    pub should_quit: bool,
    pub sidebar: SidebarState,
    pub view: View,
    pub history: Vec<String>,
    pub search: Option<SearchState>,
    pub input: Option<InputState>,
    pub input_purpose: Option<InputPurpose>,
    pub props: Option<PropsState>,
    pub pending: u32,
    pub pending_d: bool,
    pub undo_stack: Vec<notion_store::EditReceipt>,
    pub confirm: Option<crate::ui::confirm::ConfirmState>,
    pub comments: Option<crate::ui::comments::CommentsState>,
    pub queue_return: Option<Box<View>>,
    pub conflicted: u32,
    pub remote: Option<RemoteHandle>,
    pub editor_override: Option<String>,
    pub keymap: crate::keymap::Keymap,
    pub theme: crate::ui::theme::Theme,
    pub help_open: bool,
    pub palette: Option<crate::ui::palette::PaletteState>,
    pub picker: Option<crate::ui::picker::PickerState>,
    pub picker_purpose: Option<PickerPurpose>,
    /// Mirrors the config's mouse-capture setting so terminal handoffs
    /// (external `$EDITOR`) can restore mouse capture on re-entry.
    pub mouse: bool,
    /// Set when a subprocess has drawn over the screen; the event loop must
    /// clear the terminal before the next draw instead of diffing.
    pub force_redraw: bool,
    /// User-facing explanation for an action that silently found nothing to
    /// do (e.g. opening an unsynced record). Cleared on the next keypress.
    pub notice: Option<String>,
    pending_editor: Option<(String, Vec<crate::markdown::Unit>)>,
    /// An edited document held back because its parse produced warnings; the
    /// user must confirm (`ApplyDespiteWarnings`) before it is applied.
    pub pending_editor_text: Option<(String, Vec<crate::markdown::Unit>, String)>,
    pub sync_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    /// Where each interactive region was rendered last frame; used by
    /// `dispatch_mouse` for hit-testing.
    pub last_layout: crate::ui::LayoutRects,
    /// The board `(col, card)` a left-button-down started on, if a
    /// drag-in-progress is being tracked.
    pub drag_card: Option<(usize, usize)>,
    store: SharedStore,
}

impl App {
    pub fn new(store: SharedStore) -> App {
        App {
            focus: Focus::Sidebar,
            sync_status: SyncStatus::Starting,
            should_quit: false,
            sidebar: SidebarState::new(Vec::new()),
            view: View::Empty,
            history: Vec::new(),
            search: None,
            input: None,
            input_purpose: None,
            props: None,
            pending: 0,
            pending_d: false,
            undo_stack: Vec::new(),
            confirm: None,
            comments: None,
            queue_return: None,
            conflicted: 0,
            remote: None,
            editor_override: None,
            keymap: crate::keymap::Keymap::new(),
            theme: crate::ui::theme::named("default"),
            help_open: false,
            palette: None,
            picker: None,
            picker_purpose: None,
            mouse: false,
            force_redraw: false,
            notice: None,
            pending_editor: None,
            pending_editor_text: None,
            sync_notify: None,
            last_layout: crate::ui::LayoutRects::default(),
            drag_card: None,
            store,
        }
    }

    pub fn toggle_todo(&mut self, block_id: &str) {
        if let Ok(receipt) = self.store.lock().unwrap().edit_toggle_todo(block_id) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
    }

    pub fn delete_block(&mut self, block_id: &str) {
        if let Ok(receipt) = self.store.lock().unwrap().edit_delete_block(block_id) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
    }

    pub fn undo(&mut self) {
        if let Some(receipt) = self.undo_stack.pop() {
            self.store.lock().unwrap().undo(receipt).ok();
            self.refresh_current_view();
            self.refresh_sidebar();
        }
    }

    pub fn edit_block_text(&mut self, block_id: &str, text: &str) {
        if let Ok(receipt) = self.store.lock().unwrap().edit_update_block_text(block_id, text) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
    }

    pub fn insert_block(&mut self, page_id: &str, after: Option<&str>, text: &str) {
        if let Ok((_, receipt)) =
            self.store
                .lock()
                .unwrap()
                .edit_insert_block_after(page_id, after, "paragraph", text)
        {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
    }

    pub fn create_row(&mut self, data_source_id: &str, title_prop_name: &str, text: &str) {
        let props = serde_json::json!({
            title_prop_name: {
                "type": "title",
                "title": [{"plain_text": text, "type": "text", "text": {"content": text}}]
            }
        });
        if let Ok((_, receipt)) = self.store.lock().unwrap().edit_create_row(data_source_id, props) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
    }

    pub fn delete_row(&mut self, row_id: &str) {
        if let Ok(receipt) = self.store.lock().unwrap().edit_delete_row(row_id) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
    }

    pub fn start_rename(&mut self) {
        match &self.view {
            View::Page(v) => {
                self.input = Some(InputState::new("rename page", v.page.title.clone()));
                self.input_purpose = Some(InputPurpose::RenamePage {
                    page_id: v.page.id.clone(),
                });
            }
            View::Table(v) => {
                let title_col = v.columns.iter().find(|c| c.prop_type == "title").cloned();
                if let (Some(row), Some(col)) = (v.visible().get(v.cursor).copied(), title_col) {
                    self.input = Some(InputState::new("rename", v.cell(row, &col)));
                    self.input_purpose = Some(InputPurpose::RenameRow {
                        row_id: row.id.clone(),
                        title_prop_name: col.name,
                    });
                }
            }
            View::Board(v) => {
                let title_col = crate::ui::table::schema_columns(&v.ds.schema_json)
                    .into_iter()
                    .find(|c| c.prop_type == "title");
                if let (Some(row), Some(col)) = (v.cards_in(v.col).get(v.card).copied(), title_col) {
                    let text = crate::ui::table::cell_text(
                        &serde_json::from_str::<serde_json::Value>(&row.properties).unwrap_or_default()
                            [&col.name],
                    );
                    self.input = Some(InputState::new("rename", text));
                    self.input_purpose = Some(InputPurpose::RenameRow {
                        row_id: row.id.clone(),
                        title_prop_name: col.name,
                    });
                }
            }
            _ => self.notice = Some("nothing to rename here".into()),
        }
    }

    pub fn rename_page(&mut self, page_id: &str, new_title: &str) {
        if let Ok(receipt) = self.store.lock().unwrap().edit_rename_page(page_id, new_title) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
        self.refresh_sidebar();
    }

    pub fn start_move_page(&mut self) {
        let View::Page(v) = &self.view else {
            self.notice = Some("open a page to move it".into());
            return;
        };
        let page_id = v.page.id.clone();
        let nodes = self.store.lock().unwrap().sidebar_nodes().unwrap_or_default();
        // Excluding the moved page's full descendant subtree, not just itself:
        // moving a page onto its own descendant would create a parent cycle that
        // hangs `breadcrumb()`'s upward walk forever.
        let mut excluded: std::collections::HashSet<String> = std::collections::HashSet::new();
        excluded.insert(page_id.clone());
        loop {
            let mut grew = false;
            for n in &nodes {
                if let Some(pid) = &n.parent_id {
                    if excluded.contains(pid) && !excluded.contains(&n.id) {
                        excluded.insert(n.id.clone());
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let items: Vec<(String, String)> = nodes
            .into_iter()
            .filter(|n| n.kind == NodeKind::Page && !excluded.contains(&n.id))
            .map(|n| (n.id, n.title))
            .collect();
        self.picker = Some(crate::ui::picker::PickerState::new("move to…", items));
        self.picker_purpose = Some(PickerPurpose::MovePage { page_id });
    }

    pub fn move_page(&mut self, page_id: &str, new_parent_id: &str) {
        if let Ok(receipt) = self.store.lock().unwrap().edit_move_page(page_id, new_parent_id) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();
        self.refresh_sidebar();
    }

    pub fn start_group_by(&mut self) {
        let View::Board(v) = &self.view else {
            self.notice = Some("open a board to change its grouping".into());
            return;
        };
        let data_source_id = v.ds.id.clone();
        let items: Vec<(String, String)> = crate::ui::table::schema_columns(&v.ds.schema_json)
            .into_iter()
            .filter(|c| c.prop_type == "select" || c.prop_type == "status")
            .map(|c| (format!("{}\u{1}{}", c.name, c.prop_type), c.name))
            .collect();
        if items.is_empty() {
            self.notice = Some("no select/status property to group by".into());
            return;
        }
        self.picker = Some(crate::ui::picker::PickerState::new("group by…", items));
        self.picker_purpose = Some(PickerPurpose::GroupBy { data_source_id });
    }

    pub fn set_board_group(&mut self, encoded: &str) {
        let Some((name, ptype)) = encoded.split_once('\u{1}') else {
            return;
        };
        if let View::Board(v) = std::mem::replace(&mut self.view, View::Empty) {
            self.view = View::Board(BoardView::with_group(
                v.ds,
                v.rows,
                Some((name.to_string(), ptype.to_string())),
            ));
        }
    }

    pub fn request_sync_now(&mut self) {
        match &self.sync_notify {
            Some(n) => {
                n.notify_one();
                self.notice = Some("sync requested".into());
            }
            None => self.notice = Some("sync not available".into()),
        }
    }

    pub fn update_row_property(&mut self, row_id: &str, prop_name: &str, prop_type: &str, text: &str) {
        let existing = current_property_value(&self.view, row_id, prop_name);
        let Some(value) = build_property_value(prop_type, text, existing.as_ref()) else {
            self.notice = Some(format!(
                "{prop_type} properties can't be edited in notion-tui yet"
            ));
            return;
        };
        let patch = serde_json::json!({ prop_name: value });
        if let Ok(receipt) = self.store.lock().unwrap().edit_update_row(row_id, patch) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();

        if self.props.is_some() {
            let owning_view = match &self.view {
                View::Table(view) => Some((&view.ds, &view.rows)),
                View::Board(view) => Some((&view.ds, &view.rows)),
                _ => None,
            };
            if let Some(fields) = owning_view.and_then(|(ds, rows)| {
                rows.iter()
                    .find(|r| r.id == row_id)
                    .map(|row| build_fields(&ds.schema_json, row))
            }) {
                let cursor = self.props.as_ref().map(|p| p.cursor).unwrap_or(0);
                self.props = Some(PropsState {
                    row_id: row_id.to_string(),
                    fields,
                    cursor,
                    editor: None,
                    read_only_notice: None,
                });
            }
        }
    }

    /// Opens the current page's Markdown in an editor. Takes an injectable
    /// `run_editor` function so tests don't spawn a real subprocess; production
    /// code passes a closure around `editor::edit_text(&editor::editor_command(), initial)`.
    pub fn edit_in_editor(&mut self, run_editor: impl FnOnce(&str) -> anyhow::Result<String>) {
        let View::Page(view) = &self.view else { return };
        let page_id = view.page.id.clone();
        let blocks = view.blocks.clone();
        let (md, units) = crate::markdown::blocks_to_markdown(&blocks);

        // Even a failed editor run has drawn over the screen.
        self.force_redraw = true;
        let edited = match run_editor(&md) {
            Ok(text) => text,
            Err(e) => {
                self.notice = Some(format!("editor failed: {e}"));
                return;
            }
        };

        let (_, warnings) = crate::markdown::parse_markdown_checked(&edited);
        if let Some(crate::markdown::ParseWarning::UnclosedFence { line }) = warnings.first() {
            self.confirm = Some(crate::ui::confirm::ConfirmState {
                message: format!(
                    "unclosed code fence at line {line} — everything after it becomes one code \
                     block. Apply anyway?"
                ),
                ids: Vec::new(),
                kind: crate::ui::confirm::ConfirmKind::ApplyDespiteWarnings,
            });
            self.pending_editor_text = Some((page_id, units, edited));
            return;
        }

        self.apply_editor_result(page_id, units, edited);
    }

    /// The post-editor half of `edit_in_editor`: applies the edited Markdown to
    /// the store and reports the outcome. Shared by the direct path and the
    /// `ApplyDespiteWarnings` confirm path.
    fn apply_editor_result(&mut self, page_id: String, units: Vec<crate::markdown::Unit>, edited: String) {
        let empty = std::collections::HashSet::new();
        let mut guard = self.store.lock().unwrap();
        let applied = crate::markdown::apply_edited_markdown(&mut guard, &page_id, &units, &edited, &empty);
        drop(guard);

        match applied {
            Ok(result) if !result.protected_missing.is_empty() => {
                self.confirm = Some(crate::ui::confirm::ConfirmState {
                    message: format!(
                        "Delete {} block(s) that no longer appear in the edited text?",
                        result.protected_missing.len()
                    ),
                    ids: result.protected_missing,
                    kind: crate::ui::confirm::ConfirmKind::DeleteProtected,
                });
                self.pending_editor = Some((page_id, units));
            }
            Ok(result) => {
                self.notice = Some(summarize_applied(&result));
                self.refresh_current_view();
            }
            Err(e) => {
                self.notice = Some(format!("edit failed: {e}"));
            }
        }
    }

    fn confirm_apply_despite_warnings(&mut self, confirm: bool) {
        self.confirm = None;
        let Some((page_id, units, edited)) = self.pending_editor_text.take() else {
            return;
        };
        if confirm {
            self.apply_editor_result(page_id, units, edited);
        } else {
            self.notice = Some("edit discarded".into());
        }
    }

    fn confirm_delete_protected(&mut self, confirm: bool) {
        if let Some((page_id, units)) = self.pending_editor.take() {
            if confirm {
                if let Some(state) = self.confirm.take() {
                    let ids: std::collections::HashSet<String> = state.ids.into_iter().collect();
                    let mut guard = self.store.lock().unwrap();
                    let _ = crate::markdown::apply_edited_markdown(&mut guard, &page_id, &units, "", &ids);
                    drop(guard);
                }
            } else {
                self.confirm = None;
            }
        }
        self.refresh_current_view();
    }

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

    /// Page view: prefer the cursor block's comments if it has any; else page-level.
    /// Table/Board: the selected row is itself a page — show its comments.
    fn open_comments(&mut self) {
        let (parent_id, parent_kind) = match &self.view {
            View::Page(v) => {
                let block = v.block_id_at_cursor();
                let page = v.page.id.clone();
                match block {
                    Some(b)
                        if !self
                            .store
                            .lock()
                            .unwrap()
                            .comments_for(&b)
                            .unwrap_or_default()
                            .is_empty() =>
                    {
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
        let items = self
            .store
            .lock()
            .unwrap()
            .comments_for(&parent_id)
            .unwrap_or_default();
        self.comments = Some(crate::ui::comments::CommentsState {
            parent_id,
            parent_kind,
            items,
            cursor: 0,
            list_state: ratatui::widgets::ListState::default(),
        });
        self.request_comment_refresh();
    }

    pub fn refresh_comments(&mut self) {
        if let Some(panel) = &mut self.comments {
            panel.items = self
                .store
                .lock()
                .unwrap()
                .comments_for(&panel.parent_id)
                .unwrap_or_default();
            panel.cursor = panel.cursor.min(panel.items.len().saturating_sub(1));
        }
    }

    fn add_comment(&mut self, parent_id: &str, parent_kind: &str, thread_id: Option<&str>, body: &str) {
        self.store
            .lock()
            .unwrap()
            .edit_add_comment(parent_id, parent_kind, thread_id, body)
            .ok();
        self.refresh_comments();
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

    pub fn toggle_board(&mut self) {
        match std::mem::replace(&mut self.view, View::Empty) {
            View::Table(t) => {
                if crate::ui::board::group_property(&t.ds.schema_json).is_some() {
                    self.view = View::Board(BoardView::new(t.ds, t.rows));
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

    pub fn run_command(&mut self, cmd: &str) {
        match cmd {
            "help" => self.help_open = true,
            "queue" => {
                if !matches!(self.view, View::Queue(_)) {
                    self.toggle_queue();
                }
            }
            "board" | "table" => {
                let want_board = cmd == "board";
                let is_board = matches!(self.view, View::Board(_));
                let is_table = matches!(self.view, View::Table(_));
                if (want_board && is_table) || (!want_board && is_board) {
                    self.toggle_board();
                }
            }
            "quit" => self.should_quit = true,
            "rename" => self.start_rename(),
            "move page" => self.start_move_page(),
            "group by" => self.start_group_by(),
            "sync now" => self.request_sync_now(),
            _ => {}
        }
    }

    pub fn toggle_queue(&mut self) {
        match std::mem::replace(&mut self.view, View::Empty) {
            View::Queue(_) => {
                self.view = self.queue_return.take().map(|b| *b).unwrap_or(View::Empty);
            }
            other => {
                self.queue_return = Some(Box::new(other));
                let guard = self.store.lock().unwrap();
                let ops = guard.ops().unwrap_or_default();
                let summaries: Vec<String> = ops
                    .iter()
                    .map(|o| crate::describe::describe_op(o, &guard).summary)
                    .collect();
                drop(guard);
                self.view = View::Queue(crate::ui::queue::QueueView::new(ops, summaries));
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
            let guard = self.store.lock().unwrap();
            let ops = guard.ops().unwrap_or_default();
            let summaries: Vec<String> = ops
                .iter()
                .map(|o| crate::describe::describe_op(o, &guard).summary)
                .collect();
            drop(guard);
            let mut nq = crate::ui::queue::QueueView::new(ops, summaries);
            nq.cursor = cursor.min(nq.ops.len().saturating_sub(1));
            self.view = View::Queue(nq);
        }
        self.refresh_conflicted();
    }

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

    pub fn open_merge_editor(
        &mut self,
        msg: AppMsg,
        run_editor: impl FnOnce(&str) -> anyhow::Result<String>,
    ) {
        let AppMsg::MergeReady {
            op_seq,
            block_id,
            local_text,
            remote_text,
            remote_edited_time,
        } = msg
        else {
            return;
        };
        let doc = format!("<<<<<<< local\n{local_text}\n=======\n{remote_text}\n>>>>>>> remote\n");
        self.force_redraw = true;
        let merged = match run_editor(&doc) {
            Ok(merged) => merged,
            Err(e) => {
                self.notice = Some(format!("editor failed: {e}"));
                return;
            }
        };
        let merged = merged.trim_end_matches('\n').to_string();
        self.store
            .lock()
            .unwrap()
            .resolve_conflict_merge(op_seq, &block_id, &merged, &remote_edited_time)
            .ok();
        self.refresh_queue();
        self.refresh_current_view();
    }

    pub fn refresh_sidebar(&mut self) {
        if let Ok(nodes) = self.store.lock().unwrap().sidebar_nodes() {
            self.sidebar.nodes = nodes;
        }
    }

    pub fn open_node(&mut self, node: &TreeNode) {
        match node.kind {
            NodeKind::Page => self.open_page(&node.id),
            NodeKind::DataSource => self.open_table(&node.id),
        }
    }

    fn open_table(&mut self, data_source_id: &str) {
        let guard = self.store.lock().unwrap();
        let ds = guard.get_data_source(data_source_id).ok().flatten();
        let rows = guard.rows(data_source_id).unwrap_or_default();
        drop(guard);
        match ds {
            Some(ds) => self.view = View::Table(TableView::new(ds, rows)),
            None => self.notice = Some(format!("database not found (not synced yet?): {data_source_id}")),
        }
    }

    /// Opens a page by id; if the id is actually a data source or a database
    /// (child_database blocks carry the database id, not the data source id),
    /// opens the table view instead.
    pub fn open_page(&mut self, page_id: &str) {
        let guard = self.store.lock().unwrap();
        if let Ok(Some(page)) = guard.get_page(page_id) {
            let blocks = guard.page_blocks(page_id).unwrap_or_default();
            drop(guard);
            self.view = View::Page(PageView::new(page, blocks));
            self.request_page_liveness_check(page_id);
            return;
        }
        if let Ok(Some(ds)) = guard.get_data_source(page_id) {
            let rows = guard.rows(page_id).unwrap_or_default();
            drop(guard);
            self.view = View::Table(TableView::new(ds, rows));
            return;
        }
        if let Ok(Some(ds)) = guard.get_data_source_by_database_id(page_id) {
            let rows = guard.rows(&ds.id).unwrap_or_default();
            drop(guard);
            self.view = View::Table(TableView::new(ds, rows));
            return;
        }
        drop(guard);
        self.notice = Some(format!("not found (not synced yet?): {page_id}"));
    }

    pub fn handle_page_gone(&mut self, page_id: &str) {
        let Ok(forgot) = self.store.lock().unwrap().forget_page(page_id) else {
            return;
        };
        if !forgot {
            // Pending local edits guard it (same guard `prune_missing` uses) — a
            // false-positive liveness check must not destroy unpushed work, so
            // leave the page open/untouched.
            return;
        }
        self.notice = Some("that page was deleted or unshared in Notion".into());
        let showing_it = matches!(&self.view, View::Page(v) if v.page.id == page_id);
        if showing_it {
            match self.history.pop() {
                Some(prev) => self.open_page(&prev),
                None => self.view = View::Empty,
            }
        }
        self.refresh_sidebar();
    }

    /// Fires a background 404/archived check for a page the user just opened;
    /// on either signal, `AppMsg::PageGone` lets the caller forget it locally
    /// instead of leaving a ghost that resurfaces every sidebar refresh.
    fn request_page_liveness_check(&mut self, page_id: &str) {
        let Some(remote) = &self.remote else { return };
        let (client, tx) = (remote.client.clone(), remote.tx.clone());
        let page_id = page_id.to_string();
        tokio::spawn(async move {
            match client.get_page(&page_id).await {
                Ok(meta) if meta.archived => {
                    tx.send(AppMsg::PageGone(page_id)).ok();
                }
                Err(notion_api::ApiError::Api { status: 404, .. }) => {
                    tx.send(AppMsg::PageGone(page_id)).ok();
                }
                _ => {}
            }
        });
    }

    /// "workspace → Ancestor → … → current" for the open page; for a table/board,
    /// just "workspace → {data source title}" (data sources have no tracked page
    /// ancestry in this schema). `None` when nothing is open.
    pub fn breadcrumb(&self) -> Option<String> {
        match &self.view {
            View::Page(v) => {
                let guard = self.store.lock().unwrap();
                let mut chain = vec![v.page.title.clone()];
                let mut parent_id = v.page.parent_id.clone();
                // Defense-in-depth against a cyclic parent chain (should never exist —
                // `start_move_page` excludes descendants — but a corrupt store or a
                // bad crawl could still produce one): cap walk depth and bail on a
                // repeated id instead of looping forever, since this runs every draw().
                let mut visited = std::collections::HashSet::new();
                visited.insert(v.page.id.clone());
                const MAX_DEPTH: usize = 64;
                while let Some(pid) = parent_id {
                    if chain.len() >= MAX_DEPTH || !visited.insert(pid.clone()) {
                        break;
                    }
                    let Some(p) = guard.get_page(&pid).ok().flatten() else {
                        break;
                    };
                    chain.push(p.title.clone());
                    parent_id = p.parent_id.clone();
                }
                drop(guard);
                chain.reverse();
                Some(format!("workspace → {}", chain.join(" → ")))
            }
            View::Table(v) => Some(format!("workspace → {}", v.ds.title)),
            View::Board(v) => Some(format!("workspace → {}", v.ds.title)),
            View::Queue(_) | View::Empty => None,
        }
    }

    fn current_location_id(&self) -> Option<String> {
        match &self.view {
            View::Page(v) => Some(v.page.id.clone()),
            View::Table(v) => Some(v.ds.id.clone()),
            View::Board(v) => Some(v.ds.id.clone()),
            View::Queue(_) | View::Empty => None,
        }
    }

    fn push_history(&mut self) {
        if let Some(id) = self.current_location_id() {
            self.history.push(id);
        }
    }

    #[doc(hidden)]
    pub fn push_history_for_test(&mut self) {
        self.push_history();
    }

    pub fn refresh_search(&mut self) {
        let query = match &self.search {
            Some(s) => s.input.text().to_string(),
            None => return,
        };
        let mut hits = self.store.lock().unwrap().search(&query).unwrap_or_default();
        if !query.trim().is_empty() {
            let ranked_indices: Vec<usize> = crate::fuzzy::subsequence_rank(
                &query,
                hits.iter()
                    .enumerate()
                    .map(|(i, h)| (i, format!("{} {}", h.title, h.snippet)))
                    .collect(),
            );
            let mut matched = vec![false; hits.len()];
            for &i in &ranked_indices {
                matched[i] = true;
            }
            let mut order = ranked_indices;
            order.extend((0..matched.len()).filter(|&i| !matched[i])); // non-matches keep FTS order
            let mut slots: Vec<Option<notion_store::SearchHit>> = hits.into_iter().map(Some).collect();
            hits = order.into_iter().map(|i| slots[i].take().unwrap()).collect();
        }
        if let Some(search) = &mut self.search {
            search.results = hits;
            search.cursor = 0;
        }
    }

    pub fn set_table_filter(&mut self, data_source_id: &str, col: Option<usize>, text: &str) {
        let View::Table(v) = &mut self.view else { return };
        if v.ds.id != data_source_id {
            return;
        }
        v.filter = if text.is_empty() {
            None
        } else {
            Some(crate::ui::table::FilterState {
                col,
                query: text.to_string(),
            })
        };
        v.cursor = 0;
    }

    pub fn refresh_current_view(&mut self) {
        if matches!(self.view, View::Queue(_)) {
            self.refresh_queue();
            return;
        }
        match &self.view {
            View::Page(v) => {
                let (id, cursor, collapsed) = (v.page.id.clone(), v.cursor, v.collapsed_toggles.clone());
                self.open_page(&id);
                if let View::Page(nv) = &mut self.view {
                    nv.cursor = cursor.min(nv.lines().len().saturating_sub(1));
                    nv.collapsed_toggles = collapsed;
                }
            }
            View::Table(v) => {
                let (id, cursor, sort, sort_col, selected, filter) = (
                    v.ds.id.clone(),
                    v.cursor,
                    v.sort,
                    v.sort_col,
                    v.selected_row_id(),
                    v.filter.clone(),
                );
                self.open_table(&id);
                if let View::Table(nv) = &mut self.view {
                    // A background sync may have shrunk the schema (columns removed);
                    // drop a carried-over sort/highlight that no longer fits rather than
                    // indexing out of bounds.
                    nv.sort = sort.filter(|(col_idx, _)| *col_idx < nv.columns.len());
                    nv.sort_col = sort_col.min(nv.columns.len().saturating_sub(1));
                    // Re-apply the previously chosen sort to the freshly loaded rows
                    // the same way `toggle_sort` does (shared via `apply_sort`).
                    nv.apply_sort();
                    nv.filter = filter;
                    nv.cursor = selected
                        .and_then(|sid| nv.rows_iter_position(&sid))
                        .unwrap_or_else(|| cursor.min(nv.row_count().saturating_sub(1)));
                }
            }
            View::Board(v) => {
                let (id, col, card, selected, group_override) = (
                    v.ds.id.clone(),
                    v.col,
                    v.card,
                    v.selected_row_id(),
                    v.group_override.clone(),
                );
                let guard = self.store.lock().unwrap();
                let ds = guard.get_data_source(&id).ok().flatten();
                let rows = guard.rows(&id).unwrap_or_default();
                drop(guard);
                if let Some(ds) = ds {
                    let mut b = BoardView::with_group(ds, rows, group_override);
                    // Prefer re-locating the previously selected row (its group property
                    // may have just changed, e.g. via the board props modal) over reusing
                    // stale col/card indices that could now point at an unrelated card.
                    let relocated = selected.and_then(|id| {
                        b.columns.iter().enumerate().find_map(|(ci, _)| {
                            b.cards_in(ci).iter().position(|r| r.id == id).map(|ri| (ci, ri))
                        })
                    });
                    match relocated {
                        Some((ci, ri)) => {
                            b.col = ci;
                            b.card = ri;
                        }
                        None => {
                            b.col = col.min(b.columns.len().saturating_sub(1));
                            b.card = card.min(b.cards_in(b.col).len().saturating_sub(1));
                        }
                    }
                    self.view = View::Board(b);
                }
            }
            View::Queue(_) => unreachable!(),
            View::Empty => {}
        }
    }
}

/// Renders a human-readable summary of an applied Markdown edit, e.g.
/// `edited: 1 updated · 2 added` or `no changes` when nothing differs.
fn summarize_applied(a: &crate::markdown::Applied) -> String {
    let mut parts = Vec::new();
    if a.updated > 0 {
        parts.push(format!("{} updated", a.updated));
    }
    if a.inserted > 0 {
        parts.push(format!("{} added", a.inserted));
    }
    if a.deleted > 0 {
        parts.push(format!("{} deleted", a.deleted));
    }
    if a.reordered > 0 {
        parts.push(format!("{} moved", a.reordered));
    }
    if parts.is_empty() {
        "no changes".to_string()
    } else {
        format!("edited: {}", parts.join(" · "))
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    let km = app.keymap.clone();
    if km.is("quit", key) {
        app.should_quit = true;
        return Action::None;
    }
    if key.code == KeyCode::Tab {
        app.focus = match app.focus {
            Focus::Sidebar => Focus::Main,
            Focus::Main => Focus::Sidebar,
        };
        return Action::None;
    }
    if km.is("sidebar", key) {
        app.sidebar.hidden = !app.sidebar.hidden;
        return Action::None;
    }
    if km.is("undo", key) {
        app.pending_d = false;
        return Action::Undo;
    }
    if matches!(app.focus, Focus::Main) {
        if km.is("delete", key) {
            let target_id = match &app.view {
                View::Page(view) => view.block_id_at_cursor(),
                View::Table(view) => view.selected_row_id(),
                View::Board(view) => view.selected_row_id(),
                View::Queue(_) | View::Empty => None,
            };
            if app.pending_d {
                app.pending_d = false;
                if let Some(id) = target_id {
                    return match &app.view {
                        View::Page(_) => Action::DeleteBlock(id),
                        View::Table(_) => Action::DeleteRow(id),
                        View::Board(_) => Action::DeleteRow(id),
                        View::Queue(_) | View::Empty => Action::None,
                    };
                }
            } else if target_id.is_some() {
                app.pending_d = true;
            }
            return Action::None;
        }
        app.pending_d = false;
    }
    if matches!(app.focus, Focus::Sidebar) {
        if km.is("down", key) || key.code == KeyCode::Down {
            app.sidebar.move_cursor(1);
        } else if km.is("up", key) || key.code == KeyCode::Up {
            app.sidebar.move_cursor(-1);
        } else if km.is("left", key) || km.is("right", key) {
            app.sidebar.toggle_collapse();
        } else if km.is("open", key) {
            if let Some(node) = app.sidebar.selected().cloned() {
                app.focus = Focus::Main;
                return Action::OpenNode(node);
            }
        }
    }
    if matches!(app.focus, Focus::Main) {
        if let View::Page(view) = &mut app.view {
            if km.is("down", key) || key.code == KeyCode::Down {
                view.move_cursor(1);
            } else if km.is("up", key) || key.code == KeyCode::Up {
                view.move_cursor(-1);
            } else if km.is("top", key) {
                view.cursor = 0;
            } else if km.is("bottom", key) {
                view.cursor = view.lines().len().saturating_sub(1);
            } else if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor(10);
            } else if key.code == KeyCode::Char('u') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor(-10);
            } else if km.is("left", key) || km.is("right", key) {
                view.toggle_at_cursor();
            } else if km.is("toggle", key) {
                if let Some(block_id) = view.todo_block_at_cursor() {
                    return Action::ToggleTodo(block_id);
                }
            } else if km.is("open", key) {
                if let Some(target) = view.link_at_cursor() {
                    return Action::OpenPage(target);
                }
            } else if km.is("back", key) || key.code == KeyCode::Backspace {
                if let Some(prev) = app.history.pop() {
                    return Action::Back(prev);
                }
            }
        }
        if let View::Table(view) = &mut app.view {
            if km.is("down", key) || key.code == KeyCode::Down {
                view.move_cursor(1);
            } else if km.is("up", key) || key.code == KeyCode::Up {
                view.move_cursor(-1);
            } else if km.is("top", key) {
                view.cursor = 0;
            } else if km.is("bottom", key) {
                view.cursor = view.row_count().saturating_sub(1);
            } else if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor(10);
            } else if key.code == KeyCode::Char('u') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor(-10);
            } else if km.is("left", key) {
                view.sort_col = view.sort_col.saturating_sub(1);
            } else if km.is("right", key) {
                if view.sort_col + 1 < view.columns.len() {
                    view.sort_col += 1;
                }
            } else if km.is("sort", key) {
                view.toggle_sort(view.sort_col);
            } else if km.is("open", key) {
                if let Some(row_id) = view.selected_row_id() {
                    return Action::OpenPage(row_id);
                }
            }
        }
        if let View::Board(view) = &mut app.view {
            if (km.is("down", key) && key.modifiers == KeyModifiers::NONE) || key.code == KeyCode::Down {
                view.move_cursor_card(1);
            } else if (km.is("up", key) && key.modifiers == KeyModifiers::NONE) || key.code == KeyCode::Up {
                view.move_cursor_card(-1);
            } else if key.code == KeyCode::Char('d') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor_card(10);
            } else if key.code == KeyCode::Char('u') && key.modifiers == KeyModifiers::CONTROL {
                view.move_cursor_card(-10);
            } else if km.is("top", key) {
                view.card = 0;
            } else if km.is("bottom", key) {
                view.card = view.cards_in(view.col).len().saturating_sub(1);
            } else if km.is("left", key) {
                view.move_cursor_col(-1);
            } else if km.is("right", key) {
                view.move_cursor_col(1);
            } else if km.is("move_card_next", key) {
                if let Some((row_id, value)) = view.move_card(1) {
                    return Action::MoveCard {
                        row_id,
                        prop_name: view.group_prop.clone(),
                        prop_type: view.group_type.clone(),
                        value,
                    };
                }
            } else if km.is("move_card_prev", key) {
                if let Some((row_id, value)) = view.move_card(-1) {
                    return Action::MoveCard {
                        row_id,
                        prop_name: view.group_prop.clone(),
                        prop_type: view.group_type.clone(),
                        value,
                    };
                }
            } else if km.is("open", key) {
                if let Some(row_id) = view.selected_row_id() {
                    return Action::OpenPage(row_id);
                }
            }
        }
    }
    Action::None
}

/// Looks up the row's current value for `prop_name` from whichever view (table or
/// board) currently owns it, so `update_row_property` can pass it to
/// `build_property_value` as `existing` (needed to preserve fields like a date's `end`
/// that a plain text edit never touches).
fn current_property_value(view: &View, row_id: &str, prop_name: &str) -> Option<serde_json::Value> {
    let rows = match view {
        View::Table(v) => &v.rows,
        View::Board(v) => &v.rows,
        _ => return None,
    };
    let row = rows.iter().find(|r| r.id == row_id)?;
    let props: serde_json::Value = serde_json::from_str(&row.properties).ok()?;
    props.get(prop_name).cloned()
}

/// Top-level key entry point: routes to the search modal when open, applies
/// `handle_key`'s Action against the store otherwise, and owns the '/' /
/// Ctrl+P shortcuts that open search from any focus.
pub fn dispatch_key(app: &mut App, key: KeyEvent) {
    let km = app.keymap.clone();
    app.notice = None;
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
    if app.picker.is_some() {
        let action = app.picker.as_mut().unwrap().on_key(key);
        match action {
            crate::ui::picker::PickerAction::None | crate::ui::picker::PickerAction::Changed => {}
            crate::ui::picker::PickerAction::Close => {
                app.picker = None;
                app.picker_purpose = None;
            }
            crate::ui::picker::PickerAction::Choose(id) => {
                app.picker = None;
                if let Some(purpose) = app.picker_purpose.take() {
                    match purpose {
                        PickerPurpose::MovePage { page_id } => app.move_page(&page_id, &id),
                        PickerPurpose::GroupBy { .. } => app.set_board_group(&id),
                    }
                }
            }
        }
        return;
    }
    if app.confirm.is_some() {
        let action = app.confirm.as_mut().unwrap().on_key(key);
        let confirmed = match action {
            crate::ui::confirm::ConfirmAction::None => return,
            crate::ui::confirm::ConfirmAction::Yes => true,
            crate::ui::confirm::ConfirmAction::No => false,
        };
        match app.confirm.as_ref().unwrap().kind {
            crate::ui::confirm::ConfirmKind::DeleteProtected => app.confirm_delete_protected(confirmed),
            crate::ui::confirm::ConfirmKind::ApplyDespiteWarnings => {
                app.confirm_apply_despite_warnings(confirmed)
            }
            crate::ui::confirm::ConfirmKind::DeleteBlock | crate::ui::confirm::ConfirmKind::DeleteRow => {
                app.confirm_delete_target(confirmed)
            }
        }
        return;
    }
    if app.props.is_some() {
        let action = app.props.as_mut().unwrap().on_key(key);
        match action {
            PropsAction::None => {}
            PropsAction::Close => app.props = None,
            PropsAction::Commit {
                prop_name,
                prop_type,
                text,
            } => {
                let row_id = app.props.as_ref().unwrap().row_id.clone();
                app.update_row_property(&row_id, &prop_name, &prop_type, &text);
            }
        }
        return;
    }
    if app.input.is_some() {
        let action = app.input.as_mut().unwrap().on_key(key);
        match action {
            InputAction::None | InputAction::Changed => {}
            InputAction::Cancel => {
                app.input = None;
                app.input_purpose = None;
            }
            InputAction::Submit(text) => {
                if let Some(purpose) = app.input_purpose.take() {
                    match purpose {
                        InputPurpose::EditBlockText { block_id } => app.edit_block_text(&block_id, &text),
                        InputPurpose::InsertBlockAfter {
                            page_id,
                            after_block_id,
                        } => app.insert_block(&page_id, after_block_id.as_deref(), &text),
                        InputPurpose::NewRow {
                            data_source_id,
                            title_prop_name,
                        } => app.create_row(&data_source_id, &title_prop_name, &text),
                        InputPurpose::NewComment {
                            parent_id,
                            parent_kind,
                            thread_id,
                        } => app.add_comment(&parent_id, &parent_kind, thread_id.as_deref(), &text),
                        InputPurpose::RenamePage { page_id } => app.rename_page(&page_id, &text),
                        InputPurpose::RenameRow {
                            row_id,
                            title_prop_name,
                        } => app.update_row_property(&row_id, &title_prop_name, "title", &text),
                        InputPurpose::FilterTable { data_source_id, col } => {
                            app.set_table_filter(&data_source_id, col, &text)
                        }
                    }
                }
                app.input = None;
            }
        }
        return;
    }
    if app.search.is_some() {
        let action = app.search.as_mut().unwrap().on_key(key);
        match action {
            SearchAction::None => {}
            SearchAction::QueryChanged => app.refresh_search(),
            SearchAction::Open(id) => {
                app.search = None;
                app.push_history();
                app.open_page(&id);
            }
            SearchAction::Close => app.search = None,
        }
        return;
    }
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
    let opens_search = km.is("search", key)
        || (key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL));
    if opens_search {
        app.search = Some(SearchState::new());
        return;
    }
    if km.is("help", key) {
        app.help_open = true;
        return;
    }
    if km.is("palette", key) {
        app.palette = Some(crate::ui::palette::PaletteState::new());
        return;
    }
    if matches!(app.focus, Focus::Main) && km.is("comments", key) {
        app.open_comments();
        return;
    }
    if matches!(app.focus, Focus::Sidebar) && km.is("board", key) {
        if let Some(node) = app.sidebar.selected().cloned() {
            if node.kind == NodeKind::DataSource {
                app.focus = Focus::Main;
                app.open_node(&node);
                if matches!(app.view, View::Table(_)) {
                    app.toggle_board();
                }
            }
        }
        return;
    }
    if matches!(app.focus, Focus::Main) {
        if let View::Page(view) = &app.view {
            if km.is("insert", key) {
                if let Some(block_id) = view.block_id_at_cursor() {
                    let initial = view
                        .blocks
                        .iter()
                        .find(|b| b.id == block_id)
                        .map(|b| b.plain_text.clone())
                        .unwrap_or_default();
                    app.input = Some(InputState::new("edit block", initial));
                    app.input_purpose = Some(InputPurpose::EditBlockText { block_id });
                    return;
                }
            }
            if km.is("append", key) {
                let page_id = view.page.id.clone();
                let after = view.block_id_at_cursor();
                app.input = Some(InputState::new("new block", ""));
                app.input_purpose = Some(InputPurpose::InsertBlockAfter {
                    page_id,
                    after_block_id: after,
                });
                return;
            }
            if km.is("edit", key) {
                let editor = crate::editor::editor_command(app.editor_override.as_deref());
                let mouse = app.mouse;
                app.edit_in_editor(move |initial| {
                    crate::terminal::with_suspended(mouse, || crate::editor::edit_text(&editor, initial))
                });
                return;
            }
        }
        if km.is("board", key) {
            if let View::Page(view) = &app.view {
                if let Some(target) = view.child_database_id_at_cursor() {
                    app.open_page(&target);
                    if matches!(app.view, View::Table(_)) {
                        app.toggle_board();
                    }
                    return;
                }
            }
            app.toggle_board();
            return;
        }
        if let View::Table(view) = &app.view {
            if km.is("new_row", key) {
                if let Some(title_col) = view.columns.iter().find(|c| c.prop_type == "title") {
                    app.input = Some(InputState::new("new row", ""));
                    app.input_purpose = Some(InputPurpose::NewRow {
                        data_source_id: view.ds.id.clone(),
                        title_prop_name: title_col.name.clone(),
                    });
                    return;
                }
            }
            if km.is("props", key) {
                if let Some(row) = view.visible().get(view.cursor).copied() {
                    let fields = build_fields(&view.ds.schema_json, row);
                    app.props = Some(PropsState::new(row.id.clone(), fields));
                    return;
                }
            }
            if km.is("filter", key) {
                let prefill = view
                    .filter
                    .as_ref()
                    .filter(|f| f.col == Some(view.sort_col))
                    .map(|f| f.query.clone())
                    .unwrap_or_default();
                let col_name = view
                    .columns
                    .get(view.sort_col)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                app.input = Some(InputState::new(format!("filter {col_name}"), prefill));
                app.input_purpose = Some(InputPurpose::FilterTable {
                    data_source_id: view.ds.id.clone(),
                    col: Some(view.sort_col),
                });
                return;
            }
            if km.is("filter_row", key) {
                let prefill = view
                    .filter
                    .as_ref()
                    .filter(|f| f.col.is_none())
                    .map(|f| f.query.clone())
                    .unwrap_or_default();
                app.input = Some(InputState::new("filter row", prefill));
                app.input_purpose = Some(InputPurpose::FilterTable {
                    data_source_id: view.ds.id.clone(),
                    col: None,
                });
                return;
            }
        }
        if let View::Board(view) = &app.view {
            if km.is("props", key) {
                if let Some(row) = view.cards_in(view.col).get(view.card).copied() {
                    let fields = build_fields(&view.ds.schema_json, row);
                    app.props = Some(PropsState::new(row.id.clone(), fields));
                    return;
                }
            }
        }
    }
    if km.is("queue", key) {
        app.toggle_queue();
        return;
    }
    if let View::Queue(q) = &mut app.view {
        if km.is("down", key) || key.code == KeyCode::Down {
            q.move_cursor(1);
        } else if km.is("up", key) || key.code == KeyCode::Up {
            q.move_cursor(-1);
        } else if key.code == KeyCode::Esc {
            app.toggle_queue();
        } else if key.code == KeyCode::Char('r') {
            if let Some(op) = q.selected() {
                let seq = op.seq;
                app.store.lock().unwrap().retry_op(seq).ok();
                app.refresh_queue();
            }
        } else if key.code == KeyCode::Char('p') {
            if let Some(op) = q.selected() {
                let seq = op.seq;
                app.store.lock().unwrap().resolve_keep_mine(seq).ok();
                app.refresh_queue();
            }
        } else if key.code == KeyCode::Char('t') {
            if let Some(op) = q.selected() {
                let seq = op.seq;
                let target = app.store.lock().unwrap().resolve_take_theirs(seq).ok().flatten();
                if let Some(target) = target {
                    app.request_refetch(target);
                }
                app.refresh_queue();
            }
        } else if key.code == KeyCode::Char('e') {
            if let Some(op) = q.selected() {
                if op.state == "conflicted" && op.op_type == "update_block" {
                    let op = op.clone();
                    app.request_merge(op);
                }
            }
        }
        return;
    }
    let action = handle_key(app, key);
    apply_action(app, action);
}

/// Applies a navigation/edit `Action` against the store. This is the single
/// funnel for both keyboard (`dispatch_key`) and mouse dispatch: every
/// forward navigation pushes history here, so sidebar opens, table/board row
/// opens, and search jumps all participate in `Backspace`/`-` the same way
/// in-page link follows already did.
pub fn apply_action(app: &mut App, action: Action) {
    match action {
        Action::None => {}
        Action::OpenNode(node) => {
            app.push_history();
            app.open_node(&node);
        }
        Action::OpenPage(id) => {
            app.push_history();
            app.open_page(&id);
        }
        Action::Back(id) => app.open_page(&id),
        Action::ToggleTodo(id) => app.toggle_todo(&id),
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
        Action::MoveCard {
            row_id,
            prop_name,
            prop_type,
            value,
        } => app.update_row_property(&row_id, &prop_name, &prop_type, &value),
        Action::Undo => app.undo(),
    }
}

/// Forwards mouse-wheel scroll to whichever view is focused.
pub fn scroll(app: &mut App, delta: isize) {
    match &mut app.view {
        View::Page(v) => v.move_cursor(delta),
        View::Table(v) => v.move_cursor(delta),
        View::Board(v) => v.move_cursor_card(delta),
        View::Queue(v) => v.move_cursor(delta),
        View::Empty => {}
    }
}

/// Routes a crossterm mouse event. Modals (palette/picker/confirm/props/
/// input/search/comments) don't participate in mouse routing yet — clicks
/// while one is open are ignored, matching how they already swallow all
/// keyboard input except their own handlers.
pub fn dispatch_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    use crossterm::event::{MouseButton, MouseEventKind};
    if app.palette.is_some()
        || app.picker.is_some()
        || app.confirm.is_some()
        || app.props.is_some()
        || app.input.is_some()
        || app.search.is_some()
        || app.comments.is_some()
    {
        return;
    }
    let pt = (m.column, m.row);
    match m.kind {
        MouseEventKind::ScrollDown => scroll(app, 3),
        MouseEventKind::ScrollUp => scroll(app, -3),
        MouseEventKind::Down(MouseButton::Left) => {
            let action = handle_click(app, pt);
            apply_action(app, action);
        }
        MouseEventKind::Drag(MouseButton::Left) => handle_drag(app, pt),
        MouseEventKind::Up(MouseButton::Left) => {
            let action = handle_drop(app, pt);
            apply_action(app, action);
        }
        _ => {}
    }
}

fn point_in(rect: ratatui::layout::Rect, pt: (u16, u16)) -> bool {
    pt.0 >= rect.x && pt.0 < rect.x + rect.width && pt.1 >= rect.y && pt.1 < rect.y + rect.height
}

fn handle_click(app: &mut App, pt: (u16, u16)) -> Action {
    let layout = app.last_layout.clone();
    if let Some(sidebar) = layout.sidebar {
        if point_in(sidebar, pt) {
            app.focus = Focus::Sidebar;
            let row = pt.1.saturating_sub(sidebar.y + 1) as usize;
            let idx = app.sidebar.list_state.offset() + row;
            if let Some(node) = app.sidebar.visible().get(idx).map(|v| v.node.clone()) {
                app.sidebar.cursor = idx;
                return Action::OpenNode(node);
            }
            return Action::None;
        }
    }
    if !point_in(layout.main, pt) {
        return Action::None;
    }
    app.focus = Focus::Main;

    if let (Some(header_y), View::Table(_)) = (layout.table_header_y, &app.view) {
        if pt.1 == header_y {
            let col = layout.table_col_x.iter().rposition(|&x| pt.0 >= x).unwrap_or(0);
            if let View::Table(v) = &mut app.view {
                v.toggle_sort(col);
            }
            return Action::None;
        }
    }
    if let View::Page(v) = &app.view {
        let row = pt.1.saturating_sub(layout.main.y + 1) as usize;
        let idx = v.list_state.offset() + row;
        if let Some(line) = v.lines().get(idx) {
            if let Some(target) = line.link_page_id.clone() {
                return Action::OpenPage(target);
            }
        }
        return Action::None;
    }
    if let View::Board(_) = &app.view {
        if let Some((ci, _)) = layout
            .board_columns
            .iter()
            .enumerate()
            .find(|(_, r)| point_in(**r, pt))
        {
            if let View::Board(v) = &mut app.view {
                let row = pt.1.saturating_sub(layout.board_columns[ci].y + 1) as usize;
                // board.rs render only scrolls the ACTIVE column via `list_state`;
                // every other column renders unscrolled from item 0.
                let offset = if ci == v.col { v.list_state.offset() } else { 0 };
                let card_idx = offset + row;
                if card_idx < v.cards_in(ci).len() {
                    v.col = ci;
                    v.card = card_idx;
                    app.drag_card = Some((ci, card_idx));
                }
            }
        }
    }
    Action::None
}

fn handle_drag(app: &mut App, pt: (u16, u16)) {
    if app.drag_card.is_none() {
        return;
    }
    let layout = app.last_layout.clone();
    if let Some((ci, _)) = layout
        .board_columns
        .iter()
        .enumerate()
        .find(|(_, r)| point_in(**r, pt))
    {
        if let View::Board(v) = &mut app.view {
            v.col = ci; // live preview: highlight the column under the cursor
        }
    }
}

fn handle_drop(app: &mut App, _pt: (u16, u16)) -> Action {
    let Some((origin_col, _)) = app.drag_card.take() else {
        return Action::None;
    };
    let View::Board(v) = &mut app.view else {
        return Action::None;
    };
    let target = v.col;
    v.col = origin_col; // move_card_to validates target != current col itself
    match v.move_card_to(target) {
        Some((row_id, value)) => Action::MoveCard {
            row_id,
            prop_name: v.group_prop.clone(),
            prop_type: v.group_type.clone(),
            value,
        },
        None => Action::None,
    }
}
