use notion_store::{DataSourceRec, RowRec};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Row as TRow, Table, TableState};
use ratatui::Frame;
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::ui::theme::Theme;

#[derive(Clone)]
pub struct Column {
    pub name: String,
    pub prop_type: String,
}

#[derive(Clone)]
pub struct FilterState {
    /// `Some(col_idx)` filters that one column; `None` is free-text across the whole row.
    pub col: Option<usize>,
    pub query: String,
}

pub struct TableView {
    pub ds: DataSourceRec,
    pub columns: Vec<Column>,
    pub rows: Vec<RowRec>,
    pub cursor: usize,
    pub sort: Option<(usize, bool)>,
    pub sort_col: usize,
    pub table_state: TableState,
    pub filter: Option<FilterState>,
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
            .map(|n| {
                if n.fract() == 0.0 {
                    format!("{}", n as i64)
                } else {
                    n.to_string()
                }
            })
            .unwrap_or_default(),
        "select" => prop["select"]["name"].as_str().unwrap_or("").into(),
        "status" => prop["status"]["name"].as_str().unwrap_or("").into(),
        "multi_select" => prop["multi_select"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
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
        "people" => prop["people"]
            .as_array()
            .map(|a| format!("👤 {}", a.len()))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// Reads the defined options for a select/status/multi_select property from
/// the schema; empty for types that don't declare options.
pub fn schema_options(schema_json: &str, prop: &str, prop_type: &str) -> Vec<String> {
    let schema: Value = serde_json::from_str(schema_json).unwrap_or_default();
    schema[prop][prop_type]["options"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|o| o["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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
        b_title
            .cmp(&a_title)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    columns
}

pub fn column_widths(columns: &[Column]) -> Vec<Constraint> {
    columns
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if i == 0 {
                Constraint::Min(20)
            } else {
                Constraint::Length(14)
            }
        })
        .collect()
}

/// x-coordinate where each column starts, for header-click hit-testing.
/// Mirrors the same `Layout::horizontal(widths)` split the `Table` widget
/// itself performs, so header clicks line up with rendered columns (modulo
/// ratatui's own internal cell-spacing, which this doesn't attempt to
/// replicate exactly — acceptable for a best-effort, non-required input).
pub fn column_x_starts(area: Rect, columns: &[Column]) -> Vec<u16> {
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints(column_widths(columns))
        .split(inner)
        .iter()
        .map(|r| r.x)
        .collect()
}

impl TableView {
    pub fn new(ds: DataSourceRec, rows: Vec<RowRec>) -> TableView {
        let columns = schema_columns(&ds.schema_json);
        TableView {
            ds,
            columns,
            rows,
            cursor: 0,
            sort: None,
            sort_col: 0,
            table_state: TableState::default(),
            filter: None,
        }
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
        self.apply_sort();
        self.cursor = 0;
    }

    /// Re-applies the current `self.sort` (if any) to `self.rows` in place,
    /// without touching `self.cursor`. Used both by `toggle_sort` and by
    /// refresh flows that need to restore a previously chosen sort onto a
    /// freshly loaded row set.
    ///
    /// If `sort` references a column that no longer exists (e.g. the schema
    /// shrank between refreshes), the sort is dropped instead of panicking.
    pub fn apply_sort(&mut self) {
        let Some((col_idx, asc)) = self.sort else { return };
        let Some(col) = self.columns.get(col_idx) else {
            self.sort = None;
            return;
        };
        let col_name = col.name.clone();
        self.rows.sort_by_cached_key(|r| {
            let props: Value = serde_json::from_str(&r.properties).unwrap_or_default();
            cell_text(&props[&col_name]).to_lowercase()
        });
        if !asc {
            self.rows.reverse();
        }
    }

    fn matches_filter(&self, row: &RowRec) -> bool {
        let Some(f) = &self.filter else { return true };
        let q = f.query.to_lowercase();
        match f.col {
            Some(i) => self
                .columns
                .get(i)
                .map(|c| self.cell(row, c).to_lowercase().contains(&q))
                .unwrap_or(true),
            None => self
                .columns
                .iter()
                .any(|c| self.cell(row, c).to_lowercase().contains(&q)),
        }
    }

    /// The rows actually shown: `self.rows` (already sorted by `apply_sort`)
    /// with the active filter, if any, applied on top.
    pub fn visible(&self) -> Vec<&RowRec> {
        self.rows.iter().filter(|r| self.matches_filter(r)).collect()
    }

    /// Position of the row with the given id in the current display order
    /// (post-sort, post-filter), if it still exists.
    pub fn rows_iter_position(&self, id: &str) -> Option<usize> {
        self.visible().iter().position(|r| r.id == id)
    }

    /// Number of rows currently displayed.
    pub fn row_count(&self) -> usize {
        self.visible().len()
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    pub fn selected_row_id(&self) -> Option<String> {
        self.visible().get(self.cursor).map(|r| r.id.clone())
    }
}

/// Column width by property type (spec M8.8: checkbox narrow, URL wide).
fn column_constraint(col: &Column, is_title: bool) -> Constraint {
    if is_title {
        return Constraint::Min(20);
    }
    match col.prop_type.as_str() {
        "checkbox" => Constraint::Length(6),
        "number" => Constraint::Length(10),
        "date" => Constraint::Length(12),
        "url" | "email" => Constraint::Length(28),
        _ => Constraint::Length(14),
    }
}

/// Truncates to at most `max_cols` display columns, appending '…' when
/// anything was cut. Grapheme-safe: never slices inside an emoji/CJK char.
pub fn truncate_ellipsis(s: &str, max_cols: u16) -> String {
    if (s.width() as u16) <= max_cols {
        return s.to_string();
    }
    let budget = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut used: u16 = 0;
    for g in s.graphemes(true) {
        let gw = g.width() as u16;
        if used + gw > budget {
            break;
        }
        out.push_str(g);
        used += gw;
    }
    out.push('…');
    out
}

fn filter_suffix(filter: &Option<FilterState>, columns: &[Column]) -> String {
    match filter {
        None => String::new(),
        Some(f) => match f.col.and_then(|i| columns.get(i)) {
            Some(c) => format!(" — filter: {}~\"{}\"", c.name, f.query),
            None => format!(" — filter: \"{}\"", f.query),
        },
    }
}

pub fn render(f: &mut Frame, area: Rect, view: &mut TableView, focused: bool, theme: &Theme) {
    let block_title = format!(
        " {}{} ",
        view.ds.title,
        filter_suffix(&view.filter, &view.columns)
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border)
        .title(Span::styled(block_title, theme.title));
    let visible = view.visible();
    if visible.is_empty() {
        let hint = if view.filter.is_some() && !view.rows.is_empty() {
            "no rows match the filter"
        } else {
            "no rows — press o to add one"
        };
        f.render_widget(
            ratatui::widgets::Paragraph::new(hint)
                .style(Style::default().add_modifier(Modifier::DIM))
                .block(block),
            area,
        );
        return;
    }
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
    let widths: Vec<Constraint> = view
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| column_constraint(c, i == 0))
        .collect();
    // Concrete per-column budgets for ellipsis (ratatui clips silently otherwise):
    // fixed columns take their Length; the title column gets the leftover.
    let fixed_sum: u16 = widths
        .iter()
        .map(|w| if let Constraint::Length(n) = w { *n } else { 0 })
        .sum();
    let spacing = view.columns.len().saturating_sub(1) as u16; // Table's default column_spacing = 1
    let title_budget = area
        .width
        .saturating_sub(2) // borders
        .saturating_sub(spacing)
        .saturating_sub(fixed_sum)
        .max(20);
    let budgets: Vec<u16> = widths
        .iter()
        .map(|w| {
            if let Constraint::Length(n) = w {
                *n
            } else {
                title_budget
            }
        })
        .collect();
    let rows: Vec<TRow> = visible
        .iter()
        .map(|r| {
            let cells: Vec<String> = view
                .columns
                .iter()
                .enumerate()
                .map(|(i, c)| truncate_ellipsis(&view.cell(r, c), budgets[i]))
                .collect();
            TRow::new(cells)
        })
        .collect();
    let highlight = if focused {
        theme.highlight
    } else {
        Style::default()
    };
    view.table_state.select(Some(view.cursor));
    f.render_stateful_widget(
        Table::new(rows, widths)
            .header(header)
            .row_highlight_style(highlight)
            .block(block),
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
    fn schema_options_reads_defined_options_empty_when_none() {
        let schema_json = json!({
            "Status": {"type": "status", "status": {"options": [
                {"name": "Todo"}, {"name": "Doing"}, {"name": "Done"}]}},
            "Name": {"type": "title"}
        })
        .to_string();
        assert_eq!(
            schema_options(&schema_json, "Status", "status"),
            vec!["Todo", "Doing", "Done"]
        );
        assert_eq!(
            schema_options(&schema_json, "Name", "title"),
            Vec::<String>::new()
        );
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
            vec![
                row("r1", "b task", false, "Low"),
                row("r2", "a task", false, "High"),
            ],
        );
        v.toggle_sort(0);
        assert_eq!(v.selected_row_id().as_deref(), Some("r2")); // "a task" first
        v.toggle_sort(0);
        assert_eq!(v.selected_row_id().as_deref(), Some("r1")); // descending
    }

    #[test]
    fn column_x_starts_matches_column_widths_order() {
        let cols = vec![
            Column {
                name: "Name".into(),
                prop_type: "title".into(),
            },
            Column {
                name: "Done".into(),
                prop_type: "checkbox".into(),
            },
        ];
        let area = Rect::new(0, 0, 50, 10);
        let xs = column_x_starts(area, &cols);
        assert_eq!(xs.len(), 2);
        assert!(xs[1] > xs[0], "second column must start after the first");
    }

    #[test]
    fn column_filter_hides_non_matching_rows() {
        let mut v = TableView::new(
            ds(),
            vec![
                row("r1", "Buy milk", false, "High"),
                row("r2", "Buy eggs", false, "Low"),
            ],
        );
        v.filter = Some(FilterState {
            col: Some(2),
            query: "high".into(),
        }); // Prio column
        let visible: Vec<&str> = v.visible().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(visible, vec!["r1"]);
    }

    #[test]
    fn free_text_filter_matches_any_column() {
        let mut v = TableView::new(
            ds(),
            vec![
                row("r1", "Buy milk", false, "High"),
                row("r2", "Buy eggs", false, "Low"),
            ],
        );
        v.filter = Some(FilterState {
            col: None,
            query: "eggs".into(),
        });
        let visible: Vec<&str> = v.visible().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(visible, vec!["r2"]);
    }

    #[test]
    fn no_filter_shows_every_row() {
        let v = TableView::new(ds(), vec![row("r1", "Buy milk", false, "High")]);
        assert_eq!(v.visible().len(), 1);
    }

    #[test]
    fn cell_text_variants() {
        assert_eq!(cell_text(&json!({"type": "number", "number": 42})), "42");
        assert_eq!(
            cell_text(&json!({"type": "multi_select", "multi_select": [{"name": "a"}, {"name": "b"}]})),
            "a, b"
        );
        assert_eq!(
            cell_text(&json!({"type": "date", "date": {"start": "2026-07-05"}})),
            "2026-07-05"
        );
        assert_eq!(cell_text(&json!({"type": "checkbox", "checkbox": false})), "☐");
    }

    #[test]
    fn truncate_ellipsis_is_width_aware_and_grapheme_safe() {
        assert_eq!(truncate_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_ellipsis("hello world", 6), "hello…");
        // CJK chars are 2 columns wide: 日本 = 4 cols, +… = 5.
        assert_eq!(truncate_ellipsis("日本語テスト", 5), "日本…");
        // A budget of 3 can't fit the first 2-wide char plus…: just the char that fits.
        assert_eq!(truncate_ellipsis("日本語", 3), "日…");
    }

    #[test]
    fn column_widths_are_type_aware() {
        let cb = Column {
            name: "Done".into(),
            prop_type: "checkbox".into(),
        };
        let url = Column {
            name: "Link".into(),
            prop_type: "url".into(),
        };
        match column_constraint(&cb, false) {
            Constraint::Length(n) => assert!(n <= 8, "checkbox must be narrow, got {n}"),
            other => panic!("expected Length, got {other:?}"),
        }
        match column_constraint(&url, false) {
            Constraint::Length(n) => assert!(n >= 24, "url must be wide, got {n}"),
            other => panic!("expected Length, got {other:?}"),
        }
        assert!(matches!(
            column_constraint(
                &Column {
                    name: "Name".into(),
                    prop_type: "title".into()
                },
                true
            ),
            Constraint::Min(20)
        ));
    }

    #[test]
    fn render_clips_long_cells_with_ellipsis() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut v = TableView::new(
            ds(),
            vec![row(
                "r1",
                "a task with a very long name that cannot fit",
                false,
                "High",
            )],
        );
        let theme = crate::ui::theme::named("default");
        let backend = TestBackend::new(46, 8); // narrow: title column gets squeezed
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &mut v, true, &theme)).unwrap();
        let content: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            content.contains('…'),
            "long cell should be ellipsized:\n{content}"
        );
    }
}
