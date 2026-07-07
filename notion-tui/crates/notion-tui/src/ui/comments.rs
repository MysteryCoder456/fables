use crossterm::event::{KeyCode, KeyEvent};
use notion_store::CommentRec;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub struct CommentsState {
    pub parent_id: String,
    pub parent_kind: String,
    pub items: Vec<CommentRec>,
    pub cursor: usize,
    pub list_state: ListState,
}

pub enum CommentsAction {
    None,
    Close,
    NewThread,
    Reply,
}

impl CommentsState {
    pub fn on_key(&mut self, key: KeyEvent) -> CommentsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('c') => CommentsAction::Close,
            KeyCode::Char('n') => CommentsAction::NewThread,
            KeyCode::Char('r') => CommentsAction::Reply,
            KeyCode::Char('j') | KeyCode::Down => {
                if !self.items.is_empty() {
                    self.cursor = (self.cursor + 1).min(self.items.len() - 1);
                }
                CommentsAction::None
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                CommentsAction::None
            }
            _ => CommentsAction::None,
        }
    }

    pub fn selected_thread(&self) -> Option<String> {
        self.items.get(self.cursor).and_then(|c| c.thread_id.clone())
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &mut CommentsState, theme: &Theme) {
    let items: Vec<ListItem> =
        state.items.iter().map(|c| ListItem::new(format!("{}: {}", c.author, c.body))).collect();
    state.list_state.select(Some(state.cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(theme.highlight)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.border)
                    .title(Span::styled(" comments (n new · r reply) ", theme.title)),
            ),
        area,
        &mut state.list_state,
    );
}
