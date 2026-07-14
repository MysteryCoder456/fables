pub mod board;
pub mod comments;
pub mod confirm;
pub mod help;
pub mod input;
pub mod page;
pub mod palette;
pub mod picker;
pub mod props;
pub mod queue;
pub mod search;
pub mod sidebar;
pub mod table;
pub mod textline;
pub mod theme;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::app::{App, Focus, View};
use notion_sync::SyncStatus;

/// Snapshot of where each interactive region was rendered this frame, used by
/// `dispatch_mouse` to hit-test clicks against exactly what the user sees.
#[derive(Clone)]
pub struct LayoutRects {
    pub sidebar: Option<Rect>,
    pub main: Rect,
    pub table_header_y: Option<u16>,
    pub table_col_x: Vec<u16>,
    pub board_columns: Vec<Rect>,
}

impl Default for LayoutRects {
    fn default() -> Self {
        LayoutRects {
            sidebar: None,
            main: Rect::new(0, 0, 0, 0),
            table_header_y: None,
            table_col_x: Vec::new(),
            board_columns: Vec::new(),
        }
    }
}

/// Renders a `u32` with thousands separators, e.g. `1893` -> `"1,893"`.
fn with_thousands_separators(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn status_line(
    breadcrumb: Option<&str>,
    status: &SyncStatus,
    pending: u32,
    conflicted: u32,
    notice: Option<&str>,
    hints: Option<&str>,
    pending_key: Option<char>,
) -> String {
    let base = match status {
        SyncStatus::Starting => "starting…".into(),
        SyncStatus::Syncing { done, total } if *total > 0 => format!(
            "⟳ syncing {}/{} pages",
            with_thousands_separators(*done),
            with_thousands_separators(*total)
        ),
        SyncStatus::Syncing { .. } => "⟳ syncing…".into(),
        SyncStatus::Idle { updated: 0 } => "✓ synced".into(),
        SyncStatus::Idle { updated } => format!("✓ synced ({updated} updated)"),
        SyncStatus::Offline => "⚠ offline".into(),
        SyncStatus::Failed(msg) => format!("✗ sync failed: {msg}"),
    };
    let mut out = match breadcrumb {
        Some(b) => format!("{b} · {base}"),
        None => base,
    };
    if pending > 0 {
        out = format!("{out} · {pending} pending");
    }
    if conflicted > 0 {
        out = format!("{out} · ⚠ {conflicted} conflicted");
    }
    if let Some(notice) = notice {
        out = format!("{out} · {notice}");
    }
    if let Some(hints) = hints {
        out = format!("{out} · {hints}");
    }
    if let Some(k) = pending_key {
        out = format!("{out} · {k} pending");
    }
    out
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let theme = app.theme.clone();
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

    let sidebar_focused = matches!(app.focus, Focus::Sidebar);
    if !app.sidebar.hidden {
        sidebar::render(f, cols[0], &mut app.sidebar, sidebar_focused, &theme);
    }
    let full_main = *cols.last().unwrap();
    let main_area = if let Some(comments_state) = &mut app.comments {
        let halves = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1), Constraint::Length(36)])
            .split(full_main);
        comments::render(f, halves[1], comments_state, &theme);
        halves[0]
    } else {
        full_main
    };
    let main_focused = matches!(app.focus, Focus::Main);
    match &mut app.view {
        View::Page(view) => page::render(f, main_area, view, main_focused, &theme),
        View::Table(view) => table::render(f, main_area, view, main_focused, &theme),
        View::Board(view) => board::render(f, main_area, view, main_focused, &theme),
        View::Queue(view) => queue::render(f, main_area, view, main_focused, &theme),
        View::Empty => f.render_widget(Block::default().borders(Borders::ALL), main_area),
    }

    let hints = matches!(app.view, View::Board(_)).then(|| help::board_hints(&app.keymap));
    f.render_widget(
        Paragraph::new(status_line(
            app.breadcrumb().as_deref(),
            &app.sync_status,
            app.pending,
            app.conflicted,
            app.notice.as_deref(),
            hints.as_deref(),
            app.pending_d.then_some('d'),
        ))
        .style(theme.status),
        rows[1],
    );

    if let Some(search_state) = &mut app.search {
        search::render(f, search_state, &theme);
    }
    if let Some(input_state) = &app.input {
        input::render(f, input_state, &theme);
    }
    if let Some(props_state) = &app.props {
        props::render(f, props_state, &theme);
    }
    if let Some(confirm_state) = &app.confirm {
        confirm::render(f, confirm_state, &theme);
    }
    if let Some(palette_state) = &mut app.palette {
        palette::render(f, palette_state, &theme);
    }
    if let Some(picker_state) = &mut app.picker {
        picker::render(f, picker_state, &theme);
    }
    if app.help_open {
        help::render(f, &app.keymap, &theme);
    }

    let mut layout = LayoutRects {
        sidebar: (!app.sidebar.hidden).then_some(cols[0]),
        main: main_area,
        ..LayoutRects::default()
    };
    if let View::Table(view) = &app.view {
        layout.table_header_y = Some(main_area.y + 1); // border + header row
        layout.table_col_x = table::column_x_starts(main_area, &view.columns);
    }
    if let View::Board(view) = &app.view {
        let n = view.columns.len().max(1) as u32;
        let constraints: Vec<Constraint> = view.columns.iter().map(|_| Constraint::Ratio(1, n)).collect();
        layout.board_columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(constraints)
            .split(main_area)
            .to_vec();
    }
    app.last_layout = layout;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_line_appends_pending_count() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 0, 0, None, None, None),
            "✓ synced"
        );
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 3, 0, None, None, None),
            "✓ synced · 3 pending"
        );
    }

    #[test]
    fn status_line_appends_conflicted_count() {
        assert_eq!(
            status_line(None, &SyncStatus::Idle { updated: 0 }, 1, 2, None, None, None),
            "✓ synced · 1 pending · ⚠ 2 conflicted"
        );
    }

    #[test]
    fn status_line_appends_notice() {
        assert_eq!(
            status_line(
                None,
                &SyncStatus::Idle { updated: 0 },
                0,
                0,
                Some("no table open to show as a board"),
                None,
                None
            ),
            "✓ synced · no table open to show as a board"
        );
    }

    #[test]
    fn status_line_appends_hints() {
        assert_eq!(
            status_line(
                None,
                &SyncStatus::Idle { updated: 0 },
                0,
                0,
                None,
                Some("J move card right"),
                None
            ),
            "✓ synced · J move card right"
        );
    }

    #[test]
    fn status_line_prepends_breadcrumb() {
        assert_eq!(
            status_line(
                Some("workspace → Roadmap"),
                &SyncStatus::Idle { updated: 0 },
                0,
                0,
                None,
                None,
                None
            ),
            "workspace → Roadmap · ✓ synced"
        );
    }

    #[test]
    fn status_line_appends_pending_key() {
        assert_eq!(
            status_line(
                None,
                &SyncStatus::Idle { updated: 0 },
                0,
                0,
                None,
                None,
                Some('d')
            ),
            "✓ synced · d pending"
        );
    }

    #[test]
    fn syncing_status_shows_done_over_total_with_thousands_separators() {
        assert_eq!(
            status_line(
                None,
                &SyncStatus::Syncing {
                    done: 240,
                    total: 1893
                },
                0,
                0,
                None,
                None,
                None
            ),
            "⟳ syncing 240/1,893 pages"
        );
        assert_eq!(
            status_line(
                None,
                &SyncStatus::Syncing { done: 0, total: 0 },
                0,
                0,
                None,
                None,
                None
            ),
            "⟳ syncing…"
        );
    }
}
