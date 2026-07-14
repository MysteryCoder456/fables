use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use crate::ui::textline::{TextLine, TextLineEvent};
use crate::ui::theme::Theme;

pub struct InputState {
    pub title: String,
    pub line: TextLine,
}

pub enum InputAction {
    None,
    Changed,
    Submit(String),
    Cancel,
}

impl InputState {
    pub fn new(title: impl Into<String>, initial: impl Into<String>) -> InputState {
        InputState {
            title: title.into(),
            line: TextLine::new(initial),
        }
    }

    pub fn value(&self) -> &str {
        self.line.text()
    }

    pub fn on_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Esc => InputAction::Cancel,
            KeyCode::Enter => InputAction::Submit(self.line.text().to_string()),
            _ => match self.line.on_key(key) {
                TextLineEvent::Edited => InputAction::Changed,
                TextLineEvent::Moved | TextLineEvent::Ignored => InputAction::None,
            },
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

pub fn render(f: &mut Frame, state: &InputState, theme: &Theme) {
    let area = f.area();
    let popup_width = (area.width * 2 / 3).clamp(20, 70);
    let popup = centered_rect(popup_width, 3, area);

    f.render_widget(Clear, popup);
    f.render_widget(
        Paragraph::new(state.line.text())
            .block(crate::ui::theme::popup_block(format!(" {} ", state.title), theme)),
        popup,
    );
    f.set_cursor_position(ratatui::layout::Position::new(
        popup.x + 1 + state.line.cursor_cols().min(popup.width.saturating_sub(2)),
        popup.y + 1,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_backspace_and_submit() {
        let mut s = InputState::new("t", "");
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Char('h'))),
            InputAction::Changed
        ));
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Char('i'))),
            InputAction::Changed
        ));
        assert_eq!(s.value(), "hi");
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Backspace)),
            InputAction::Changed
        ));
        assert_eq!(s.value(), "h");
        s.on_key(KeyEvent::from(KeyCode::Left));
        s.on_key(KeyEvent::from(KeyCode::Char('a')));
        assert_eq!(s.value(), "ah"); // typed before the final grapheme
        match s.on_key(KeyEvent::from(KeyCode::Enter)) {
            InputAction::Submit(v) => assert_eq!(v, "ah"),
            _ => panic!("expected Submit"),
        }
    }

    #[test]
    fn esc_cancels() {
        let mut s = InputState::new("t", "x");
        assert!(matches!(
            s.on_key(KeyEvent::from(KeyCode::Esc)),
            InputAction::Cancel
        ));
    }
}
