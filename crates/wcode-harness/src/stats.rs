//! Session summary for `/usage`.
//!
//! One shared implementation of the aggregate view both front-ends print: the
//! REPL's text line and the TUI's notice. Keeping it in the kernel means the
//! tally and its wording live in one place; each front-end adds only its own
//! surrounding chrome.
//!
//! Why a new module rather than folding into `message.rs`: `message.rs` is the
//! serde wire model (`ContentBlock` at `message.rs:5`, `AgentMessage` at
//! `message.rs:47`); `stats` is *analysis over* that model. Keeping the model
//! pure means the derived view can change (add fields, change the cap) without
//! touching the wire format or its round-trip tests.
//!
//! Source of the tool tally: `AgentMessage::Assistant` `ContentBlock::ToolCall`
//! blocks (`message.rs:11`), i.e. the model's *intent*, exactly once per call.
//! We deliberately do NOT use the paired `ToolResult.name` (`message.rs:63`):
//! the two are 1:1 by `tool_call_id` (the loop synthesizes an error ToolResult
//! for every unexecuted/blocked call — `loop_.rs:683`), so they count the same
//! calls, but the Assistant block is the canonical, dedup-free definition and
//! does not depend on the loop's pairing invariant holding.

use std::collections::HashMap;

use crate::message::{AgentMessage, ContentBlock};

/// How many tool names the rendered `tools:` segment lists before `+K more`.
/// Many tools exist (`crates/wcode-cli/src/tools/`); the list is a glance, not
/// a manifest.
pub const TOOL_LIST_CAP: usize = 5;

/// Aggregate view of a conversation for `/usage`: how many turns reported
/// usage, the summed token counts, and which tools ran.
///
/// Pure data — no timing, no cost (there is no timestamp in `AgentMessage`).
/// Derive order is fixed so tests can compare by value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionStats {
    /// Assistant messages that carried `Some(usage)`.
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache counts absent on a call are treated as 0.
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Tool name → call count, one entry per distinct name, sorted by count
    /// **descending**, ties broken by name **ascending**. Empty when no tool
    /// has been called.
    pub tools: Vec<(String, u64)>,
    /// Provider-reported input tokens of the most recent Assistant message with
    /// `Some(usage)`: how full the context was on the last request. `None`
    /// until a turn reports usage.
    pub last_input_tokens: Option<u64>,
}

/// Fold a conversation into a [`SessionStats`].
///
/// Contract:
/// - `turns`/token sums iterate every `AgentMessage::Assistant { usage: Some(u), .. }`;
///   a missing `cache_read_tokens`/`cache_write_tokens` counts as 0.
/// - `tools` tallies every `ContentBlock::ToolCall { name, .. }` across
///   Assistant messages (independently of whether a matching ToolResult
///   exists), then sorts by count desc, name asc.
/// - `last_input_tokens` is the `input_tokens` of the last Assistant with
///   `Some(usage)`.
/// - Empty input ⇒ `SessionStats::default()`.
pub fn session_stats(messages: &[AgentMessage]) -> SessionStats {
    let mut stats = SessionStats::default();
    let mut tally: HashMap<&str, u64> = HashMap::new();
    for m in messages {
        let AgentMessage::Assistant { content, usage, .. } = m else {
            continue;
        };
        if let Some(u) = usage {
            stats.turns += 1;
            stats.input_tokens += u.input_tokens;
            stats.output_tokens += u.output_tokens;
            stats.cache_read_tokens += u.cache_read_tokens.unwrap_or(0);
            stats.cache_write_tokens += u.cache_write_tokens.unwrap_or(0);
            stats.last_input_tokens = Some(u.input_tokens);
        }
        for block in content {
            if let ContentBlock::ToolCall { name, .. } = block {
                *tally.entry(name.as_str()).or_insert(0) += 1;
            }
        }
    }
    let mut tools: Vec<(String, u64)> =
        tally.into_iter().map(|(n, c)| (n.to_string(), c)).collect();
    tools.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    stats.tools = tools;
    stats
}

impl SessionStats {
    /// The one-line summary both front-ends print: plain text — no colors, no
    /// layout, no context; front-ends add chrome.
    ///
    /// Contract:
    /// - `turns == 0` ⇒ `"(no usage reported)"`.
    /// - otherwise ⇒
    ///   `"usage: N turns, X in, Y out[, C cache read][, W cache write][, tools: …]"`
    ///   where the cache fields are omitted when 0, the `tools:` segment is
    ///   omitted when no tool ran, and the tool names are capped at
    ///   [`TOOL_LIST_CAP`], the remainder folded into `"+K more"`.
    /// - The trailing `context U/L` readout is intentionally NOT here: its
    ///   limit comes from the front-end (REPL `model_limit`, TUI
    ///   `status.context_limit`), so each wraps/appends it.
    pub fn summary(&self) -> String {
        if self.turns == 0 {
            return "(no usage reported)".to_string();
        }
        let mut parts = vec![
            format!(
                "{} turn{}",
                self.turns,
                if self.turns == 1 { "" } else { "s" }
            ),
            format!("{} in", self.input_tokens),
            format!("{} out", self.output_tokens),
        ];
        if self.cache_read_tokens > 0 {
            parts.push(format!("{} cache read", self.cache_read_tokens));
        }
        if self.cache_write_tokens > 0 {
            parts.push(format!("{} cache write", self.cache_write_tokens));
        }
        if let Some(tools) = self.tools_segment() {
            parts.push(format!("tools: {tools}"));
        }
        format!("usage: {}", parts.join(", "))
    }

    /// The `read×12 grep×4 … +K more` segment, or `None` when no tool ran.
    /// Factored out so a front-end can place the tool list independently if it
    /// ever wants to; `summary()` uses it for the inline form.
    fn tools_segment(&self) -> Option<String> {
        if self.tools.is_empty() {
            return None;
        }
        let shown = self.tools.len().min(TOOL_LIST_CAP);
        let mut seg = self.tools[..shown]
            .iter()
            .map(|(name, count)| format!("{name}×{count}"))
            .collect::<Vec<_>>()
            .join(" ");
        if self.tools.len() > TOOL_LIST_CAP {
            seg.push_str(&format!(" +{} more", self.tools.len() - TOOL_LIST_CAP));
        }
        Some(seg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{StopReason, Usage};
    use serde_json::json;

    // --- helpers (mirror the ones deleted from repl.rs `assistant_with_usage`) ---

    fn assistant_usage(input: u64, output: u64, read: Option<u64>, write: Option<u64>) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text { text: "a".into() }],
            stop_reason: StopReason::Stop,
            usage: Some(Usage {
                input_tokens: input,
                output_tokens: output,
                cache_read_tokens: read,
                cache_write_tokens: write,
            }),
            model: None,
        }
    }

    fn assistant_tools(names: &[&str]) -> AgentMessage {
        AgentMessage::Assistant {
            content: names
                .iter()
                .enumerate()
                .map(|(i, n)| ContentBlock::ToolCall {
                    id: format!("t{i}"),
                    name: (*n).into(),
                    arguments: json!({}),
                })
                .collect(),
            stop_reason: StopReason::ToolUse,
            usage: None,
            model: None,
        }
    }

    #[test]
    fn empty_is_default() {
        assert_eq!(session_stats(&[]), SessionStats::default());
    }

    #[test]
    fn counts_usage_only_on_reported_turns() {
        // Two assistants with usage, one without, plus a user message → turns 2,
        // summed tokens, cache absent == 0.
        let messages = vec![
            AgentMessage::user_text("q"),
            assistant_usage(10, 20, Some(3), None),
            AgentMessage::Assistant {
                content: vec![ContentBlock::Text { text: "x".into() }],
                stop_reason: StopReason::Stop,
                usage: None,
                model: None,
            },
            assistant_usage(30, 40, None, Some(5)),
        ];
        assert_eq!(
            session_stats(&messages),
            SessionStats {
                turns: 2,
                input_tokens: 40,
                output_tokens: 60,
                cache_read_tokens: 3,
                cache_write_tokens: 5,
                tools: vec![],
                last_input_tokens: Some(30),
            }
        );
    }

    #[test]
    fn tallies_tool_calls_by_name() {
        // assistant_tools(&["read","read","grep"]) → [("read",2),("grep",1)].
        let stats = session_stats(&[assistant_tools(&["read", "read", "grep"])]);
        assert_eq!(
            stats.tools,
            vec![("read".to_string(), 2), ("grep".to_string(), 1)]
        );
        // Tool calls are counted with or without usage on the message.
        assert_eq!(stats.turns, 0);
    }

    #[test]
    fn tools_sorted_count_desc_then_name_asc() {
        // ["zzz","aaa","aaa"] → [("aaa",2),("zzz",1)] (tie on count → name asc).
        let stats = session_stats(&[assistant_tools(&["zzz", "aaa", "aaa"])]);
        assert_eq!(
            stats.tools,
            vec![("aaa".to_string(), 2), ("zzz".to_string(), 1)]
        );
    }

    #[test]
    fn last_input_tokens_is_most_recent() {
        // [usage(10,..), usage(40,..)] → Some(40); [] → None.
        let messages = vec![
            assistant_usage(10, 20, None, None),
            AgentMessage::user_text("q"),
            assistant_usage(40, 3, None, None),
            assistant_tools(&["read"]), // no usage: skipped
        ];
        assert_eq!(session_stats(&messages).last_input_tokens, Some(40));
        assert_eq!(session_stats(&[]).last_input_tokens, None);
        assert_eq!(
            session_stats(&[AgentMessage::user_text("q")]).last_input_tokens,
            None
        );
    }

    #[test]
    fn summary_empty_and_full_line() {
        // default → "(no usage reported)".
        assert_eq!(SessionStats::default().summary(), "(no usage reported)");

        // turns 2, 40 in, 60 out, read 3, write 5, tools read×1 →
        // "usage: 2 turns, 40 in, 60 out, 3 cache read, 5 cache write, tools: read×1".
        let messages = vec![
            assistant_usage(40, 60, Some(3), Some(5)),
            assistant_tools(&["read"]),
            AgentMessage::user_text("q"),
            assistant_usage(0, 0, None, None),
        ];
        assert_eq!(
            session_stats(&messages).summary(),
            "usage: 2 turns, 40 in, 60 out, 3 cache read, 5 cache write, tools: read×1"
        );

        // Singular "1 turn".
        let one = SessionStats {
            turns: 1,
            input_tokens: 10,
            output_tokens: 20,
            ..Default::default()
        };
        assert_eq!(one.summary(), "usage: 1 turn, 10 in, 20 out");
    }

    #[test]
    fn summary_caps_tools_and_says_more() {
        // 7 distinct tools → 5 names then "+2 more".
        let stats = SessionStats {
            turns: 1,
            input_tokens: 1,
            output_tokens: 1,
            tools: vec![
                ("a".to_string(), 1),
                ("b".to_string(), 1),
                ("c".to_string(), 1),
                ("d".to_string(), 1),
                ("e".to_string(), 1),
                ("f".to_string(), 1),
                ("g".to_string(), 1),
            ],
            last_input_tokens: Some(1),
            ..Default::default()
        };
        assert_eq!(
            stats.summary(),
            "usage: 1 turn, 1 in, 1 out, tools: a×1 b×1 c×1 d×1 e×1 +2 more"
        );
    }

    #[test]
    fn summary_omits_empty_tools_segment() {
        // usage reported, no tool calls → no ", tools:" suffix.
        let stats = SessionStats {
            turns: 1,
            input_tokens: 10,
            output_tokens: 20,
            ..Default::default()
        };
        let line = stats.summary();
        assert_eq!(line, "usage: 1 turn, 10 in, 20 out");
        assert!(!line.contains("tools:"));
    }
}
