use serde::{Deserialize, Serialize};

pub enum Role {
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Deferred,
    Aborted,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum AgentMessage {
    User {
        content: Vec<ContentBlock>,
    },
    Assistant {
        content: Vec<ContentBlock>,
        stop_reason: StopReason,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        name: String,
        output: String,
        is_error: bool,
    },
}

impl AgentMessage {
    pub fn user_text(s: impl Into<String>) -> AgentMessage {
        AgentMessage::User {
            content: vec![ContentBlock::Text { text: s.into() }],
        }
    }

    pub fn as_text(&self) -> String {
        let content = match self {
            AgentMessage::User { content } | AgentMessage::Assistant { content, .. } => content,
            AgentMessage::ToolResult { .. } => return String::new(),
        };
        content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    pub fn tool_calls(&self) -> Vec<&ContentBlock> {
        match self {
            AgentMessage::Assistant { content, .. } => content
                .iter()
                .filter(|b| matches!(b, ContentBlock::ToolCall { .. }))
                .collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn content_block_roundtrip_and_tags() {
        let blocks = vec![
            ContentBlock::Text { text: "hi".into() },
            ContentBlock::Thinking { text: "hmm".into() },
            ContentBlock::ToolCall {
                id: "t1".into(),
                name: "run".into(),
                arguments: json!({"x": 1}),
            },
        ];
        for b in blocks {
            let v = serde_json::to_value(&b).unwrap();
            let back: ContentBlock = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(back, b);
            let expected = match &b {
                ContentBlock::Text { .. } => "text",
                ContentBlock::Thinking { .. } => "thinking",
                ContentBlock::ToolCall { .. } => "tool_call",
            };
            assert_eq!(v["type"], expected);
        }
    }

    #[test]
    fn usage_roundtrip_and_omits_none() {
        let u = Usage::default();
        let v = serde_json::to_value(u).unwrap();
        assert_eq!(v, json!({"input_tokens": 0, "output_tokens": 0}));
        let back: Usage = serde_json::from_value(v).unwrap();
        assert_eq!(back, u);

        let u2 = Usage {
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: Some(5),
            cache_write_tokens: Some(7),
        };
        let v2 = serde_json::to_value(u2).unwrap();
        assert_eq!(
            v2,
            json!({
                "input_tokens": 10,
                "output_tokens": 20,
                "cache_read_tokens": 5,
                "cache_write_tokens": 7
            })
        );
        assert_eq!(serde_json::from_value::<Usage>(v2).unwrap(), u2);
    }

    #[test]
    fn stop_reason_roundtrip_snake_case() {
        for (r, s) in [
            (StopReason::Stop, "stop"),
            (StopReason::Length, "length"),
            (StopReason::ToolUse, "tool_use"),
            (StopReason::Deferred, "deferred"),
            (StopReason::Aborted, "aborted"),
            (StopReason::Error, "error"),
        ] {
            let v = serde_json::to_value(r).unwrap();
            assert_eq!(v, json!(s));
            assert_eq!(serde_json::from_value::<StopReason>(v).unwrap(), r);
        }
    }

    #[test]
    fn agent_message_roundtrip_and_role_tags() {
        let msgs = vec![
            AgentMessage::User {
                content: vec![ContentBlock::Text { text: "q".into() }],
            },
            AgentMessage::Assistant {
                content: vec![ContentBlock::Text { text: "a".into() }],
                stop_reason: StopReason::Stop,
                usage: Some(Usage::default()),
                model: Some("m".into()),
            },
            AgentMessage::ToolResult {
                tool_call_id: "t1".into(),
                name: "run".into(),
                output: "out".into(),
                is_error: false,
            },
        ];
        for m in msgs {
            let v = serde_json::to_value(&m).unwrap();
            let back: AgentMessage = serde_json::from_value(v.clone()).unwrap();
            assert_eq!(back, m);
            let expected = match &m {
                AgentMessage::User { .. } => "user",
                AgentMessage::Assistant { .. } => "assistant",
                AgentMessage::ToolResult { .. } => "tool_result",
            };
            assert_eq!(v["role"], expected);
        }
    }

    #[test]
    fn assistant_optional_fields_default_and_skip() {
        let v = json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "a"}],
            "stop_reason": "stop"
        });
        let m: AgentMessage = serde_json::from_value(v).unwrap();
        match &m {
            AgentMessage::Assistant {
                content,
                usage,
                model,
                ..
            } => {
                assert_eq!(content.len(), 1);
                assert_eq!(*usage, None);
                assert_eq!(*model, None);
            }
            _ => panic!("wrong variant"),
        }
        let out = serde_json::to_value(&m).unwrap();
        assert!(out.get("usage").is_none());
        assert!(out.get("model").is_none());
    }

    #[test]
    fn user_text_and_as_text() {
        let m = AgentMessage::user_text("hello");
        assert_eq!(
            m,
            AgentMessage::User {
                content: vec![ContentBlock::Text {
                    text: "hello".into()
                }]
            }
        );
        assert_eq!(m.as_text(), "hello");

        let m2 = AgentMessage::Assistant {
            content: vec![
                ContentBlock::Text { text: "a".into() },
                ContentBlock::Thinking { text: "t".into() },
                ContentBlock::Text { text: "b".into() },
            ],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        };
        assert_eq!(m2.as_text(), "ab");
    }

    #[test]
    fn tool_calls_returns_assistant_tool_call_blocks() {
        let m = AgentMessage::Assistant {
            content: vec![
                ContentBlock::Text { text: "a".into() },
                ContentBlock::ToolCall {
                    id: "t1".into(),
                    name: "run".into(),
                    arguments: json!({}),
                },
                ContentBlock::ToolCall {
                    id: "t2".into(),
                    name: "ls".into(),
                    arguments: json!({}),
                },
            ],
            stop_reason: StopReason::ToolUse,
            usage: None,
            model: None,
        };
        assert_eq!(m.tool_calls().len(), 2);
        assert!(matches!(m.tool_calls()[0], ContentBlock::ToolCall { .. }));
        assert!(
            m.tool_calls().iter().all(
                |b| matches!(b, ContentBlock::ToolCall { id, .. } if id == "t1" || id == "t2")
            )
        );

        let u = AgentMessage::user_text("x");
        assert!(u.tool_calls().is_empty());
    }
}
