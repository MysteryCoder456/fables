use std::sync::{Arc, Mutex};

use notion_tui::app::App;
use notion_tui::ui;
use ratatui::style::Color;
use ratatui::{backend::TestBackend, Terminal};

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

/// The dark theme's border color (theme.rs: Color::Indexed(240)) must appear
/// in the frame whenever a modal is open — proving the modal took the theme.
fn assert_dark_border(app: &mut App, which: &str) {
    app.theme = notion_tui::ui::theme::named("dark");
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(60, 20);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, app)).unwrap();
    let themed = term
        .backend()
        .buffer()
        .content
        .iter()
        .any(|c| c.style().fg == Some(Color::Indexed(240)));
    assert!(themed, "{which} modal ignored the dark theme's border color");
}

#[test]
fn search_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true; // sidebar draws its own themed border; hide it so only the modal can pass
    app.search = Some(notion_tui::ui::search::SearchState::new());
    assert_dark_border(&mut app, "search");
}

#[test]
fn input_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.input = Some(notion_tui::ui::input::InputState::new("t", ""));
    assert_dark_border(&mut app, "input");
}

#[test]
fn confirm_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.confirm = Some(notion_tui::ui::confirm::ConfirmState {
        message: "sure?".into(),
        ids: vec![],
        kind: notion_tui::ui::confirm::ConfirmKind::DeleteProtected,
    });
    assert_dark_border(&mut app, "confirm");
}

#[test]
fn palette_and_help_are_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.palette = Some(notion_tui::ui::palette::PaletteState::new());
    assert_dark_border(&mut app, "palette");
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.help_open = true;
    assert_dark_border(&mut app, "help");
}

#[test]
fn props_modal_is_themed() {
    let mut app = App::new(test_store());
    app.sidebar.hidden = true;
    app.props = Some(notion_tui::ui::props::PropsState::new(
        "r1".into(),
        vec![notion_tui::ui::props::PropField {
            name: "Name".into(),
            prop_type: "title".into(),
            value_text: "x".into(),
            options: vec![],
        }],
    ));
    assert_dark_border(&mut app, "props");
}
