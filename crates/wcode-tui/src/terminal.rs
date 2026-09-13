//! Terminal lifecycle: alt-screen + raw mode in, full restore on *every* exit
//! path (normal, error, panic). The RAII guard is the contract — nothing else
//! in the crate may leave the terminal dirty.

use std::io::{self, Stdout};
use std::sync::Once;

use crossterm::cursor::Show;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

/// The terminal the event loop draws to.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Restores the terminal on drop. Hold it for the whole session.
pub struct TerminalGuard;

/// Enter raw mode + the alternate screen, and arm the panic hook.
pub fn enter() -> io::Result<(TerminalGuard, Tui)> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen, EnableBracketedPaste)?;
    install_panic_hook();
    let terminal = Terminal::new(CrosstermBackend::new(out))?;
    Ok((TerminalGuard, terminal))
}

/// Undo everything [`enter`] set up. Idempotent — safe to call twice (guard
/// drop *and* panic hook).
fn restore() {
    let _ = disable_raw_mode();
    let mut out = io::stdout();
    let _ = execute!(out, DisableBracketedPaste, LeaveAlternateScreen, Show);
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore();
    }
}

static HOOK: Once = Once::new();

/// Restore the terminal before a panic message prints, then defer to the
/// previous hook. Installed once.
fn install_panic_hook() {
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore();
            previous(info);
        }));
    });
}
