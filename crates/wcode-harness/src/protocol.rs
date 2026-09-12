//! The kernel's request/event interface — one protocol for every peer.
//!
//! The kernel already exposes one half of a protocol: [`AgentEvent`] is a
//! typed, serializable stream of everything a session does (text deltas, tool
//! lifecycle, turn boundaries). This module adds the inbound half, [`Request`],
//! and a versioned, addressed envelope, [`Frame`], so that a client and a peer
//! become the same kind of thing:
//!
//! - a **request** goes into a session's mailbox,
//! - an **event** comes out of its stream.
//!
//! A TUI client, a script, and another session differ only in which address
//! ([`Frame::session`]) they hold and how they render the stream — not in the
//! protocol they speak. That is what lets the client seam and the future
//! agent-to-agent seam share one implementation.
//!
//! Transport is deliberately absent: today the seam is an in-process
//! `tokio::mpsc`; framing and sockets live above the kernel (a future
//! `wcode-protocol` crate). This module is serde types only, so it adds no
//! behavior — it is the shape the actor (S1) and the socket (S2) will speak.
//!
//! See `docs/interface-protocol-brainstorm.md` for the design and the staged
//! path that lands these types.

use serde::{Deserialize, Serialize};

use crate::event::AgentEvent;

/// Wire-protocol major version. Bumped only by a breaking change; additive
/// changes leave it alone. [`Request`] already carries a [`Request::Unknown`]
/// fallback so an older peer survives a newer request; the matching catch-all
/// on the event side lands with the actor (S1), where an unknown event can be
/// handled rather than silently dropped.
pub const PROTOCOL_VERSION: u32 = 1;

/// A session's stable address.
///
/// Reuses the id already written in `SessionEntry::Header`, so a session is
/// reachable by the same name it persists under. Transitively serialized as a
/// bare string. Newtypes a plain `String` only to keep addressing legible at
/// call sites — the conventions (the human is `"user"`, a peer is
/// `"agent:<id>"`) are not encoded in the type yet.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for SessionId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Inbound intent: the requests a session's mailbox accepts.
///
/// Every variant maps one-to-one onto an existing [`crate::agent::Agent`]
/// method, so the current session behavior is expressible without the actor
/// inventing anything new:
///
/// | `Request` | today |
/// |-----------|-------|
/// | `Submit` | `Agent::run` |
/// | `Steer` | `Agent::steer` |
/// | `FollowUp` | `Agent::follow_up` |
/// | `Cancel` | `Agent::cancel` |
/// | `SetModel` | `Agent::set_model` |
/// | `SetEffort` | `Agent::set_effort` |
/// | `Compact` | `Agent::compact` |
///
/// Inbound payloads carry user **content**, not an `AgentMessage`: a remote peer
/// must not be able to inject an `Assistant`/`ToolResult` message and impersonate
/// the model. The session wraps content as a user message, exactly as
/// `Agent::run` does. Read-back requests (`GetHistory`) and the reply events
/// they need arrive with the actor (S1), when there is something to answer with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Start a turn: persist the text as a user message and run the loop until
    /// it stops. The run's progress and result stream back as [`AgentEvent`]s.
    Submit { text: String },

    /// Inject content at the next turn boundary of the in-flight run — a soft
    /// interrupt that does not cancel the run.
    Steer { content: String },

    /// Run this content after the loop would otherwise stop.
    FollowUp { content: String },

    /// Cancel the in-flight run (and any retry/backoff wait). Idempotent.
    Cancel,

    /// Swap the model for subsequent turns.
    SetModel { model: String },

    /// Set the reasoning effort, or clear it (`None` = send nothing).
    SetEffort { effort: Option<String> },

    /// Summarize the older prefix of the conversation now, optionally focused
    /// by `instructions`.
    Compact { instructions: Option<String> },

    /// Forward-compatibility catch-all: an older peer must skip a request it
    /// does not understand rather than fail the connection.
    #[serde(other)]
    Unknown,
}

/// Versioned, addressed envelope around a [`Request`] or an [`AgentEvent`].
///
/// Generic over the body so the same frame carries either direction: the
/// request-to-a-session frame and the event-from-a-session frame have identical
/// headers. The body is `#[serde(flatten)]`ed, so on the wire a frame is one
/// flat JSON object — `{"v":1,"id":7,"session":"s1","type":"cancel"}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame<P> {
    /// Protocol major version ([`PROTOCOL_VERSION`]).
    pub v: u32,
    /// Sender-assigned id, monotonic per connection; answers echo it in
    /// `reply_to`.
    pub id: u64,
    /// The request this frame answers, if any. Streaming events that are not
    /// direct replies leave it absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<u64>,
    /// The session this frame concerns — the target of a request, the origin of
    /// an event. The whole addressing scheme.
    pub session: SessionId,
    #[serde(flatten)]
    pub body: P,
}

impl<P> Frame<P> {
    /// A fresh top-level frame at the current version, not a reply.
    pub fn new(id: u64, session: SessionId, body: P) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id,
            reply_to: None,
            session,
            body,
        }
    }
}

/// A client-to-session frame.
pub type RequestFrame = Frame<Request>;
/// A session-to-client frame.
pub type EventFrame = Frame<AgentEvent>;

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    fn roundtrip<P>(p: P, expected_tag: &str)
    where
        P: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["type"], expected_tag);
        let back: P = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn request_roundtrip_all_variants() {
        roundtrip(Request::Submit { text: "hi".into() }, "submit");
        roundtrip(
            Request::Steer {
                content: "s".into(),
            },
            "steer",
        );
        roundtrip(
            Request::FollowUp {
                content: "f".into(),
            },
            "follow_up",
        );
        roundtrip(Request::Cancel, "cancel");
        roundtrip(Request::SetModel { model: "m1".into() }, "set_model");
        roundtrip(
            Request::SetEffort {
                effort: Some("high".into()),
            },
            "set_effort",
        );
        roundtrip(Request::SetEffort { effort: None }, "set_effort");
        roundtrip(
            Request::Compact {
                instructions: Some("focus on the parser".into()),
            },
            "compact",
        );
        roundtrip(Request::Compact { instructions: None }, "compact");
    }

    #[test]
    fn request_unknown_tag_is_caught_not_fatal() {
        let back: Request = serde_json::from_str(r#"{"type":"from_the_future"}"#).unwrap();
        assert_eq!(back, Request::Unknown);
    }

    #[test]
    fn frame_flattens_body_and_omits_absent_reply_to() {
        let f = Frame::new(7, SessionId::new("s1"), Request::Cancel);
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(
            v,
            json!({"v": PROTOCOL_VERSION, "id": 7, "session": "s1", "type": "cancel"})
        );
        let back: Frame<Request> = serde_json::from_value(v).unwrap();
        assert_eq!(back, f);
    }

    #[test]
    fn frame_carries_reply_to_and_an_event_body() {
        let f = EventFrame {
            v: PROTOCOL_VERSION,
            id: 8,
            reply_to: Some(7),
            session: SessionId::from("s1"),
            body: AgentEvent::AgentEnd,
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(
            v,
            json!({"v": PROTOCOL_VERSION, "id": 8, "reply_to": 7, "session": "s1", "type": "agent_end"})
        );
        // `AgentEvent` is not `PartialEq` (see event.rs); compare re-serialized
        // JSON, matching that module's round-trip convention.
        let back: EventFrame = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), v);
    }

    #[test]
    fn session_id_serializes_transparently() {
        assert_eq!(
            serde_json::to_value(SessionId::new("abc")).unwrap(),
            json!("abc")
        );
        assert_eq!(SessionId::new("abc").as_str(), "abc");
        assert_eq!(SessionId::new("abc").to_string(), "abc");
        assert_eq!(SessionId::from("x"), SessionId(String::from("x")));
    }
}
