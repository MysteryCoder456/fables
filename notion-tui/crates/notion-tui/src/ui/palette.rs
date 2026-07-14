use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::ui::textline::TextLine;
use crate::ui::theme::Theme;

pub const COMMANDS: &[&str] = &[
    "help",
    "queue",
    "board",
    "table",
    "quit",
    "rename",
    "move page",
    "group by",
    "sync now",
];

pub struct PaletteState {
    pub input: TextLine,
    pub cursor: usize,
    pub list_state: ListState,
}

pub enum PaletteAction {
    None,
    Changed,
    Run(&'static str),
    Close,
}

impl PaletteState {
    pub fn new() -> PaletteState {
        PaletteState {
            input: TextLine::new(""),
            cursor: 0,
            list_state: ListState::default(),
        }
    }

    pub fn matches(&self) -> Vec<&'static str> {
        let items: Vec<(&'static str, String)> = COMMANDS.iter().map(|c| (*c, c.to_string())).collect();
        crate::fuzzy::subsequence_rank(self.input.text(), items)
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PaletteAction {
        match key.code {
            KeyCode::Esc => PaletteAction::Close,
            KeyCode::Enter => match self.matches().get(self.cursor) {
                Some(cmd) => PaletteAction::Run(cmd),
                None => PaletteAction::Close,
            },
            KeyCode::Down => {
                let n = self.matches().len();
                if n > 0 {
                    self.cursor = (self.cursor + 1).min(n - 1);
                }
                PaletteAction::None
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                PaletteAction::None
            }
            _ => match self.input.on_key(key) {
                crate::ui::textline::TextLineEvent::Edited => {
                    self.cursor = 0;
                    PaletteAction::Changed
                }
                _ => PaletteAction::None,
            },
        }
    }
}

impl Default for PaletteState {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

pub fn render(f: &mut Frame, state: &mut PaletteState, theme: &Theme) {
    let area = f.area();
    let popup = centered_rect((area.width / 2).clamp(24, 50), 12, area);
    f.render_widget(Clear, popup);
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);
    f.render_widget(
        Paragraph::new(state.input.text()).block(crate::ui::theme::popup_block(" : ".into(), theme)),
        inner[0],
    );
    f.set_cursor_position(ratatui::layout::Position::new(
        inner[0].x + 1 + state.input.cursor_cols().min(inner[0].width.saturating_sub(2)),
        inner[0].y + 1,
    ));
    let items: Vec<ListItem> = state.matches().iter().map(|c| ListItem::new(*c)).collect();
    state.list_state.select(Some(state.cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(theme.highlight)
            .block(crate::ui::theme::popup_block(" : ".into(), theme)),
        inner[1],
        &mut state.list_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_are_subsequence_not_substring() {
        let mut p = PaletteState::new();
        p.input = TextLine::new("bd"); // not a substring of "board", but is a subsequence
        assert!(p.matches().contains(&"board"));
    }

    #[test]
    fn non_subsequence_input_matches_nothing() {
        let mut p = PaletteState::new();
        p.input = TextLine::new("zzz");
        assert!(p.matches().is_empty());
    }
}
