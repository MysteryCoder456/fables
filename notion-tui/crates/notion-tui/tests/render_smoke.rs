use std::sync::{Arc, Mutex};

use notion_tui::app::App;
use notion_tui::ui;
use ratatui::{backend::TestBackend, Terminal};

fn test_store() -> notion_sync::SharedStore {
    Arc::new(Mutex::new(notion_store::Store::open_in_memory().unwrap()))
}

#[test]
fn renders_frame_with_status_bar() {
    let mut app = App::new(test_store());
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    insta::assert_snapshot!(term.backend());
}

#[test]
fn q_quits() {
    use crossterm::event::{KeyCode, KeyEvent};
    let mut app = App::new(test_store());
    notion_tui::app::handle_key(&mut app, KeyEvent::from(KeyCode::Char('q')));
    assert!(app.should_quit);
}

#[test]
fn renders_sidebar_tree() {
    use notion_store::{NodeKind, TreeNode};
    use notion_tui::ui::sidebar::SidebarState;

    let mut app = App::new(test_store());
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    app.sidebar = SidebarState::new(vec![
        TreeNode {
            id: "p1".into(),
            title: "Roadmap".into(),
            parent_id: None,
            kind: NodeKind::Page,
        },
        TreeNode {
            id: "p2".into(),
            title: "Notes".into(),
            parent_id: None,
            kind: NodeKind::Page,
        },
        TreeNode {
            id: "ds1".into(),
            title: "Tasks".into(),
            parent_id: None,
            kind: NodeKind::DataSource,
        },
    ]);
    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    insta::assert_snapshot!(term.backend());
}

#[test]
fn renders_page_view() {
    use notion_store::{BlockRec, PageRec};
    use notion_tui::app::{Focus, View};
    use notion_tui::ui::page::PageView;

    let mut app = App::new(test_store());
    app.focus = Focus::Main;
    app.sync_status = notion_sync::SyncStatus::Idle { updated: 0 };
    let page_rec = PageRec {
        id: "p1".into(),
        parent_type: "workspace".into(),
        parent_id: None,
        title: "Roadmap".into(),
        icon: None,
        archived: false,
        last_edited_time: "t".into(),
    };
    let blocks = vec![
        BlockRec {
            id: "h1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 0,
            block_type: "heading_1".into(),
            payload: "{}".into(),
            plain_text: "Q3 Plan".into(),
            has_children: false,
        },
        BlockRec {
            id: "t1".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 1,
            block_type: "to_do".into(),
            payload: r#"{"checked": false}"#.into(),
            plain_text: "Ship it".into(),
            has_children: false,
        },
        BlockRec {
            id: "cp".into(),
            page_id: "p1".into(),
            parent_block_id: None,
            ordinal: 2,
            block_type: "child_page".into(),
            payload: "{}".into(),
            plain_text: "Sub page".into(),
            has_children: false,
        },
    ];
    app.view = View::Page(PageView::new(page_rec, blocks));

    let backend = TestBackend::new(60, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| ui::draw(f, &mut app)).unwrap();
    insta::assert_snapshot!(term.backend());
}
