//! Serve sessions over a socket: accept connections, bridge frames ↔ handles.
//!
//! Each connection gets a writer draining an outbound queue, one **fan** task per
//! served session (forwarding that session's events, each frame stamped with its
//! origin session), and the read loop answering inbound requests. A request is
//! demuxed to the handle named by its `Frame::session`; a single-session server
//! also accepts any id (the legacy default, so a `--socket` client's placeholder
//! still reaches it). Requests are answered with the actor's reply (correlated by
//! the envelope's `reply_to`), so a client's `ask` resolves even while streamed
//! events flow on the same connection.
//!
//! The roster is **live**: beside an explicit root (served under its own session
//! id, distinct from the registry's `agent:orchestrator` alias), the server
//! follows a [`watch::Receiver`] of the extra sessions. A session registered
//! after the server started — a worker a running orchestrator spawns — is fanned
//! to every connection, and the grown roster is **pushed** as an unsolicited
//! [`AgentEvent::Sessions`] frame, so a client learns new ids without polling.

use std::collections::HashSet;
use std::io;
use std::path::Path;

use tokio::io::{AsyncRead, AsyncWrite, BufReader, BufWriter};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc, watch};
use wcode_harness::actor::SessionHandle;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId, SessionInfo};

use crate::Registry;
use crate::frame::{read_frame, write_frame};

/// The live roster of *extra* sessions served beside the root — a session is
/// `(SessionId, SessionHandle)`. Empty means "root only"; the value is refreshed
/// by a [`crate::Registry`] as sessions are registered.
pub type Roster = watch::Receiver<Vec<(SessionId, SessionHandle)>>;

/// A served session: the id every outbound frame carrying it is stamped with,
/// and the handle the server reads events from / writes requests to.
pub type Root = (SessionId, SessionHandle);

/// The arguments of a [`wcode_harness::protocol::Request::Define`], passed to a
/// [`DefineHandler`]. Mirrors the CLI's worker spec.
#[derive(Clone, Debug, Default)]
pub struct DefineArgs {
    /// The worker's address; auto-assigned when absent.
    pub name: Option<String>,
    /// Model id override; `None` inherits the server's model.
    pub model: Option<String>,
    /// Role text appended to the worker's system prompt.
    pub role: Option<String>,
    /// Tool allow-list; `None` = the full default set.
    pub tools: Option<Vec<String>>,
    /// Provider base URL override; `None` inherits the server's.
    pub base_url: Option<String>,
    /// Provider API key override; `None` inherits the server's.
    pub api_key: Option<String>,
    /// Enforce read-only for the defined worker (D1/D2).
    pub read_only: bool,
    /// Reasoning-effort override (D3); `None` inherits, synonyms clear.
    pub effort: Option<String>,
}

/// Builds a worker from [`DefineArgs`] and returns its address. A socket server
/// uses it to answer [`wcode_harness::protocol::Request::Define`]; the
/// composition root injects one, since this crate cannot name a session factory.
pub type DefineHandler =
    std::sync::Arc<dyn Fn(DefineArgs) -> Result<SessionId, String> + Send + Sync>;

/// Bind `path` and serve the root session plus `roster` until the listener
/// errors.
///
/// `root` is served first and labelled by its own `SessionId` (the agent's
/// session id — *not* the registry's `agent:orchestrator` alias). Every session
/// named by `roster` is served too, now and as it grows. A client addresses a
/// session by `frame.session`; a server whose *current* served set is exactly
/// one session also accepts any id, preserving the single-session flow (the
/// `--socket` client's placeholder `"remote"` still reaches it). `define`, when
/// installed, answers `Request::Define` — the composition root's worker factory.
///
/// `registry` is consulted (read-only) for each served session's model, so the
/// roster this server pushes names models the server actually knows.
#[cfg(unix)]
pub async fn serve(
    registry: Registry,
    roster: Roster,
    root: Root,
    define: Option<DefineHandler>,
    listener: UnixListener,
) -> io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(serve_stream(
            registry.clone(),
            roster.clone(),
            root.clone(),
            define.clone(),
            stream,
        ));
    }
}

/// Bind `path`, then [`serve`].
#[cfg(unix)]
pub async fn serve_at(
    registry: Registry,
    roster: Roster,
    root: Root,
    define: Option<DefineHandler>,
    path: &Path,
) -> io::Result<()> {
    let listener = crate::socket::bind(path).await?;
    serve(registry, roster, root, define, listener).await
}

/// Serve the frame loop over the process's own stdin/stdout — the `--stdio`
/// transport (`docs/vscode-extension-plan.md` §3.1). **Exactly one client**:
/// there is no listener, so no second connection is even expressible. Returns
/// `Ok(())` when the client's stdin reaches EOF (clean shutdown; the spawning
/// client owns the process lifetime).
pub async fn serve_stdio(
    registry: Registry,
    roster: Roster,
    root: Root,
    define: Option<DefineHandler>,
) -> io::Result<()> {
    let stream = tokio::io::join(tokio::io::stdin(), tokio::io::stdout());
    serve_stream(registry, roster, root, define, stream).await;
    Ok(())
}

/// Drive **one** connection over any byte stream: the frame loop (read → demux
/// → actor), the per-session [`fan`] tasks, the live roster push, and the
/// request replies. Transport-free — the caller supplies the bytes. Returns
/// when the peer's stream reaches a clean EOF (`read_frame` → `Ok(None)`) or
/// the reader/writer errors.
///
/// The ONE unix assumption removed: `UnixStream::into_split` becomes
/// `tokio::io::split`, which needs only `AsyncRead + AsyncWrite`.
pub async fn serve_stream<S>(
    registry: Registry,
    mut roster: Roster,
    root: Root,
    define: Option<DefineHandler>,
    stream: S,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    let (out, out_rx) = mpsc::unbounded_channel::<Frame<AgentEvent>>();
    tokio::spawn(write_loop(write, out_rx));

    let (root_id, root_handle) = root;
    // Every id already fanned to this connection, so a session is fanned once.
    let mut seen: HashSet<SessionId> = HashSet::new();

    // Fan the root, then every session already in the roster. `borrow_and_update`
    // marks the current value seen, so the watch task below only reacts to
    // changes made *after* this snapshot — no id is fanned twice and none is
    // missed.
    seen.insert(root_id.clone());
    fan(&out, &root_id, &root_handle);
    let initial = roster.borrow_and_update().clone();
    for (id, handle) in &initial {
        if seen.insert(id.clone()) {
            fan(&out, id, handle);
        }
    }
    // Seed the connection with the roster it starts with, so a client learns the
    // served ids on connect — and a session registered in the window before this
    // task first ran (already folded into the snapshot above) is not lost to the
    // watch's "seen" mark.
    let _ = out.send(Frame {
        v: PROTOCOL_VERSION,
        id: 0,
        reply_to: None,
        session: root_id.clone(),
        sender: None,
        body: AgentEvent::Sessions {
            sessions: roster_infos(&registry, &root_id, &initial),
        },
    });

    // The read loop needs the live roster too, for routing and `ListSessions`.
    // A clone is a read-only view (its own seen-pointer is irrelevant); the
    // watch task owns the original, whose seen-pointer stays at the snapshot.
    let read_roster = roster.clone();

    // Follow the roster: fan each newly-seen session once and push the grown
    // roster to this connection, so a client learns late ids without polling.
    {
        let out = out.clone();
        let root_id = root_id.clone();
        let registry = registry.clone();
        tokio::spawn(async move {
            loop {
                if roster.changed().await.is_err() {
                    break;
                }
                let current = roster.borrow_and_update().clone();
                // Push on ANY roster change, not just growth: a state-only
                // transition (a worker goes Running) leaves `locals()` identical,
                // so a `grew` test would miss it — but `set_state` bumped the
                // watch, so this task ran. The payload is tiny (one `SessionInfo`
                // per member). `fan()` is still gated on `seen.insert`, so no
                // event is fanned twice.
                for (id, handle) in &current {
                    if seen.insert(id.clone()) {
                        fan(&out, id, handle);
                    }
                }
                let sessions = roster_infos(&registry, &root_id, &current);
                let _ = out.send(Frame {
                    v: PROTOCOL_VERSION,
                    id: 0,
                    reply_to: None,
                    session: root_id.clone(),
                    sender: None,
                    body: AgentEvent::Sessions { sessions },
                });
            }
        });
    }

    // Answer each request (in its own task, so a long run never blocks reads).
    let mut read = BufReader::new(read);
    while let Ok(Some(frame)) = read_frame::<_, Request>(&mut read).await {
        let Frame {
            id, session, sender, body, ..
        } = frame;

        // The served set: the root first, then the live roster (deduplicated).
        let live: Vec<(SessionId, SessionHandle)> = read_roster
            .borrow()
            .iter()
            .filter(|(sid, _)| sid != &root_id)
            .cloned()
            .collect();

        // `ListSessions` names no session: the server answers it directly with
        // the roster (root first), before any demux.
        if matches!(body, Request::ListSessions) {
            let sessions = roster_infos(&registry, &root_id, &live);
            let _ = out.send(Frame {
                v: PROTOCOL_VERSION,
                id,
                reply_to: Some(id),
                session,
                sender: None,
                body: AgentEvent::Sessions { sessions },
            });
            continue;
        }

        // `Define` names no session either: the server answers it directly, via
        // the injected handler (or a clean error when none is installed). The
        // handler registers into the same registry the roster watches, so the
        // new worker is pushed to every client with no extra work.
        if let Request::Define {
            name,
            model,
            role,
            tools,
            base_url,
            api_key,
            read_only,
            effort,
        } = &body
        {
            let reply = match &define {
                Some(handler) => match handler(DefineArgs {
                    name: name.clone(),
                    model: model.clone(),
                    role: role.clone(),
                    tools: tools.clone(),
                    base_url: base_url.clone(),
                    api_key: api_key.clone(),
                    read_only: *read_only,
                    effort: effort.clone(),
                }) {
                    Ok(id) => AgentEvent::Spawned { worker: id },
                    Err(message) => AgentEvent::Error { message },
                },
                None => AgentEvent::Error {
                    message: "agent definition is not enabled on this server (serve --agents)"
                        .into(),
                },
            };
            let _ = out.send(Frame {
                v: PROTOCOL_VERSION,
                id,
                reply_to: Some(id),
                session,
                sender: None,
                body: reply,
            });
            continue;
        }

        // Demux on `frame.session`: the root; a roster session of that name; the
        // sole session of a single-session server (any id — the legacy default);
        // or, on a multi-session server, a correlated error echoing the request.
        let handle = if session == root_id {
            Some(root_handle.clone())
        } else if let Some((_, handle)) = live.iter().find(|(sid, _)| sid == &session) {
            Some(handle.clone())
        } else if live.is_empty() {
            Some(root_handle.clone())
        } else {
            None
        };
        let Some(handle) = handle else {
            let message = format!("unknown session {session}");
            let _ = out.send(Frame {
                v: PROTOCOL_VERSION,
                id,
                reply_to: Some(id),
                session,
                sender: None,
                body: AgentEvent::Error { message },
            });
            continue;
        };

        let out = out.clone();
        tokio::spawn(async move {
            // A peer's frame carries its address in `from`; feed it to the
            // target so `before_inbound` and the sender tag see it (A2A).
            let reply = match sender {
                Some(from) => handle.ask_from(from, body).await,
                None => handle.ask(body).await,
            };
            if let Ok(reply) = reply {
                let frame = Frame {
                    v: PROTOCOL_VERSION,
                    id,
                    reply_to: Some(id),
                    session,
                    sender: None,
                    body: reply,
                };
                let _ = out.send(frame);
            }
        });
    }
}

/// The served sessions in order: the root first, then each roster session (minus
/// the root, if a roster ever names it) — each joined with the model the registry
/// knows for it (`None` when unset). The push payload of `AgentEvent::Sessions`.
fn roster_infos(
    registry: &Registry,
    root_id: &SessionId,
    roster: &[(SessionId, SessionHandle)],
) -> Vec<SessionInfo> {
    let mut ids = vec![root_id.clone()];
    for (id, _) in roster {
        if id != root_id {
            ids.push(id.clone());
        }
    }
    ids.into_iter()
        .map(|id| {
            let model = registry.model_of(&id);
            let state = registry.state_of(&id);
            SessionInfo { id, model, state }
        })
        .collect()
}

/// Forward one session's events to a connection, each frame stamped with its
/// origin session — the routing key the client demuxes on. Ends when the
/// subscriber closes or the connection's queue is gone.
fn fan(out: &mpsc::UnboundedSender<Frame<AgentEvent>>, session: &SessionId, handle: &SessionHandle) {
    let mut events = handle.subscribe();
    let out = out.clone();
    let session = session.clone();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let frame = Frame {
                        v: PROTOCOL_VERSION,
                        id: 0,
                        reply_to: None,
                        session: session.clone(),
                        sender: None,
                        body: event,
                    };
                    if out.send(frame).is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

async fn write_loop<W>(write: W, mut rx: mpsc::UnboundedReceiver<Frame<AgentEvent>>)
where
    W: AsyncWrite + Unpin,
{
    let mut write = BufWriter::new(write);
    while let Some(frame) = rx.recv().await {
        if write_frame(&mut write, &frame).await.is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use wcode_harness::actor::SessionActor;
    use wcode_harness::agent::{Agent, AgentConfig};
    use wcode_harness::event::LlmStreamEvent;
    use wcode_harness::message::StopReason;
    use wcode_harness::streamfn::{LlmOpts, LlmStream, StreamFn};

    /// A root session whose model streams one text delta then stops — enough to
    /// drive a real `Submit` turn over the transport. There is no shared
    /// constructor to reuse (`registry.rs`'s `session_with` is private to its
    /// own test module), so this is the minimal one, mirroring
    /// `tests/roundtrip.rs`.
    fn test_root() -> (Root, Registry) {
        let script = Arc::new(Mutex::new(VecDeque::from([vec![
            LlmStreamEvent::TextDelta("hi".into()),
            LlmStreamEvent::Done {
                stop_reason: StopReason::Stop,
                usage: None,
            },
        ]])));
        let stream_fn: StreamFn = Arc::new(move |_ctx, _sys, _tools, _opts| {
            let events = script.lock().unwrap().pop_front().unwrap_or_default();
            Box::pin(futures::stream::iter(events)) as LlmStream
        });
        let handle = SessionActor::spawn(Agent::new(AgentConfig {
            system: "sys".into(),
            tools: Vec::new(),
            llm: LlmOpts {
                model: "m".into(),
                ..LlmOpts::default()
            },
            stream_fn,
            hooks: wcode_harness::hooks::HooksSet::default(),
            session: None,
            context: Vec::new(),
            working_dir: std::path::PathBuf::new(),
            max_turns: wcode_harness::loop_::DEFAULT_MAX_TURNS,
            parallel_tools: true,
            compaction: wcode_harness::compaction::CompactionPolicy::default(),
            plan_mode: wcode_harness::hooks::PlanModeHandle::new(),
        }));
        ((SessionId::new("test"), handle), Registry::new())
    }

    /// A `serve_stream` driver over a fresh duplex pair, with an empty roster.
    fn driver(server: tokio::io::DuplexStream) -> tokio::task::JoinHandle<()> {
        let (root, registry) = test_root();
        let roster: Roster = tokio::sync::watch::channel(Vec::new()).1;
        tokio::spawn(serve_stream(registry, roster, root, None, server))
    }

    #[tokio::test]
    async fn stdio_duplex_round_trips_submit_to_events() {
        let (server, client) = tokio::io::duplex(64 * 1024);
        let driver = driver(server);
        let (cr, mut cw) = tokio::io::split(client);

        let submit = Frame::new(1, SessionId::new("test"), Request::Submit { text: "hi".into() });
        write_frame(&mut cw, &submit).await.unwrap();

        // The NDJSON contract: every line is exactly one JSON object.
        let mut reader = BufReader::new(cr);
        let mut line = String::new();
        let mut saw_message_end = false;
        let mut saw_stopped = false;
        for _ in 0..64 {
            line.clear();
            let n = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
                .await
                .expect("a frame in time")
                .expect("read");
            assert!(n > 0, "EOF before the run finished");
            let value: serde_json::Value =
                serde_json::from_str(line.trim()).expect("each line is one JSON object");
            assert!(value.is_object(), "a frame is a flat JSON object");
            match value["type"].as_str() {
                Some("message_end") => saw_message_end = true,
                Some("stopped") => {
                    saw_stopped = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_message_end, "the Submit's streamed events arrived");
        assert!(saw_stopped, "the run's correlated reply arrived");

        // Dropping BOTH halves closes the duplex (tokio's `split` keeps the
        // stream alive until the last half drops), so the driver sees a clean
        // EOF and returns, no panic.
        drop(cw);
        drop(reader);
        let joined = tokio::time::timeout(Duration::from_secs(5), driver)
            .await
            .expect("driver ends on EOF");
        assert!(joined.is_ok());
    }

    #[tokio::test]
    async fn a_malformed_line_ends_the_connection() {
        let (server, client) = tokio::io::duplex(64 * 1024);
        let driver = driver(server);
        let (_cr, mut cw) = tokio::io::split(client);

        cw.write_all(b"not json\n").await.unwrap();
        cw.flush().await.unwrap();

        // `read_frame` yields `InvalidData`; the loop stops rather than hanging.
        let joined = tokio::time::timeout(Duration::from_secs(5), driver)
            .await
            .expect("driver stops, no hang");
        assert!(joined.is_ok());
    }

    #[tokio::test]
    async fn a_clean_eof_shuts_the_driver_down() {
        let (server, client) = tokio::io::duplex(64 * 1024);
        let driver = driver(server);
        let (cr, mut cw) = tokio::io::split(client);

        // A valid request gets a `Sessions` reply; then a clean EOF ends it.
        let frame = Frame::new(1, SessionId::new("test"), Request::ListSessions);
        write_frame(&mut cw, &frame).await.unwrap();

        let mut reader = BufReader::new(cr);
        let mut line = String::new();
        let n = tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
            .await
            .expect("a frame in time")
            .expect("read");
        assert!(n > 0);
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["type"], "sessions");

        // Both halves must drop to close the duplex and reach a clean EOF.
        drop(cw);
        drop(reader);
        let joined = tokio::time::timeout(Duration::from_secs(5), driver)
            .await
            .expect("driver ends on EOF");
        assert!(joined.is_ok());
    }
}
