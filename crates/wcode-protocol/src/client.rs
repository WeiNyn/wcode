//! A remote client: the [`SessionHandle`] API over a socket.
//!
//! The client demuxes the connection the same way the actor demuxes its
//! channels: a supervisor writes queued requests and reads inbound frames,
//! routing *streaming* events to [`Client::subscribe`] and *replies* to
//! [`Client::ask`] (matched by the envelope's `reply_to`). So a caller cannot
//! tell a [`Client`] from a [`SessionHandle`] — which is the whole point of
//! [`crate::Backend`].
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
use tokio::sync::{broadcast, mpsc, oneshot};
use wcode_harness::actor::EVENT_BUFFER;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

use crate::Closed;
use crate::frame::{read_frame, write_frame};

/// Reconnect backoff: first wait, doubling up to the cap.
const RECONNECT_BASE: Duration = Duration::from_millis(200);
const RECONNECT_CAP: Duration = Duration::from_secs(5);

/// A connection to a remote session, reconnecting automatically if it drops.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
    /// Requests, drained by the supervisor across reconnects.
    out: mpsc::UnboundedSender<Frame<Request>>,
}

struct Inner {
    path: PathBuf,
    events: broadcast::Sender<AgentEvent>,
    /// Reply waiters, keyed by request id.
    pending: Mutex<HashMap<u64, oneshot::Sender<AgentEvent>>>,
    next_id: AtomicU64,
    session: SessionId,
}

impl Client {
    /// Connect to a server listening at `path`.
    ///
    /// The first connect is eager — `Err` if nothing is listening — but after
    /// that the client reconnects on its own if the connection drops.
    pub async fn connect(path: &Path) -> io::Result<Client> {
        let stream = crate::socket::connect(path).await?;
        Ok(Client::adopt(path.to_path_buf(), stream))
    }

    fn adopt(path: PathBuf, stream: UnixStream) -> Client {
        let (out, out_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let inner = Arc::new(Inner {
            path,
            events,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            // Single-session server (S2): the routing field is inert here.
            session: SessionId::new("remote"),
        });
        tokio::spawn(supervise(inner.clone(), out_rx, Some(stream)));
        Client { inner, out }
    }

    /// Fire-and-forget: queue a request. Streaming events (and the eventual
    /// reply) arrive on the subscription from [`Client::subscribe`]. Fails only
    /// once every clone has dropped and the supervisor has ended.
    pub fn send(&self, request: Request) -> Result<(), Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.out.send(self.frame(id, request)).map_err(|_| Closed)
    }

    /// Send a request and await its correlated reply. Fails with [`Closed`] if
    /// the connection drops before the reply arrives.
    pub async fn ask(&self, request: Request) -> Result<AgentEvent, Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        if self.out.send(self.frame(id, request)).is_err() {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(Closed);
        }
        rx.await.map_err(|_| Closed)
    }

    /// Stream the server's events. Each subscriber gets its own receiver, which
    /// survives a reconnect (only events during the gap are lost).
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.inner.events.subscribe()
    }

    fn frame(&self, id: u64, body: Request) -> Frame<Request> {
        Frame {
            v: PROTOCOL_VERSION,
            id,
            reply_to: None,
            session: self.inner.session.clone(),
            body,
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
            let _ = inner.events.send(frame.body);
        }
    }
}
