use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::ui::textline::TextLine;
use crate::ui::theme::{popup_block, Theme};

pub struct PickerState {
    pub title: String,
    pub input: TextLine,
    pub items: Vec<(String, String)>,
    pub cursor: usize,
    pub list_state: ListState,
}

pub enum PickerAction {
    None,
    Changed,
    Close,
    Choose(String),
}

impl PickerState {
    pub fn new(title: impl Into<String>, items: Vec<(String, String)>) -> PickerState {
        PickerState {
            title: title.into(),
            input: TextLine::new(""),
            items,
            cursor: 0,
            list_state: ListState::default(),
        }
    }

    pub fn matches(&self) -> Vec<&(String, String)> {
        let candidates: Vec<(&(String, String), String)> =
            self.items.iter().map(|it| (it, it.1.clone())).collect();
        crate::fuzzy::subsequence_rank(self.input.text(), candidates)
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Esc => PickerAction::Close,
            KeyCode::Enter => match self.matches().get(self.cursor) {
                Some((id, _)) => PickerAction::Choose(id.clone()),
                None => PickerAction::Close,
            },
            KeyCode::Down => {
                let n = self.matches().len();
                if n > 0 {
                    self.cursor = (self.cursor + 1).min(n - 1);
                }
                PickerAction::None
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                PickerAction::None
            }
            _ => match self.input.on_key(key) {
                crate::ui::textline::TextLineEvent::Edited => {
                    self.cursor = 0;
                    PickerAction::Changed
                }
                _ => PickerAction::None,
            },
        }
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

pub fn render(f: &mut Frame, state: &mut PickerState, theme: &Theme) {
    let area = f.area();
    let popup = centered_rect((area.width * 2 / 3).clamp(24, 60), 14, area);
    f.render_widget(Clear, popup);
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);
    f.render_widget(
        Paragraph::new(state.input.text()).block(popup_block(format!(" {} ", state.title), theme)),
        inner[0],
    );
    f.set_cursor_position(ratatui::layout::Position::new(
        inner[0].x + 1 + state.input.cursor_cols().min(inner[0].width.saturating_sub(2)),
        inner[0].y + 1,
    ));
    let items: Vec<ListItem> = state
        .matches()
        .iter()
        .map(|(_, label)| ListItem::new(label.clone()))
        .collect();
    state.list_state.select(Some(state.cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(theme.highlight)
            .block(popup_block(String::new(), theme)),
        inner[1],
        &mut state.list_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<(String, String)> {
        vec![
            ("id1".into(), "Roadmap".into()),
            ("id2".into(), "Retro Notes".into()),
            ("id3".into(), "Budget".into()),
        ]
    }

    #[test]
    fn typing_filters_by_subsequence_on_label() {
        let mut p = PickerState::new("move to…", items());
        for c in "rmap".chars() {
            p.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let labels: Vec<&str> = p.matches().iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, vec!["Roadmap"]);
    }

    #[test]
    fn enter_chooses_the_highlighted_id() {
        let mut p = PickerState::new("move to…", items());
        match p.on_key(KeyEvent::from(KeyCode::Enter)) {
            PickerAction::Choose(id) => assert_eq!(id, "id1"),
            _ => panic!("expected Choose"),
        }
    }

    #[test]
    fn esc_closes() {
        let mut p = PickerState::new("move to…", items());
        assert!(matches!(
            p.on_key(KeyEvent::from(KeyCode::Esc)),
            PickerAction::Close
        ));
    }
}
