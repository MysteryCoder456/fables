use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use notion_sync::SyncStatus;

pub fn status_line(status: &SyncStatus) -> String {
    match status {
        SyncStatus::Starting => "starting…".into(),
        SyncStatus::Syncing { .. } => "⟳ syncing…".into(),
        SyncStatus::Idle { updated: 0 } => "✓ synced".into(),
        SyncStatus::Idle { updated } => format!("✓ synced ({updated} updated)"),
        SyncStatus::Offline => "⚠ offline".into(),
        SyncStatus::Failed(msg) => format!("✗ sync failed: {msg}"),
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(1)])
        .split(rows[0]);

    f.render_widget(Block::default().borders(Borders::ALL).title(" notion "), cols[0]);
    f.render_widget(Block::default().borders(Borders::ALL), cols[1]);
    f.render_widget(
        Paragraph::new(status_line(&app.sync_status))
            .style(Style::default().add_modifier(Modifier::REVERSED)),
        rows[1],
    );
}
