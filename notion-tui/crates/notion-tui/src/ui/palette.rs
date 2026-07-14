use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

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
    pub input: String,
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
            input: String::new(),
            cursor: 0,
            list_state: ListState::default(),
        }
    }

    pub fn matches(&self) -> Vec<&'static str> {
        let items: Vec<(&'static str, String)> = COMMANDS.iter().map(|c| (*c, c.to_string())).collect();
        crate::fuzzy::subsequence_rank(&self.input, items)
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
            KeyCode::Backspace => {
                self.input.pop();
                self.cursor = 0;
                PaletteAction::Changed
            }
            KeyCode::Char(c) => {
                self.input.push(c);
                self.cursor = 0;
                PaletteAction::Changed
            }
            _ => PaletteAction::None,
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

pub fn render(f: &mut Frame, state: &mut PaletteState) {
    let area = f.area();
    let popup = centered_rect((area.width / 2).clamp(24, 50), 12, area);
    f.render_widget(Clear, popup);
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);
    f.render_widget(
        Paragraph::new(state.input.as_str()).block(Block::default().borders(Borders::ALL).title(" : ")),
        inner[0],
    );
    let items: Vec<ListItem> = state.matches().iter().map(|c| ListItem::new(*c)).collect();
    state.list_state.select(Some(state.cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .block(Block::default().borders(Borders::ALL)),
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
        p.input = "bd".to_string(); // not a substring of "board", but is a subsequence
        assert!(p.matches().contains(&"board"));
    }

    #[test]
    fn non_subsequence_input_matches_nothing() {
        let mut p = PaletteState::new();
        p.input = "zzz".to_string();
        assert!(p.matches().is_empty());
    }
}
