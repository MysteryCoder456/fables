use std::collections::HashSet;

use notion_store::{NodeKind, TreeNode};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct VisibleNode<'a> {
    pub node: &'a TreeNode,
    pub depth: usize,
}

pub struct SidebarState {
    pub nodes: Vec<TreeNode>,
    pub collapsed: HashSet<String>,
    pub cursor: usize,
    pub hidden: bool,
}

impl SidebarState {
    pub fn new(nodes: Vec<TreeNode>) -> SidebarState {
        SidebarState { nodes, collapsed: HashSet::new(), cursor: 0, hidden: false }
    }

    pub fn visible(&self) -> Vec<VisibleNode> {
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
                && n.parent_id.as_deref().map_or(true, |p| !page_ids.contains(p))
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

pub fn render(f: &mut Frame, area: Rect, state: &SidebarState, focused: bool) {
    let items: Vec<ListItem> = state
        .visible()
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let marker = match v.node.kind {
                NodeKind::Page => "▸ ",
                NodeKind::DataSource => "▦ ",
            };
            let line = format!("{}{}{}", "  ".repeat(v.depth), marker, v.node.title);
            let mut item = ListItem::new(Line::from(line));
            if i == state.cursor && focused {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    let title = if focused { " notion ● " } else { " notion " };
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        area,
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
}
