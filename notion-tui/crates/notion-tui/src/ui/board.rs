use notion_store::{DataSourceRec, RowRec};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;
use serde_json::Value;

use crate::ui::table::cell_text;
use crate::ui::theme::Theme;

pub struct BoardView {
    pub ds: DataSourceRec,
    pub group_prop: String,
    pub group_type: String,
    pub columns: Vec<String>,
    pub rows: Vec<RowRec>,
    pub col: usize,
    pub card: usize,
    pub list_state: ListState,
}

/// Picks the grouping property: first `status`, else first `select`, in schema
/// key order. Returns (property name, property type).
pub fn group_property(schema_json: &str) -> Option<(String, String)> {
    let schema: Value = serde_json::from_str(schema_json).ok()?;
    let map = schema.as_object()?;
    for wanted in ["status", "select"] {
        if let Some((name, _)) = map.iter().find(|(_, def)| def["type"] == wanted) {
            return Some((name.clone(), wanted.to_string()));
        }
    }
    None
}

fn schema_options(schema_json: &str, prop: &str, prop_type: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(schema_json).unwrap_or_default();
    schema[prop][prop_type]["options"]
        .as_array()
        .map(|a| a.iter().filter_map(|o| o["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

impl BoardView {
    pub fn new(ds: DataSourceRec, rows: Vec<RowRec>) -> BoardView {
        let (group_prop, group_type) =
            group_property(&ds.schema_json).unwrap_or_else(|| ("".into(), "".into()));
        let mut columns = schema_options(&ds.schema_json, &group_prop, &group_type);
        columns.push("(none)".to_string());
        BoardView { ds, group_prop, group_type, columns, rows, col: 0, card: 0, list_state: ListState::default() }
    }

    fn row_group(&self, row: &RowRec) -> String {
        let props: Value = serde_json::from_str(&row.properties).unwrap_or_default();
        let name = props[&self.group_prop][&self.group_type]["name"].as_str().unwrap_or("");
        if name.is_empty() { "(none)".to_string() } else { name.to_string() }
    }

    pub fn cards_in(&self, col: usize) -> Vec<&RowRec> {
        let Some(col_name) = self.columns.get(col) else { return Vec::new() };
        self.rows.iter().filter(|r| &self.row_group(r) == col_name).collect()
    }

    pub fn selected_row_id(&self) -> Option<String> {
        self.cards_in(self.col).get(self.card).map(|r| r.id.clone())
    }

    pub fn move_cursor_card(&mut self, delta: isize) {
        let len = self.cards_in(self.col).len();
        if len == 0 { return; }
        self.card = (self.card as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn move_cursor_col(&mut self, delta: isize) {
        if self.columns.is_empty() { return; }
        self.col = (self.col as isize + delta).clamp(0, self.columns.len() as isize - 1) as usize;
        self.card = self.card.min(self.cards_in(self.col).len().saturating_sub(1));
    }

    /// Returns (row_id, new option name) for the optimistic write, or None if
    /// there is no selected card or the move runs off the board. Moving into
    /// the trailing `(none)` column clears the value (empty string sentinel).
    pub fn move_card(&mut self, delta: isize) -> Option<(String, String)> {
        let row_id = self.selected_row_id()?;
        let target = self.col as isize + delta;
        if target < 0 || target as usize >= self.columns.len() {
            return None;
        }
        let target = target as usize;
        let value = if self.columns[target] == "(none)" { String::new() } else { self.columns[target].clone() };
        self.col = target;
        Some((row_id, value))
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &mut BoardView, focused: bool, theme: &Theme) {
    let n = view.columns.len().max(1) as u32;
    let constraints: Vec<Constraint> = view.columns.iter().map(|_| Constraint::Ratio(1, n)).collect();
    let cols = Layout::default().direction(Direction::Horizontal).constraints(constraints).split(area);
    let active_col = view.col;
    let active_card = view.card;
    for (ci, rect) in cols.iter().enumerate() {
        let cards = view.cards_in(ci);
        let items: Vec<ListItem> = cards
            .iter()
            .map(|r| {
                let props: Value = serde_json::from_str(&r.properties).unwrap_or_default();
                let title = props
                    .as_object()
                    .and_then(|m| m.values().find(|p| p["type"] == "title"))
                    .map(cell_text)
                    .unwrap_or_default();
                ListItem::new(title)
            })
            .collect();
        let title = format!(" {} ({}) ", view.columns[ci], cards.len());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.border)
            .title(ratatui::text::Span::styled(title, theme.title));
        if focused && ci == active_col {
            view.list_state.select(Some(active_card));
            f.render_stateful_widget(
                List::new(items).highlight_style(theme.highlight).block(block),
                *rect,
                &mut view.list_state,
            );
        } else {
            f.render_widget(List::new(items).block(block), *rect);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{DataSourceRec, RowRec};
    use serde_json::json;

    fn ds() -> DataSourceRec {
        DataSourceRec {
            id: "ds".into(), database_id: "db".into(), title: "Tasks".into(),
            schema_json: json!({
                "Name": {"type": "title"},
                "Status": {"type": "status", "status": {"options": [
                    {"name": "Todo"}, {"name": "Doing"}, {"name": "Done"}]}}
            }).to_string(),
            last_edited_time: "t".into(),
        }
    }

    fn row(id: &str, name: &str, status: Option<&str>) -> RowRec {
        let status_val = match status {
            Some(s) => json!({"type": "status", "status": {"name": s}}),
            None => json!({"type": "status", "status": null}),
        };
        RowRec {
            id: id.into(), data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": name}]},
                "Status": status_val
            }).to_string(),
            last_edited_time: "t".into(), archived: false,
        }
    }

    #[test]
    fn groups_rows_by_status_options_plus_none_column() {
        let v = BoardView::new(ds(), vec![row("r1", "A", Some("Todo")), row("r2", "B", Some("Done")), row("r3", "C", None)]);
        assert_eq!(v.group_prop, "Status");
        assert_eq!(v.columns, vec!["Todo", "Doing", "Done", "(none)"]);
        assert_eq!(v.cards_in(0).len(), 1);
        assert_eq!(v.cards_in(1).len(), 0);
        assert_eq!(v.cards_in(3)[0].id, "r3");
    }

    #[test]
    fn move_card_returns_row_and_target_option() {
        let mut v = BoardView::new(ds(), vec![row("r1", "A", Some("Todo"))]);
        v.col = 0;
        v.card = 0;
        assert_eq!(v.move_card(1), Some(("r1".to_string(), "Doing".to_string())));
        // Moving off either end is a no-op:
        v.col = 3; // (none) column is empty now locally — nothing to move
        assert_eq!(v.move_card(1), None);
    }

    #[test]
    fn no_groupable_property_means_no_board() {
        let plain = DataSourceRec {
            id: "ds".into(), database_id: "db".into(), title: "T".into(),
            schema_json: json!({"Name": {"type": "title"}}).to_string(),
            last_edited_time: "t".into(),
        };
        assert!(group_property(&plain.schema_json).is_none());
    }
}
