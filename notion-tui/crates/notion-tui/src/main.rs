use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{Event, EventStream, MouseEventKind};
use futures::StreamExt;
use notion_tui::{app, config, terminal::TerminalGuard, ui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = config::load()?;
    let store = Arc::new(Mutex::new(notion_store::Store::open(&cfg.db_path)?));
    let client = notion_api::NotionClient::new(cfg.token.clone());
    let mut handle =
        notion_sync::spawn_sync(client, store.clone(), Duration::from_secs(cfg.poll_interval_secs));

    let _guard = TerminalGuard::enter(cfg.mouse)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;

    let mut app = app::App::new(store);
    app.editor_override = cfg.editor.clone();
    app.keymap = notion_tui::keymap::Keymap::with_overrides(&cfg.keys);
    app.refresh_sidebar();
    let mut events = EventStream::new();

    // Separate client instance for interactive, user-triggered background
    // fetches (take-theirs refetch, comment refresh, merge editor) — these are
    // rare one-shots, so a second independent pacing budget is fine.
    let remote_client = std::sync::Arc::new(notion_api::NotionClient::new(cfg.token.clone()));
    let (app_tx, mut app_rx) = tokio::sync::mpsc::unbounded_channel();
    app.remote = Some(app::RemoteHandle { client: remote_client, tx: app_tx });

    while !app.should_quit {
        term.draw(|f| ui::draw(f, &app))?;
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(key))) => app::dispatch_key(&mut app, key),
                Some(Ok(Event::Mouse(m))) => match m.kind {
                    MouseEventKind::ScrollDown => app::scroll(&mut app, 3),
                    MouseEventKind::ScrollUp => app::scroll(&mut app, -3),
                    _ => {}
                },
                Some(Ok(_)) => {}
                _ => break,
            },
            _ = handle.data_version.changed() => {
                app.refresh_sidebar();
                app.refresh_current_view();
            }
            _ = handle.status.changed() => {
                app.sync_status = handle.status.borrow().clone();
            }
            _ = handle.pending.changed() => {
                app.pending = *handle.pending.borrow();
                app.refresh_conflicted();
            }
            msg = app_rx.recv() => match msg {
                Some(app::AppMsg::Refreshed) => {
                    app.refresh_current_view();
                    app.refresh_comments();
                }
                Some(m @ app::AppMsg::MergeReady { .. }) => {
                    let editor = notion_tui::editor::editor_command(app.editor_override.as_deref());
                    app.open_merge_editor(m, move |initial| {
                        notion_tui::editor::edit_text(&editor, initial)
                    });
                }
                None => {}
            },
        }
    }
    Ok(())
}
