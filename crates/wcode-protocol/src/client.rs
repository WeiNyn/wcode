//! A remote client: the [`SessionHandle`] API over a socket.
//!
//! The client demuxes the connection the same way the actor demuxes its
//! channels: a reader task splits inbound frames into *streaming* events
//! ([`Client::subscribe`]) and *replies* ([`Client::ask`], matched by the
//! envelope's `reply_to`), and a writer task serializes outbound requests. So a
//! caller cannot tell a [`Client`] from a [`SessionHandle`] — which is the
//! whole point of [`crate::Backend`].
//!
//! [`SessionHandle`]: wcode_harness::actor::SessionHandle

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufRead, AsyncWrite, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc, oneshot};
use wcode_harness::actor::EVENT_BUFFER;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Frame, PROTOCOL_VERSION, Request, SessionId};

use crate::Closed;
use crate::frame::{read_frame, write_frame};

/// A connection to a remote session.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

struct Inner {
    /// Outbound requests, drained by the writer task.
    out: mpsc::UnboundedSender<Frame<Request>>,
    /// Streaming events from the server (replies are routed to `pending`).
    events: broadcast::Sender<AgentEvent>,
    /// Reply waiters, keyed by request id.
    pending: Mutex<HashMap<u64, oneshot::Sender<AgentEvent>>>,
    next_id: AtomicU64,
    session: SessionId,
}

impl Client {
    /// Connect to a server listening at `path`.
    pub async fn connect(path: &Path) -> io::Result<Client> {
        let stream = crate::socket::connect(path).await?;
        Ok(Client::from_stream(stream))
    }

    fn from_stream(stream: UnixStream) -> Client {
        let (read, write) = stream.into_split();
        let (out, out_rx) = mpsc::unbounded_channel();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let inner = Arc::new(Inner {
            out,
            events,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            // Single-session server (S2): the routing field is inert here.
            session: SessionId::new("remote"),
        });
        tokio::spawn(write_loop(write, out_rx));
        tokio::spawn(read_loop(BufReader::new(read), inner.clone()));
        Client { inner }
    }

    /// Fire-and-forget: send a request and return. Streaming events (and the
    /// eventual reply) arrive on the subscription from [`Client::subscribe`].
    pub fn send(&self, request: Request) -> Result<(), Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner
            .out
            .send(self.frame(id, request))
            .map_err(|_| Closed)
    }

    /// Send a request and await its correlated reply.
    pub async fn ask(&self, request: Request) -> Result<AgentEvent, Closed> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id, tx);
        if self.inner.out.send(self.frame(id, request)).is_err() {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(Closed);
        }
        rx.await.map_err(|_| Closed)
    }

    /// Stream the server's events. Each subscriber gets its own receiver.
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

async fn write_loop<W>(mut write: W, mut rx: mpsc::UnboundedReceiver<Frame<Request>>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(frame) = rx.recv().await {
        if write_frame(&mut write, &frame).await.is_err() {
            break;
        }
    }
}

async fn read_loop<R>(mut read: R, inner: Arc<Inner>)
where
    R: AsyncBufRead + Unpin,
{
    loop {
        match read_frame::<_, AgentEvent>(&mut read).await {
            Ok(Some(frame)) => match frame.reply_to {
                Some(id) => {
                    if let Some(tx) = inner.pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(frame.body);
                    }
                }
                None => {
                    let _ = inner.events.send(frame.body);
                }
            },
            Ok(None) => break,
            Err(_) => break,
        }
    }
    // The connection is gone: drop every waiter so their `ask` fails rather
    // than hanging forever.
    inner.pending.lock().unwrap().clear();
}
