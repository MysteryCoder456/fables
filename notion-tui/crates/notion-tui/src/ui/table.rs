use notion_store::{DataSourceRec, RowRec};
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Row as TRow, Table, TableState};
use ratatui::Frame;
use serde_json::Value;

use crate::ui::theme::Theme;

pub struct Column {
    pub name: String,
    pub prop_type: String,
}

pub struct TableView {
    pub ds: DataSourceRec,
    pub columns: Vec<Column>,
    pub rows: Vec<RowRec>,
    pub cursor: usize,
    pub sort: Option<(usize, bool)>,
    pub sort_col: usize,
    pub table_state: TableState,
}

fn rich_text_plain(v: &Value) -> String {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["plain_text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

pub fn cell_text(prop: &Value) -> String {
    match prop["type"].as_str().unwrap_or("") {
        "title" => rich_text_plain(&prop["title"]),
        "rich_text" => rich_text_plain(&prop["rich_text"]),
        "number" => prop["number"]
            .as_f64()
            .map(|n| if n.fract() == 0.0 { format!("{}", n as i64) } else { n.to_string() })
            .unwrap_or_default(),
        "select" => prop["select"]["name"].as_str().unwrap_or("").into(),
        "status" => prop["status"]["name"].as_str().unwrap_or("").into(),
        "multi_select" => prop["multi_select"]
            .as_array()
            .map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", "))
            .unwrap_or_default(),
        "date" => prop["date"]["start"].as_str().unwrap_or("").into(),
        "checkbox" => {
            if prop["checkbox"].as_bool().unwrap_or(false) {
                "☑".into()
            } else {
                "☐".into()
            }
        }
        "url" => prop["url"].as_str().unwrap_or("").into(),
        "email" => prop["email"].as_str().unwrap_or("").into(),
        "phone_number" => prop["phone_number"].as_str().unwrap_or("").into(),
        "people" => prop["people"].as_array().map(|a| format!("👤 {}", a.len())).unwrap_or_default(),
        _ => String::new(),
    }
}

/// Derives display columns from a data source schema: title property first,
/// the rest alphabetically.
pub fn schema_columns(schema_json: &str) -> Vec<Column> {
    let schema: Value = serde_json::from_str(schema_json).unwrap_or_default();
    let mut columns: Vec<Column> = schema
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(name, def)| Column {
                    name: name.clone(),
                    prop_type: def["type"].as_str().unwrap_or("").into(),
                })
                .collect()
        })
        .unwrap_or_default();
    columns.sort_by(|a, b| {
        let a_title = a.prop_type == "title";
        let b_title = b.prop_type == "title";
        b_title.cmp(&a_title).then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    columns
}

impl TableView {
    pub fn new(ds: DataSourceRec, rows: Vec<RowRec>) -> TableView {
        let columns = schema_columns(&ds.schema_json);
        TableView { ds, columns, rows, cursor: 0, sort: None, sort_col: 0, table_state: TableState::default() }
    }

    pub fn cell(&self, row: &RowRec, col: &Column) -> String {
        let props: Value = serde_json::from_str(&row.properties).unwrap_or_default();
        cell_text(&props[&col.name])
    }

    pub fn toggle_sort(&mut self, col_idx: usize) {
        let asc = match self.sort {
            Some((c, asc)) if c == col_idx => !asc,
            _ => true,
        };
        self.sort = Some((col_idx, asc));
        let col_name = self.columns[col_idx].name.clone();
        self.rows.sort_by_cached_key(|r| {
            let props: Value = serde_json::from_str(&r.properties).unwrap_or_default();
            cell_text(&props[&col_name]).to_lowercase()
        });
        if !asc {
            self.rows.reverse();
        }
        self.cursor = 0;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.rows.is_empty() {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, self.rows.len() as isize - 1) as usize;
    }

    pub fn selected_row_id(&self) -> Option<String> {
        self.rows.get(self.cursor).map(|r| r.id.clone())
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &mut TableView, focused: bool, theme: &Theme) {
    let header = TRow::new(
        view.columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let mut name = c.name.clone();
                if let Some((sc, asc)) = view.sort {
                    if sc == i {
                        name.push_str(if asc { " ▲" } else { " ▼" });
                    }
                }
                if i == view.sort_col {
                    format!("[{name}]")
                } else {
                    name
                }
            })
            .collect::<Vec<_>>(),
    )
    .style(Style::default().add_modifier(Modifier::BOLD));
    let rows: Vec<TRow> = view
        .rows
        .iter()
        .map(|r| {
            let cells: Vec<String> = view.columns.iter().map(|c| view.cell(r, c)).collect();
            TRow::new(cells)
        })
        .collect();
    let widths: Vec<Constraint> = view
        .columns
        .iter()
        .enumerate()
        .map(|(i, _)| if i == 0 { Constraint::Min(20) } else { Constraint::Length(14) })
        .collect();
    let highlight = if focused { theme.highlight } else { Style::default() };
    view.table_state.select(Some(view.cursor));
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .row_highlight_style(highlight)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(theme.border)
                    .title(Span::styled(format!(" {} ", view.ds.title), theme.title)),
            ),
        area,
        &mut view.table_state,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use notion_store::{DataSourceRec, RowRec};
    use serde_json::json;

    fn ds() -> DataSourceRec {
        DataSourceRec {
            id: "ds".into(),
            database_id: "db".into(),
            title: "Tasks".into(),
            schema_json: json!({
                "Name": {"type": "title"},
                "Done": {"type": "checkbox"},
                "Prio": {"type": "select"}
            })
            .to_string(),
            last_edited_time: "t".into(),
        }
    }

    fn row(id: &str, name: &str, done: bool, prio: &str) -> RowRec {
        RowRec {
            id: id.into(),
            data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": name}]},
                "Done": {"type": "checkbox", "checkbox": done},
                "Prio": {"type": "select", "select": {"name": prio}}
            })
            .to_string(),
            last_edited_time: "t".into(),
            archived: false,
        }
    }

    #[test]
    fn schema_columns_puts_title_first_then_alphabetical() {
        let cols = schema_columns(&ds().schema_json);
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["Name", "Done", "Prio"]);
        assert_eq!(cols[0].prop_type, "title");
    }

    #[test]
    fn title_column_first_and_cells_render() {
        let v = TableView::new(ds(), vec![row("r1", "Buy milk", true, "High")]);
        assert_eq!(v.columns[0].name, "Name");
        assert_eq!(v.cell(&v.rows[0], &v.columns[0]), "Buy milk");
        let done_col = v.columns.iter().position(|c| c.name == "Done").unwrap();
        assert_eq!(v.cell(&v.rows[0], &v.columns[done_col]), "☑");
    }

    #[test]
    fn sort_toggles_direction() {
        let mut v = TableView::new(
            ds(),
            vec![row("r1", "b task", false, "Low"), row("r2", "a task", false, "High")],
        );
        v.toggle_sort(0);
        assert_eq!(v.selected_row_id().as_deref(), Some("r2")); // "a task" first
        v.toggle_sort(0);
        assert_eq!(v.selected_row_id().as_deref(), Some("r1")); // descending
    }

    #[test]
    fn cell_text_variants() {
        assert_eq!(cell_text(&json!({"type": "number", "number": 42})), "42");
        assert_eq!(
            cell_text(&json!({"type": "multi_select", "multi_select": [{"name": "a"}, {"name": "b"}]})),
            "a, b"
        );
        assert_eq!(cell_text(&json!({"type": "date", "date": {"start": "2026-07-05"}})), "2026-07-05");
        assert_eq!(cell_text(&json!({"type": "checkbox", "checkbox": false})), "☐");
    }
}
