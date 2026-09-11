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

/// Tokens held free by default before compaction is considered: the trigger
/// fires once fewer than this many remain in the window.
pub const DEFAULT_MIN_REMAINING: u64 = 16_384;

/// Default share of the window retained as recent verbatim context.
pub const DEFAULT_KEEP_PERCENT: u8 = 50;

/// When to compact and how much recent context to retain.
///
/// The window is not stored here: it comes from [`crate::limits`] (the model's
/// advertised context) or an explicit override, and is passed to the methods
/// below. `min_remaining` is the "minimum remaining context" trigger;
/// `keep_percent` is the share of the window kept verbatim after compacting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactionPolicy {
    /// Compact once fewer than this many tokens remain free
    /// (`used >= window - min_remaining`).
    pub min_remaining: u64,
    /// After compacting, retain roughly this percentage of the window as recent
    /// verbatim messages; the older remainder is summarized. Clamped to 100.
    pub keep_percent: u8,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            min_remaining: DEFAULT_MIN_REMAINING,
            keep_percent: DEFAULT_KEEP_PERCENT,
        }
    }
}

impl CompactionPolicy {
    /// True once the provider-reported `used` input tokens leave less than
    /// `min_remaining` free in a `window`-token context.
    pub fn should_compact(&self, window: u64, used: u64) -> bool {
        used >= window.saturating_sub(self.min_remaining)
    }

    /// Token budget retained as recent verbatim context after compacting
    /// (`keep_percent`% of the window).
    pub fn keep_budget(&self, window: u64) -> u64 {
        window.saturating_mul(self.keep_percent.min(100) as u64) / 100
    }
}

/// Rough per-message token estimate (~4 chars/token plus a small fixed
/// overhead). Used only to place the keep-budget boundary; the compaction
/// *trigger* uses provider-reported token counts, never this.
pub fn estimate_tokens(msg: &AgentMessage) -> u64 {
    let bytes = match msg {
        AgentMessage::User { content } | AgentMessage::Assistant { content, .. } => {
            content.iter().map(block_bytes).sum()
        }
        AgentMessage::ToolResult {
            output,
            tool_call_id,
            name,
            ..
        } => (output.len() + tool_call_id.len() + name.len()) as u64,
    };
    bytes / 4 + 4
}

fn block_bytes(block: &ContentBlock) -> u64 {
    match block {
        ContentBlock::Text { text } | ContentBlock::Thinking { text } => text.len() as u64,
        ContentBlock::ToolCall {
            name, arguments, ..
        } => (name.len() + arguments.to_string().len()) as u64,
    }
}

/// Oldest index to keep when retaining `keep_budget` tokens of the most recent
/// history, adjusted by [`safe_cut`] so the kept suffix never orphans a tool
/// result. Walks backward accumulating [`estimate_tokens`] until the budget is
/// met, then hands the boundary to `safe_cut`.
pub fn cut_for_budget(ctx: &[AgentMessage], keep_budget: u64) -> usize {
    let mut acc = 0u64;
    let mut idx = ctx.len();
    while idx > 0 && acc < keep_budget {
        idx -= 1;
        acc += estimate_tokens(&ctx[idx]);
    }
    safe_cut(ctx, idx)
}

/// Convenience: the cut for `policy` against a `window`-token model.
pub fn cut_for_policy(ctx: &[AgentMessage], window: u64, policy: &CompactionPolicy) -> usize {
    cut_for_budget(ctx, policy.keep_budget(window))
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

#[cfg(test)]
mod policy_tests {
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

    fn assistant_tool_call(id: &str, args: String) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::ToolCall {
                id: id.into(),
                name: "t".into(),
                arguments: serde_json::json!({ "d": args }),
            }],
            stop_reason: StopReason::ToolUse,
            usage: None,
            model: None,
        }
    }

    fn tool_result(id: &str, output: String) -> AgentMessage {
        AgentMessage::ToolResult {
            tool_call_id: id.into(),
            name: "t".into(),
            output,
            is_error: false,
        }
    }

    #[test]
    fn trigger_fires_at_min_remaining() {
        let p = CompactionPolicy { min_remaining: 100, keep_percent: 50 };
        assert!(!p.should_compact(1000, 899));
        assert!(p.should_compact(1000, 900), "window - min_remaining");
        assert!(p.should_compact(1000, 1000));
        assert!(p.should_compact(1000, 5000), "past the window still triggers");
    }

    #[test]
    fn keep_budget_is_percent_of_window_clamped() {
        let quarter = CompactionPolicy { min_remaining: 0, keep_percent: 25 };
        assert_eq!(quarter.keep_budget(1000), 250);
        let all = CompactionPolicy { min_remaining: 0, keep_percent: 100 };
        assert_eq!(all.keep_budget(1000), 1000);
        let over = CompactionPolicy { min_remaining: 0, keep_percent: 200 };
        assert_eq!(over.keep_budget(1000), 1000, "keep_percent clamps to 100");
    }

    #[test]
    fn estimate_grows_with_content_and_counts_tool_output() {
        assert!(estimate_tokens(&user("hello there")) > estimate_tokens(&user("hi")));
        let big = AgentMessage::ToolResult {
            tool_call_id: "c".into(),
            name: "t".into(),
            output: "x".repeat(400),
            is_error: false,
        };
        assert!(estimate_tokens(&big) >= 100, "~400 chars -> ~100 tokens");
    }

    #[test]
    fn cut_keeps_the_newest_within_budget() {
        // Six ~54-token messages; a 120-token budget keeps the last three.
        let ctx = vec![
            user(&"a".repeat(200)),
            assistant_text(&"b".repeat(200)),
            user(&"c".repeat(200)),
            assistant_text(&"d".repeat(200)),
            user(&"e".repeat(200)),
            assistant_text(&"f".repeat(200)),
        ];
        assert_eq!(cut_for_budget(&ctx, 120), 3, "keeps the newest that fit");
    }

    #[test]
    fn cut_boundary_is_made_tool_safe() {
        // Budget lands on a tool result, so safe_cut pulls its call in.
        let ctx = vec![
            user("q"),
            assistant_tool_call("c1", "x".repeat(400)),
            tool_result("c1", "y".repeat(400)),
            assistant_text("done"),
        ];
        // A tiny budget keeps only the trailing assistant (no adjustment).
        assert_eq!(cut_for_budget(&ctx, 1), ctx.len() - 1);
        // A budget that reaches the result extends back to the call.
        assert_eq!(cut_for_budget(&ctx, 120), 1, "the call joins the suffix");
    }

    #[test]
    fn cut_for_policy_uses_keep_percent_of_window() {
        let ctx = vec![
            user(&"a".repeat(200)),
            assistant_text(&"b".repeat(200)),
            user(&"c".repeat(200)),
            assistant_text(&"d".repeat(200)),
        ];
        // ~54 tokens each; 25% of 400 = 100 tokens -> keeps the newest two.
        let p = CompactionPolicy { min_remaining: 0, keep_percent: 25 };
        assert_eq!(cut_for_policy(&ctx, 400, &p), 2);
    }
}
