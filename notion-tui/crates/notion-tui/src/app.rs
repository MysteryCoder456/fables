use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use notion_store::TreeNode;
use notion_sync::SyncStatus;

use crate::ui::page::PageView;
use crate::ui::sidebar::SidebarState;

pub enum Focus {
    Sidebar,
    Main,
}

pub enum View {
    Empty,
    Page(PageView),
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
}

impl App {
    pub fn new() -> App {
        App {
            focus: Focus::Sidebar,
            sync_status: SyncStatus::Starting,
            should_quit: false,
            sidebar: SidebarState::new(Vec::new()),
            view: View::Empty,
            history: Vec::new(),
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
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
    }
    Action::None
}
