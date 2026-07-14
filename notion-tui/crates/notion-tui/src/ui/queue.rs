use notion_store::OpRec;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub struct QueueView {
    pub ops: Vec<OpRec>,
    /// Parallel to `ops`: human summaries from `crate::describe::describe_op`.
    pub summaries: Vec<String>,
    pub cursor: usize,
    pub list_state: ListState,
}

impl QueueView {
    pub fn new(ops: Vec<OpRec>, summaries: Vec<String>) -> QueueView {
        QueueView {
            ops,
            summaries,
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
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Span::styled(
            " queue (r retry · p keep mine · t take theirs · e merge) ",
            theme.title,
        ));
    if view.ops.is_empty() {
        f.render_widget(
            ratatui::widgets::Paragraph::new("queue is empty — local edits appear here until synced")
                .style(ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::DIM))
                .block(block),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = view
        .ops
        .iter()
        .enumerate()
        .map(|(i, op)| {
            let err = op.error.as_deref().unwrap_or("");
            let summary = view
                .summaries
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("{} {}", op.op_type, op.target_id));
            ListItem::new(format!("#{} {} [{}] {}", op.seq, summary, op.state, err))
        })
        .collect();
    let highlight = if focused {
        theme.highlight
    } else {
        ratatui::style::Style::default()
    };
    view.list_state.select(Some(view.cursor));
    f.render_stateful_widget(
        List::new(items).highlight_style(highlight).block(block),
        area,
        &mut view.list_state,
    );
}
