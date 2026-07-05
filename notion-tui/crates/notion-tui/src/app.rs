use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::{NodeKind, TreeNode};
use notion_sync::{SharedStore, SyncStatus};

use crate::ui::page::PageView;
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
}

pub struct App {
    pub focus: Focus,
    pub sync_status: SyncStatus,
    pub should_quit: bool,
    pub sidebar: SidebarState,
    pub view: View,
    pub history: Vec<String>,
    pub search: Option<SearchState>,
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
            store,
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
    if matches!(app.focus, Focus::Main) {
        if let View::Page(view) = &mut app.view {
            match (key.code, key.modifiers) {
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => view.move_cursor(1),
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => view.move_cursor(-1),
                (KeyCode::Char('g'), _) => view.cursor = 0,
                (KeyCode::Char('G'), _) => view.cursor = view.lines().len().saturating_sub(1),
                (KeyCode::Char('d'), KeyModifiers::CONTROL) => view.move_cursor(10),
                (KeyCode::Char('u'), KeyModifiers::CONTROL) => view.move_cursor(-10),
                (KeyCode::Char('h'), _) | (KeyCode::Char('l'), _) | (KeyCode::Char(' '), _) => {
                    view.toggle_at_cursor()
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
    match handle_key(app, key) {
        Action::None => {}
        Action::OpenNode(node) => app.open_node(&node),
        Action::OpenPage(id) => app.open_page(&id),
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
