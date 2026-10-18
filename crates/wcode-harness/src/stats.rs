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
//! We deliberately do NOT use the paired `ToolResult.name`
//! (`message.rs:63`) for the CALL tally: the two are 1:1 by
//! `tool_call_id` (the loop synthesizes an error ToolResult for every
//! unexecuted/blocked call — `loop_.rs:688`, `synthesize_unexecuted`
//! at `loop_.rs:798`), so they count the same calls, but the Assistant
//! block is the canonical, dedup-free definition.
//!
//! The ERROR tally DOES depend on that pairing, and says so: `errors`
//! and `result_bytes` are attributed by `tool_call_id` — never by the
//! result's own `name`, which is the loop's copy — so a call whose
//! result the transcript lost (a compacted prefix, a mid-run snapshot)
//! contributes its call but no error, and a result with no call
//! contributes nothing at all.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::message::{AgentMessage, ContentBlock};

/// One tool's tally: how many times it was called, how many of those calls did
/// not succeed, and how many bytes its results fed the context.
///
/// `errors` counts every `ToolResult { is_error: true }` paired to this tool's
/// `ToolCall` — INCLUDING the loop's synthesized aborts for blocked/unexecuted
/// calls (`loop_.rs:650` `output: "aborted before execution"`,
/// `fn synthesize_unexecuted` at `loop_.rs:798`). So "error" here means *the
/// call did not succeed*, not *the tool is buggy*; v1 does not separate
/// "blocked by a hook" from "failed" (that would need a new transcript flag).
///
/// `result_bytes` = Σ `ToolResult.output.len()` — the honest stand-in for a
/// per-tool token count, which nothing records: `ToolOutput` has no token field
/// (`tool.rs:23` `pub struct ToolOutput {` … fields end at `tool.rs:35`
/// `pub path: Option<String>,`) and usage is per TURN, provider-billed
/// (`message.rs:21`). It answers "which tool is eating my window?" without
/// inventing a number.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStat {
    pub name: String,
    pub calls: u64,
    pub errors: u64,
    pub result_bytes: u64,
}

/// A partial cost: what we COULD price, and how many turns we could not.
/// Micro-USD (1e-6 USD), integers only — so it stays `Eq` and serde-stable, and
/// the `$` a front-end prints is reproducible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cost {
    pub micro_usd: u64,
    /// Turns that reported usage but had no known price (an unknown model, or a
    /// mid-run switch to one). `summary()` names them: a total that hid them
    /// would misreport money.
    pub unpriced_turns: u64,
}

/// How many tool names the rendered `tools:` segment lists before `+K more`.
/// Many tools exist (`crates/wcode-cli/src/tools/`); the list is a glance, not
/// a manifest.
pub const TOOL_LIST_CAP: usize = 5;

/// Aggregate view of a conversation for `/usage`: how many turns reported
/// usage, the summed token counts, and which tools ran.
///
/// Pure data — no timing, and no cost OF ITS OWN: `cost` is filled by
/// the caller that knows the provider (`pricing::cost_of`, called from
/// the actor's `Request::Usage` reply), because pricing keys on a
/// `base_url`/`model` pair that an `AgentMessage` cannot carry. Every
/// other field folds from the transcript alone.
/// Derive order is fixed so tests can compare by value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStats {
    /// Assistant messages that carried `Some(usage)`.
    pub turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cache counts absent on a call are treated as 0.
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    /// Assistant messages that reported AT LEAST ONE cache field — the honest
    /// denominator for `cache_hit_pct()`. A provider that reports nothing must
    /// OMIT the rate rather than print `0%` (`streamfn.rs:1255`
    /// `cache_read_tokens: (usage.cached_input_tokens > 0).then_some(...)`:
    /// zero is rig's sentinel for "not reported").
    pub cache_turns: u64,
    /// One entry per distinct tool name, sorted by `calls` **descending**, ties
    /// by name **ascending**. Empty when no tool has been called. (WAS
    /// `Vec<(String, u64)>` — same sort, more facts.)
    pub tools: Vec<ToolStat>,
    /// Σ `tools[..].errors` — the headline count `summary()` prints.
    pub tool_errors: u64,
    /// Provider-reported input tokens of the most recent Assistant message with
    /// `Some(usage)`. `None` until a turn reports usage. NOT summed by
    /// `aggregate()` (see below): it is a "how full was the last request" fact.
    pub last_input_tokens: Option<u64>,
    /// The session's `$` cost. `None` from the pure fold (the fold has no
    /// provider); `Some` once the actor prices it. `skip_serializing_if` keeps
    /// it off the wire when absent — mandatory for an additive variant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
}

/// Fold a conversation into a [`SessionStats`].
///
/// Contract:
/// - `turns`/token sums iterate every `AgentMessage::Assistant { usage: Some(u), .. }`;
///   a missing `cache_read_tokens`/`cache_write_tokens` counts as 0.
/// - (+) `cache_turns` counts those same messages when EITHER cache field is
///   `Some` — the rate's denominator, so an unreported cache is omitted, never
///   `0%`.
/// - `tools` tallies every `ContentBlock::ToolCall { name, .. }` across
///   Assistant messages (independently of whether a matching ToolResult
///   exists), then sorts by `calls` desc, name asc.
/// - (+) a first pass maps `ContentBlock::ToolCall { id, name, .. }` →
///   `(name, tally index)`; a second walk over `AgentMessage::ToolResult
///   { tool_call_id, is_error, output, .. }` bumps `errors` (when `is_error`)
///   and adds `output.len()` to `result_bytes` on THAT call's row. The result's
///   own `name` is never used for attribution — the id is the pairing
///   (`message.rs:59` `tool_call_id: String,`).
/// - (+) a `ToolCall` with NO matching `ToolResult` is `calls += 1,
///   errors += 0`: a mid-run snapshot can legitimately catch a call in flight,
///   and a lost result is not a failure. Counting it as an error would invent
///   one.
/// - (+) a `ToolResult` whose `tool_call_id` matches no `ToolCall` bumps
///   nothing: there is no name to attribute it to (this is what a compacted
///   prefix looks like). The old doc's "does not depend on the pairing
///   invariant" claim is thereby AMENDED, not weakened — it still holds for
///   `calls`, and no longer for `errors`/`result_bytes`.
/// - (+) `tool_errors` = Σ errors over the tallies.
/// - (+) `cost` is left `None`: pricing needs the provider's `base_url`/`model`,
///   which no `AgentMessage` carries. The caller that knows it (the actor's
///   `Request::Usage` arm) sets `cost` with `pricing::cost_of(..)` on the SAME
///   slice, so `turns == priced + unpriced` there.
/// - `last_input_tokens` is the `input_tokens` of the last Assistant with
///   `Some(usage)`.
/// - (+) `is_error` includes the loop's synthesized aborts
///   (`loop_.rs:650`/`loop_.rs:798`), so `errors` means "the call did not
///   succeed", not "the tool failed" — say it in the doc, and in D028.
/// - Empty input ⇒ `SessionStats::default()`.
pub fn session_stats(messages: &[AgentMessage]) -> SessionStats {
    let mut stats = SessionStats::default();
    // name -> index into `stats.tools`
    let mut index: HashMap<String, usize> = HashMap::new();
    // tool_call_id -> the tool's name (the error/byte attribution key)
    let mut call_name: HashMap<&str, &str> = HashMap::new();

    // Pass 1: usage + the canonical call tally.
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
            if u.cache_read_tokens.is_some() || u.cache_write_tokens.is_some() {
                stats.cache_turns += 1;
            }
            stats.last_input_tokens = Some(u.input_tokens);
        }
        for block in content {
            if let ContentBlock::ToolCall { id, name, .. } = block {
                call_name.insert(id.as_str(), name.as_str());
                let i = match index.get(name) {
                    Some(&i) => i,
                    None => {
                        stats.tools.push(ToolStat {
                            name: name.clone(),
                            calls: 0,
                            errors: 0,
                            result_bytes: 0,
                        });
                        let i = stats.tools.len() - 1;
                        index.insert(name.clone(), i);
                        i
                    }
                };
                stats.tools[i].calls += 1;
            }
        }
    }

    // Pass 2: attribute each ToolResult to its call's row, by id.
    for m in messages {
        let AgentMessage::ToolResult {
            tool_call_id,
            output,
            is_error,
            ..
        } = m
        else {
            continue;
        };
        let Some(name) = call_name.get(tool_call_id.as_str()) else {
            continue;
        };
        if let Some(&i) = index.get(*name) {
            if *is_error {
                stats.tools[i].errors += 1;
            }
            stats.tools[i].result_bytes += output.len() as u64;
        }
    }

    stats.tool_errors = stats.tools.iter().map(|t| t.errors).sum();
    stats
        .tools
        .sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
    stats
}

/// Sum per-session views into a total (the `/usage` `total` row).
///
/// Contract:
/// - `turns`, `input_tokens`, `output_tokens`, `cache_read_tokens`,
///   `cache_write_tokens`, `cache_turns`, `tool_errors`: plain sums.
/// - `tools`: merged BY NAME — `calls`/`errors`/`result_bytes` summed per name —
///   then re-sorted `calls` desc, name asc, so the total reads like a single
///   session.
/// - `cost`: `Some(Cost { micro_usd: Σ, unpriced_turns: Σ })` when AT LEAST ONE
///   member has `Some`; `None` when none does. A member with `cost: None`
///   contributes nothing — its sessions are unknown, not "0 unpriced turns".
/// - `last_input_tokens`: **always `None`**. A total has no "last request"; the
///   context gauge is a per-surface fact and must never be summed.
/// - empty slice ⇒ `SessionStats::default()`.
///
/// Does NOT promise: deduplication. Two members that share a transcript (a
/// peer and its mirror) are counted twice — a caller passes each session once.
pub fn aggregate(members: &[SessionStats]) -> SessionStats {
    let mut out = SessionStats::default();
    let mut index: HashMap<String, usize> = HashMap::new();
    for m in members {
        out.turns += m.turns;
        out.input_tokens += m.input_tokens;
        out.output_tokens += m.output_tokens;
        out.cache_read_tokens += m.cache_read_tokens;
        out.cache_write_tokens += m.cache_write_tokens;
        out.cache_turns += m.cache_turns;
        out.tool_errors += m.tool_errors;
        for t in &m.tools {
            let i = match index.get(&t.name) {
                Some(&i) => i,
                None => {
                    out.tools.push(ToolStat {
                        name: t.name.clone(),
                        calls: 0,
                        errors: 0,
                        result_bytes: 0,
                    });
                    let i = out.tools.len() - 1;
                    index.insert(t.name.clone(), i);
                    i
                }
            };
            out.tools[i].calls += t.calls;
            out.tools[i].errors += t.errors;
            out.tools[i].result_bytes += t.result_bytes;
        }
    }
    let micro_usd = members
        .iter()
        .filter_map(|m| m.cost.as_ref())
        .map(|c| c.micro_usd)
        .sum();
    let unpriced_turns = members
        .iter()
        .map(|m| match &m.cost {
            Some(c) => c.unpriced_turns,
            None => m.turns, // not one turn priced ⇒ every reported turn is unpriced
        })
        .sum();
    out.cost = members
        .iter()
        .any(|m| m.cost.is_some())
        .then_some(Cost {
            micro_usd,
            unpriced_turns,
        });
    // `last_input_tokens` stays `None`: a total has no "last request".
    out.tools
        .sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.name.cmp(&b.name)));
    out
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
    /// - appends `, N% cache` when `cache_hit_pct()` is `Some`,
    ///   `, N tool error(s)` when `tool_errors > 0`, and `, ~$d.dddd`
    ///   when `cost` is `Some` — `(N turns unpriced)` when
    ///   `Cost::unpriced_turns > 0`. `% cache` sits right after the cache
    ///   tokens, `tool error(s)` after `tools:`, and `~$` last.
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
        if let Some(pct) = self.cache_hit_pct() {
            parts.push(format!("{pct}% cache"));
        }
        if let Some(tools) = self.tools_segment() {
            parts.push(format!("tools: {tools}"));
        }
        if self.tool_errors > 0 {
            parts.push(format!(
                "{} tool error{}",
                self.tool_errors,
                if self.tool_errors == 1 { "" } else { "s" }
            ));
        }
        if let Some(cost) = &self.cost {
            parts.push(if cost.unpriced_turns == 0 {
                format!("~{}", Self::format_usd(cost.micro_usd))
            } else {
                format!(
                    "~{} ({} turn{} unpriced)",
                    Self::format_usd(cost.micro_usd),
                    cost.unpriced_turns,
                    if cost.unpriced_turns == 1 { "" } else { "s" }
                )
            });
        }
        format!("usage: {}", parts.join(", "))
    }

    /// Cache hit rate as an integer percent, rounded to the nearest percent.
    ///
    /// Contract:
    /// - `Some((cache_read * 100 + input / 2) / input)` when `input_tokens > 0
    ///   && cache_turns > 0`, else `None`: a session that reported no cache
    ///   OMITS the clause instead of claiming `0%`.
    /// - the denominator is `input_tokens` ALONE. On the chat-completions wire a
    ///   cached read is a SUBSET of `input_tokens` (rig maps
    ///   `prompt_tokens_details.cached_tokens`), so adding `cache_read` back
    ///   would double-count. A provider on the Anthropic wire — where
    ///   `input_tokens` EXCLUDES cache reads/writes — would understate here;
    ///   wcode speaks the OpenAI-compatible wire only, and that is the
    ///   documented assumption (D028).
    /// - saturates at 100: more cached tokens than input is a provider bug, and
    ///   `120%` on screen would be worse than the clamp.
    /// - `cache_write` is not a hit: it is reported separately and priced.
    pub fn cache_hit_pct(&self) -> Option<u8> {
        if self.input_tokens == 0 || self.cache_turns == 0 {
            return None;
        }
        let pct =
            (self.cache_read_tokens * 100 + self.input_tokens / 2) / self.input_tokens;
        Some(pct.min(100) as u8)
    }

    /// Share of tool calls that did not succeed, rounded to the nearest percent.
    ///
    /// `Some((tool_errors * 100 + calls / 2) / calls)` over Σ`calls` when a tool
    /// ran at all, else `None`. `errors` counts hook-blocked and synthesized
    /// aborts (see `ToolStat`), so this is "not a success", never "the tool is
    /// buggy"; saturates at 100 for the same reason as `cache_hit_pct`.
    pub fn tool_error_pct(&self) -> Option<u8> {
        let calls: u64 = self.tools.iter().map(|t| t.calls).sum();
        if calls == 0 {
            return None;
        }
        let pct = (self.tool_errors * 100 + calls / 2) / calls;
        Some(pct.min(100) as u8)
    }

    /// `$0.0123` — dollars, four decimals, TRUNCATED at the last digit so the
    /// printed value never overstates the micro-USD it came from (brief §7.2:
    /// "the printed `~$` must agree to the truncated micro-USD").
    ///
    /// A priced-but-tiny cost prints `$0.0000`; the `~` `summary()` prefixes
    /// marks it approximate. Private: only `summary()` renders money.
    fn format_usd(micro_usd: u64) -> String {
        format!(
            "${}.{:04}",
            micro_usd / 1_000_000,
            (micro_usd % 1_000_000) / 100
        )
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
            .map(|t| format!("{}×{}", t.name, t.calls))
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

    fn result(id: &str, name: &str, output: &str, is_error: bool) -> AgentMessage {
        AgentMessage::ToolResult {
            tool_call_id: id.into(),
            name: name.into(),
            output: output.into(),
            is_error,
        }
    }

    fn stat(name: &str, calls: u64, errors: u64, bytes: u64) -> ToolStat {
        ToolStat {
            name: name.into(),
            calls,
            errors,
            result_bytes: bytes,
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
                cache_turns: 2,
                tools: vec![],
                tool_errors: 0,
                last_input_tokens: Some(30),
                cost: None,
            }
        );
    }

    #[test]
    fn tallies_tool_calls_by_name() {
        // assistant_tools(&["read","read","grep"]) → [read×2, grep×1].
        let stats = session_stats(&[assistant_tools(&["read", "read", "grep"])]);
        assert_eq!(stats.tools, vec![stat("read", 2, 0, 0), stat("grep", 1, 0, 0)]);
        // Tool calls are counted with or without usage on the message.
        assert_eq!(stats.turns, 0);
    }

    #[test]
    fn tools_sorted_count_desc_then_name_asc() {
        // ["zzz","aaa","aaa","bbb"] → [aaa×2, bbb×1, zzz×1]:
        // count desc, then name asc on the tied count-1 entries.
        let stats = session_stats(&[assistant_tools(&["zzz", "aaa", "aaa", "bbb"])]);
        assert_eq!(
            stats.tools,
            vec![stat("aaa", 2, 0, 0), stat("bbb", 1, 0, 0), stat("zzz", 1, 0, 0)]
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
        // "usage: 2 turns, 40 in, 60 out, 3 cache read, 5 cache write, 8% cache, tools: read×1".
        let messages = vec![
            assistant_usage(40, 60, Some(3), Some(5)),
            assistant_tools(&["read"]),
            AgentMessage::user_text("q"),
            assistant_usage(0, 0, None, None),
        ];
        assert_eq!(
            session_stats(&messages).summary(),
            "usage: 2 turns, 40 in, 60 out, 3 cache read, 5 cache write, 8% cache, tools: read×1"
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
                stat("a", 1, 0, 0),
                stat("b", 1, 0, 0),
                stat("c", 1, 0, 0),
                stat("d", 1, 0, 0),
                stat("e", 1, 0, 0),
                stat("f", 1, 0, 0),
                stat("g", 1, 0, 0),
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

    /// One call, one `is_error: true` result ⇒ calls 1, errors 1, bytes =
    /// `output.len()`. The pairing is the ID, not the result's `name`: give the
    /// result a DIFFERENT name and assert it still lands on the call's row.
    #[test]
    fn tool_errors_pair_by_call_id() {
        let messages = vec![
            assistant_tools(&["read"]), // ToolCall id "t0", name "read"
            result("t0", "grep", "boom", true), // name differs; id is the key
        ];
        let stats = session_stats(&messages);
        assert_eq!(stats.tools, vec![stat("read", 1, 1, 4)]);
        assert_eq!(stats.tool_errors, 1);
    }

    /// A call with no result (a mid-run snapshot) ⇒ calls 1, errors 0,
    /// result_bytes 0 — a call in flight is not a failure.
    #[test]
    fn a_call_without_a_result_counts_as_a_call_not_an_error() {
        let stats = session_stats(&[assistant_tools(&["read"])]);
        assert_eq!(stats.tools, vec![stat("read", 1, 0, 0)]);
        assert_eq!(stats.tool_errors, 0);
    }

    /// The loop's synthesized `"aborted before execution"` result
    /// (`loop_.rs:650`, `synthesize_unexecuted` at `loop_.rs:798`) carries
    /// `is_error: true` ⇒ it DOES count: "not a success", not "tool bug".
    #[test]
    fn a_synthesized_abort_result_is_counted_as_an_error() {
        let messages = vec![
            assistant_tools(&["read"]),
            result("t0", "read", "aborted before execution", true),
        ];
        let stats = session_stats(&messages);
        assert_eq!(stats.tools[0].errors, 1);
        assert_eq!(stats.tool_errors, 1);
    }

    /// `result_bytes` is the per-name Σ of `output.len()` (bytes, not chars, not
    /// tokens), and a result with an unknown id adds nothing.
    #[test]
    fn result_bytes_sum_per_tool() {
        let messages = vec![
            assistant_tools(&["read", "read"]), // t0, t1
            result("t0", "read", "abc", false),  // 3 bytes
            result("t1", "read", "de", false),   // 2 bytes
            result("gone", "read", "ignored", false), // unknown id: nothing
        ];
        let stats = session_stats(&messages);
        assert_eq!(stats.tools, vec![stat("read", 2, 0, 5)]);
    }

    /// `cache_read`/`cache_write` both absent on every turn ⇒ `cache_turns == 0`
    /// ⇒ `cache_hit_pct() == None` ⇒ no `% cache` clause (never `0%`).
    #[test]
    fn cache_hit_pct_is_none_without_cache_reporting() {
        let stats = session_stats(&[assistant_usage(100, 10, None, None)]);
        assert_eq!(stats.cache_turns, 0);
        assert_eq!(stats.cache_hit_pct(), None);
        assert!(!stats.summary().contains("% cache"));
    }

    /// Rounding: read 1 of input 3 ⇒ 33%; read 2 of 3 ⇒ 67%; read 3 of 3 ⇒ 100%
    /// (and a hypothetical read > input saturates, never 120%).
    #[test]
    fn cache_hit_pct_rounds_to_nearest_percent() {
        assert_eq!(
            session_stats(&[assistant_usage(3, 0, Some(1), None)]).cache_hit_pct(),
            Some(33)
        );
        assert_eq!(
            session_stats(&[assistant_usage(3, 0, Some(2), None)]).cache_hit_pct(),
            Some(67)
        );
        assert_eq!(
            session_stats(&[assistant_usage(3, 0, Some(3), None)]).cache_hit_pct(),
            Some(100)
        );
        assert_eq!(
            session_stats(&[assistant_usage(3, 0, Some(6), None)]).cache_hit_pct(),
            Some(100)
        );
    }

    /// No tool ran ⇒ `tool_error_pct() == None` (not 0%), for the same reason.
    #[test]
    fn tool_error_pct_is_none_with_no_calls() {
        let stats = session_stats(&[assistant_usage(1, 1, None, None)]);
        assert_eq!(stats.tool_error_pct(), None);
        assert!(!stats.summary().contains("tool error"));
    }

    /// Two members ⇒ scalar sums, tools merged by name and re-sorted, `cost`
    /// summed when one side has `Some`, and `last_input_tokens == None`.
    #[test]
    fn aggregate_sums_two_members_and_sorts_tools() {
        let a = SessionStats {
            turns: 1,
            input_tokens: 10,
            output_tokens: 20,
            cache_turns: 1,
            tools: vec![stat("read", 2, 0, 5)],
            cost: Some(Cost {
                micro_usd: 4_000,
                unpriced_turns: 0,
            }),
            last_input_tokens: Some(10),
            ..Default::default()
        };
        let b = SessionStats {
            turns: 2,
            input_tokens: 30,
            output_tokens: 40,
            // `tool_errors` is a plain sum (contract), so a hand-built member
            // carries the headline its tools imply (as `session_stats` would).
            tools: vec![stat("read", 1, 1, 7), stat("grep", 3, 0, 2)],
            tool_errors: 1,
            ..Default::default()
        };
        let total = aggregate(&[a, b]);
        assert_eq!(total.turns, 3);
        assert_eq!(total.input_tokens, 40);
        assert_eq!(total.output_tokens, 60);
        assert_eq!(total.cache_turns, 1);
        assert_eq!(total.tool_errors, 1);
        // read 2+1 = 3, grep 3 → tie on count, name asc puts grep first.
        assert_eq!(
            total.tools,
            vec![stat("grep", 3, 0, 2), stat("read", 3, 1, 12)]
        );
        // `a` priced, `b` unpriced ⇒ micro summed, and b's 2 turns counted unpriced.
        assert_eq!(
            total.cost,
            Some(Cost {
                micro_usd: 4_000,
                unpriced_turns: 2,
            })
        );
        assert_eq!(total.last_input_tokens, None);
    }

    /// `cost: Some(Cost { micro_usd: 4_000, unpriced_turns: 0 })` ⇒ the line
    /// ends `~$0.0040`; `unpriced_turns: 1` ⇒ `~$0.0040 (1 turn unpriced)`.
    #[test]
    fn summary_reports_cost_and_unpriced_turns() {
        let priced = SessionStats {
            turns: 1,
            input_tokens: 1,
            output_tokens: 1,
            cost: Some(Cost {
                micro_usd: 4_000,
                unpriced_turns: 0,
            }),
            ..Default::default()
        };
        assert!(
            priced.summary().ends_with("~$0.0040"),
            "{}",
            priced.summary()
        );

        let partial = SessionStats {
            turns: 1,
            input_tokens: 1,
            output_tokens: 1,
            cost: Some(Cost {
                micro_usd: 4_000,
                unpriced_turns: 1,
            }),
            ..Default::default()
        };
        assert!(
            partial.summary().ends_with("~$0.0040 (1 turn unpriced)"),
            "{}",
            partial.summary()
        );
    }
}
