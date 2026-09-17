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

pub use crate::app::{Action, App, AppEvent, Block, Key, SessionItem, Status, Tool};

/// A team member's live state, shown in the sidebar and by `/team`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamState {
    /// Not running (the default until an event says otherwise).
    Idle,
    /// A run is in flight.
    Running,
    /// The run finished.
    Done,
}

impl TeamState {
    /// The lowercase label used in the sidebar and `/team`.
    pub fn label(self) -> &'static str {
        match self {
            TeamState::Idle => "idle",
            TeamState::Running => "running",
            TeamState::Done => "done",
        }
    }
}

/// A static roster entry: a member's name and its **effective** model (the
/// composition root resolves an inherited model to a concrete id).
#[derive(Debug, Clone)]
pub struct Teammate {
    pub name: String,
    pub model: String,
}

/// A live state change for one member, pushed by the composition root.
#[derive(Debug, Clone)]
pub struct TeamUpdate {
    pub name: String,
    pub state: TeamState,
}

/// Spinner/status refresh cadence, only consulted while a run is in flight.
const TICK: Duration = Duration::from_millis(120);

/// Everything the composition root injects for a run — state this crate cannot
/// derive (it holds no `LlmOpts`, no session dir, no model list, no fs).
#[derive(Clone, Debug)]
pub struct Options {
    pub status: Status,
    /// Model ids for the `/model` picker (from `list_models`).
    pub models: Vec<String>,
    /// Resumable sessions for the `/resume` picker (local sessions only; empty
    /// when the client is remote and cannot see the session dir).
    pub sessions: Vec<SessionItem>,
    /// The team roster (name + effective model) for the sidebar and `/team`;
    /// empty when there is no team.
    pub teammates: Vec<Teammate>,
    /// Where prompt history is persisted (`None` keeps it in memory only).
    pub history: Option<PathBuf>,
}

/// How a TUI run ended — the return value tells the composition root whether to
/// continue or hand off.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// The user quit; nothing more to do.
    Quit,
    /// The user picked a session to resume (`/resume`); the caller should
    /// re-exec with `--resume <path>`.
    Resume(PathBuf),
}

/// Run the TUI against `backend` until the user quits. Enters the alternate
/// screen; restores it on every exit path. See [`Options`] for the injected
/// state and [`Outcome`] for the handoff on exit.
pub async fn run(
    backend: Backend,
    options: Options,
    team: Option<mpsc::UnboundedReceiver<TeamUpdate>>,
) -> io::Result<Outcome> {
    let Options {
        status,
        models,
        sessions,
        teammates,
        history,
    } = options;
    let (guard, mut terminal) = terminal::enter()?;

    let mut app = App::new();
    app.set_status(status);
    app.set_models(models);
    app.set_sessions(sessions);
    app.set_teammates(teammates);
    // Attach-replay: a resumed session already has turns; show them, so the
    // transcript is never mysteriously empty (`GetHistory` is the seam for it).
    if let Ok(AgentEvent::History { messages }) = backend.ask(Request::GetHistory).await {
        app.seed_history(&messages);
    }
    if let Some(path) = &history {
        app.load_history(read_history(path));
    }

    let result = event_loop(&mut terminal, backend, &mut app, team).await;

    if let Some(path) = &history {
        write_history(path, app.history());
    }
    // Drop the terminal (flush) before leaving the alternate screen.
    drop(terminal);
    drop(guard);
    result?;

    // The TUI only *decides* to resume; the composition root owns the re-exec.
    Ok(match app.pending_resume() {
        Some(path) => Outcome::Resume(path.to_path_buf()),
        None => Outcome::Quit,
    })
}

async fn event_loop(
    terminal: &mut terminal::Tui,
    backend: Backend,
    app: &mut App,
    mut team: Option<mpsc::UnboundedReceiver<TeamUpdate>>,
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
            // Live team status from the composition root's forwarders.
            Some(update) = recv_team(&mut team) => app.handle(AppEvent::Team(update)),
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

/// Await the next team update; parks forever when there is no receiver (so the
/// `select!` branch simply never fires).
async fn recv_team(team: &mut Option<mpsc::UnboundedReceiver<TeamUpdate>>) -> Option<TeamUpdate> {
    match team {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
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
