//! Context compaction — deciding *when* to compact and *where* to cut.
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
//!
//! [`CompactionPolicy`] decides *when* to compact (the trigger) and *how much*
//! recent history to keep. Retention is absolute — a token budget plus a turn
//! floor — not a fraction of the window, so it means the same thing on an 8k
//! and a 1M model. The *trigger* uses the provider-reported input-token count
//! (never an estimate); only the cut boundary uses [`estimate_tokens`].

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

/// Tokens held free below the working ceiling by default: the trigger fires
/// once fewer than this many remain.
pub const DEFAULT_MIN_REMAINING: u64 = 16_384;

/// Recent context kept verbatim by default, in tokens — an absolute budget
/// (pi-style), independent of the model's window size.
pub const DEFAULT_KEEP_RECENT_TOKENS: u64 = 20_000;

/// Complete turns always kept by default, whatever their token size.
pub const DEFAULT_KEEP_RECENT_TURNS: usize = 2;

/// When to compact and how much recent context to retain.
///
/// The model window is not stored here: it comes from [`crate::limits`] (the
/// model's advertised context) or an override, and is passed to
/// [`CompactionPolicy::should_compact`]. `budget` is an optional working
/// ceiling (cost control) that defaults to the window; `min_remaining` is the
/// "minimum remaining context" trigger; the `keep_recent_*` fields are the
/// absolute recent history retained after compacting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactionPolicy {
    /// Working ceiling in tokens. `None` = the model window. Capped to the
    /// window at trigger time, so it can never disable window safety.
    pub budget: Option<u64>,
    /// Compact once fewer than this many tokens remain below the ceiling
    /// (`used >= ceiling - min_remaining`).
    pub min_remaining: u64,
    /// Recent context kept verbatim after compacting, in tokens.
    pub keep_recent_tokens: u64,
    /// Complete turns always kept, whatever their token size.
    pub keep_recent_turns: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            budget: None,
            min_remaining: DEFAULT_MIN_REMAINING,
            keep_recent_tokens: DEFAULT_KEEP_RECENT_TOKENS,
            keep_recent_turns: DEFAULT_KEEP_RECENT_TURNS,
        }
    }
}

impl CompactionPolicy {
    /// The working ceiling: the configured budget, capped to the model window.
    pub fn ceiling(&self, window: u64) -> u64 {
        self.budget.map_or(window, |b| b.min(window))
    }

    /// True once the provider-reported `used` input tokens leave less than
    /// `min_remaining` free below the ceiling.
    pub fn should_compact(&self, window: u64, used: u64) -> bool {
        used >= self.ceiling(window).saturating_sub(self.min_remaining)
    }
}

/// Rough per-message token estimate (~4 chars/token plus a small fixed
/// overhead). Used only to place the cut boundary; the compaction *trigger*
/// uses provider-reported token counts, never this.
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
/// result.
pub fn cut_for_budget(ctx: &[AgentMessage], keep_budget: u64) -> usize {
    safe_cut(ctx, token_cut(ctx, keep_budget))
}

/// Newest-first accumulation of [`estimate_tokens`] until `budget` is met:
/// the rough index where the retained verbatim tail begins.
fn token_cut(ctx: &[AgentMessage], budget: u64) -> usize {
    let mut acc = 0u64;
    let mut idx = ctx.len();
    while idx > 0 && acc < budget {
        idx -= 1;
        acc += estimate_tokens(&ctx[idx]);
    }
    idx
}

/// Index of the user message that begins the `turns`-th-most-recent turn, so
/// keeping from here retains at least `turns` complete turns. `0` when the
/// conversation has fewer than `turns` user messages; `ctx.len()` (no floor)
/// when `turns` is 0.
fn turn_start_cut(ctx: &[AgentMessage], turns: usize) -> usize {
    if turns == 0 {
        return ctx.len();
    }
    let mut seen = 0usize;
    for idx in (0..ctx.len()).rev() {
        if matches!(&ctx[idx], AgentMessage::User { .. }) {
            seen += 1;
            if seen == turns {
                return idx;
            }
        }
    }
    0
}

/// Cut for `policy`: keep the newest messages up to `keep_recent_tokens` but
/// never fewer than `keep_recent_turns` turns, made tool-safe by [`safe_cut`].
pub fn cut_for_policy(ctx: &[AgentMessage], policy: &CompactionPolicy) -> usize {
    let by_tokens = token_cut(ctx, policy.keep_recent_tokens);
    let by_turns = turn_start_cut(ctx, policy.keep_recent_turns);
    safe_cut(ctx, by_tokens.min(by_turns))
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
    fn trigger_uses_min_remaining_and_budget() {
        // No budget: the ceiling is the model window.
        let p = CompactionPolicy {
            min_remaining: 100,
            ..Default::default()
        };
        assert!(!p.should_compact(1000, 899));
        assert!(p.should_compact(1000, 900), "window - min_remaining");
        assert!(p.should_compact(1000, 5000), "past the window still triggers");

        // A working budget below the window triggers far earlier.
        let p = CompactionPolicy {
            budget: Some(500),
            min_remaining: 100,
            ..Default::default()
        };
        assert!(!p.should_compact(1_000_000, 399));
        assert!(p.should_compact(1_000_000, 400));

        // A budget above the window is capped to the window.
        let p = CompactionPolicy {
            budget: Some(2_000_000),
            min_remaining: 100,
            ..Default::default()
        };
        assert!(!p.should_compact(1000, 899));
        assert!(p.should_compact(1000, 900));
    }

    #[test]
    fn ceiling_is_budget_capped_to_window() {
        let p = CompactionPolicy {
            budget: Some(500),
            ..Default::default()
        };
        assert_eq!(p.ceiling(1_000_000), 500);
        let p = CompactionPolicy {
            budget: Some(2_000_000),
            ..Default::default()
        };
        assert_eq!(p.ceiling(1_000_000), 1_000_000);
        let p = CompactionPolicy::default();
        assert_eq!(p.ceiling(1234), 1234);
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
    fn cut_for_policy_keeps_recent_tokens() {
        let ctx = vec![
            user(&"a".repeat(200)),
            assistant_text(&"b".repeat(200)),
            user(&"c".repeat(200)),
            assistant_text(&"d".repeat(200)),
        ];
        // ~54 tokens each; a 100-token budget keeps the newest two.
        let p = CompactionPolicy {
            keep_recent_tokens: 100,
            keep_recent_turns: 0,
            ..Default::default()
        };
        assert_eq!(cut_for_policy(&ctx, &p), 2);
    }

    #[test]
    fn cut_for_policy_keeps_at_least_the_turn_floor() {
        // A tiny token budget must not drop below the last two turns.
        let ctx = vec![
            user("u1"),
            assistant_text("a1"),
            user("u2"),
            assistant_text("a2"),
            user("u3"),
            assistant_text("a3"),
        ];
        let p = CompactionPolicy {
            keep_recent_tokens: 0,
            keep_recent_turns: 2,
            ..Default::default()
        };
        assert_eq!(cut_for_policy(&ctx, &p), 2, "last two turns start at u2");
    }
}
