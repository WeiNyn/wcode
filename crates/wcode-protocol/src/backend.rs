//! A session handle that is local *or* remote.
//!
//! [`Backend`] is the seam the client code targets: the REPL, the TUI, and (S4)
//! a peer session call `send`/`ask`/`subscribe` and never branch on whether the
//! session runs in this process ([`SessionHandle`]) or across a socket
//! ([`Client`]). Swapping the transport is swapping the variant.

use tokio::sync::broadcast;
use wcode_harness::actor::SessionHandle;
use wcode_harness::event::AgentEvent;
use wcode_harness::protocol::{Request, SessionId};

#[cfg(unix)]
use crate::client::Client;

/// The session is gone — locally (the actor shut down) or remotely (the
/// connection dropped).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed;

impl std::fmt::Display for Closed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("session is closed")
    }
}

impl std::error::Error for Closed {}

/// A handle to a session, local or remote.
#[derive(Clone)]
pub enum Backend {
    /// In-process: the kernel actor.
    Local(SessionHandle),
    /// Across a socket: a remote client.
    #[cfg(unix)]
    Remote(Client),
}

impl Backend {
    /// Fire-and-forget.
    pub fn send(&self, request: Request) -> Result<(), Closed> {
        match self {
            Backend::Local(handle) => handle.send(request).map_err(|_| Closed),
            #[cfg(unix)]
            Backend::Remote(client) => client.send(request),
        }
    }

    /// Fire-and-forget, attributed to `from` (A2A): the sender reaches the
    /// target's `before_inbound` and sender tag, local or over a socket.
    pub fn send_from(&self, from: SessionId, request: Request) -> Result<(), Closed> {
        match self {
            Backend::Local(handle) => handle.send_from(from, request).map_err(|_| Closed),
            #[cfg(unix)]
            Backend::Remote(client) => client.send_from(from, request),
        }
    }

    /// Send and await the correlated reply.
    pub async fn ask(&self, request: Request) -> Result<AgentEvent, Closed> {
        match self {
            Backend::Local(handle) => handle.ask(request).await.map_err(|_| Closed),
            #[cfg(unix)]
            Backend::Remote(client) => client.ask(request).await,
        }
    }

    /// Stream the session's events.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        match self {
            Backend::Local(handle) => handle.subscribe(),
            #[cfg(unix)]
            Backend::Remote(client) => client.subscribe(),
        }
    }
}

impl From<SessionHandle> for Backend {
    fn from(handle: SessionHandle) -> Self {
        Backend::Local(handle)
    }
}

#[cfg(unix)]
impl From<Client> for Backend {
    fn from(client: Client) -> Self {
        Backend::Remote(client)
    }
}
