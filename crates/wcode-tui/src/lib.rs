//! wcode-tui — the full-screen client.
//!
//! A thin, immediate-mode front-end over a session. In P0c it consumes a
//! `wcode-protocol` `Backend` (`Local`/`Remote`); for now it is the skeleton —
//! terminal lifecycle, input, and the event loop. See `docs/tui-plan.md` and
//! `docs/tui-design.md`.
//!
//! The loop is one `select!` over three sources (terminal input, session
//! events, a run-only tick) feeding a pure reducer, drawing only when the
//! reducer reports a change.

pub mod app;

mod event;
mod terminal;
mod ui;

use std::io;
use std::time::Duration;

use crossterm::event::EventStream;
use futures::StreamExt;
use tokio::time::{MissedTickBehavior, interval};

use crate::app::{App, AppEvent};

/// Spinner/status refresh cadence, only consulted while a run is in flight.
const TICK: Duration = Duration::from_millis(120);

/// Run the TUI until the user quits. Enters the alternate screen; restores it
/// on every exit path.
pub async fn run() -> io::Result<()> {
    let (guard, mut terminal) = terminal::enter()?;
    let result = event_loop(&mut terminal).await;
    // Drop the terminal (flush) before leaving the alternate screen.
    drop(terminal);
    drop(guard);
    result
}

async fn event_loop(terminal: &mut terminal::Tui) -> io::Result<()> {
    let mut app = App::new();
    let mut events = EventStream::new();
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tick.tick().await; // consume the immediate first tick

    draw(terminal, &mut app)?;

    loop {
        tokio::select! {
            maybe = events.next() => match maybe {
                Some(Ok(event)) => {
                    for app_event in event::translate(event) {
                        app.handle(app_event);
                    }
                }
                // Stream ended or errored: the terminal is gone; bail.
                Some(Err(_)) | None => break,
            },
            // Only fires while a run is in flight; idle, the loop parks on
            // input and draws nothing.
            _ = tick.tick(), if app.running() => app.handle(AppEvent::Tick),
        }

        if app.dirty() {
            draw(terminal, &mut app)?;
        }
        if app.should_quit() {
            break;
        }
    }
    Ok(())
}

fn draw(terminal: &mut terminal::Tui, app: &mut App) -> io::Result<()> {
    terminal.draw(|frame| ui::draw(frame, app))?;
    app.clear_dirty();
    Ok(())
}
