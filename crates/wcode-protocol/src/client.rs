//! A remote client: the [`SessionHandle`] API over a socket.
//!
//! The client demuxes the connection the same way the actor demuxes its
//! channels: a supervisor writes queued requests and reads inbound frames,
//! routing *streaming* events by `Frame::session` to [`Client::subscribe`] and
//! *replies* to [`Client::ask`] (matched by the envelope's `reply_to`). A
//! [`Client`] is a per-session **view** over one connection: [`Client::with_session`]
//! narrows it to one session, while [`Client::connect`]/[`Client::lazy`] is the
//! legacy connection-wide view. So a caller cannot tell a [`Client`] from a
//! [`SessionHandle`] — which is the whole point of [`crate::Backend`].
//!
//! It also **reconnects**: if the connection drops (a server restart, a blip),
//! the supervisor re-establishes it with backoff and keeps going. Requests
//! queued during the gap are delivered once it is back. An in-flight `ask`
//! whose reply was lost to the drop fails with [`Closed`] — the reply is simply
//! not coming — and the caller (or the user) retries.
//!
//! [`SessionHandle`]: wcode_harness::actor::SessionHandle

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use wcode_harness::actor::EVENT_BUFFER;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

use crate::Closed;
use crate::frame::{read_frame, write_frame};

/// Reconnect backoff: first wait, doubling up to the cap.
const RECONNECT_BASE: Duration = Duration::from_millis(200);
const RECONNECT_CAP: Duration = Duration::from_secs(5);

/// The wire placeholder an unnarrowed view addresses: a single-session server
/// routes any id to its sole session, so `connect`/`lazy` reach it without
/// knowing the served id.
const DEFAULT_SESSION: &str = "remote";

/// A connection to a remote session, reconnecting automatically if it drops.
///
/// A clone is a **view**: it shares the connection and request queue but may
/// address a different session ([`Client::with_session`]).
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
    /// Requests, drained by the supervisor across reconnects.
    out: mpsc::UnboundedSender<Frame<Request>>,
    /// The session this view addresses; `None` is the connection-wide view.
    session: Option<SessionId>,
}

struct Inner {
    path: PathBuf,
    /// Per-session event fans, created on first use — a [`Client::with_session`]
    /// view hears only its own session's.
    events: Mutex<HashMap<SessionId, broadcast::Sender<AgentEvent>>>,
    /// The connection-wide fan: the legacy view (`connect`/`lazy`) hears every
    /// session's events, exactly as the single-session client did before
    /// multiplexing.
    all_events: broadcast::Sender<AgentEvent>,
    /// The connection-wide roster of served sessions, updated when the server
    /// pushes an unsolicited `AgentEvent::Sessions` frame (a session registered
    /// at runtime). Shared across every view — see [`Client::subscribe_roster`].
    roster: watch::Sender<Vec<SessionId>>,
    /// Reply waiters, keyed by request id.
    pending: Mutex<HashMap<u64, oneshot::Sender<AgentEvent>>>,
    next_id: AtomicU64,
}

impl Client {
    /// Connect to a server listening at `path`.
    ///
    /// The first connect is eager — `Err` if nothing is listening — but after
    /// that the client reconnects on its own if the connection drops.
    pub async fn connect(path: &Path) -> io::Result<Client> {
        let stream = crate::socket::connect(path).await?;
        Ok(Client::adopt(path.to_path_buf(), Some(stream)))
    }

    /// Connect **lazily**: never fails. The supervisor keeps trying `path` in the
    /// background, so a peer that is not up yet (or restarts) is reached once it
    /// appears — the right mode for an A2A peer (`--peer`/`[peers]`), where two
    /// sessions may each be waiting for the other.
    pub fn lazy(path: &Path) -> Client {
        Client::adopt(path.to_path_buf(), None)
    }

    fn adopt(path: PathBuf, stream: Option<UnixStream>) -> Client {
        let (out, out_rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            path,
            events: Mutex::new(HashMap::new()),
            all_events: broadcast::channel(EVENT_BUFFER).0,
            roster: watch::channel(Vec::new()).0,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        });
        tokio::spawn(supervise(inner.clone(), out_rx, stream));
        Client {
            inner,
            out,
            session: None,
        }
    }

    /// A per-session view over the same connection: a cheap clone sharing the
    /// `Inner` and request queue but addressed to `session`. Its
    /// [`Client::subscribe`] hears only `session`'s events; replies are matched
    /// by `reply_to` and are independent of the view.
    ///
    /// To hear a session's stream, `session` must be the id the server actually
    /// **serves** (the id it stamps outbound frames with — learn it from
    /// [`Request::ListSessions`]). A view narrowed to any other id hears
    /// nothing; over a single-session server, where the served id is not
    /// otherwise known, only the connection-wide view (`connect`/`lazy`) is safe.
    pub fn with_session(&self, session: SessionId) -> Client {
        Client {
            inner: self.inner.clone(),
            out: self.out.clone(),
            session: Some(session),
        }
    }

    /// Fire-and-forget: queue a request. Streaming events (and the eventual
    /// reply) arrive on the subscription from [`Client::subscribe`]. Fails only
    /// once every clone has dropped and the supervisor has ended.
    pub fn send(&self, request: Request) -> Result<(), Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.out.send(self.frame(id, None, request)).map_err(|_| Closed)
    }

    /// Fire-and-forget, attributing the request to a remote `from` address
    /// (A2A). The server feeds `from` into the target's `before_inbound` and its
    /// sender tag, exactly as an in-process `send_from` does.
    pub fn send_from(&self, from: SessionId, request: Request) -> Result<(), Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.out
            .send(self.frame(id, Some(from), request))
            .map_err(|_| Closed)
    }

    /// Send a request and await its correlated reply. Fails with [`Closed`] if
    /// the connection drops before the reply arrives.
    pub async fn ask(&self, request: Request) -> Result<AgentEvent, Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        if self.out.send(self.frame(id, None, request)).is_err() {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(Closed);
        }
        rx.await.map_err(|_| Closed)
    }

    /// Stream the events this view hears: its own session's, or — for the
    /// connection-wide view (`connect`/`lazy`) — every session's. Each subscriber
    /// gets its own receiver, which survives a reconnect (only events during the
    /// gap are lost).
    ///
    /// The view must address the **served** id to hear its stream (see
    /// [`Client::with_session`]).
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.inner.sender(self.session.as_ref()).subscribe()
    }

    /// A live view of the **connection-wide** roster of served sessions,
    /// **seeded** with the current value. The server pushes an unsolicited
    /// [`AgentEvent::Sessions`] frame when the roster grows (a session
    /// registered at runtime); this receiver yields the new list. It is
    /// connection-wide, *not* per-session: every view over this connection
    /// shares it, so an unnarrowed and a `with_session` view see the same ids.
    pub fn subscribe_roster(&self) -> watch::Receiver<Vec<SessionId>> {
        self.inner.roster.subscribe()
    }

    fn frame(&self, id: u64, sender: Option<SessionId>, body: Request) -> Frame<Request> {
        Frame {
            v: PROTOCOL_VERSION,
            id,
            reply_to: None,
            session: self
                .session
                .clone()
                .unwrap_or_else(|| SessionId::new(DEFAULT_SESSION)),
            sender,
            body,
        }
    }
}

impl Inner {
    /// The event fan for a view: the connection-wide one for an unnarrowed view,
    /// else the (lazily created) fan for that session. Kept in the shared `Inner`,
    /// not on a view, so subscribers survive a reconnect — only events during the
    /// gap are lost.
    fn sender(&self, session: Option<&SessionId>) -> broadcast::Sender<AgentEvent> {
        match session {
            None => self.all_events.clone(),
            Some(session) => self
                .events
                .lock()
                .unwrap()
                .entry(session.clone())
                .or_insert_with(|| broadcast::channel(EVENT_BUFFER).0)
                .clone(),
        }
    }
}

/// Own the connection: write queued requests and read events, reconnecting on
/// drop until every [`Client`] clone is gone.
async fn supervise(
    inner: Arc<Inner>,
    mut out: mpsc::UnboundedReceiver<Frame<Request>>,
    mut next: Option<UnixStream>,
) {
    let mut backoff = RECONNECT_BASE;
    loop {
        let stream = match next.take() {
            Some(stream) => stream,
            None => match crate::socket::connect(&inner.path).await {
                Ok(stream) => stream,
                Err(_) => {
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(RECONNECT_CAP);
                    continue;
                }
            },
        };
        backoff = RECONNECT_BASE;

        let (read, mut write) = stream.into_split();
        let mut read = BufReader::new(read);
        loop {
            tokio::select! {
                request = out.recv() => match request {
                    Some(frame) => {
                        if write_frame(&mut write, &frame).await.is_err() {
                            break;
                        }
                    }
                    // Every clone dropped: nothing left to serve.
                    None => return,
                },
                event = read_frame::<_, AgentEvent>(&mut read) => match event {
                    Ok(Some(frame)) => deliver(&inner, frame),
                    // EOF or a broken read: the connection is gone.
                    _ => break,
                },
            }
        }
        // The connection dropped: fail every in-flight `ask` (its reply is not
        // coming), then loop to reconnect.
        inner.pending.lock().unwrap().clear();
    }
}

fn deliver(inner: &Inner, frame: Frame<AgentEvent>) {
    match frame.reply_to {
        Some(id) => {
            if let Some(tx) = inner.pending.lock().unwrap().remove(&id) {
                let _ = tx.send(frame.body);
            }
        }
        None => {
            // An unsolicited `Sessions` frame is the server's roster push (a
            // session registered at runtime): record it connection-wide, so a
            // `subscribe_roster` view learns the new id without polling.
            if let AgentEvent::Sessions { ids } = &frame.body {
                let _ = inner.roster.send(ids.clone());
            }
            // Ids are unique per connection (a single `next_id` atomic), so a
            // reply and a streamed event never collide — no re-namespacing is
            // needed. Route the event to its own session's fan *and* the
            // connection-wide fan, so the legacy view still hears it all.
            let _ = inner.all_events.send(frame.body.clone());
            let _ = inner.sender(Some(&frame.session)).send(frame.body);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inner() -> Inner {
        Inner {
            path: PathBuf::from("/nonexistent"),
            events: Mutex::new(HashMap::new()),
            all_events: broadcast::channel(EVENT_BUFFER).0,
            roster: watch::channel(Vec::new()).0,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn stream_frame(session: SessionId, body: AgentEvent) -> Frame<AgentEvent> {
        Frame {
            v: PROTOCOL_VERSION,
            id: 0,
            reply_to: None,
            session,
            sender: None,
            body,
        }
    }

    #[tokio::test]
    async fn deliver_splits_replies_from_streamed_events() {
        let inner = inner();
        // A reply resolves the matching waiter and is never streamed.
        let (tx, rx) = oneshot::channel();
        inner.pending.lock().unwrap().insert(7, tx);
        let mut stream = inner.sender(None).subscribe();
        deliver(
            &inner,
            Frame {
                v: PROTOCOL_VERSION,
                id: 7,
                reply_to: Some(7),
                session: SessionId::new("s"),
                sender: None,
                body: AgentEvent::Ack,
            },
        );
        assert!(matches!(rx.await.unwrap(), AgentEvent::Ack));
        assert!(stream.try_recv().is_err(), "a reply is not streamed");
    }

    #[tokio::test]
    async fn deliver_routes_a_streamed_event_by_session() {
        let inner = inner();
        let a = SessionId::agent("a");
        let b = SessionId::agent("b");
        let mut ra = inner.sender(Some(&a)).subscribe();
        let mut rb = inner.sender(Some(&b)).subscribe();
        let mut all = inner.sender(None).subscribe();
        deliver(&inner, stream_frame(a.clone(), AgentEvent::AgentEnd));
        assert!(matches!(ra.try_recv().unwrap(), AgentEvent::AgentEnd));
        assert!(matches!(all.try_recv().unwrap(), AgentEvent::AgentEnd));
        assert!(rb.try_recv().is_err(), "b did not hear a's event");
    }
}
