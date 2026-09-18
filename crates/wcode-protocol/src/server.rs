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

use std::io;
use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncWrite, BufReader, BufWriter};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use wcode_harness::actor::SessionHandle;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

use crate::frame::{read_frame, write_frame};

/// Bind `path` and serve `sessions` until the listener errors.
///
/// Each entry names one served session — the routing key every outbound frame
/// carries. A client addresses a session by `frame.session`; a server serving
/// exactly one session also accepts any id, preserving the single-session flow
/// (the `--socket` client's placeholder `"remote"` still reaches it).
pub async fn serve(
    sessions: Vec<(SessionId, SessionHandle)>,
    listener: UnixListener,
) -> io::Result<()> {
    let sessions: Arc<[(SessionId, SessionHandle)]> = sessions.into();
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(connection(sessions.clone(), stream));
    }
}

/// Bind `path`, then [`serve`].
pub async fn serve_at(sessions: Vec<(SessionId, SessionHandle)>, path: &Path) -> io::Result<()> {
    let listener = crate::socket::bind(path).await?;
    serve(sessions, listener).await
}

async fn connection(sessions: Arc<[(SessionId, SessionHandle)]>, stream: UnixStream) {
    let (read, write) = stream.into_split();
    let (out, out_rx) = mpsc::unbounded_channel::<Frame<AgentEvent>>();
    tokio::spawn(write_loop(write, out_rx));

    // Fan every served session's events out to this connection, stamping each
    // frame with its origin session — the routing key the client demuxes on.
    for (session, handle) in sessions.iter() {
        let mut events = handle.subscribe();
        let event_out = out.clone();
        let event_session = session.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let frame = Frame {
                            v: PROTOCOL_VERSION,
                            id: 0,
                            reply_to: None,
                            session: event_session.clone(),
                            sender: None,
                            body: event,
                        };
                        if event_out.send(frame).is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
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

        // Demux on `frame.session`: the served session of that name; the sole
        // session of a single-session server (any id — the legacy default); or,
        // on a multi-session server, a correlated error echoing the request.
        let handle = match sessions.iter().find(|(sid, _)| sid == &session) {
            Some((_, handle)) => handle.clone(),
            None if sessions.len() == 1 => sessions[0].1.clone(),
            None => {
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
            }
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
