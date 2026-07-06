pub mod board;
pub mod comments;
pub mod confirm;
pub mod help;
pub mod input;
pub mod page;
pub mod palette;
pub mod props;
pub mod queue;
pub mod search;
pub mod sidebar;
pub mod table;
pub mod theme;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus, View};
use notion_sync::SyncStatus;

pub fn status_line(status: &SyncStatus, pending: u32, conflicted: u32) -> String {
    let base = match status {
        SyncStatus::Starting => "starting…".into(),
        SyncStatus::Syncing { .. } => "⟳ syncing…".into(),
        SyncStatus::Idle { updated: 0 } => "✓ synced".into(),
        SyncStatus::Idle { updated } => format!("✓ synced ({updated} updated)"),
        SyncStatus::Offline => "⚠ offline".into(),
        SyncStatus::Failed(msg) => format!("✗ sync failed: {msg}"),
    };
    let mut out = base;
    if pending > 0 {
        out = format!("{out} · {pending} pending");
    }
    if conflicted > 0 {
        out = format!("{out} · ⚠ {conflicted} conflicted");
    }
    out
}

pub fn draw(f: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    let cols: Vec<ratatui::layout::Rect> = if app.sidebar.hidden {
        vec![rows[0]]
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(28), Constraint::Min(1)])
            .split(rows[0])
            .to_vec()
    };

    if !app.sidebar.hidden {
        sidebar::render(f, cols[0], &app.sidebar, matches!(app.focus, Focus::Sidebar), &app.theme);
    }
    let full_main = *cols.last().unwrap();
    let main_area = if let Some(comments_state) = &app.comments {
        let halves = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(36)])
            .split(full_main);
        comments::render(f, halves[1], comments_state, &app.theme);
        halves[0]
    } else {
        full_main
    };
    let main_focused = matches!(app.focus, Focus::Main);
    let theme = &app.theme;
    match &app.view {
        View::Page(view) => page::render(f, main_area, view, main_focused, theme),
        View::Table(view) => table::render(f, main_area, view, main_focused, theme),
        View::Board(view) => board::render(f, main_area, view, main_focused, theme),
        View::Queue(view) => queue::render(f, main_area, view, main_focused, theme),
        View::Empty => f.render_widget(Block::default().borders(Borders::ALL), main_area),
    }

    f.render_widget(
        Paragraph::new(status_line(&app.sync_status, app.pending, app.conflicted)).style(theme.status),
        rows[1],
    );

    if let Some(search_state) = &app.search {
        search::render(f, search_state);
    }
    if let Some(input_state) = &app.input {
        input::render(f, input_state);
    }
    if let Some(props_state) = &app.props {
        props::render(f, props_state);
    }
    if let Some(confirm_state) = &app.confirm {
        confirm::render(f, confirm_state);
    }
    if let Some(palette_state) = &app.palette {
        palette::render(f, palette_state);
    }
    if app.help_open {
        help::render(f, &app.keymap);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_appends_pending_count() {
        assert_eq!(status_line(&SyncStatus::Idle { updated: 0 }, 0, 0), "✓ synced");
        assert_eq!(status_line(&SyncStatus::Idle { updated: 0 }, 3, 0), "✓ synced · 3 pending");
    }

    #[test]
    fn status_line_appends_conflicted_count() {
        assert_eq!(
            status_line(&SyncStatus::Idle { updated: 0 }, 1, 2),
            "✓ synced · 1 pending · ⚠ 2 conflicted"
        );
    }
}
