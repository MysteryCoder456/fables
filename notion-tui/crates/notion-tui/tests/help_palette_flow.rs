use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent};
use notion_store::Store;
use notion_tui::app::{dispatch_key, App, Focus, View};

fn app() -> App {
    let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
    let mut a = App::new(store);
    a.focus = Focus::Main;
    a
}

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}

#[test]
fn question_mark_toggles_help_and_any_key_closes() {
    let mut a = app();
    dispatch_key(&mut a, key('?'));
    assert!(a.help_open);
    dispatch_key(&mut a, key('j'));
    assert!(!a.help_open);
}

#[test]
fn palette_filters_and_runs_queue_command() {
    let mut a = app();
    dispatch_key(&mut a, key(':'));
    assert!(a.palette.is_some());
    dispatch_key(&mut a, key('q'));
    dispatch_key(&mut a, key('u'));
    dispatch_key(&mut a, KeyEvent::from(KeyCode::Enter));
    assert!(a.palette.is_none());
    assert!(matches!(a.view, View::Queue(_)));
}

#[test]
fn palette_esc_closes_without_running() {
    let mut a = app();
    dispatch_key(&mut a, key(':'));
    dispatch_key(&mut a, KeyEvent::from(KeyCode::Esc));
    assert!(a.palette.is_none());
    assert!(matches!(a.view, View::Empty));
}

#[test]
fn help_overlay_lists_fixed_keys_and_session_only_undo() {
    use notion_tui::ui;
    use ratatui::{backend::TestBackend, Terminal};

    let store = std::sync::Arc::new(std::sync::Mutex::new(
        notion_store::Store::open_in_memory().unwrap(),
    ));
    let mut app = notion_tui::app::App::new(store);
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    app.help_open = true;
    let backend = TestBackend::new(80, 50);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    let content: String = term
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    for needle in ["fixed keys", "ctrl+d", "ctrl+p", "backspace", "session-only"] {
        assert!(content.contains(needle), "help missing {needle:?}:\n{content}");
    }
}
