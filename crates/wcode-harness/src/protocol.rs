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

/// The well-known address of the human peer (brainstorm §5.2): the sender an
/// in-process actor attributes an inbound message to until addressing lands
/// (S4-2).
pub const USER: &str = "user";

/// Whether `request` is an A2A inbound message — the delivery verbs a peer sends
/// (`Notify`/`Interrupt`/`Wake`), as opposed to a local command. The actor runs
/// its `before_inbound` policy on exactly these.
pub fn is_inbound(request: &Request) -> bool {
    matches!(
        request,
        Request::Notify { .. } | Request::Interrupt { .. } | Request::Wake { .. }
    )
}

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

    /// The well-known address of the human peer (see [`USER`]).
    pub fn user() -> Self {
        Self(USER.to_string())
    }

    /// A peer session's address: `"agent:<id>"` (§5.2).
    pub fn agent(id: impl AsRef<str>) -> Self {
        Self(format!("agent:{}", id.as_ref()))
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

/// A served session's address plus the little metadata a roster shows: today
/// only its **effective model**. Carried by
/// [`crate::event::AgentEvent::Sessions`], so a client can label each member by
/// its own model instead of substituting the root's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: SessionId,
    /// The session's effective model id, when the server knows one (a worker
    /// registered with its model); `None` for a peer whose model is unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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
/// | `Notify` | `Agent::notify` (append, no turn) |
/// | `Interrupt` | `Agent::steer` |
/// | `Wake` | run — a turn even when idle |
/// | `Cancel` | `Agent::cancel` |
/// | `SetModel` | `Agent::set_model` |
/// | `SetEffort` | `Agent::set_effort` |
/// | `Compact` | `Agent::compact` |
///
/// The one exception is [`Request::ListSessions`]: a transport-level probe the
/// server answers, not an `Agent` method.
///
/// Inbound payloads carry user **content**, not an `AgentMessage`: a remote peer
/// must not be able to inject an `Assistant`/`ToolResult` message and impersonate
/// the model. The session wraps content as a user message, exactly as
/// `Agent::run` does. `GetHistory` is a read: it carries no payload and its
/// answer comes back as [`AgentEvent::History`]
/// (with the other replies — [`AgentEvent::Ack`],
/// and [`AgentEvent::Error`] — alongside it).
/// These reply variants are never *streamed*: they answer a specific request,
/// which the transport correlates (an in-process oneshot today, the envelope's
/// `reply_to` on the wire).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    /// Start a turn: persist the text as a user message and run the loop until
    /// it stops. The run's progress and result stream back as [`AgentEvent`]s.
    Submit { text: String },

    /// Append `content` to the conversation *without* starting a turn (recorded
    /// now if idle, or at the next turn boundary if a run is in flight). The
    /// "tell, don't ask" delivery mode — `Agent::notify`.
    Notify { content: String },

    /// Inject `content` at the next turn boundary of the in-flight run — a soft
    /// interrupt that does not cancel the run. Formerly `Steer`; `"steer"`
    /// still deserializes here.
    #[serde(alias = "steer")]
    Interrupt { content: String },

    /// Wake the session: run `content` as a turn **even if it is idle** (mid-run
    /// it rides the follow-up channel instead). Formerly `FollowUp`;
    /// `"follow_up"` still deserializes here.
    /// `"follow_up"` still deserializes here.
    #[serde(alias = "follow_up")]
    Wake { content: String },

    /// Cancel the in-flight run (and any retry/backoff wait). Idempotent.
    Cancel,

    /// Swap the model for subsequent turns.
    SetModel { model: String },

    /// Set the reasoning effort, or clear it (`None` = send nothing).
    SetEffort { effort: Option<String> },

    /// Summarize the older prefix of the conversation now, optionally focused
    /// by `instructions`.
    Compact { instructions: Option<String> },

    /// Toggle plan mode. The actor flips the agent's shared `PlanModeHandle`
    /// (removing mutating tools + mutating `bash`) and recomposes the system
    /// prompt. Infallible — replies [`AgentEvent::Ack`].
    SetPlanMode { on: bool },

    /// Ask a side question (`/btw`): answer `text` tool-free, from the current
    /// context, WITHOUT recording it. A read — carries user content, names no
    /// `AgentMessage` (see the enum doc above). Reply:
    /// [`AgentEvent::SideAnswer`]. A `SideAsk` arriving mid-run is deferred to
    /// the next turn boundary (like any other inbox request), not refused.
    SideAsk { text: String },
    /// Read back the conversation so far. Reply:
    /// [`AgentEvent::History`].
    GetHistory,

    /// Ask the **transport** for the sessions a server serves — serve order,
    /// root first. The one request that names no `Agent` method: an in-process
    /// session has no roster, so the actor answers it with [`AgentEvent::Ack`]
    /// (like [`Request::Unknown`]); a socket server intercepts it and replies
    /// [`AgentEvent::Sessions`].
    ListSessions,

    /// Define a worker **on a served peer** (§8, S4). Like [`Request::ListSessions`]
    /// it names no `Agent` method: a socket server intercepts it and asks an
    /// injected handler to build a session, replying [`AgentEvent::Spawned`] with
    /// its address (or a correlated [`AgentEvent::Error`]). Mirrors the CLI's
    /// worker spec, so a peer can customize the worker's model, role, tool set,
    /// and provider. An in-process session has no factory, so the actor answers
    /// it with [`AgentEvent::Error`].
    Define {
        /// The worker's address; auto-assigned (`w1`, `w2`, …) when absent.
        name: Option<String>,
        /// Model id override; `None` inherits the server's model.
        model: Option<String>,
        /// Role text appended to the worker's system prompt.
        role: Option<String>,
        /// Tool allow-list; `None` = the full default set.
        tools: Option<Vec<String>>,
        /// Provider base URL override; `None` inherits the server's.
        base_url: Option<String>,
        /// Provider API key override; `None` inherits the server's.
        api_key: Option<String>,
        /// Enforce read-only for the defined worker (D1/D2); default false.
        #[serde(default)]
        read_only: bool,
        /// Reasoning-effort override (D3); `None` inherits, synonyms clear.
        #[serde(default)]
        effort: Option<String>,
    },

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
    /// The sender's address, when it is not the transport's own client (an
    /// in-process handle leaves it unset; an A2A peer sets its `"agent:<id>"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender: Option<SessionId>,
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
            sender: None,
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
    use crate::event::{TodoItem, TodoStatus};
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
        roundtrip(Request::SetPlanMode { on: true }, "set_plan_mode");
        roundtrip(Request::Notify { content: "n".into() }, "notify");
        roundtrip(Request::Interrupt { content: "i".into() }, "interrupt");
        roundtrip(Request::Wake { content: "w".into() }, "wake");
        roundtrip(Request::SideAsk { text: "why?".into() }, "side_ask");
        roundtrip(Request::ListSessions, "list_sessions");
        roundtrip(
            Request::Define {
                name: Some("w1".into()),
                model: Some("m".into()),
                role: Some("reviewer".into()),
                tools: Some(vec!["read".into()]),
                base_url: Some("http://w/v1".into()),
                api_key: Some("k".into()),
                read_only: true,
                effort: Some("high".into()),
            },
            "define",
        );
        roundtrip(
            Request::Define {
                name: None,
                model: None,
                role: None,
                tools: None,
                base_url: None,
                api_key: None,
                read_only: false,
                effort: None,
            },
            "define",
        );
    }

    #[test]
    fn legacy_verb_tags_deserialize_to_canonical_variants() {
        let steer: Request = serde_json::from_str(r#"{"type":"steer","content":"s"}"#).unwrap();
        assert_eq!(steer, Request::Interrupt { content: "s".into() });
        let follow: Request = serde_json::from_str(r#"{"type":"follow_up","content":"f"}"#).unwrap();
        assert_eq!(follow, Request::Wake { content: "f".into() });
    }

    #[test]
    fn request_unknown_tag_is_caught_not_fatal() {
        let back: Request = serde_json::from_str(r#"{"type":"from_the_future"}"#).unwrap();
        assert_eq!(back, Request::Unknown);
    }

    #[test]
    fn is_inbound_marks_only_the_delivery_verbs() {
        assert!(is_inbound(&Request::Notify { content: "x".into() }));
        assert!(is_inbound(&Request::Interrupt { content: "x".into() }));
        assert!(is_inbound(&Request::Wake { content: "x".into() }));
        assert!(!is_inbound(&Request::Submit { text: "x".into() }));
        assert!(!is_inbound(&Request::GetHistory));
        assert!(!is_inbound(&Request::SideAsk { text: "x".into() }));
        assert!(!is_inbound(&Request::SetPlanMode { on: true }));
        assert!(!is_inbound(&Request::Cancel));
        assert!(!is_inbound(&Request::ListSessions));
        assert!(!is_inbound(&Request::Define {
            name: None,
            model: None,
            role: None,
            tools: None,
            base_url: None,
            api_key: None,
            read_only: false,
            effort: None,
        }));
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
            sender: None,
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
    fn frame_carries_a_todo_event_body() {
        let f = EventFrame {
            v: PROTOCOL_VERSION,
            id: 9,
            reply_to: None,
            session: SessionId::from("s1"),
            sender: None,
            body: AgentEvent::Todo {
                todos: vec![TodoItem {
                    content: "write tests".into(),
                    status: TodoStatus::Pending,
                }],
            },
        };
        let v = serde_json::to_value(&f).unwrap();
        assert_eq!(
            v,
            json!({
                "v": PROTOCOL_VERSION, "id": 9, "session": "s1", "type": "todo",
                "todos": [{"content": "write tests", "status": "pending"}],
            })
        );
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
        assert_eq!(SessionId::user().as_str(), USER);
        assert_eq!(SessionId::agent("abc").as_str(), "agent:abc");
    }
}
