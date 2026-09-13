//! Serve a session over a socket: accept connections, bridge frames ↔ handle.
//!
//! Each connection gets three tasks: a writer draining an outbound queue, a fan
//! task forwarding the session's events, and the accept loop answering inbound
//! requests. Requests are answered with the actor's reply (correlated by the
//! envelope's `reply_to`), so a client's `ask` resolves even while streamed
//! events flow on the same connection.

use std::io;
use std::path::Path;

use tokio::io::{AsyncWrite, BufReader, BufWriter};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, mpsc};
use wcode_harness::actor::SessionHandle;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

use crate::frame::{read_frame, write_frame};

/// Bind `path` and serve `handle` until the listener errors.
///
/// `session` names the served session in every outbound frame — the routing
/// field a multi-session client would use (S4); a single-session server just
/// stamps it.
pub async fn serve(
    handle: SessionHandle,
    session: SessionId,
    listener: UnixListener,
) -> io::Result<()> {
    loop {
        let (stream, _addr) = listener.accept().await?;
        tokio::spawn(connection(handle.clone(), session.clone(), stream));
    }
}

/// Bind `path`, then [`serve`].
pub async fn serve_at(handle: SessionHandle, session: SessionId, path: &Path) -> io::Result<()> {
    let listener = crate::socket::bind(path).await?;
    serve(handle, session, listener).await
}

async fn connection(handle: SessionHandle, session: SessionId, stream: UnixStream) {
    let (read, write) = stream.into_split();
    let (out, out_rx) = mpsc::unbounded_channel::<Frame<AgentEvent>>();
    tokio::spawn(write_loop(write, out_rx));

    // Fan the session's events out to this connection.
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

    // Answer each request (in its own task, so a long run never blocks reads).
    let mut read = BufReader::new(read);
    while let Ok(Some(frame)) = read_frame::<_, Request>(&mut read).await {
        let Frame {
            id, session, body, ..
        } = frame;
        let handle = handle.clone();
        let out = out.clone();
        tokio::spawn(async move {
            if let Ok(reply) = handle.ask(body).await {
                let frame = Frame {
                    v: PROTOCOL_VERSION,
                    id,
                    reply_to: Some(id),
                    session,
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
