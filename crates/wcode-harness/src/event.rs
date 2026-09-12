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
    },
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
    },
    TurnEnd {
        message: AgentMessage,
    },
    /// Auto-compaction summarized `summarized` older messages, keeping `kept`.
    Compaction {
        summarized: usize,
        kept: usize,
    },
    /// The adapter retried a failed connect after a transient error.
    Retrying {
        attempt: u32,
        max: u32,
        reason: String,
    },
    /// Stream-level failure; the run ends with `StopReason::Error`.
    Error {
        message: String,
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
            AgentEvent::Retrying {
                attempt: 2,
                max: 3,
                reason: "503".into(),
            },
            "retrying",
        );
        roundtrip(AgentEvent::AgentEnd, "agent_end");
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
