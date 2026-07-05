use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

pub struct InputState {
    pub title: String,
    pub value: String,
}

pub enum InputAction {
    None,
    Changed,
    Submit(String),
    Cancel,
}

impl InputState {
    pub fn new(title: impl Into<String>, initial: impl Into<String>) -> InputState {
        InputState { title: title.into(), value: initial.into() }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Esc => InputAction::Cancel,
            KeyCode::Enter => InputAction::Submit(self.value.clone()),
            KeyCode::Backspace => {
                self.value.pop();
                InputAction::Changed
            }
            KeyCode::Char(c) => {
                self.value.push(c);
                InputAction::Changed
            }
            _ => InputAction::None,
        }
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect { x, y, width, height }
}

pub fn render(f: &mut Frame, state: &InputState) {
    let area = f.area();
    let popup_width = (area.width * 2 / 3).clamp(20, 70);
    let popup = centered_rect(popup_width, 3, area);

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(state.value.as_str())
            .block(Block::default().borders(Borders::ALL).title(format!(" {} ", state.title))),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_backspace_and_submit() {
        let mut s = InputState::new("t", "");
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Char('h'))), InputAction::Changed));
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Char('i'))), InputAction::Changed));
        assert_eq!(s.value, "hi");
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Backspace)), InputAction::Changed));
        assert_eq!(s.value, "h");
        match s.on_key(KeyEvent::from(KeyCode::Enter)) {
            InputAction::Submit(v) => assert_eq!(v, "h"),
            _ => panic!("expected Submit"),
        }
    }

    #[test]
    fn esc_cancels() {
        let mut s = InputState::new("t", "x");
        assert!(matches!(s.on_key(KeyEvent::from(KeyCode::Esc)), InputAction::Cancel));
    }
}
