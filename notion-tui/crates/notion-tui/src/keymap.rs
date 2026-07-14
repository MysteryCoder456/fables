use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const DEFAULTS: &[(&str, &str, KeyCode)] = &[
    ("quit", "quit the app", KeyCode::Char('q')),
    ("sidebar", "toggle sidebar", KeyCode::Char('1')),
    ("search", "search", KeyCode::Char('/')),
    ("undo", "undo last edit (session-only)", KeyCode::Char('u')),
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
    ("filter", "filter column", KeyCode::Char('f')),
    ("filter_row", "filter row (free text)", KeyCode::Char('F')),
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

pub type Chord = (KeyCode, KeyModifiers);

/// Chords the app hard-codes (spec §5 descope: fixed keys stay fixed).
/// `with_overrides_checked` refuses to bind actions onto these.
pub const FIXED_CHORDS: &[Chord] = &[
    (KeyCode::Tab, KeyModifiers::NONE),
    (KeyCode::Char('d'), KeyModifiers::CONTROL),
    (KeyCode::Char('u'), KeyModifiers::CONTROL),
    (KeyCode::Char('p'), KeyModifiers::CONTROL),
];

/// Rows for the help overlay's fixed-keys section (documentation superset of
/// FIXED_CHORDS: backspace/esc are hard-coded in handlers, not collision-checked).
pub const FIXED_KEYS_HELP: &[(&str, &str)] = &[
    ("tab", "switch sidebar/main focus (fixed)"),
    ("ctrl+d", "half-page down (fixed)"),
    ("ctrl+u", "half-page up (fixed)"),
    ("ctrl+p", "search (fixed)"),
    ("backspace", "back to previous page (fixed alias of 'back')"),
    ("esc", "close modal / cancel (fixed)"),
];

fn parse_key(s: &str) -> Option<Chord> {
    if let Some(rest) = s.strip_prefix("ctrl+") {
        let mut chars = rest.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) => Some((KeyCode::Char(c.to_ascii_lowercase()), KeyModifiers::CONTROL)),
            _ => None,
        };
    }
    let code = match s {
        "esc" => KeyCode::Esc,
        "enter" => KeyCode::Enter,
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "backspace" => KeyCode::Backspace,
        _ => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    };
    Some((code, KeyModifiers::NONE))
}

#[derive(Clone)]
pub struct Keymap {
    map: HashMap<String, Chord>,
}

impl Keymap {
    pub fn new() -> Keymap {
        Keymap {
            map: DEFAULTS
                .iter()
                .map(|(a, _, k)| (a.to_string(), (*k, KeyModifiers::NONE)))
                .collect(),
        }
    }

    pub fn with_overrides_checked(overrides: &HashMap<String, String>) -> (Keymap, Vec<String>) {
        let mut km = Keymap::new();
        let mut warnings = Vec::new();
        for (action, key_str) in overrides {
            if !DEFAULTS.iter().any(|(a, _, _)| a == action) {
                warnings.push(format!("keys.{action}: unknown action"));
                continue;
            }
            match parse_key(key_str) {
                Some(chord) => {
                    if FIXED_CHORDS.contains(&chord) {
                        warnings.push(format!(
                            "keys.{action}: \"{key_str}\" is a fixed key and can't be rebound"
                        ));
                        continue;
                    }
                    km.map.insert(action.clone(), chord);
                }
                None => warnings.push(format!("keys.{action}: can't parse key \"{key_str}\"")),
            }
        }
        (km, warnings)
    }

    pub fn with_overrides(overrides: &HashMap<String, String>) -> Keymap {
        Self::with_overrides_checked(overrides).0
    }

    /// Matches code + the binding's CONTROL bit. ALT always rejects; SHIFT
    /// rides along because uppercase Char events carry it.
    pub fn is(&self, action: &str, key: KeyEvent) -> bool {
        if key.modifiers.contains(KeyModifiers::ALT) {
            return false;
        }
        let Some((code, mods)) = self.map.get(action) else {
            return false;
        };
        key.modifiers.contains(KeyModifiers::CONTROL) == mods.contains(KeyModifiers::CONTROL)
            && *code == key.code
    }

    pub fn key_for(&self, action: &str) -> Option<Chord> {
        self.map.get(action).copied()
    }

    pub fn actions() -> &'static [(&'static str, &'static str)] {
        const LIST: &[(&str, &str)] = &[
            ("quit", "quit the app"),
            ("sidebar", "toggle sidebar"),
            ("search", "search"),
            ("undo", "undo last edit (session-only)"),
            ("edit", "edit page in $EDITOR"),
            ("comments", "comments panel"),
            ("queue", "queue/conflicts screen"),
            ("board", "toggle board view"),
            ("help", "help overlay"),
            ("palette", "command palette"),
            ("insert", "edit block text"),
            ("append", "add block below"),
            ("new_row", "new database row"),
            ("props", "property form"),
            ("delete", "delete (press twice)"),
            ("sort", "sort by column"),
            ("filter", "filter column"),
            ("filter_row", "filter row (free text)"),
            ("up", "cursor up"),
            ("down", "cursor down"),
            ("left", "left / collapse"),
            ("right", "right / expand"),
            ("top", "jump to top"),
            ("bottom", "jump to bottom"),
            ("open", "open / follow"),
            ("back", "back"),
            ("toggle", "toggle to-do"),
            ("move_card_next", "move card right"),
            ("move_card_prev", "move card left"),
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

    #[test]
    fn bad_overrides_warn_and_keep_defaults() {
        let mut o = HashMap::new();
        o.insert("quit".to_string(), "ctrl+shift+q".to_string()); // unparseable
        o.insert("nosuch".to_string(), "x".to_string()); // unknown action
        let (km, warnings) = Keymap::with_overrides_checked(&o);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings
            .iter()
            .any(|w| w.contains("nosuch") && w.contains("unknown action")));
        assert!(warnings.iter().any(|w| w.contains("ctrl+shift+q")));
        // the bad override must NOT clobber the default:
        assert!(km.is("quit", KeyEvent::from(KeyCode::Char('q'))));
    }

    #[test]
    fn ctrl_syntax_binds_a_control_chord() {
        let mut o = HashMap::new();
        o.insert("undo".to_string(), "ctrl+z".to_string());
        let (km, warnings) = Keymap::with_overrides_checked(&o);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(km.is("undo", KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL)));
        assert!(!km.is("undo", KeyEvent::from(KeyCode::Char('z')))); // plain z is not the chord
        assert!(!km.is("undo", KeyEvent::from(KeyCode::Char('u')))); // old default replaced
    }

    #[test]
    fn fixed_chords_cannot_be_claimed() {
        for reserved in ["ctrl+d", "ctrl+u", "ctrl+p", "tab"] {
            let mut o = HashMap::new();
            o.insert("quit".to_string(), reserved.to_string());
            let (km, warnings) = Keymap::with_overrides_checked(&o);
            assert_eq!(warnings.len(), 1, "{reserved} must warn");
            assert!(warnings[0].contains("fixed"), "{}", warnings[0]);
            assert!(
                km.is("quit", KeyEvent::from(KeyCode::Char('q'))),
                "default must survive"
            );
        }
    }

    #[test]
    fn plain_bindings_still_reject_ctrl_chords() {
        let km = Keymap::new();
        assert!(!km.is("down", KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL)));
    }
}
