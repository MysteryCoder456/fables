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

    let _guard = TerminalGuard::enter()?;
    let mut term = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout()))?;

    let mut app = app::App::new(store);
    app.refresh_sidebar();
    let mut events = EventStream::new();

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
        }
    }
    Ok(())
}
