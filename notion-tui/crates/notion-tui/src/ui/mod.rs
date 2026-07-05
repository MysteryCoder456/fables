pub mod input;
pub mod page;
pub mod props;
pub mod search;
pub mod sidebar;
pub mod table;

use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus, View};
use notion_sync::SyncStatus;

pub fn status_line(status: &SyncStatus, pending: u32) -> String {
    let base = match status {
        SyncStatus::Starting => "starting…".into(),
        SyncStatus::Syncing { .. } => "⟳ syncing…".into(),
        SyncStatus::Idle { updated: 0 } => "✓ synced".into(),
        SyncStatus::Idle { updated } => format!("✓ synced ({updated} updated)"),
        SyncStatus::Offline => "⚠ offline".into(),
        SyncStatus::Failed(msg) => format!("✗ sync failed: {msg}"),
    };
    if pending > 0 {
        format!("{base} · {pending} pending")
    } else {
        base
    }
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
        sidebar::render(f, cols[0], &app.sidebar, matches!(app.focus, Focus::Sidebar));
    }
    let main_area = *cols.last().unwrap();
    let main_focused = matches!(app.focus, Focus::Main);
    match &app.view {
        View::Page(view) => page::render(f, main_area, view, main_focused),
        View::Table(view) => table::render(f, main_area, view, main_focused),
        View::Empty => f.render_widget(Block::default().borders(Borders::ALL), main_area),
    }

    f.render_widget(
        Paragraph::new(status_line(&app.sync_status, app.pending))
            .style(Style::default().add_modifier(Modifier::REVERSED)),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_appends_pending_count() {
        assert_eq!(status_line(&SyncStatus::Idle { updated: 0 }, 0), "✓ synced");
        assert_eq!(status_line(&SyncStatus::Idle { updated: 0 }, 3), "✓ synced · 3 pending");
    }
}
