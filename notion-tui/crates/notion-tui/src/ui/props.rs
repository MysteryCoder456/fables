use crossterm::event::{KeyCode, KeyEvent};
use notion_store::RowRec;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use serde_json::{json, Value};

use crate::ui::table::{cell_text, TableView};

pub struct PropField {
    pub name: String,
    pub prop_type: String,
    pub value_text: String,
}

pub struct PropsState {
    pub row_id: String,
    pub fields: Vec<PropField>,
    pub cursor: usize,
    pub edit_buffer: Option<String>,
}

pub enum PropsAction {
    None,
    Close,
    Commit { prop_name: String, prop_type: String, text: String },
}

pub fn build_fields(view: &TableView, row: &RowRec) -> Vec<PropField> {
    let props: Value = serde_json::from_str(&row.properties).unwrap_or_default();
    view.columns
        .iter()
        .map(|c| PropField {
            name: c.name.clone(),
            prop_type: c.prop_type.clone(),
            value_text: cell_text(&props[&c.name]),
        })
        .collect()
}

impl PropsState {
    pub fn new(row_id: String, fields: Vec<PropField>) -> PropsState {
        PropsState { row_id, fields, cursor: 0, edit_buffer: None }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PropsAction {
        if let Some(buf) = &mut self.edit_buffer {
            match key.code {
                KeyCode::Esc => {
                    self.edit_buffer = None;
                    PropsAction::None
                }
                KeyCode::Enter => {
                    let text = buf.clone();
                    self.edit_buffer = None;
                    let field = &self.fields[self.cursor];
                    PropsAction::Commit {
                        prop_name: field.name.clone(),
                        prop_type: field.prop_type.clone(),
                        text,
                    }
                }
                KeyCode::Backspace => {
                    buf.pop();
                    PropsAction::None
                }
                KeyCode::Char(c) => {
                    buf.push(c);
                    PropsAction::None
                }
                _ => PropsAction::None,
            }
        } else {
            match key.code {
                KeyCode::Esc => PropsAction::Close,
                KeyCode::Char('j') | KeyCode::Down => {
                    if !self.fields.is_empty() {
                        self.cursor = (self.cursor + 1).min(self.fields.len() - 1);
                    }
                    PropsAction::None
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.cursor = self.cursor.saturating_sub(1);
                    PropsAction::None
                }
                KeyCode::Enter => {
                    if let Some(field) = self.fields.get(self.cursor) {
                        if field.prop_type == "checkbox" {
                            let toggled = if field.value_text == "☑" { "false" } else { "true" };
                            return PropsAction::Commit {
                                prop_name: field.name.clone(),
                                prop_type: field.prop_type.clone(),
                                text: toggled.to_string(),
                            };
                        }
                        self.edit_buffer = Some(field.value_text.clone());
                    }
                    PropsAction::None
                }
                _ => PropsAction::None,
            }
        }
    }
}

/// Converts a raw text edit back into a full Notion property value for the given type.
pub fn build_property_value(prop_type: &str, text: &str) -> Value {
    match prop_type {
        "title" => {
            json!({"type": "title", "title": [{"type": "text", "text": {"content": text}, "plain_text": text}]})
        }
        "rich_text" => {
            json!({"type": "rich_text", "rich_text": [{"type": "text", "text": {"content": text}, "plain_text": text}]})
        }
        "number" => json!({"type": "number", "number": text.parse::<f64>().ok()}),
        "select" => {
            if text.is_empty() { json!({"type": "select", "select": null}) }
            else { json!({"type": "select", "select": {"name": text}}) }
        }
        "status" => {
            if text.is_empty() { json!({"type": "status", "status": null}) }
            else { json!({"type": "status", "status": {"name": text}}) }
        }
        "multi_select" => json!({
            "type": "multi_select",
            "multi_select": text.split(',').map(|s| json!({"name": s.trim()})).collect::<Vec<_>>()
        }),
        "date" => json!({"type": "date", "date": {"start": text}}),
        "checkbox" => json!({"type": "checkbox", "checkbox": text == "true"}),
        "url" => json!({"type": "url", "url": text}),
        "email" => json!({"type": "email", "email": text}),
        "phone_number" => json!({"type": "phone_number", "phone_number": text}),
        _ => Value::Null,
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

pub fn render(f: &mut Frame, state: &PropsState) {
    let area = f.area();
    let popup_width = (area.width * 3 / 4).clamp(30, 70);
    let popup_height = (area.height * 3 / 4).clamp(6, 20);
    let popup = centered_rect(popup_width, popup_height, area);
    f.render_widget(Clear, popup);

    if let Some(buf) = &state.edit_buffer {
        let field = &state.fields[state.cursor];
        f.render_widget(
            Paragraph::new(buf.as_str())
                .block(Block::default().borders(Borders::ALL).title(format!(" {} ", field.name))),
            popup,
        );
        return;
    }

    let items: Vec<ListItem> = state
        .fields
        .iter()
        .enumerate()
        .map(|(i, field)| {
            let mut item = ListItem::new(format!("{}: {}", field.name, field.value_text));
            if i == state.cursor {
                item = item.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            item
        })
        .collect();
    f.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(" properties ")),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_property_value_variants() {
        assert_eq!(build_property_value("checkbox", "true")["checkbox"], true);
        assert_eq!(build_property_value("select", "High")["select"]["name"], "High");
        assert_eq!(build_property_value("number", "42")["number"], 42.0);
    }
}
