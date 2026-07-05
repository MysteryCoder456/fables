use crossterm::event::{KeyCode, KeyEvent};
use notion_sync::SyncStatus;

pub enum Focus {
    Sidebar,
    Main,
}

pub struct App {
    pub focus: Focus,
    pub sync_status: SyncStatus,
    pub should_quit: bool,
}

impl App {
    pub fn new() -> App {
        App {
            focus: Focus::Sidebar,
            sync_status: SyncStatus::Starting,
            should_quit: false,
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if let KeyCode::Char('q') = key.code {
        app.should_quit = true;
    }
}
