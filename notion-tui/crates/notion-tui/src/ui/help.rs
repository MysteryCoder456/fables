use crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use ratatui::Frame;

use crate::keymap::Keymap;

fn key_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(' ') => "space".into(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "enter".into(),
        KeyCode::Esc => "esc".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::Backspace => "backspace".into(),
        other => format!("{other:?}"),
    }
}

fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

/// A short reminder of the board-only keys, shown in the status bar so
/// card-moving and property-editing aren't undiscoverable outside the help overlay.
pub fn board_hints(keymap: &Keymap) -> String {
    let entries = [
        ("move_card_next", "move card right"),
        ("move_card_prev", "move card left"),
        ("props", "edit properties"),
    ];
    entries
        .iter()
        .filter_map(|(action, desc)| keymap.key_for(action).map(|k| format!("{} {desc}", key_label(k))))
        .collect::<Vec<_>>()
        .join(" · ")
}

pub fn render(f: &mut Frame, keymap: &Keymap) {
    let area = f.area();
    let popup = centered_rect(
        (area.width * 3 / 4).clamp(40, 70),
        (area.height * 3 / 4).clamp(10, 32),
        area,
    );
    f.render_widget(Clear, popup);
    let items: Vec<ListItem> = Keymap::actions()
        .iter()
        .map(|(action, desc)| {
            let key = keymap.key_for(action).map(key_label).unwrap_or_default();
            ListItem::new(format!("{key:>10}  {desc}"))
        })
        .collect();
    f.render_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" help (any key to close) "),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_hints_lists_card_move_and_props_keys() {
        let hints = board_hints(&Keymap::new());
        assert_eq!(hints, "J move card right · K move card left · p edit properties");
    }
}
