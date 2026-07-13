use notion_store::OpRec;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub struct QueueView {
    pub ops: Vec<OpRec>,
    pub cursor: usize,
    pub list_state: ListState,
}

impl QueueView {
    pub fn new(ops: Vec<OpRec>) -> QueueView {
        QueueView {
            ops,
            cursor: 0,
            list_state: ListState::default(),
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.ops.is_empty() {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, self.ops.len() as isize - 1) as usize;
    }

    pub fn selected(&self) -> Option<&OpRec> {
        self.ops.get(self.cursor)
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &mut QueueView, focused: bool, theme: &Theme) {
    let items: Vec<ListItem> = view
        .ops
        .iter()
        .map(|op| {
            let err = op.error.as_deref().unwrap_or("");
            ListItem::new(format!(
                "#{} {} {} [{}] {}",
                op.seq, op.op_type, op.target_id, op.state, err
            ))
        })
        .collect();
    let highlight = if focused {
        theme.highlight
    } else {
        ratatui::style::Style::default()
    };
    view.list_state.select(Some(view.cursor));
    f.render_stateful_widget(
        List::new(items).highlight_style(highlight).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme.border)
                .title(Span::styled(
                    " queue (r retry · p keep mine · t take theirs · e merge) ",
                    theme.title,
                )),
        ),
        area,
        &mut view.list_state,
    );
}
