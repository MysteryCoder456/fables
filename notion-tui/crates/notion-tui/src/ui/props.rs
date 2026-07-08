use crossterm::event::{KeyCode, KeyEvent};
use notion_store::RowRec;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use serde_json::{json, Value};

use crate::ui::table::{cell_text, schema_columns, schema_options};

pub struct PropField {
    pub name: String,
    pub prop_type: String,
    pub value_text: String,
    pub options: Vec<String>,
}

/// In-progress edit for the currently focused field. Select/MultiSelect are
/// only used when the schema defines options for that field; otherwise every
/// type (including select/status without options) falls back to Text.
pub enum Editor {
    Text { buffer: String, error: Option<String> },
    /// `options[0]` is always the synthetic "(clear)" entry.
    Select { options: Vec<String>, cursor: usize },
    MultiSelect { options: Vec<String>, checked: Vec<bool>, cursor: usize },
}

pub struct PropsState {
    pub row_id: String,
    pub fields: Vec<PropField>,
    pub cursor: usize,
    pub editor: Option<Editor>,
}

pub enum PropsAction {
    None,
    Close,
    Commit { prop_name: String, prop_type: String, text: String },
}

pub fn build_fields(schema_json: &str, row: &RowRec) -> Vec<PropField> {
    let props: Value = serde_json::from_str(&row.properties).unwrap_or_default();
    schema_columns(schema_json)
        .iter()
        .map(|c| PropField {
            name: c.name.clone(),
            prop_type: c.prop_type.clone(),
            value_text: cell_text(&props[&c.name]),
            options: schema_options(schema_json, &c.name, &c.prop_type),
        })
        .collect()
}

impl PropsState {
    pub fn new(row_id: String, fields: Vec<PropField>) -> PropsState {
        PropsState { row_id, fields, cursor: 0, editor: None }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> PropsAction {
        if let Some(editor) = &mut self.editor {
            match editor {
                Editor::Text { buffer, error } => match key.code {
                    KeyCode::Esc => {
                        self.editor = None;
                        PropsAction::None
                    }
                    KeyCode::Enter => {
                        let field = &self.fields[self.cursor];
                        match validate_input(&field.prop_type, buffer) {
                            Ok(()) => {
                                let text = buffer.clone();
                                let prop_name = field.name.clone();
                                let prop_type = field.prop_type.clone();
                                self.editor = None;
                                PropsAction::Commit { prop_name, prop_type, text }
                            }
                            Err(msg) => {
                                *error = Some(msg);
                                PropsAction::None
                            }
                        }
                    }
                    KeyCode::Backspace => {
                        buffer.pop();
                        *error = None;
                        PropsAction::None
                    }
                    KeyCode::Char(c) => {
                        buffer.push(c);
                        *error = None;
                        PropsAction::None
                    }
                    _ => PropsAction::None,
                },
                Editor::Select { options, cursor } => match key.code {
                    KeyCode::Esc => {
                        self.editor = None;
                        PropsAction::None
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        *cursor = (*cursor + 1).min(options.len() - 1);
                        PropsAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        *cursor = cursor.saturating_sub(1);
                        PropsAction::None
                    }
                    KeyCode::Enter => {
                        let chosen = &options[*cursor];
                        let text = if chosen == "(clear)" { String::new() } else { chosen.clone() };
                        let field = &self.fields[self.cursor];
                        let prop_name = field.name.clone();
                        let prop_type = field.prop_type.clone();
                        self.editor = None;
                        PropsAction::Commit { prop_name, prop_type, text }
                    }
                    _ => PropsAction::None,
                },
                Editor::MultiSelect { options, checked, cursor } => match key.code {
                    KeyCode::Esc => {
                        self.editor = None;
                        PropsAction::None
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        *cursor = (*cursor + 1).min(options.len() - 1);
                        PropsAction::None
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        *cursor = cursor.saturating_sub(1);
                        PropsAction::None
                    }
                    KeyCode::Char(' ') => {
                        checked[*cursor] = !checked[*cursor];
                        PropsAction::None
                    }
                    KeyCode::Enter => {
                        let text = options
                            .iter()
                            .zip(checked.iter())
                            .filter(|(_, c)| **c)
                            .map(|(o, _)| o.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                        let field = &self.fields[self.cursor];
                        let prop_name = field.name.clone();
                        let prop_type = field.prop_type.clone();
                        self.editor = None;
                        PropsAction::Commit { prop_name, prop_type, text }
                    }
                    _ => PropsAction::None,
                },
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
                        self.editor = Some(if field.prop_type == "multi_select" && !field.options.is_empty() {
                            let checked = field
                                .options
                                .iter()
                                .map(|o| field.value_text.split(", ").any(|v| v == o))
                                .collect();
                            Editor::MultiSelect { options: field.options.clone(), checked, cursor: 0 }
                        } else if !field.options.is_empty() {
                            let mut options = vec!["(clear)".to_string()];
                            options.extend(field.options.iter().cloned());
                            let cursor = options.iter().position(|o| o == &field.value_text).unwrap_or(0);
                            Editor::Select { options, cursor }
                        } else {
                            Editor::Text { buffer: field.value_text.clone(), error: None }
                        });
                    }
                    PropsAction::None
                }
                _ => PropsAction::None,
            }
        }
    }
}

fn is_valid_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [y, m, d] = parts[..] else { return false };
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<i32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    if !(1..=12).contains(&m) {
        return false;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days_in_month = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => unreachable!(),
    };
    (1..=days_in_month).contains(&d)
}

/// Validates a raw text edit against the constraints of the given property
/// type before it's converted and committed. Empty text always passes (it
/// clears the value).
pub fn validate_input(prop_type: &str, text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    match prop_type {
        "number" => text.parse::<f64>().map(|_| ()).map_err(|_| "not a number".to_string()),
        "date" => {
            if is_valid_date(text) {
                Ok(())
            } else {
                Err("not a valid date (expected YYYY-MM-DD)".to_string())
            }
        }
        "url" => {
            if text.contains("://") {
                Ok(())
            } else {
                Err("not a valid url (missing scheme)".to_string())
            }
        }
        "email" => {
            let Some((local, domain)) = text.split_once('@') else {
                return Err("not a valid email".to_string());
            };
            if !local.is_empty() && !domain.is_empty() && domain.contains('.') {
                Ok(())
            } else {
                Err("not a valid email".to_string())
            }
        }
        "phone_number" => {
            if text.chars().all(|c| c.is_ascii_digit() || " +-()".contains(c)) {
                Ok(())
            } else {
                Err("not a valid phone number".to_string())
            }
        }
        _ => Ok(()),
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

    if let Some(editor) = &state.editor {
        let field = &state.fields[state.cursor];
        match editor {
            Editor::Text { buffer, error } => {
                let text = match error {
                    Some(msg) => format!("{buffer}\n✗ {msg}"),
                    None => buffer.clone(),
                };
                f.render_widget(
                    Paragraph::new(text)
                        .block(Block::default().borders(Borders::ALL).title(format!(" {} ", field.name))),
                    popup,
                );
            }
            Editor::Select { options, cursor } => {
                let items: Vec<ListItem> = options
                    .iter()
                    .enumerate()
                    .map(|(i, o)| {
                        let mut item = ListItem::new(o.as_str());
                        if i == *cursor {
                            item = item.style(Style::default().add_modifier(Modifier::REVERSED));
                        }
                        item
                    })
                    .collect();
                f.render_widget(
                    List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" {} (enter select · esc cancel) ", field.name)),
                    ),
                    popup,
                );
            }
            Editor::MultiSelect { options, checked, cursor } => {
                let items: Vec<ListItem> = options
                    .iter()
                    .zip(checked.iter())
                    .enumerate()
                    .map(|(i, (o, c))| {
                        let mark = if *c { "[x]" } else { "[ ]" };
                        let mut item = ListItem::new(format!("{mark} {o}"));
                        if i == *cursor {
                            item = item.style(Style::default().add_modifier(Modifier::REVERSED));
                        }
                        item
                    })
                    .collect();
                f.render_widget(
                    List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" {} (space toggle · enter commit · esc cancel) ", field.name)),
                    ),
                    popup,
                );
            }
        }
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

    fn field_with_options(prop_type: &str, value_text: &str, options: &[&str]) -> PropField {
        PropField {
            name: "Status".into(),
            prop_type: prop_type.into(),
            value_text: value_text.into(),
            options: options.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn enter_on_option_field_opens_select_preselected_on_current_value() {
        let mut state =
            PropsState::new("r1".into(), vec![field_with_options("status", "Doing", &["Todo", "Doing", "Done"])]);
        state.on_key(KeyEvent::from(KeyCode::Enter));
        match state.editor {
            Some(Editor::Select { ref options, cursor }) => {
                assert_eq!(options, &vec!["(clear)", "Todo", "Doing", "Done"]);
                assert_eq!(cursor, 2);
            }
            _ => panic!("expected a Select editor"),
        }
    }

    #[test]
    fn select_editor_enter_commits_the_highlighted_option() {
        let mut state =
            PropsState::new("r1".into(), vec![field_with_options("status", "Doing", &["Todo", "Doing", "Done"])]);
        state.on_key(KeyEvent::from(KeyCode::Enter));
        state.on_key(KeyEvent::from(KeyCode::Char('j'))); // Doing -> Done
        let action = state.on_key(KeyEvent::from(KeyCode::Enter));
        match action {
            PropsAction::Commit { prop_name, prop_type, text } => {
                assert_eq!(prop_name, "Status");
                assert_eq!(prop_type, "status");
                assert_eq!(text, "Done");
            }
            _ => panic!("expected a Commit action"),
        }
        assert!(state.editor.is_none());
    }

    #[test]
    fn select_editor_clear_entry_commits_empty_text() {
        let mut state =
            PropsState::new("r1".into(), vec![field_with_options("status", "Doing", &["Todo", "Doing", "Done"])]);
        state.on_key(KeyEvent::from(KeyCode::Enter));
        state.on_key(KeyEvent::from(KeyCode::Char('k')));
        state.on_key(KeyEvent::from(KeyCode::Char('k'))); // cursor 2 -> 1 -> 0 == (clear)
        let action = state.on_key(KeyEvent::from(KeyCode::Enter));
        match action {
            PropsAction::Commit { text, .. } => assert_eq!(text, ""),
            _ => panic!("expected a Commit action"),
        }
    }

    #[test]
    fn multi_select_space_toggles_then_enter_commits_joined_set() {
        let mut state = PropsState::new(
            "r1".into(),
            vec![field_with_options("multi_select", "bug", &["bug", "infra", "urgent"])],
        );
        state.on_key(KeyEvent::from(KeyCode::Enter));
        state.on_key(KeyEvent::from(KeyCode::Char('j')));
        state.on_key(KeyEvent::from(KeyCode::Char('j')));
        state.on_key(KeyEvent::from(KeyCode::Char(' '))); // toggle "urgent" on
        let action = state.on_key(KeyEvent::from(KeyCode::Enter));
        match action {
            PropsAction::Commit { text, .. } => assert_eq!(text, "bug, urgent"),
            _ => panic!("expected a Commit action"),
        }
    }

    #[test]
    fn invalid_number_keeps_editor_open_with_error_then_clears_on_keystroke() {
        let mut state = PropsState::new(
            "r1".into(),
            vec![PropField { name: "Age".into(), prop_type: "number".into(), value_text: "".into(), options: vec![] }],
        );
        state.on_key(KeyEvent::from(KeyCode::Enter));
        for c in "abc".chars() {
            state.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let action = state.on_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(action, PropsAction::None));
        match &state.editor {
            Some(Editor::Text { error: Some(_), .. }) => {}
            _ => panic!("expected a Text editor with an error set"),
        }
        state.on_key(KeyEvent::from(KeyCode::Char('1')));
        match &state.editor {
            Some(Editor::Text { error: None, .. }) => {}
            _ => panic!("error should clear on the next keystroke"),
        }
    }

    #[test]
    fn option_type_without_defined_options_falls_back_to_text_editor() {
        let mut state = PropsState::new(
            "r1".into(),
            vec![PropField { name: "Priority".into(), prop_type: "select".into(), value_text: "".into(), options: vec![] }],
        );
        state.on_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(state.editor, Some(Editor::Text { .. })));
    }

    #[test]
    fn build_fields_populates_options_for_status() {
        let schema_json = json!({
            "Name": {"type": "title"},
            "Status": {"type": "status", "status": {"options": [
                {"name": "Todo"}, {"name": "Doing"}, {"name": "Done"}]}}
        })
        .to_string();
        let row = RowRec {
            id: "r1".into(),
            data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Ship it"}]},
                "Status": {"type": "status", "status": {"name": "Doing"}}
            })
            .to_string(),
            last_edited_time: "t".into(),
            archived: false,
        };
        let fields = build_fields(&schema_json, &row);
        let status = fields.iter().find(|f| f.name == "Status").unwrap();
        assert_eq!(status.options, vec!["Todo", "Doing", "Done"]);
        let name = fields.iter().find(|f| f.name == "Name").unwrap();
        assert!(name.options.is_empty());
    }

    #[test]
    fn validate_input_empty_always_ok() {
        for t in ["number", "date", "url", "email", "phone_number", "title"] {
            assert!(validate_input(t, "").is_ok(), "{t} should accept empty");
        }
    }

    #[test]
    fn validate_input_number() {
        assert!(validate_input("number", "42").is_ok());
        assert!(validate_input("number", "3.5").is_ok());
        assert!(validate_input("number", "abc").is_err());
    }

    #[test]
    fn validate_input_date() {
        assert!(validate_input("date", "2026-07-08").is_ok());
        assert!(validate_input("date", "2026-13-01").is_err());
        assert!(validate_input("date", "2026-02-30").is_err());
        assert!(validate_input("date", "2024-02-29").is_ok());
        assert!(validate_input("date", "not-a-date").is_err());
    }

    #[test]
    fn validate_input_url() {
        assert!(validate_input("url", "https://example.com").is_ok());
        assert!(validate_input("url", "example.com").is_err());
    }

    #[test]
    fn validate_input_email() {
        assert!(validate_input("email", "a@b.com").is_ok());
        assert!(validate_input("email", "not-an-email").is_err());
        assert!(validate_input("email", "a@b").is_err());
    }

    #[test]
    fn validate_input_phone_number() {
        assert!(validate_input("phone_number", "+1 (555) 123-4567").is_ok());
        assert!(validate_input("phone_number", "call me").is_err());
    }

    #[test]
    fn build_property_value_variants() {
        assert_eq!(build_property_value("checkbox", "true")["checkbox"], true);
        assert_eq!(build_property_value("select", "High")["select"]["name"], "High");
        assert_eq!(build_property_value("number", "42")["number"], 42.0);
    }

    #[test]
    fn build_fields_derives_from_schema_json_not_a_table_view() {
        let schema_json = json!({
            "Name": {"type": "title"},
            "Status": {"type": "status"}
        })
        .to_string();
        let row = RowRec {
            id: "r1".into(),
            data_source_id: "ds".into(),
            properties: json!({
                "Name": {"type": "title", "title": [{"plain_text": "Ship it"}]},
                "Status": {"type": "status", "status": {"name": "Doing"}}
            })
            .to_string(),
            last_edited_time: "t".into(),
            archived: false,
        };

        let fields = build_fields(&schema_json, &row);

        assert_eq!(fields[0].name, "Name");
        assert_eq!(fields[0].value_text, "Ship it");
        let status = fields.iter().find(|f| f.name == "Status").unwrap();
        assert_eq!(status.value_text, "Doing");
    }
}
