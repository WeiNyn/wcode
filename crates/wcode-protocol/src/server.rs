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

use tokio::io::{AsyncWrite, BufReader, BufWriter};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc, watch};
use wcode_harness::actor::SessionHandle;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

use crate::frame::{read_frame, write_frame};

/// The live roster of *extra* sessions served beside the root — a session is
/// `(SessionId, SessionHandle)`. Empty means "root only"; the value is refreshed
/// by a [`crate::Registry`] as sessions are registered.
pub type Roster = watch::Receiver<Vec<(SessionId, SessionHandle)>>;

/// A served session: the id every outbound frame carrying it is stamped with,
/// and the handle the server reads events from / writes requests to.
pub type Root = (SessionId, SessionHandle);

/// Bind `path` and serve the root session plus `roster` until the listener
/// errors.
///
/// `root` is served first and labelled by its own `SessionId` (the agent's
/// session id — *not* the registry's `agent:orchestrator` alias). Every session
/// named by `roster` is served too, now and as it grows. A client addresses a
/// session by `frame.session`; a server whose *current* served set is exactly
/// one session also accepts any id, preserving the single-session flow (the
/// `--socket` client's placeholder `"remote"` still reaches it).
pub async fn serve(roster: Roster, root: Root, listener: UnixListener) -> io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(connection(roster.clone(), root.clone(), stream));
    }
}

/// Bind `path`, then [`serve`].
pub async fn serve_at(roster: Roster, root: Root, path: &Path) -> io::Result<()> {
    let listener = crate::socket::bind(path).await?;
    serve(roster, root, listener).await
}

async fn connection(mut roster: Roster, root: Root, stream: UnixStream) {
    let (read, write) = stream.into_split();
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
            ids: roster_ids(&root_id, &initial),
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
        tokio::spawn(async move {
            loop {
                if roster.changed().await.is_err() {
                    break;
                }
                let current = roster.borrow_and_update().clone();
                let mut grew = false;
                for (id, handle) in &current {
                    if seen.insert(id.clone()) {
                        fan(&out, id, handle);
                        grew = true;
                    }
                }
                if grew {
                    let ids = roster_ids(&root_id, &current);
                    let _ = out.send(Frame {
                        v: PROTOCOL_VERSION,
                        id: 0,
                        reply_to: None,
                        session: root_id.clone(),
                        sender: None,
                        body: AgentEvent::Sessions { ids },
                    });
                }
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
            let ids = roster_ids(&root_id, &live);
            let _ = out.send(Frame {
                v: PROTOCOL_VERSION,
                id,
                reply_to: Some(id),
                session,
                sender: None,
                body: AgentEvent::Sessions { ids },
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

/// The served ids in order: the root first, then each roster session (minus the
/// root, if a roster ever names it).
fn roster_ids(root_id: &SessionId, roster: &[(SessionId, SessionHandle)]) -> Vec<SessionId> {
    let mut ids = vec![root_id.clone()];
    for (id, _) in roster {
        if id != root_id {
            ids.push(id.clone());
        }
    }
    ids
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
