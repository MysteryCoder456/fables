use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub struct ConfirmState {
    pub message: String,
    pub ids: Vec<String>,
    pub kind: ConfirmKind,
}

/// What a `y` on the confirm popup commits the user to.
pub enum ConfirmKind {
    /// Delete protected blocks whose marker lines were removed in the editor.
    DeleteProtected,
    /// Apply an edited document whose parse produced warnings (e.g. an
    /// unclosed code fence swallowing the rest of the page).
    ApplyDespiteWarnings,
    /// `dd` on a block in the page view.
    DeleteBlock,
    /// `dd` on a row in table/board views.
    DeleteRow,
}

pub enum ConfirmAction {
    None,
    Yes,
    No,
}

impl ConfirmState {
    pub fn on_key(&mut self, key: KeyEvent) -> ConfirmAction {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmAction::Yes,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => ConfirmAction::No,
            _ => ConfirmAction::None,
        }
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

pub fn render(f: &mut Frame, state: &ConfirmState, theme: &Theme) {
    let area = f.area();
    let popup = centered_rect((area.width * 2 / 3).clamp(30, 70), 5, area);
    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(format!("{}\n(y/n)", state.message))
            .block(crate::ui::theme::popup_block(" confirm ".to_string(), theme)),
        popup,
    );
}
