use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use notion_tui::{app, config, terminal::TerminalGuard, ui};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = match config::load() {
        Ok(cfg) => cfg,
        Err(e) if e.to_string().contains("NOTION_TOKEN") => {
            let token = notion_tui::wizard::run().await?;
            config::load_with_token(Some(token))?
        }
        Err(e) => return Err(e),
    };
    let store = Arc::new(Mutex::new(notion_store::Store::open(&cfg.db_path)?));
    let client = notion_api::NotionClient::new(cfg.token.clone());
    let mut handle =
        notion_sync::spawn_sync(client, store.clone(), Duration::from_secs(cfg.poll_interval_secs));

    let _guard = TerminalGuard::enter(cfg.mouse)?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;

    let mut app = app::App::new(store);
    app.sync_notify = Some(handle.notify.clone());
    app.mouse = cfg.mouse;
    app.editor_override = cfg.editor.clone();
    let (keymap, key_warnings) = notion_tui::keymap::Keymap::with_overrides_checked(&cfg.keys);
    app.keymap = keymap;
    let startup_warnings: Vec<String> = cfg.warnings.iter().cloned().chain(key_warnings).collect();
    if !startup_warnings.is_empty() {
        app.notice = Some(startup_warnings.join(" · "));
    }
    app.theme = notion_tui::ui::theme::named(&cfg.theme);
    app.refresh_sidebar();
    let mut events = EventStream::new();

    // Separate client instance for interactive, user-triggered background
    // fetches (take-theirs refetch, comment refresh, merge editor) — these are
    // rare one-shots, so a second independent pacing budget is fine.
    let remote_client = std::sync::Arc::new(notion_api::NotionClient::new(cfg.token.clone()));
    let (app_tx, mut app_rx) = tokio::sync::mpsc::unbounded_channel();
    app.remote = Some(app::RemoteHandle {
        client: remote_client,
        tx: app_tx,
    });

    while !app.should_quit {
        if std::mem::take(&mut app.force_redraw) {
            term.clear()?;
        }
        term.draw(|f| ui::draw(f, &mut app))?;
        tokio::select! {
            ev = events.next() => match ev {
                Some(Ok(Event::Key(key))) => app::dispatch_key(&mut app, key),
                Some(Ok(Event::Mouse(m))) => app::dispatch_mouse(&mut app, m),
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
                Some(app::AppMsg::PageGone(page_id)) => app.handle_page_gone(&page_id),
                Some(m @ app::AppMsg::MergeReady { .. }) => {
                    let editor = notion_tui::editor::editor_command(app.editor_override.as_deref());
                    let mouse = app.mouse;
                    app.open_merge_editor(m, move |initial| {
                        notion_tui::terminal::with_suspended(mouse, || {
                            notion_tui::editor::edit_text(&editor, initial)
                        })
                    });
                }
                None => {}
            },
        }
    }
    Ok(())
}
