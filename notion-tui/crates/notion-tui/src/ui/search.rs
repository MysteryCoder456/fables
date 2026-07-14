use crossterm::event::{KeyCode, KeyEvent};
use notion_store::SearchHit;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

use crate::ui::textline::TextLine;
use crate::ui::theme::Theme;

pub struct SearchState {
    pub input: TextLine,
    pub results: Vec<SearchHit>,
    pub cursor: usize,
    pub list_state: ListState,
}

#[derive(Debug)]
pub enum SearchAction {
    None,
    QueryChanged,
    Open(String),
    Close,
}

impl SearchState {
    pub fn new() -> SearchState {
        SearchState {
            input: TextLine::new(""),
            results: Vec::new(),
            cursor: 0,
            list_state: ListState::default(),
        }
    }

    /// Typing edits the query; Up/Down move the result cursor (not j/k, which must
    /// remain typeable); Enter opens the selected hit; Esc closes the modal.
    pub fn on_key(&mut self, key: KeyEvent) -> SearchAction {
        match key.code {
            KeyCode::Esc => SearchAction::Close,
            KeyCode::Enter => match self.results.get(self.cursor) {
                Some(hit) => SearchAction::Open(hit.page_id.clone()),
                None => SearchAction::None,
            },
            KeyCode::Down => {
                if !self.results.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.results.len() - 1);
                }
                SearchAction::None
            }
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                SearchAction::None
            }
            _ => match self.input.on_key(key) {
                crate::ui::textline::TextLineEvent::Edited => SearchAction::QueryChanged,
                _ => SearchAction::None,
            },
        }
    }
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

pub fn render(f: &mut Frame, state: &mut SearchState, theme: &Theme) {
    let area = f.area();
    let popup_width = (area.width * 3 / 4).clamp(20, 80);
    let popup_height = (area.height * 3 / 4).clamp(6, 20);
    let popup = centered_rect(popup_width, popup_height, area);

    f.render_widget(Clear, popup);
    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup);

    f.render_widget(
        Paragraph::new(state.input.text()).block(crate::ui::theme::popup_block(" search ".into(), theme)),
        inner[0],
    );
    f.set_cursor_position(ratatui::layout::Position::new(
        inner[0].x + 1 + state.input.cursor_cols().min(inner[0].width.saturating_sub(2)),
        inner[0].y + 1,
    ));

    if !state.input.text().is_empty() && state.results.is_empty() {
        f.render_widget(
            Paragraph::new("no results")
                .style(ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM))
                .block(crate::ui::theme::popup_block(String::new(), theme)),
            inner[1],
        );
    } else {
        let items: Vec<ListItem> = state
            .results
            .iter()
            .map(|h| ListItem::new(format!("{}  {}", h.title, h.snippet)))
            .collect();
        state.list_state.select(Some(state.cursor));
        f.render_stateful_widget(
            List::new(items)
                .highlight_style(theme.highlight)
                .block(crate::ui::theme::popup_block(String::new(), theme)),
            inner[1],
            &mut state.list_state,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_updates_query_and_esc_closes() {
        let mut s = SearchState::new();
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Char('a'))),
            SearchAction::QueryChanged
        ));
        assert_eq!(s.input.text(), "a");
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Backspace)),
            SearchAction::QueryChanged
        ));
        assert_eq!(s.input.text(), "");
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Esc)),
            SearchAction::Close
        ));
    }

    #[test]
    fn enter_opens_selected() {
        let mut s = SearchState::new();
        s.results = vec![SearchHit {
            page_id: "p9".into(),
            title: "T".into(),
            snippet: "…".into(),
        }];
        match s.on_key(KeyEvent::from(KeyCode::Enter)) {
            SearchAction::Open(id) => assert_eq!(id, "p9"),
            other => panic!("expected Open, got {other:?}"),
        }
    }
}
