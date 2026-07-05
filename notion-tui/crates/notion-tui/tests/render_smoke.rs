use notion_tui::app::App;
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};

#[test]
fn renders_frame_with_status_bar() {
    let mut app = App::new();
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(term.backend());
}

#[test]
fn q_quits() {
    use crossterm::event::{KeyCode, KeyEvent};
    let mut app = App::new();
    notion_tui::app::handle_key(&mut app, KeyEvent::from(KeyCode::Char('q')));
    assert!(app.should_quit);
}
