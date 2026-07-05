use std::collections::HashSet;

use notion_store::{BlockRec, PageRec};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct BlockLine {
    pub block_id: String,
    pub text: String,
    pub indent: usize,
    pub link_page_id: Option<String>,
}

pub struct PageView {
    pub page: PageRec,
    pub blocks: Vec<BlockRec>,
    pub cursor: usize,
    pub scroll: u16,
    pub collapsed_toggles: HashSet<String>,
}

impl PageView {
    pub fn new(page: PageRec, blocks: Vec<BlockRec>) -> PageView {
        PageView { page, blocks, cursor: 0, scroll: 0, collapsed_toggles: HashSet::new() }
    }

    pub fn lines(&self) -> Vec<BlockLine> {
        let mut out = Vec::new();
        self.push_children(None, 0, &mut out);
        out
    }

    fn push_children(&self, parent: Option<&str>, indent: usize, out: &mut Vec<BlockLine>) {
        let mut numbered = 0usize;
        let siblings: Vec<&BlockRec> = self
            .blocks
            .iter()
            .filter(|b| b.parent_block_id.as_deref() == parent)
            .collect();
        for b in siblings {
            if b.block_type == "numbered_list_item" {
                numbered += 1;
            } else {
                numbered = 0;
            }
            let payload: serde_json::Value = serde_json::from_str(&b.payload).unwrap_or_default();
            let collapsed = self.collapsed_toggles.contains(&b.id);
            let (text, link) = match b.block_type.as_str() {
                "heading_1" => (format!("# {}", b.plain_text), None),
                "heading_2" => (format!("## {}", b.plain_text), None),
                "heading_3" => (format!("### {}", b.plain_text), None),
                "to_do" => {
                    let mark = if payload["checked"].as_bool().unwrap_or(false) { "x" } else { " " };
                    (format!("[{mark}] {}", b.plain_text), None)
                }
                "bulleted_list_item" => (format!("• {}", b.plain_text), None),
                "numbered_list_item" => (format!("{numbered}. {}", b.plain_text), None),
                "toggle" => {
                    let arrow = if collapsed { "▸" } else { "▾" };
                    (format!("{arrow} {}", b.plain_text), None)
                }
                "code" => (format!("│ {}", b.plain_text), None),
                "quote" => (format!("┃ {}", b.plain_text), None),
                "callout" => (format!("💡 {}", b.plain_text), None),
                "divider" => ("────────".to_string(), None),
                "child_page" | "child_database" => (format!("→ {}", b.plain_text), Some(b.id.clone())),
                "paragraph" => (b.plain_text.clone(), None),
                _ => (format!("⍰ {}", b.plain_text), None),
            };
            out.push(BlockLine { block_id: b.id.clone(), text, indent, link_page_id: link });
            if !(b.block_type == "toggle" && collapsed) {
                self.push_children(Some(&b.id), indent + 1, out);
            }
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.lines().len();
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn toggle_at_cursor(&mut self) {
        if let Some(line) = self.lines().get(self.cursor) {
            let id = line.block_id.clone();
            let is_toggle = self.blocks.iter().any(|b| b.id == id && b.block_type == "toggle");
            if is_toggle && !self.collapsed_toggles.remove(&id) {
                self.collapsed_toggles.insert(id);
            }
        }
    }

    pub fn link_at_cursor(&self) -> Option<String> {
        self.lines().get(self.cursor).and_then(|l| l.link_page_id.clone())
    }

    pub fn block_id_at_cursor(&self) -> Option<String> {
        self.lines().get(self.cursor).map(|l| l.block_id.clone())
    }

    pub fn todo_block_at_cursor(&self) -> Option<String> {
        let id = self.block_id_at_cursor()?;
        self.blocks
            .iter()
            .any(|b| b.id == id && b.block_type == "to_do")
            .then_some(id)
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &PageView, focused: bool) {
    let items: Vec<ListItem> = view
        .lines()
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let mut item = ListItem::new(Line::from(format!("{}{}", "  ".repeat(l.indent), l.text)));
            if i == view.cursor && focused {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    let title = format!(" {} ", view.page.title);
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, parent: Option<&str>, ord: i64, ty: &str, text: &str, payload: &str) -> BlockRec {
        BlockRec {
            id: id.into(),
            page_id: "p".into(),
            parent_block_id: parent.map(Into::into),
            ordinal: ord,
            block_type: ty.into(),
            payload: payload.into(),
            plain_text: text.into(),
            has_children: false,
        }
    }

    fn page() -> PageRec {
        PageRec {
            id: "p".into(),
            parent_type: "workspace".into(),
            parent_id: None,
            title: "T".into(),
            icon: None,
            archived: false,
            last_edited_time: "t".into(),
        }
    }

    #[test]
    fn renders_prefixes_and_numbering() {
        let v = PageView::new(
            page(),
            vec![
                rec("h", None, 0, "heading_1", "Title", "{}"),
                rec("t1", None, 1, "to_do", "done thing", r#"{"checked": true}"#),
                rec("n1", None, 2, "numbered_list_item", "first", "{}"),
                rec("n2", None, 3, "numbered_list_item", "second", "{}"),
                rec("d", None, 4, "divider", "", "{}"),
            ],
        );
        let texts: Vec<String> = v.lines().iter().map(|l| l.text.clone()).collect();
        assert_eq!(texts[0], "# Title");
        assert_eq!(texts[1], "[x] done thing");
        assert_eq!(texts[2], "1. first");
        assert_eq!(texts[3], "2. second");
        assert!(texts[4].starts_with("────"));
    }

    #[test]
    fn toggle_collapse_hides_children() {
        let mut v = PageView::new(
            page(),
            vec![
                rec("tg", None, 0, "toggle", "More", "{}"),
                rec("c1", Some("tg"), 0, "paragraph", "hidden", "{}"),
            ],
        );
        assert_eq!(v.lines().len(), 2);
        assert_eq!(v.lines()[1].indent, 1);
        v.cursor = 0;
        v.toggle_at_cursor();
        assert_eq!(v.lines().len(), 1);
        assert!(v.lines()[0].text.starts_with("▸"));
    }

    #[test]
    fn child_page_is_a_link() {
        let v = PageView::new(page(), vec![rec("cp", None, 0, "child_page", "Sub page", "{}")]);
        assert_eq!(v.lines()[0].link_page_id.as_deref(), Some("cp"));
        assert_eq!(v.lines()[0].text, "→ Sub page");
    }
}
