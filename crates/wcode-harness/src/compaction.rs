//! Context compaction — step 1: choosing *where* to cut.
//!
//! Compaction summarizes an old prefix of the conversation and keeps a recent
//! suffix. The prefix is discarded from what the model sees, so only the kept
//! suffix must be *valid* on the wire. The one invariant that makes a suffix
//! invalid is an orphaned `ToolResult`: a kept tool result whose matching
//! `ToolCall` (which lives in a preceding `Assistant` message) was summarized
//! away. Providers reject that as an unmatched tool result.
//!
//! [`safe_cut`] returns the earliest index at which the conversation may be
//! split so the kept suffix `ctx[cut..]` is self-consistent. It only ever moves
//! the cut *earlier* than requested — summarizing less rather than breaking the
//! pairing — and returns `0` when no boundary pairs everything.
//!
//! The reverse direction needs no check: the agent loop never leaves a
//! `ToolCall` without its `ToolResult`, and a `ToolResult` always follows its
//! `ToolCall`, so a kept call's results (later indices) are kept too.

use std::collections::HashSet;

use crate::message::{AgentMessage, ContentBlock};

/// Earliest index at which `ctx` may be split so the kept suffix `ctx[cut..]`
/// has no orphaned tool result.
///
/// `initial_cut` is the caller's preferred split (e.g. `ctx.len() - keep`); the
/// result is always `<= initial_cut` (clamped to `ctx.len()`). Returns `0` —
/// keep everything — when walking all the way back still can't pair a kept
/// result with its call.
pub fn safe_cut(ctx: &[AgentMessage], initial_cut: usize) -> usize {
    let cut = initial_cut.min(ctx.len());

    let mut available: HashSet<&str> = HashSet::new();
    let mut missing: HashSet<&str> = HashSet::new();
    for msg in &ctx[cut..] {
        absorb(msg, &mut available, &mut missing);
    }
    if missing.is_empty() {
        return cut;
    }

    // Grow the kept suffix backward, one message at a time, until every kept
    // result has its call. Adding a call removes the corresponding missing
    // result; adding a result whose call isn't yet seen records a new one.
    for idx in (0..cut).rev() {
        absorb(&ctx[idx], &mut available, &mut missing);
        if missing.is_empty() {
            return idx;
        }
    }

    0
}

/// Fold one message's tool calls/results into the pairing trackers.
fn absorb<'a>(
    msg: &'a AgentMessage,
    available: &mut HashSet<&'a str>,
    missing: &mut HashSet<&'a str>,
) {
    match msg {
        AgentMessage::Assistant { content, .. } => {
            for block in content {
                if let ContentBlock::ToolCall { id, .. } = block {
                    missing.remove(id.as_str());
                    available.insert(id.as_str());
                }
            }
        }
        AgentMessage::ToolResult { tool_call_id, .. } => {
            if !available.contains(tool_call_id.as_str()) {
                missing.insert(tool_call_id.as_str());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::StopReason;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::user_text(text)
    }

    fn assistant_text(text: &str) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: StopReason::Stop,
            usage: None,
            model: None,
        }
    }

    fn assistant_calls(ids: &[&str]) -> AgentMessage {
        AgentMessage::Assistant {
            content: ids
                .iter()
                .map(|id| ContentBlock::ToolCall {
                    id: (*id).into(),
                    name: "t".into(),
                    arguments: serde_json::json!({}),
                })
                .collect(),
            stop_reason: StopReason::ToolUse,
            usage: None,
            model: None,
        }
    }

    fn tool_result(id: &str) -> AgentMessage {
        AgentMessage::ToolResult {
            tool_call_id: id.into(),
            name: "t".into(),
            output: "o".into(),
            is_error: false,
        }
    }

    #[test]
    fn empty_and_out_of_range_are_clamped() {
        assert_eq!(safe_cut(&[], 0), 0);
        assert_eq!(safe_cut(&[], 5), 0);
        let ctx = vec![user("a")];
        assert_eq!(safe_cut(&ctx, 0), 0);
        assert_eq!(safe_cut(&ctx, 1), 1);
        assert_eq!(safe_cut(&ctx, 9), 1, "clamps to ctx.len()");
    }

    #[test]
    fn tool_free_context_keeps_initial_cut() {
        let ctx = vec![user("a"), assistant_text("x"), user("b"), assistant_text("y")];
        assert_eq!(safe_cut(&ctx, 2), 2);
        assert_eq!(safe_cut(&ctx, 3), 3);
    }

    #[test]
    fn cut_inside_a_tool_exchange_pulls_in_the_call() {
        // [user, assistant(c1), result(c1), assistant]
        let ctx = vec![
            user("q"),
            assistant_calls(&["c1"]),
            tool_result("c1"),
            assistant_text("done"),
        ];
        // Cutting at the result (2) would orphan it -> extend back to the call.
        assert_eq!(safe_cut(&ctx, 2), 1);
        assert_eq!(safe_cut(&ctx, 3), 3, "cut past the exchange drops it whole");
        // A cut already at the call boundary is safe as-is.
        assert_eq!(safe_cut(&ctx, 1), 1);
        // A cut at the trailing boundary keeps only the final assistant.
        assert_eq!(safe_cut(&ctx, 4), 4);
    }

    #[test]
    fn multi_call_assistant_is_pulled_in_whole() {
        // One assistant issuing two calls; results follow in order.
        let ctx = vec![
            user("q"),
            assistant_calls(&["c1", "c2"]),
            tool_result("c1"),
            tool_result("c2"),
            assistant_text("done"),
        ];
        // Cutting between the two results pulls the shared assistant back.
        assert_eq!(safe_cut(&ctx, 3), 1);
        assert_eq!(safe_cut(&ctx, 2), 1);
        // Cutting at a clean turn boundary stays put.
        assert_eq!(safe_cut(&ctx, 5), 5);
    }

    #[test]
    fn cut_at_a_user_boundary_after_a_completed_turn_is_kept() {
        let ctx = vec![
            user("q1"),
            assistant_calls(&["c1"]),
            tool_result("c1"),
            user("q2"),
            assistant_text("done"),
        ];
        assert_eq!(safe_cut(&ctx, 3), 3, "the q2 boundary needs no adjustment");
    }

    #[test]
    fn unpaired_result_walks_all_the_way_back() {
        // A result whose call is absent from the whole context cannot be
        // paired: a cut that keeps it gives up, one that drops it is fine.
        let ctx = vec![user("q"), tool_result("ghost"), assistant_text("x")];
        assert_eq!(safe_cut(&ctx, 1), 0);
        assert_eq!(safe_cut(&ctx, 2), 2, "dropping the orphan is fine");
    }

    #[test]
    fn kept_call_with_its_results_needs_no_extension() {
        // A kept call whose results are also kept (they follow it) is valid.
        let ctx = vec![
            user("q"),
            assistant_calls(&["c1"]),
            tool_result("c1"),
            user("next"),
        ];
        assert_eq!(safe_cut(&ctx, 3), 3, "keeps only the trailing user message");
        assert_eq!(safe_cut(&ctx, 1), 1, "keeps the whole tool exchange");
    }
}
