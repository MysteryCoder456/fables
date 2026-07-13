use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};

pub struct TerminalGuard;

fn restore() {
    let _ = disable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

impl TerminalGuard {
    pub fn enter(mouse: bool) -> anyhow::Result<TerminalGuard> {
        enable_raw_mode()?;
        crossterm::execute!(std::io::stdout(), EnterAlternateScreen)?;
        if mouse {
            crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
        }
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            prev(info);
        }));
        Ok(TerminalGuard)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

/// Hands the terminal to a subprocess (e.g. `$EDITOR`): leaves raw mode and
/// the alternate screen, runs `f`, then re-enters TUI mode. Re-entry errors
/// are ignored so this stays usable headlessly (tests, non-tty stdout);
/// callers must force a full redraw afterwards — the fresh alternate screen
/// is blank, so a diff-only draw leaves stale content.
pub fn with_suspended<R>(mouse: bool, f: impl FnOnce() -> R) -> R {
    restore();
    let result = f();
    let _ = enable_raw_mode();
    let _ = crossterm::execute!(std::io::stdout(), EnterAlternateScreen);
    if mouse {
        let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn with_suspended_passes_the_closure_result_through() {
        let out = super::with_suspended(false, || 42);
        assert_eq!(out, 42);
    }
}
