use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::Store;
use notion_tui::app::{dispatch_key, App};
use notion_tui::keymap::Keymap;

#[test]
fn default_binding_and_override() {
    let km = Keymap::new();
    assert!(km.is("quit", KeyEvent::from(KeyCode::Char('q'))));

    let mut overrides = HashMap::new();
    overrides.insert("quit".to_string(), "x".to_string());
    let km = Keymap::with_overrides(&overrides);
    assert!(km.is("quit", KeyEvent::from(KeyCode::Char('x'))));
    assert!(!km.is("quit", KeyEvent::from(KeyCode::Char('q'))));
    // Named keys parse:
    overrides.insert("open".to_string(), "enter".to_string());
    let km = Keymap::with_overrides(&overrides);
    assert!(km.is("open", KeyEvent::from(KeyCode::Enter)));
}

#[test]
fn rebound_quit_key_quits_the_app() {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut app = App::new(store);
    let mut overrides = HashMap::new();
    overrides.insert("quit".to_string(), "x".to_string());
    app.keymap = Keymap::with_overrides(&overrides);

    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('q')));
    assert!(!app.should_quit); // old key no longer bound
    dispatch_key(&mut app, KeyEvent::from(KeyCode::Char('x')));
    assert!(app.should_quit);
}
