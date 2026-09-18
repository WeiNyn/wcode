//! Wire transport for wcode sessions — the socket behind the protocol.
//!
//! The kernel defines the *interface* ([`wcode_harness::protocol::Frame`] of
//! [`Request`]/[`AgentEvent`]) and the in-process actor that speaks it. This
//! crate adds the *transport*:
//!
//! - NDJSON framing ([`read_frame`]/[`write_frame`]),
//! - a [`serve`] function that re-exposes a session [`SessionHandle`] to remote
//!   clients over a Unix socket,
//! - a [`Client`] that mirrors the handle's `send`/`ask`/`subscribe` API,
//! - a [`Backend`] that makes the local and remote paths interchangeable,
//! - a [`Registry`] — the in-process A2A bus (§8 #3, §10.1): an address book
//!   plus the permitted-set routing that scopes who may message whom.
//!
//! The point (design: `docs/interface-protocol-brainstorm.md`, §8): a caller —
//! the REPL today, the TUI next, another session later — talks to a [`Backend`]
//! and never knows whether the session is in this process or across a socket.
//!
//! [`Request`]: wcode_harness::protocol::Request
//! [`AgentEvent`]: wcode_harness::event::AgentEvent
//! [`SessionHandle`]: wcode_harness::actor::SessionHandle

mod backend;
mod frame;
mod registry;

pub use backend::{Backend, Closed};
pub use frame::{read_frame, write_frame};
pub use registry::{Registry, RouteError};

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod server;
#[cfg(unix)]
mod socket;

#[cfg(unix)]
pub use client::Client;
#[cfg(unix)]
pub use server::{Root, Roster, serve, serve_at};
#[cfg(unix)]
pub use socket::{bind, connect};
