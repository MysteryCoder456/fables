use crossterm::event::{KeyCode, KeyEvent};
use notion_store::CommentRec;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct CommentsState {
    pub parent_id: String,
    pub parent_kind: String,
    pub items: Vec<CommentRec>,
    pub cursor: usize,
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

pub fn render(f: &mut Frame, area: Rect, state: &CommentsState) {
    let items: Vec<ListItem> = state
        .items
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut item = ListItem::new(format!("{}: {}", c.author, c.body));
            if i == state.cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    f.render_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(" comments (n new · r reply) ")),
        area,
    );
}
