# D028 — Usage is per-member, cost-aware, and honest about what it cannot price

- **Status:** accepted (human sign-off on the W024 brief — recommendations followed)
- **Date:** this session
- **Relates to:** [`limits.rs`](../../crates/wcode-harness/src/limits.rs) (the
  static-table precedent this copies), D026/D027 (the info modal `/usage` now
  populates), the `Status`/`GetHistory` reply pattern in `actor.rs`
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §4 (**Info modal body
  rows** — `/usage` moves from the prose list to an **item** ledger)
- **Work item:** [`work/W024-usage-telemetry.md`](../work/W024-usage-telemetry.md)

## Context

`SessionStats` (`stats.rs`) was the only telemetry in the tree: a pure fold over
one transcript summing input/output/cache tokens and counting tool **calls** —
doc'd explicitly as *"Pure data — no timing, no cost"*. `/usage` read it for the
**focused** surface only. There was no cross-member aggregation, no cache *rate*,
no tool **error** rate, no per-tool attribution, and no `$` cost anywhere.

The human asked to track **all member usage**, with input/output tokens, cache hit
rate *if possible*, tool usage/error rate *and token consumption if available*, and
a total `$` cost *if available*. The qualifiers are load-bearing: two of the asks
are not answerable from the data wcode has.

## Decision

**Cost.** A static, in-kernel price table
(`crates/wcode-harness/src/pricing.rs`, the twin of `limits.rs`), snapshotted from
`models.dev`'s `opencode-go` catalog, keyed `(base_url, model)` and gated by the
existing `limits::is_opencode_go`. Prices are **micro-USD per Mtok, integers** —
never floats — so a price and a derived cost stay `Eq`/serde-stable and the
printed `$` is byte-reproducible. **No config file** (doctrine: behaviour is code).

- **The cached read is a SUBSET of `input_tokens`** on the chat-completions wire
  (rig maps `prompt_tokens_details.cached_tokens`), so a cached token is billed at
  the cache rate **instead of** the input rate: `(input − cache_read)·in +
  cache_read·read + cache_write·write + output·out`, one numerator summed across
  turns and divided **once**.
- **Unknown model/provider ⇒ no `$`, never `$0`.** A known-free model is
  `Some(price::default())` ⇒ `~$0.0000`. This is why the lookup returns
  `Option<ModelPrice>`, not a `u64` defaulting to 0.
- **Partial coverage is named, not hidden.** `Cost { micro_usd, unpriced_turns }`;
  a mid-run model switch prints `~$x (n turns unpriced)`.
- **v1 is base-rate only.** 6 zen models carry context **tiers** (e.g.
  `claude-haiku-5-5` doubles+ above 100k); the table stores the base rate, so a
  tiered model is a **documented undercount**. Tier support is a named follow-on.
- **17 `limits.rs` ids have no price** (`glm-5` among them) → those sessions print
  **no** `$`. Pricing coverage ≠ context-window coverage; the two tables answer
  different questions and are allowed to disagree.

**Cache hit rate.** `cache_read / input_tokens`, integer percent, rounded to
nearest. `None` — the clause is **omitted**, never `0%` — when `input_tokens == 0`
or no turn reported a cache field (`Option<u64>`, and zero is rig's "not
reported" sentinel). Only the OpenAI-compatible wire is supported; an Anthropic
wire (where `input_tokens` excludes cache) would understate — documented.

**Tool accounting.** Pair `ToolCall` with `ToolResult` by `tool_call_id` in the
same fold → `ToolStat { name, calls, errors, result_bytes }`. `is_error` counts
the loop's **synthesized** results for blocked/aborted calls, so the number is
named **"errors"** = *the call did not succeed*, never "failures".

**Per-tool token consumption is NOT derivable** — usage is per turn, and
`ToolOutput`/`ToolResult` carry no token counts; a provider bills prompt tokens per
*request*, not per tool. The honest substitute is **`result_bytes`**: the summed
`ToolResult.output.len()` — exact, folded from the transcript, answering "which
tool is eating my window?". No `/4` token estimate: an invented number in a cost
report is worse than an absent one.

**All-member surfacing** is a **per-session `Request::Usage` → `AgentEvent::Usage
{ stats }`** (answered from the actor exactly like `Status`, the cost priced
server-side where `llm.base_url`/`llm.model` live), summed by a kernel
`aggregate()`, and **styled per front-end**. Rejected: pushing usage into the
roster `SessionInfo` (the registry has no messages; a re-push per `MessageEnd`;
the local TUI never reads it) and a server-side aggregate request (no server in
the in-process path). `PROTOCOL_VERSION` stays `1` (additive).

**`aggregate()` cost rule (the subtlety).** The total is `Some` iff ≥1 member
priced something; a member with `cost: None` contributes its **`turns`** as
unpriced turns (not zero) — else a fully-partly-priced `total` would print a bare
`~$x` and under-report.

**Presentation.** The kernel's `summary()` carries the shared body; the REPL
prints focused + one line per local member + `total`; the TUI `/usage` becomes one
**item** ledger (a `total` row + one per asked surface). An unavailable surface
reads `unavailable`, never zero-filled; the total carries a `(N unavailable)`
suffix — `Cost::unpriced_turns`'s honesty, applied to members.

**`AgentEvent` gains `PartialEq`** — required by the brief's protocol roundtrip
test; additive, no behaviour change.

## Mechanism

- `pricing.rs` (new): `ModelPrice`, `model_price`, `cost_of(messages, base_url,
  fallback_model)`, `static OPENCODE_GO` (32 rows). `lib.rs` gains `pub mod
  pricing;`.
- `stats.rs`: `ToolStat`, `Cost`, the amended fold (id-pairing), `cache_hit_pct`,
  `tool_error_pct`, `aggregate`, the extended `summary()`; the module doc's *"no
  cost"* line is **amended** (cost is no longer pure over messages alone).
- `protocol.rs`/`event.rs`: `Request::Usage`, `AgentEvent::Usage { stats }`.
- `actor.rs`: both arms + a shared `usage_reply(&[AgentMessage], &LlmOpts)`;
  `agent.rs`: two `llm()` accessors. The **idle** arm reads `agent.messages()`
  (live ctx) — the turn-boundary snapshot lags the final turn.
- `repl.rs`: `usage_lines`; `app.rs`/`lib.rs`: `Action::AskTo` + an `App`-level
  `usage_collect` (retiring `Surface::usage_pending`/`pending_info`).

## Consequences

- A zen session prints a `~$` that matches `(in − cache_read)·0.15 +
  cache_read·0.003 + out·0.60` per Mtok (live-verified: 11,519 in / 10,240 cached
  / 2 out → `~$0.0002`), an `89% cache` clause, and one line per member.
- **Costs, stated honestly:** tiered models undercount (≤ ~5×); 17 ids print no
  `$`; a socket REPL (`orchestrator == None`) prints the focused line only (the
  TUI covers remote peers via `AskTo`); compaction may make a folded `$` differ
  from a live one (unverified).
- **Foreclosed:** a pricing config file; pricing the cached subset at the full
  input rate; `$0` for an unknown model; per-tool token counts in v1; a
  `SessionInfo` usage push.
- **Follow-ups named, not folded in:** tier support; a shared
  `stats::summary_body()` to end the `strip_usage_prefix` duplication between
  `repl.rs` and `app.rs`; the VS Code surface reads `Request::Usage` for free but
  its panel is a later phase; the pre-existing `markdown` syntax-theme test race
  (`markdown.rs:1533` mutates a process-global, unserialized) is unrelated to
  W024 and tracked separately.
- Gates: `cargo test --workspace` (500 / 439 / 210 … all green),
  `cargo clippy --workspace --all-targets` clean, and a **live** zen run
  (`/usage` → focused line + per-member lines + total).
