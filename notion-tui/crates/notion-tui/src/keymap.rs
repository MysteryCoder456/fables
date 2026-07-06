use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const DEFAULTS: &[(&str, &str, KeyCode)] = &[
    ("quit", "quit the app", KeyCode::Char('q')),
    ("sidebar", "toggle sidebar", KeyCode::Char('1')),
    ("search", "search", KeyCode::Char('/')),
    ("undo", "undo last edit", KeyCode::Char('u')),
    ("edit", "edit page in $EDITOR", KeyCode::Char('e')),
    ("comments", "comments panel", KeyCode::Char('c')),
    ("queue", "queue/conflicts screen", KeyCode::Char('Q')),
    ("board", "toggle board view", KeyCode::Char('v')),
    ("help", "help overlay", KeyCode::Char('?')),
    ("palette", "command palette", KeyCode::Char(':')),
    ("insert", "edit block text", KeyCode::Char('i')),
    ("append", "add block below", KeyCode::Char('a')),
    ("new_row", "new database row", KeyCode::Char('o')),
    ("props", "property form", KeyCode::Char('p')),
    ("delete", "delete (press twice)", KeyCode::Char('d')),
    ("sort", "sort by column", KeyCode::Char('s')),
    ("up", "cursor up", KeyCode::Char('k')),
    ("down", "cursor down", KeyCode::Char('j')),
    ("left", "left / collapse", KeyCode::Char('h')),
    ("right", "right / expand", KeyCode::Char('l')),
    ("top", "jump to top", KeyCode::Char('g')),
    ("bottom", "jump to bottom", KeyCode::Char('G')),
    ("open", "open / follow", KeyCode::Enter),
    ("back", "back", KeyCode::Char('-')),
    ("toggle", "toggle to-do", KeyCode::Char(' ')),
    ("move_card_next", "move card right", KeyCode::Char('J')),
    ("move_card_prev", "move card left", KeyCode::Char('K')),
];

fn parse_key(s: &str) -> Option<KeyCode> {
    match s {
        "esc" => Some(KeyCode::Esc),
        "enter" => Some(KeyCode::Enter),
        "space" => Some(KeyCode::Char(' ')),
        "tab" => Some(KeyCode::Tab),
        "backspace" => Some(KeyCode::Backspace),
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(KeyCode::Char(c)),
                _ => None,
            }
        }
    }
}

#[derive(Clone)]
pub struct Keymap {
    map: HashMap<String, KeyCode>,
}

impl Keymap {
    pub fn new() -> Keymap {
        Keymap {
            map: DEFAULTS.iter().map(|(a, _, k)| (a.to_string(), *k)).collect(),
        }
    }

    pub fn with_overrides(overrides: &HashMap<String, String>) -> Keymap {
        let mut km = Keymap::new();
        for (action, key_str) in overrides {
            if let Some(code) = parse_key(key_str) {
                km.map.insert(action.clone(), code);
            }
        }
        km
    }

    /// Matches on the key code, rejecting Ctrl/Alt chords (those bindings are
    /// fixed). SHIFT is allowed through because uppercase Char events carry it.
    pub fn is(&self, action: &str, key: KeyEvent) -> bool {
        if key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) {
            return false;
        }
        self.map.get(action) == Some(&key.code)
    }

    pub fn key_for(&self, action: &str) -> Option<KeyCode> {
        self.map.get(action).copied()
    }

    pub fn actions() -> &'static [(&'static str, &'static str)] {
        const LIST: &[(&str, &str)] = &[
            ("quit", "quit the app"), ("sidebar", "toggle sidebar"), ("search", "search"),
            ("undo", "undo last edit"), ("edit", "edit page in $EDITOR"), ("comments", "comments panel"),
            ("queue", "queue/conflicts screen"), ("board", "toggle board view"), ("help", "help overlay"),
            ("palette", "command palette"), ("insert", "edit block text"), ("append", "add block below"),
            ("new_row", "new database row"), ("props", "property form"), ("delete", "delete (press twice)"),
            ("sort", "sort by column"), ("up", "cursor up"), ("down", "cursor down"),
            ("left", "left / collapse"), ("right", "right / expand"), ("top", "jump to top"),
            ("bottom", "jump to bottom"), ("open", "open / follow"), ("back", "back"),
            ("toggle", "toggle to-do"), ("move_card_next", "move card right"), ("move_card_prev", "move card left"),
        ];
        LIST
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_chords_never_match() {
        let km = Keymap::new();
        assert!(!km.is("undo", KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    }
}
