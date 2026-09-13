//! wcode-tui — the full-screen client.
//!
//! A thin, immediate-mode front-end over a session. It consumes a
//! `wcode-protocol` [`Backend`] (`Local`/`Remote`) — never an `Agent` — so
//! local and socket sessions are the same code. See `docs/tui-plan.md` and
//! `docs/tui-design.md`.
//!
//! The loop is one `select!` over four sources (terminal input, session events,
//! `ask` replies, a run-only tick) feeding a pure reducer; the reducer's
//! [`Action`]s are performed back against the backend, and a draw happens only
//! on a state change.

pub mod app;

mod clipboard;
mod event;
mod markdown;
mod terminal;
mod ui;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::EventStream;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{MissedTickBehavior, interval};
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::Request;
use wcode_protocol::Backend;

pub use crate::app::{Action, App, AppEvent, Block, Key, Status, Tool};

/// Spinner/status refresh cadence, only consulted while a run is in flight.
const TICK: Duration = Duration::from_millis(120);

/// Run the TUI against `backend` until the user quits. Enters the alternate
/// screen; restores it on every exit path. `history` is where prompt history is
/// persisted (`None` keeps it in memory only). `models` seeds the `/model`
/// picker: this crate cannot list models (no `LlmOpts`), so the caller supplies
/// them (e.g. from `list_models`).
pub async fn run(
    backend: Backend,
    status: Status,
    models: Vec<String>,
    history: Option<PathBuf>,
) -> io::Result<()> {
    let (guard, mut terminal) = terminal::enter()?;

    let mut app = App::new();
    app.set_status(status);
    app.set_models(models);
    // Attach-replay: a resumed session already has turns; show them, so the
    // transcript is never mysteriously empty (`GetHistory` is the seam for it).
    if let Ok(AgentEvent::History { messages }) = backend.ask(Request::GetHistory).await {
        app.seed_history(&messages);
    }
    if let Some(path) = &history {
        app.load_history(read_history(path));
    }

    let result = event_loop(&mut terminal, backend, &mut app).await;

    if let Some(path) = &history {
        write_history(path, app.history());
    }
    // Drop the terminal (flush) before leaving the alternate screen.
    drop(terminal);
    drop(guard);
    result
}

async fn event_loop(
    terminal: &mut terminal::Tui,
    backend: Backend,
    app: &mut App,
) -> io::Result<()> {
    let mut events = EventStream::new();
    let mut session = backend.subscribe();
    // Replies to `ask`ed requests (e.g. `/usage`) arrive here as app events.
    let (reply_tx, mut replies) = mpsc::unbounded_channel::<AppEvent>();
    let mut tick = interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    tick.tick().await; // consume the immediate first tick

    draw(terminal, app)?;

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
            incoming = session.recv() => match incoming {
                Ok(event) => app.handle(AppEvent::Agent(event)),
                // A lagging subscriber drops events rather than stalling.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                // The session is gone; keep the UI up (the user can still quit).
                Err(broadcast::error::RecvError::Closed) => {}
            },
            Some(event) = replies.recv() => app.handle(event),
            // Only fires while a run is in flight; idle, the loop parks on
            // input and draws nothing.
            _ = tick.tick(), if app.running() => app.handle(AppEvent::Tick),
        }

        for action in app.take_actions() {
            match action {
                Action::Submit(text) => {
                    let _ = backend.send(Request::Submit { text });
                }
                Action::Cancel => {
                    let _ = backend.send(Request::Cancel);
                }
                Action::Ask(request) => spawn_ask(&backend, &reply_tx, request),
                Action::Copy(text) => {
                    let _ = clipboard::copy(&text);
                }
            }
        }

        if app.dirty() {
            draw(terminal, app)?;
        }
        if app.should_quit() {
            break;
        }
    }
    Ok(())
}

/// Ask the session and feed the reply back as an app event. Runs off the main
/// loop, so a slow reply never blocks input.
fn spawn_ask(backend: &Backend, reply_tx: &mpsc::UnboundedSender<AppEvent>, request: Request) {
    let backend = backend.clone();
    let reply_tx = reply_tx.clone();
    tokio::spawn(async move {
        let event = match backend.ask(request).await {
            Ok(event) => event,
            Err(_) => AgentEvent::Error {
                message: "session closed".into(),
            },
        };
        let _ = reply_tx.send(AppEvent::Agent(event));
    });
}

/// One prompt per line; a missing/unreadable file is an empty history.
fn read_history(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|text| text.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

/// Persist prompt history, best effort (a failure is not worth a crash).
fn write_history(path: &Path, lines: &[String]) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, lines.join("\n"));
}

fn draw(terminal: &mut terminal::Tui, app: &mut App) -> io::Result<()> {
    terminal.draw(|frame| ui::draw(frame, app))?;
    app.clear_dirty();
    Ok(())
}
