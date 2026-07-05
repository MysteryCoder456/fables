use crossterm::event::{KeyCode, KeyEvent};
use notion_store::TreeNode;
use notion_sync::SyncStatus;

use crate::ui::sidebar::SidebarState;

pub enum Focus {
    Sidebar,
    Main,
}

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
    Action::None
}
