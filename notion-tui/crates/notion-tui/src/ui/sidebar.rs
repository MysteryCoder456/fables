use std::collections::HashSet;

use notion_store::{NodeKind, TreeNode};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub struct VisibleNode<'a> {
    pub node: &'a TreeNode,
    pub depth: usize,
}

pub struct SidebarState {
    pub nodes: Vec<TreeNode>,
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub hidden: bool,
    pub list_state: ListState,
}

impl SidebarState {
    pub fn new(nodes: Vec<TreeNode>) -> SidebarState {
        SidebarState {
            nodes,
            collapsed: HashSet::new(),
            cursor: 0,
            hidden: false,
            list_state: ListState::default(),
        }
    }

    pub fn visible(&self) -> Vec<VisibleNode<'_>> {
        let mut out = Vec::new();
        // Pages: DFS from roots (parent_id None or parent not present as a page node).
        let page_ids: HashSet<&str> = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Page)
            .map(|n| n.id.as_str())
            .collect();
        let roots = self.nodes.iter().filter(|n| {
            n.kind == NodeKind::Page
                && n.parent_id.as_deref().is_none_or(|p| !page_ids.contains(p))
        });
        for root in roots {
            self.push_subtree(root, 0, &mut out);
        }
        for ds in self.nodes.iter().filter(|n| n.kind == NodeKind::DataSource) {
            out.push(VisibleNode { node: ds, depth: 0 });
        }
        out
    }

    fn push_subtree<'a>(&'a self, node: &'a TreeNode, depth: usize, out: &mut Vec<VisibleNode<'a>>) {
        out.push(VisibleNode { node, depth });
        if self.collapsed.contains(&node.id) {
            return;
        }
        for child in self.nodes.iter().filter(|n| {
            n.kind == NodeKind::Page && n.parent_id.as_deref() == Some(node.id.as_str())
        }) {
            self.push_subtree(child, depth + 1, out);
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn toggle_collapse(&mut self) {
        if let Some(node) = self.selected() {
            let id = node.id.clone();
            if !self.collapsed.remove(&id) {
                self.collapsed.insert(id);
            }
        }
    }

    pub fn selected(&self) -> Option<&TreeNode> {
        self.visible().get(self.cursor).map(|v| v.node)
    }
}

pub fn render(f: &mut Frame, area: Rect, state: &mut SidebarState, focused: bool, theme: &Theme) {
    let items: Vec<ListItem> = state
        .visible()
        .iter()
        .map(|v| {
            let marker = match v.node.kind {
                NodeKind::Page => "▸ ",
                NodeKind::DataSource => "▦ ",
            };
            let line = format!("{}{}{}", "  ".repeat(v.depth), marker, v.node.title);
            ListItem::new(Line::from(line))
        })
        .collect();
    let title = if focused { " notion ● " } else { " notion " };
    let highlight = if focused { theme.highlight } else { ratatui::style::Style::default() };
    state.list_state.select(Some(state.cursor));
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(highlight)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.border)
                    .title(Span::styled(title, theme.title)),
            ),
        area,
        &mut state.list_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nodes() -> Vec<TreeNode> {
        vec![
            TreeNode { id: "a".into(), title: "Alpha".into(), parent_id: None, kind: NodeKind::Page },
            TreeNode {
                id: "a1".into(),
                title: "Alpha child".into(),
                parent_id: Some("a".into()),
                kind: NodeKind::Page,
            },
            TreeNode { id: "b".into(), title: "Beta".into(), parent_id: None, kind: NodeKind::Page },
            TreeNode {
                id: "ds".into(),
                title: "Tasks".into(),
                parent_id: None,
                kind: NodeKind::DataSource,
            },
        ]
    }

    #[test]
    fn visible_nests_children_and_lists_data_sources_last() {
        let s = SidebarState::new(nodes());
        let v = s.visible();
        let titles: Vec<(&str, usize)> = v.iter().map(|n| (n.node.title.as_str(), n.depth)).collect();
        assert_eq!(
            titles,
            vec![("Alpha", 0), ("Alpha child", 1), ("Beta", 0), ("Tasks", 0)]
        );
    }

    #[test]
    fn collapse_hides_children() {
        let mut s = SidebarState::new(nodes());
        s.cursor = 0; // Alpha
        s.toggle_collapse();
        let titles: Vec<&str> = s.visible().iter().map(|n| n.node.title.as_str()).collect();
        assert_eq!(titles, vec!["Alpha", "Beta", "Tasks"]);
    }

    #[test]
    fn cursor_clamps() {
        let mut s = SidebarState::new(nodes());
        s.move_cursor(-5);
        assert_eq!(s.cursor, 0);
        s.move_cursor(100);
        assert_eq!(s.cursor, s.visible().len() - 1);
    }

    fn many_nodes(n: usize) -> Vec<TreeNode> {
        (0..n)
            .map(|i| TreeNode {
                id: format!("p{i}"),
                title: format!("Page {i}"),
                parent_id: None,
                kind: NodeKind::Page,
            })
            .collect()
    }

    #[test]
    fn render_scrolls_so_cursor_stays_visible() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut s = SidebarState::new(many_nodes(30));
        s.cursor = 29;
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(20, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut s, true, &theme)).unwrap();
        let content: String = term.backend().buffer().content.iter().map(|c| c.symbol()).collect();
        assert!(content.contains("Page 29"), "cursor's row should be visible:\n{content}");
        assert!(!content.contains("Page 0"), "top row should have scrolled out:\n{content}");
    }
}
