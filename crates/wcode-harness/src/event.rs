use crate::message::{AgentMessage, StopReason, Usage};
use serde::{Deserialize, Serialize};

/// Internal seam between LLM adapter and agent loop. Not serialized.
#[derive(Clone, Debug, PartialEq)]
pub enum LlmStreamEvent {
    TextDelta(String),
    ThinkingDelta(String),
    /// A complete thinking block (provider restates the whole reasoning after
    /// streaming deltas). Replacement semantics: supersedes *all* thinking
    /// accumulated so far in this message — the kernel drops prior thinking
    /// blocks instead of appending, so the context never doubles up.
    ThinkingReplace(String),
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    Done {
        stop_reason: StopReason,
        usage: Option<Usage>,
    },
    /// The adapter is about to retry a failed connect after a transient error.
    /// `attempt` is 1-based; advisory — the loop only surfaces it.
    Retrying {
        attempt: u32,
        max: u32,
        reason: String,
    },
    Error {
        message: String,
        /// True when the failure is of a hard-fatal class (bad request, auth,
        /// schema, build/parse) that retrying or re-feeding cannot fix. The
        /// loop ends the run on a fatal error; a transient-class error that
        /// exhausted its retries (or landed mid-stream after content) is fed
        /// back to the model for a corrective turn instead.
        fatal: bool,
    },
}

/// The status of a [`TodoItem`]. `Cancelled` is deliberately absent in v1: the
/// model drops an item by rewriting the whole list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

/// One entry in the `todo` tool's session-local checklist. Lives in the harness
/// because it rides the serialized [`AgentEvent`] (the CLI tool reuses it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TodoItem {
    /// What the item is about (non-empty after trim; the tool validates).
    pub content: String,
    pub status: TodoStatus,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart,
    TurnStart,
    MessageStart {
        message: AgentMessage,
    },
    MessageUpdate {
        message: AgentMessage,
    },
    MessageEnd {
        message: AgentMessage,
    },
    ToolExecutionStart {
        call_id: String,
        name: String,
    },
    ToolExecutionUpdate {
        call_id: String,
        name: String,
        partial: String,
    },
    ToolExecutionEnd {
        call_id: String,
        name: String,
        output: String,
        is_error: bool,
        /// UI-only unified diff (see `ToolOutput::diff`); absent when there is
        /// nothing to show, or on an older peer that predates the field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        /// UI-only path of the changed file (see `ToolOutput::path`); absent
        /// when the tool touched no file, or on an older peer.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    TurnEnd {
        message: AgentMessage,
    },
    /// Auto-compaction summarized `summarized` older messages, keeping `kept`.
    Compaction {
        summarized: usize,
        kept: usize,
    },
    /// Best-effort auto-compaction was attempted (the context was near the
    /// ceiling) but did NOT complete — a summarizer/stream failure. The run
    /// continues. Deliberately NOT [`AgentEvent::Error`], whose contract is a
    /// *stream-level* failure: a TUI renders `Error` as a failed run, so routing
    /// a best-effort compaction miss through it would falsely mark the run
    /// failed. `reason` is the summarizer's error text, for display only — never
    /// fed back to the model and never fatal. Purely additive on the wire
    /// (`{"type":"compaction_skipped","reason":…}`), so
    /// [`crate::protocol::PROTOCOL_VERSION`] need not bump.
    CompactionSkipped {
        reason: String,
    },
    /// The adapter retried a failed connect after a transient error.
    Retrying {
        attempt: u32,
        max: u32,
        reason: String,
    },
    /// A peer delivered a message to this session (`Request::Notify`/
    /// `Interrupt`/`Wake`). `from` is the sender's address (§5.2 — the human is
    /// [`crate::protocol::USER`]); streamed, so a UI can surface an inbound
    /// message. Not a reply.
    MessageReceived {
        from: crate::protocol::SessionId,
        content: String,
    },
    /// Stream-level failure. A fatal-class failure (bad request, auth, schema)
    /// ends the run with `StopReason::Error`; a transient-class failure that
    /// exhausted its retries (or landed mid-stream) is fed back to the model
    /// and the loop continues, still bounded by a consecutive-error cap.
    Error {
        message: String,
    },
    /// Reply: a request with no richer answer succeeded (e.g. `SetModel`).
    /// Never *streamed* — it answers one request, correlated by the transport
    /// (an in-process reply channel today, the envelope's `reply_to` on the
    /// wire).
    Ack,
    /// Reply: the run finished, with why it stopped. Answers a `Submit`. Never
    /// streamed (see [`AgentEvent::Ack`]).
    Stopped {
        stop_reason: StopReason,
    },
    /// Reply: a tool-free side answer to a `Request::SideAsk` (`/btw`). Never
    /// streamed (see [`AgentEvent::Ack`]). Carries the answer text and the
    /// turn's usage (for a `/usage`-style readout); styling is the front-end's.
    SideAnswer {
        text: String,
        usage: Option<Usage>,
    },
    /// Reply: the conversation so far, answering a `Request::GetHistory`. Never
    /// streamed (see [`AgentEvent::Ack`]).
    History {
        messages: Vec<AgentMessage>,
    },
    /// Reply: the sessions a server serves, in serve order (root first), each
    /// with the model the server knows for it ([`crate::protocol::SessionInfo`]).
    /// Answers a [`crate::protocol::Request::ListSessions`]. Never streamed (see
    /// [`AgentEvent::Ack`]).
    Sessions {
        sessions: Vec<crate::protocol::SessionInfo>,
    },
    /// Reply: a peer defined a worker and returns its address, answering a
    /// [`crate::protocol::Request::Define`]. Never streamed (see
    /// [`AgentEvent::Ack`]).
    ///
    /// The field is `worker`, not `id`: the envelope already carries a top-level
    /// `id` (the request it replies to), and a flattened duplicate key would not
    /// deserialize.
    Spawned {
        worker: crate::protocol::SessionId,
    },
    /// The `todo` checklist, streamed on every write (mirror `Compaction` /
    /// `Retrying`: a plain state payload the UI reduces). The field is `todos`,
    /// not `id`: the envelope already carries a top-level `id` and a flattened
    /// duplicate key would not deserialize.
    Todo {
        todos: Vec<TodoItem>,
    },
    AgentEnd,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::ContentBlock;
    use serde_json::json;

    fn roundtrip(e: AgentEvent, expected_tag: &str) {
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["type"], expected_tag);
        let back: AgentEvent = serde_json::from_value(v.clone()).unwrap();
        let v2 = serde_json::to_value(&back).unwrap();
        assert_eq!(v, v2);
    }

    fn assistant() -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text { text: "a".into() }],
            stop_reason: StopReason::Stop,
            usage: Some(Usage::default()),
            model: None,
        }
    }

    #[test]
    fn agent_event_roundtrip_all_variants() {
        roundtrip(AgentEvent::AgentStart, "agent_start");
        roundtrip(AgentEvent::TurnStart, "turn_start");
        roundtrip(
            AgentEvent::MessageStart {
                message: assistant(),
            },
            "message_start",
        );
        roundtrip(
            AgentEvent::MessageUpdate {
                message: assistant(),
            },
            "message_update",
        );
        roundtrip(
            AgentEvent::MessageEnd {
                message: assistant(),
            },
            "message_end",
        );
        roundtrip(
            AgentEvent::ToolExecutionStart {
                call_id: "t1".into(),
                name: "run".into(),
            },
            "tool_execution_start",
        );
        roundtrip(
            AgentEvent::ToolExecutionUpdate {
                call_id: "t1".into(),
                name: "run".into(),
                partial: "out".into(),
            },
            "tool_execution_update",
        );
        roundtrip(
            AgentEvent::ToolExecutionEnd {
                call_id: "t1".into(),
                name: "run".into(),
                output: "out".into(),
                is_error: false,
                diff: Some("@@ -1 +1 @@".into()),
                path: Some("src/f.rs".into()),
            },
            "tool_execution_end",
        );
        roundtrip(
            AgentEvent::TurnEnd {
                message: assistant(),
            },
            "turn_end",
        );
        roundtrip(
            AgentEvent::Error {
                message: "boom".into(),
            },
            "error",
        );
        roundtrip(
            AgentEvent::Compaction {
                summarized: 3,
                kept: 2,
            },
            "compaction",
        );
        roundtrip(
            AgentEvent::CompactionSkipped {
                reason: "summarizer 500".into(),
            },
            "compaction_skipped",
        );
        roundtrip(
            AgentEvent::Retrying {
                attempt: 2,
                max: 3,
                reason: "503".into(),
            },
            "retrying",
        );
        roundtrip(AgentEvent::Ack, "ack");
        roundtrip(
            AgentEvent::Stopped {
                stop_reason: StopReason::Aborted,
            },
            "stopped",
        );
        roundtrip(
            AgentEvent::SideAnswer {
                text: "hi".into(),
                usage: None,
            },
            "side_answer",
        );
        roundtrip(
            AgentEvent::History {
                messages: vec![AgentMessage::user_text("hi")],
            },
            "history",
        );
        roundtrip(
            AgentEvent::MessageReceived {
                from: crate::protocol::SessionId::new("user"),
                content: "ping".into(),
            },
            "message_received",
        );
        roundtrip(AgentEvent::AgentEnd, "agent_end");
        roundtrip(
            AgentEvent::Sessions {
                sessions: vec![
                    crate::protocol::SessionInfo {
                        id: crate::protocol::SessionId::new("root"),
                        model: Some("m1".into()),
                    },
                    crate::protocol::SessionInfo {
                        id: crate::protocol::SessionId::agent("w1"),
                        model: None,
                    },
                ],
            },
            "sessions",
        );
        roundtrip(
            AgentEvent::Spawned {
                worker: crate::protocol::SessionId::agent("w1"),
            },
            "spawned",
        );
        roundtrip(
            AgentEvent::Todo {
                todos: vec![
                    TodoItem {
                        content: "write tests".into(),
                        status: TodoStatus::Pending,
                    },
                    TodoItem {
                        content: "ship it".into(),
                        status: TodoStatus::Completed,
                    },
                ],
            },
            "todo",
        );
    }

    #[test]
    fn agent_event_shape_matches_tagged_payload() {
        let v = serde_json::to_value(&AgentEvent::ToolExecutionStart {
            call_id: "t1".into(),
            name: "run".into(),
        })
        .unwrap();
        assert_eq!(
            v,
            json!({"type": "tool_execution_start", "call_id": "t1", "name": "run"})
        );

        let m = AgentMessage::user_text("hi");
        let v2 = serde_json::to_value(&AgentEvent::TurnEnd { message: m }).unwrap();
        assert_eq!(v2["type"], "turn_end");
        assert_eq!(v2["message"]["role"], "user");
    }
}
