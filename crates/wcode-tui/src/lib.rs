//! wcode-tui — the full-screen client.
//!
//! A thin, immediate-mode front-end over one or more sessions. It consumes
//! `wcode-protocol` [`Backend`]s (`Local`/`Remote`) — never an `Agent` — so
//! local and socket sessions are the same code. See `docs/tui-plan.md` and
//! `docs/tui-design.md`.
//!
//! The loop is one `select!` over four sources (terminal input, the merged
//! per-surface session events, `ask` replies, a run-only tick) feeding a pure
//! reducer; the reducer's [`Action`]s are performed back against the **focused**
//! surface's backend, and a draw happens only on a state change.

pub mod app;

mod clipboard;
mod event;
mod markdown;
mod terminal;
mod theme;
mod ui;

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::EventStream;
use futures::StreamExt;
use tokio::sync::{broadcast, mpsc};
use tokio::time::{MissedTickBehavior, interval};
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Request, SessionId};
use wcode_protocol::Backend;

pub use crate::app::{Action, App, AppEvent, Block, Key, SessionItem, Status, Tool};
pub use crate::theme::{ThemeSpec, parse_theme};

/// A surface's live state, shown in the team strip and by `/team`.
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
    /// The lowercase label used in the team strip and `/team`.
    pub fn label(self) -> &'static str {
        match self {
            TeamState::Idle => "idle",
            TeamState::Running => "running",
            TeamState::Done => "done",
        }
    }

    /// The team strip's status glyph: running · idle · done.
    pub fn glyph(self) -> &'static str {
        match self {
            TeamState::Idle => "○",
            TeamState::Running => "●",
            TeamState::Done => "✓",
        }
    }
}

/// One surface's display identity: the id its events route by, and the
/// `label`/`model` the team strip shows.
#[derive(Clone, Debug)]
pub struct SurfaceInfo {
    pub id: SessionId,
    pub label: String,
    pub model: String,
    pub is_root: bool,
}

/// A surface to run: its identity plus the backend that streams it. Index 0 is
/// the root. (`Backend` is not `Debug`, so this cannot live in [`Options`].)
pub struct SurfaceSpec {
    pub id: SessionId,
    pub label: String,
    pub model: String,
    pub is_root: bool,
    pub backend: Backend,
}

impl SurfaceSpec {
    /// The team strip/`/team` identity for this spec — the fields the reducer
    /// tracks (the `backend` belongs to the loop, not the app).
    fn info(&self) -> SurfaceInfo {
        SurfaceInfo {
            id: self.id.clone(),
            label: self.label.clone(),
            model: self.model.clone(),
            is_root: self.is_root,
        }
    }
}

/// Spinner/status refresh cadence, only consulted while a run is in flight.
const TICK: Duration = Duration::from_millis(120);

/// Everything the composition root injects for a run — state this crate cannot
/// derive (it holds no `LlmOpts`, no session dir, no model list, no fs).
#[derive(Clone, Debug)]
pub struct Options {
    /// The root surface's status line.
    pub status: Status,
    /// Model ids for the `/model` picker (from `list_models`).
    pub models: Vec<String>,
    /// Resumable sessions for the `/resume` picker (local sessions only; empty
    /// when the client is remote and cannot see the session dir).
    pub sessions: Vec<SessionItem>,
    /// A `[theme]` overlay on palette B (empty = palette B); installed before
    /// the first draw.
    pub theme: ThemeSpec,
    /// Where the root's prompt history is persisted (`None` keeps it in memory).
    /// Where the root's prompt history is persisted (`None` keeps it in memory).
    pub history: Option<PathBuf>,
    /// True when this TUI is a `--socket` client: `/reload` (rebuild + re-exec)
    /// has no local binary or session, so it is refused.
    pub remote: bool,
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
    /// The user asked to rebuild + re-exec into the current session (`/reload`);
    /// the caller runs the build then re-execs (the REPL's `/reload`).
    /// `no_session` = start fresh with `--no-session`.
    Reload { no_session: bool },
}

/// Run the TUI over `surfaces` (index 0 is the root) until the user quits.
/// Enters the alternate screen; restores it on every exit path.
pub async fn run(
    surfaces: Vec<SurfaceSpec>,
    options: Options,
    new_surfaces: Option<mpsc::UnboundedReceiver<SurfaceSpec>>,
) -> io::Result<Outcome> {
    // Index 0 (the root) is mandatory — its backend drives the loop. Guard the
    // `pub` API against an empty list rather than panicking on `backends[0]`.
    if surfaces.is_empty() {
        return Ok(Outcome::Quit);
    }
    let Options {
        status,
        models,
        sessions,
        history,
        theme,
        remote,
    } = options;
    // Install the theme before the terminal is entered and anything draws.
    theme::install(theme);
    let (guard, mut terminal) = terminal::enter()?;

    let mut app = App::new();
    app.set_models(models);
    app.set_sessions(sessions);
    app.set_surfaces(surfaces.iter().map(SurfaceSpec::info).collect());
    // The root's full status line (members derive a reduced one).
    app.set_status(status);
    app.set_remote(remote);

    let backends: Vec<(SessionId, Backend)> = surfaces
        .iter()
        .map(|s| (s.id.clone(), s.backend.clone()))
        .collect();

    // Attach-replay: the root may already have turns; show them, so the
    // transcript is never mysteriously empty (`GetHistory` is the seam for it).
    if let Some((id, backend)) = backends.first()
        && let Ok(AgentEvent::History { messages }) = backend.ask(Request::GetHistory).await
    {
        app.seed_history(id, &messages);
    }
    if let Some(path) = &history {
        app.load_history(read_history(path));
    }

    let result = event_loop(&mut terminal, backends, &mut app, new_surfaces).await;

    if let Some(path) = &history {
        write_history(path, app.history());
    }
    // Drop the terminal (flush) before leaving the alternate screen.
    drop(terminal);
    drop(guard);
    result?;

    // The TUI only *decides* to hand off; the composition root owns the re-exec
    // — a plain `--resume` for `Resume`, or the REPL's rebuild+re-exec for
    // `Reload`.
    Ok(match app.pending_reload() {
        Some(no_session) => Outcome::Reload { no_session },
        None => match app.pending_resume() {
            Some(path) => Outcome::Resume(path.to_path_buf()),
            None => Outcome::Quit,
        },
    })
}

async fn event_loop(
    terminal: &mut terminal::Tui,
    mut backends: Vec<(SessionId, Backend)>,
    app: &mut App,
    mut new_surfaces: Option<mpsc::UnboundedReceiver<SurfaceSpec>>,
) -> io::Result<()> {
    let mut events = EventStream::new();

    // Merge every surface's event stream into one channel of `(id, event)`, so
    // the reducer can route by session. A runtime-added surface spawns its own
    // forwarder into the same channel (below).
    let (agent_tx, mut agents) = mpsc::unbounded_channel::<(SessionId, AgentEvent)>();
    for (id, backend) in &backends {
        spawn_forwarder(&agent_tx, id, backend);
    }
    // `agent_tx` stays alive: the runtime-add branch spawns a further forwarder
    // with it, and the loop is driven by input/tick, not by this channel closing.

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
            Some((id, event)) = agents.recv() => app.handle(AppEvent::Agent(id, event)),
            // A worker spawned at runtime: add its surface and forward its stream.
            Some(spec) = recv_opt(&mut new_surfaces) => {
                spawn_forwarder(&agent_tx, &spec.id, &spec.backend);
                app.add_surface(spec.info());
                backends.push((spec.id.clone(), spec.backend));
            }
            Some(event) = replies.recv() => app.handle(event),
            // Only fires while a run is in flight; idle, the loop parks on
            // input and draws nothing.
            _ = tick.tick(), if app.running() => app.handle(AppEvent::Tick),
        }

        // Actions target the focused surface's backend.
        let idx = app.focus();
        let (id, backend) = &backends[idx];
        for action in app.take_actions() {
            match action {
                Action::Submit(text) => {
                    let _ = backend.send(Request::Submit { text });
                }
                Action::Cancel => {
                    let _ = backend.send(Request::Cancel);
                }
                Action::Ask(request) => spawn_ask(id, backend, &reply_tx, request),
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

/// Forward one surface's event stream into the merged `agent_tx`, tagged with
/// its id; the forwarder ends when the subscriber closes or the loop is gone.
/// Used for the surfaces at startup and for a worker added at runtime.
fn spawn_forwarder(
    agent_tx: &mpsc::UnboundedSender<(SessionId, AgentEvent)>,
    id: &SessionId,
    backend: &Backend,
) {
    let mut rx = backend.subscribe();
    let id = id.clone();
    let tx = agent_tx.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    if tx.send((id.clone(), event)).is_err() {
                        break; // the loop is gone
                    }
                }
                // A lagging subscriber drops events rather than stalling.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                // The session is gone; stop forwarding it.
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

/// The next runtime-added surface, or a future that never resolves when the
/// feed is absent (a socket client, or a served worker which has no team).
async fn recv_opt(
    rx: &mut Option<mpsc::UnboundedReceiver<SurfaceSpec>>,
) -> Option<SurfaceSpec> {
    match rx {
        Some(r) => r.recv().await,
        None => std::future::pending().await,
    }
}

/// Ask one surface and feed the reply back as an app event, tagged with that
/// surface's id. Runs off the main loop, so a slow reply never blocks input.
fn spawn_ask(
    id: &SessionId,
    backend: &Backend,
    reply_tx: &mpsc::UnboundedSender<AppEvent>,
    request: Request,
) {
    let id = id.clone();
    let backend = backend.clone();
    let reply_tx = reply_tx.clone();
    tokio::spawn(async move {
        let event = match backend.ask(request).await {
            Ok(event) => event,
            Err(_) => AgentEvent::Error {
                message: "session closed".into(),
            },
        };
        let _ = reply_tx.send(AppEvent::Agent(id, event));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use wcode_harness::actor::SessionActor;
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::compaction::CompactionPolicy;
    use wcode_harness::hooks::HooksSet;
    use wcode_harness::loop_::DEFAULT_MAX_TURNS;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    /// A live, never-streaming session backend — enough to answer `GetHistory`.
    fn backend() -> Backend {
        let stream_fn: StreamFn = Arc::new(|_c, _s, _t, _o| {
            Box::pin(futures::stream::empty()) as LlmStream
        });
        let agent = Agent::new(AgentConfig {
            system: "test".into(),
            tools: Vec::new(),
            llm: LlmOpts::default(),
            stream_fn,
            hooks: HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::env::temp_dir(),
            max_turns: DEFAULT_MAX_TURNS,
            parallel_tools: false,
            compaction: CompactionPolicy::default(),
        });
        Backend::from(SessionActor::spawn(agent))
    }

    /// A reply is tagged with the surface it was asked FROM — the id passed to
    /// `spawn_ask`, not whatever is focused when the reply lands.
    #[tokio::test]
    async fn spawn_ask_tags_the_reply_with_the_asked_surface() {
        let backend = backend();
        let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
        let asked = SessionId::agent("w7");

        spawn_ask(&asked, &backend, &tx, Request::GetHistory);

        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("a reply within the timeout")
            .expect("the channel stays open");
        match event {
            AppEvent::Agent(id, AgentEvent::History { .. }) => assert_eq!(id, asked),
            other => panic!("expected Agent(asked-id, History), got {other:?}"),
        }
    }

    /// A surface handed over the runtime feed is delivered to the loop's arm —
    /// the headless seam the event loop's runtime-add branch reads.
    #[tokio::test]
    async fn recv_opt_yields_a_runtime_surface() {
        let (tx, rx) = mpsc::unbounded_channel::<SurfaceSpec>();
        let mut feed = Some(rx);
        tx.send(SurfaceSpec {
            id: SessionId::agent("explorer"),
            label: "explorer".into(),
            model: "m".into(),
            is_root: false,
            backend: backend(),
        })
        .unwrap();

        let got = recv_opt(&mut feed).await.expect("a spec");
        assert_eq!(got.label, "explorer");
        assert_eq!(got.id.as_str(), "agent:explorer");
    }

    /// An empty surface list quits cleanly — no `backends[0]` panic.
    #[tokio::test]
    async fn run_with_no_surfaces_quits_without_panicking() {
        let options = Options {
            status: Status::default(),
            models: Vec::new(),
            sessions: Vec::new(),
            history: None,
            theme: ThemeSpec::default(),
            remote: false,
        };
        assert_eq!(run(Vec::new(), options, None).await.unwrap(), Outcome::Quit);
    }
}