use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::{NodeKind, TreeNode};
use notion_sync::{SharedStore, SyncStatus};

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
}

pub enum Action {
    None,
    OpenNode(TreeNode),
    OpenPage(String),
    ToggleTodo(String),
    DeleteBlock(String),
    DeleteRow(String),
    Undo,
}

pub enum InputPurpose {
    EditBlockText { block_id: String },
    InsertBlockAfter { page_id: String, after_block_id: Option<String> },
    NewRow { data_source_id: String, title_prop_name: String },
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
            self.store.lock().unwrap().edit_insert_block_after(page_id, after, "paragraph", text)
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

    pub fn update_row_property(&mut self, row_id: &str, prop_name: &str, prop_type: &str, text: &str) {
        let value = build_property_value(prop_type, text);
        let patch = serde_json::json!({ prop_name: value });
        if let Ok(receipt) = self.store.lock().unwrap().edit_update_row(row_id, patch) {
            self.undo_stack.push(receipt);
        }
        self.refresh_current_view();

        if let View::Table(view) = &self.view {
            if let Some(row) = view.rows.iter().find(|r| r.id == row_id) {
                let fields = build_fields(view, row);
                let cursor = self.props.as_ref().map(|p| p.cursor).unwrap_or(0);
                self.props =
                    Some(PropsState { row_id: row_id.to_string(), fields, cursor, edit_buffer: None });
            }
        }
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
        if let Some(ds) = ds {
            self.view = View::Table(TableView::new(ds, rows));
        }
    }

    /// Opens a page by id; if the id is actually a data source, opens the table view instead.
    pub fn open_page(&mut self, page_id: &str) {
        let guard = self.store.lock().unwrap();
        if let Ok(Some(page)) = guard.get_page(page_id) {
            let blocks = guard.page_blocks(page_id).unwrap_or_default();
            drop(guard);
            self.view = View::Page(PageView::new(page, blocks));
            return;
        }
        if let Ok(Some(ds)) = guard.get_data_source(page_id) {
            let rows = guard.rows(page_id).unwrap_or_default();
            drop(guard);
            self.view = View::Table(TableView::new(ds, rows));
        }
    }

    pub fn refresh_search(&mut self) {
        let query = match &self.search {
            Some(s) => s.input.clone(),
            None => return,
        };
        let hits = self.store.lock().unwrap().search(&query).unwrap_or_default();
        if let Some(search) = &mut self.search {
            search.results = hits;
            search.cursor = 0;
        }
    }

    pub fn refresh_current_view(&mut self) {
        match &self.view {
            View::Page(v) => {
                let id = v.page.id.clone();
                self.open_page(&id);
            }
            View::Table(v) => {
                let id = v.ds.id.clone();
                self.open_table(&id);
            }
            View::Empty => {}
        }
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return Action::None;
        }
        KeyCode::Tab => {
            app.focus = match app.focus {
                Focus::Sidebar => Focus::Main,
                Focus::Main => Focus::Sidebar,
            };
            return Action::None;
        }
        KeyCode::Char('1') => {
            app.sidebar.hidden = !app.sidebar.hidden;
            return Action::None;
        }
        KeyCode::Char('u') if key.modifiers.is_empty() => {
            app.pending_d = false;
            return Action::Undo;
        }
        _ => {}
    }
    if matches!(app.focus, Focus::Main) {
        if key.code == KeyCode::Char('d') {
            let target_id = match &app.view {
                View::Page(view) => view.block_id_at_cursor(),
                View::Table(view) => view.selected_row_id(),
                View::Empty => None,
            };
            if app.pending_d {
                app.pending_d = false;
                if let Some(id) = target_id {
                    return match &app.view {
                        View::Page(_) => Action::DeleteBlock(id),
                        View::Table(_) => Action::DeleteRow(id),
                        View::Empty => Action::None,
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
    if matches!(app.focus, Focus::Main) {
        if let View::Page(view) = &mut app.view {
            match (key.code, key.modifiers) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => view.move_cursor(1),
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => view.move_cursor(-1),
                (KeyCode::Char('g'), _) => view.cursor = 0,
                (KeyCode::Char('G'), _) => view.cursor = view.lines().len().saturating_sub(1),
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => view.move_cursor(10),
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => view.move_cursor(-10),
                (KeyCode::Char('h'), _) | (KeyCode::Char('l'), _) => view.toggle_at_cursor(),
                (KeyCode::Char(' '), _) => {
                    if let Some(block_id) = view.todo_block_at_cursor() {
                        return Action::ToggleTodo(block_id);
                    }
                }
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
        if let View::Table(view) = &mut app.view {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => view.move_cursor(1),
                KeyCode::Char('k') | KeyCode::Up => view.move_cursor(-1),
                KeyCode::Char('g') => view.cursor = 0,
                KeyCode::Char('G') => view.cursor = view.rows.len().saturating_sub(1),
                KeyCode::Char('h') => {
                    view.sort_col = view.sort_col.saturating_sub(1);
                }
                KeyCode::Char('l') => {
                    if view.sort_col + 1 < view.columns.len() {
                        view.sort_col += 1;
                    }
                }
                KeyCode::Char('s') => view.toggle_sort(view.sort_col),
                KeyCode::Enter => {
                    if let Some(row_id) = view.selected_row_id() {
                        return Action::OpenPage(row_id);
                    }
                }
                _ => {}
            }
        }
    }
    Action::None
}

/// Top-level key entry point: routes to the search modal when open, applies
/// `handle_key`'s Action against the store otherwise, and owns the '/' /
/// Ctrl+P shortcuts that open search from any focus.
pub fn dispatch_key(app: &mut App, key: KeyEvent) {
    if app.props.is_some() {
        let action = app.props.as_mut().unwrap().on_key(key);
        match action {
            PropsAction::None => {}
            PropsAction::Close => app.props = None,
            PropsAction::Commit { prop_name, prop_type, text } => {
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
                        InputPurpose::InsertBlockAfter { page_id, after_block_id } => {
                            app.insert_block(&page_id, after_block_id.as_deref(), &text)
                        }
                        InputPurpose::NewRow { data_source_id, title_prop_name } => {
                            app.create_row(&data_source_id, &title_prop_name, &text)
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
                app.open_page(&id);
            }
            SearchAction::Close => app.search = None,
        }
        return;
    }
    let opens_search = key.code == KeyCode::Char('/')
        || (key.code == KeyCode::Char('p') && key.modifiers.contains(KeyModifiers::CONTROL));
    if opens_search {
        app.search = Some(SearchState::new());
        return;
    }
    if matches!(app.focus, Focus::Main) {
        if let View::Page(view) = &app.view {
            if key.code == KeyCode::Char('i') {
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
            if key.code == KeyCode::Char('a') {
                let page_id = view.page.id.clone();
                let after = view.block_id_at_cursor();
                app.input = Some(InputState::new("new block", ""));
                app.input_purpose = Some(InputPurpose::InsertBlockAfter { page_id, after_block_id: after });
                return;
            }
        }
        if let View::Table(view) = &app.view {
            if key.code == KeyCode::Char('o') {
                if let Some(title_col) = view.columns.iter().find(|c| c.prop_type == "title") {
                    app.input = Some(InputState::new("new row", ""));
                    app.input_purpose = Some(InputPurpose::NewRow {
                        data_source_id: view.ds.id.clone(),
                        title_prop_name: title_col.name.clone(),
                    });
                    return;
                }
            }
            if key.code == KeyCode::Char('p') {
                if let Some(row) = view.rows.get(view.cursor) {
                    let fields = build_fields(view, row);
                    app.props = Some(PropsState::new(row.id.clone(), fields));
                    return;
                }
            }
        }
    }
    match handle_key(app, key) {
        Action::None => {}
        Action::OpenNode(node) => app.open_node(&node),
        Action::OpenPage(id) => app.open_page(&id),
        Action::ToggleTodo(id) => app.toggle_todo(&id),
        Action::DeleteBlock(id) => app.delete_block(&id),
        Action::DeleteRow(id) => app.delete_row(&id),
        Action::Undo => app.undo(),
    }
}

/// Forwards mouse-wheel scroll to whichever view is focused.
pub fn scroll(app: &mut App, delta: isize) {
    match &mut app.view {
        View::Page(v) => v.move_cursor(delta),
        View::Table(v) => v.move_cursor(delta),
        View::Empty => {}
    }
}
