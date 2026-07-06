use notion_store::OpRec;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct QueueView {
    pub ops: Vec<OpRec>,
    pub cursor: usize,
}

impl QueueView {
    pub fn new(ops: Vec<OpRec>) -> QueueView {
        QueueView { ops, cursor: 0 }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.ops.is_empty() { return; }
        self.cursor = (self.cursor as isize + delta).clamp(0, self.ops.len() as isize - 1) as usize;
    }

    pub fn selected(&self) -> Option<&OpRec> {
        self.ops.get(self.cursor)
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &QueueView, focused: bool) {
    let items: Vec<ListItem> = view
        .ops
        .iter()
        .enumerate()
        .map(|(i, op)| {
            let err = op.error.as_deref().unwrap_or("");
            let mut item = ListItem::new(format!(
                "#{} {} {} [{}] {}",
                op.seq, op.op_type, op.target_id, op.state, err
            ));
            if i == view.cursor && focused {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" queue (r retry · p keep mine · t take theirs · e merge) "),
        ),
        area,
    );
}
