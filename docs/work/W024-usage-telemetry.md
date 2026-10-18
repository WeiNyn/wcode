# Usage & telemetry: per-member tokens, cache hit rate, tool error rate, and `$` cost — brief

- **Status:** ☑ shipped — kernel `a3526f8` / `933b9b6` / `ec5f789` + front-ends `e050de9` / `54c28dd` / `d2adc4a`; **both phases second-layer APPROVED**; **live-verified** (zen: `usage: 1 turn, 11519 in, 2 out, 10240 cache read, 89% cache, ~$0.0002` + one line per member + `total`); `cargo test --workspace` + clippy clean
- **Work item:** W024
- **Decisions:** [D028](../decisions/D028-usage-telemetry.md) — cost / cache / `result_bytes` / per-member semantics
- **Author:** `agent:brainstormer`
- **Date:** 2026-10-18

---

## 0. Recon — what I verified, and what I did NOT

**Verified (each with an anchor).**

| Fact | Evidence |
|---|---|
| The only telemetry struct, a pure fold over the transcript | `stats.rs:36` `#[derive(Clone, Debug, Default, PartialEq, Eq)]` · `stats.rs:66` `pub fn session_stats(messages: &[AgentMessage]) -> SessionStats {` |
| Explicitly "no timing, no cost" | `stats.rs:34` `/// Pure data — no timing, no cost (there is no timestamp in `AgentMessage`).` |
| Tool tally is a **call** count only, from `ToolCall` blocks | `stats.rs:48` `pub tools: Vec<(String, u64)>,` · `stats.rs:82` `if let ContentBlock::ToolCall { name, .. } = block {` |
| The pairing invariant that makes error counting possible | `stats.rs:17` ``//! the two are 1:1 by `tool_call_id` (the loop synthesizes an error ToolResult`` … the real anchor is `loop_.rs:688` `if let Err(e) = synthesize_unexecuted(&mut cfg, &sink, ctx, rest) {`, `fn synthesize_unexecuted` at `loop_.rs:798`. **The doc's `loop_.rs:683` anchor has drifted** — evidence for this repo's "quote the snippet, the number locates" rule. |
| `ToolResult` (with `is_error`) reaches the transcript | `loop_.rs:669` `let result = AgentMessage::ToolResult {` · `message.rs:58-63` `ToolResult { tool_call_id, name, output, is_error }` |
| Usage is per **turn**, attached to the assistant message | `message.rs:21-27` `pub struct Usage { input_tokens, output_tokens, cache_read_tokens: Option<u64>, cache_write_tokens: Option<u64> }` · `loop_.rs:295` `Some(LlmStreamEvent::Done { stop_reason, usage: u }) => {` · `loop_.rs:379` `let assistant = assistant(&content, stop, usage, &model);` |
| Cache fields come from rig, zero ⇒ `None` | `streamfn.rs:1255` `cache_read_tokens: (usage.cached_input_tokens > 0).then_some(usage.cached_input_tokens),` |
| **On the chat wire, cached input IS a subset of `input_tokens`** | rig `openai/completion/mod.rs:1436` `usage.cached_input_tokens = …self.prompt_tokens_details…map(\|d\| d.cached_tokens…)` (i.e. out of `prompt_tokens` = rig `input_tokens`); rig says so outright at `providers/cohere/completion.rs:136` `// `cached_input_tokens` is a subset of `input_tokens`, so it's only` |
| Tool duration is measured but never tallied, and **not persisted** | `event.rs:101` `duration_ms: Option<u64>,` · `loop_.rs:634` `let duration_ms = started.elapsed().as_millis() as u64;` — it rides `ToolExecutionEnd` only; `ToolOutput` (`tool.rs:23-36`) has no duration |
| `ToolOutput` carries no token count | `tool.rs:35` `pub path: Option<String>,` (fields end there: output/is_error/diff/path) |
| Per-member metadata is a **server-side mirror**, name-shaped, no usage | `protocol.rs:175` `pub struct SessionInfo {` (id/model/effort/context_window at `:195` `pub state: MemberState,`) · `registry.rs:238` `pub fn set_model(&self, id: SessionId, model: impl Into<String>) {` · `server.rs:400` `SessionInfo { id, model, effort, context_window, state }` |
| The local TUI does **not** build its roster from that push | local seeding: `main.rs:1028` `model: info.model.clone().unwrap_or_else(\|\| llm.model.clone()),` (from a one-shot `ListSessions`) vs the socket roster feed at `main.rs:1040-1077` |
| `/usage` is per-focused-surface only | `repl.rs:1191` `Some(Command::Usage) => match backend.ask(Request::GetHistory).await {` · `app.rs:3877` `self.actions.push(Action::Ask(Request::GetHistory));` |
| **`Action::Ask` cannot target a non-focused member** | `lib.rs:568` `// Actions target the focused surface's backend.` · `lib.rs:582` `Action::Ask(request) => spawn_ask(id, backend, &reply_tx, request),` |
| Replies are routed per session on return | `app.rs:3570` `AppEvent::Agent(id, event) => {` · `app.rs:3573` `if let Some(idx) = self.surface_index(&id) {` |
| A price table has a direct in-kernel precedent | `limits.rs:63` `pub fn model_limit(base_url: Option<&str>, model: &str) -> Option<ModelLimit> {` · `limits.rs:119` `static OPENCODE_GO: &[(&str, u64, u64)] = &[` |
| The session knows its own provider (for costing) | `agent.rs:410` `llm: LlmOpts,` inside `SideHandle` · `agent.rs:417` `pub fn history(&self) -> Vec<AgentMessage> {` |
| Mid-run reads have an established seam | `actor.rs:438` `Request::Status => AgentEvent::Status {` · `actor.rs:569-574` `Message::Ask { request: Request::GetHistory, … } => { … reply.send(AgentEvent::History { messages: side.history() }); }` |
| Additive protocol changes do not bump the version | `event.rs:116-119` the `CompactionSkipped` note ("Purely additive on the wire … `PROTOCOL_VERSION` need not bump") |
| **`models.dev` really carries zen pricing** (fetched live: `https://models.dev/api.json`, HTTP 200) | `opencode-go`, 32 models, `cost: { input, output, cache_read, cache_write }` **USD per Mtok**; the human's own model `deepseek-v4.1-flash` = `{"input":0.15,"output":0.6,"cache_read":0.003}`; 4 models are legitimately `0` (`longcat-2.5-preview-free`, `cat-jam-free`, `beam-free`, `step-5-preview-free`); 8 models carry `cache_write`; **6 carry context `tiers`** (`grok-4.7`, `minimax-m3`, `gpt-5.6-luna`, `claude-haiku-5-5`, `gpt-6-luna`, `qwen3.7-plus`) — e.g. `claude-haiku-5-5` doubles+ above `size: 100000` (0.1 → 0.5 in, 0.5 → 2.5 out) |
| The human's live config is zen | `~/.config/wcode/config.toml:3-4` `model = "deepseek-v4.1-flash"` / `base_url = "https://opencode.ai/zen/go/v1"` (key present) |
| **No local endpoint is up right now** | `curl -m 3 http://localhost:11434/v1/models` → `000`, `curl_exit=7` (connection refused) |

**Nots (facts I did not verify).**

1. **The exact model-id coverage of the pricing snapshot** — `models.dev` `opencode-go` has 32 models; `limits.rs`'s `OPENCODE_GO` table lists more (e.g. `glm-5` is in `limits.rs:125` and **absent** from the pricing snapshot). Pricing coverage ≠ limits coverage; the brief assumes the table is filled from models.dev at implementation time and that misses render as "unavailable".
2. **`tiers` semantics** — whether the higher rate applies when a request's context is `>= size` or `> size`, and whether `size` is input or total context. I read the numbers, not the docs.
3. **Whether every zen model's cache_read is cheaper than input** beyond the 4 sampled (`deepseek-v4.1-flash` 0.003 < 0.15, `kimi-k3` 0.3 < 3, `claude-haiku-5-5` 0.01 < 0.1 — all cheaper).
4. **Float→integer price conversion** — I assume the snapshot's floats convert exactly enough in micro-USD; I did not check every entry for more than 4 decimal places.
5. **`Agent::messages()` vs `SideHandle::history()` equivalence after compaction** — I did not trace whether a compaction summary replaces messages that carried usage (which would change a folded total but not a live one).
6. **The VS Code reducer's full usage handling** — only grepped (`editors/vscode/src/reducer.ts:270` reads `event.message.usage` for `contextUsed`).
7. I did not run the TUI or the REPL.

---

## 1. The ask

The human asked, verbatim:

> Improve usage tracking such that:
> - tracking all member usage
> - including input token, output token, cache hit rate if possible
> - tool usage/error rate and token consumption if available
> - total $ cost if available

Today wcode tracks **one** surface's **two** numbers. `SessionStats` (`stats.rs:37`) sums input/output/cache tokens and counts tool *calls* for the *focused* conversation only; `/usage` reads it in the REPL (`repl.rs:1193`) and the TUI (`app.rs:2023`); there is no error rate, no per-member view, and no cost anywhere in the tree. The ask is to make usage a first-class, per-member, cost-aware readout — while respecting wcode's minimalism doctrine (kernel stays presentation-free; no config files for behaviour).

---

## 2. Scope / Non-scope

**In scope.**

- Kernel: extend `SessionStats` with per-tool **error** counts and per-tool **result bytes**; add a **cache hit rate**; add an **aggregate** helper (`aggregate(&[SessionStats]) -> SessionStats`); extend `summary()` with the rate and the cost.
- Kernel: a new `pricing` module (a `limits.rs` sibling) — a static, snapshot price table keyed `(base_url, model)`, `Option`-typed, integer micro-USD.
- Kernel: a **per-session** `Request::Usage` → `AgentEvent::Usage { stats }` read, answered from the actor exactly like `Status` (idle arm + mid-run `side` arm), with the cost folded server-side where `llm.base_url`/`llm.model` are known.
- Front-ends: REPL `/usage` prints the focused line **plus one line per local member**; TUI `/usage` opens the ` usage ` Info modal with a `total` row and one `Item` row per member.
- Docs: README `/usage` row, `tui-design.md` §4 (modal contents), a tracker row, a `D` record.

**Non-scope.**

- **Pushing usage into `SessionInfo`** (the roster/`watch` push). It duplicates the fold in the registry (which has no messages), re-pushes `Sessions` to every client on every `MessageEnd`, and does not serve the primary path: the local TUI seeds surfaces from a one-shot `ListSessions` (`main.rs:1028`) and never subscribes to the roster (`main.rs:1040` is socket-only). Rejected — see §4b Q1.
- **Aggregate tool *timing*.** `duration_ms` is measured (`loop_.rs:634`) but rides `ToolExecutionEnd` only and is never persisted. The TUI already shows per-call timing (`docs/next-steps.md` row 32).
- **Per-tool *token* consumption** as a token metric — not derivable; §4b Q4 recommends the honest substitute (`result_bytes`).
- **A pricing config file** (`[pricing]` in `config.toml`). Doctrine: no config for behaviour (`AGENTS.md` Conventions). A const table mirrors `limits.rs`.
- **The VS Code surface.** It gets the data for free once `Request::Usage` exists, but its panel is a follow-on (`editors/vscode` is out of scope here).
- **A usage trailer on `-p` one-shot.** `RunResult` carries no usage (`loop_.rs:81-85`), and one-shot has no roster.
- **The TUI status-line context gauge** — unchanged; it is a live-ness indicator, not a report.
- **Historical/cross-session usage** (a ledger over saved sessions with dates) — a different feature; `AgentMessage` has no timestamp (`stats.rs:34`).

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | **WS-A · kernel stats** — tool errors + result bytes + cache hit rate + `aggregate` + `summary()` wording | high | low | low | The whole feature's data lives here; blast radius is tiny — only `stats.rs`'s own tests read a field, both front-ends read `summary()`/`last_input_tokens` (`repl.rs:1196`, `app.rs:2026`) |
| P1 | **WS-B · kernel pricing** — `pricing.rs` const table + `model_price()` + integer-micro-USD cost | high | med | med | Biggest design bet: a wrong table silently misreports money; mitigated by `Option` (unknown ≠ $0) and unit tests pinned to models.dev's published numbers |
| P2 | **WS-C · protocol** — serde on `SessionStats`, `Request::Usage`/`AgentEvent::Usage`, actor arms | high | med | med | Unlocks per-member + remote reads; risk is the usual additive-protocol discipline (must not break `Request::Unknown`/`Ack` on older peers) |
| P3 | **WS-D · REPL `/usage`** aggregate lines | med | low | low | A REPL already holds the orchestrator/registry (`repl.rs:727`), so "all members" is a loop and a `println`; no UI machinery |
| P4 | **WS-E · TUI `/usage`** — `Action::AskTo` + a collector + Info rows | high | high | med | The visible payoff; the real cost is that `Action::Ask` is focus-bound (`lib.rs:568`) and the modal is assembled at `App` level (`app.rs:3581`), so this is new plumbing |
| P5 | **WS-F · `member op="usage"`** | med | low | low | Gives the orchestrator model the same read (and works in one-shot/headless where no TUI exists); ~20 lines once P2 lands |
| P6 | **WS-G · docs** — README/tui-design/tracker/D-record | med | low | low | The doctrine is a written record; the D-record locks the cost semantics and the non-scope calls |

---

## 4. Interface & structure

**Added.**

- `crates/wcode-harness/src/pricing.rs` — the price table + lookup, a deliberate twin of `limits.rs`:
  ```rust
  /// USD prices per million tokens, in MICRO-USD (1e-6 USD) — integers so cost
  /// stays `Eq`/serde-stable and never floats.
  pub struct ModelPrice {
      pub input: u64, pub output: u64,
      /// Billed read of a provider-managed cache; a SUBSET of `input` on the
      /// chat-completions wire (rig: `prompt_tokens_details.cached_tokens`).
      pub cache_read: u64,
      /// Billed write; usually 0 (most providers do not report it separately).
      pub cache_write: u64,
  }
  pub fn model_price(base_url: Option<&str>, model: &str) -> Option<ModelPrice>;
  ```
  plus `static OPENCODE_GO: &[(&str, u64, u64, u64, u64)]` (id, input, output, cache_read, cache_write) snapshotted from models.dev `opencode-go`, gated by the **existing** `limits::is_opencode_go` (`limits.rs:57`) so the two tables can never disagree about which provider they describe. `None` = unknown → **omitted**, never `$0`.
- `crates/wcode-harness/src/stats.rs` (same file, new items):
  ```rust
  /// One tool's tally: calls, failures, and how many bytes it fed the context.
  pub struct ToolStat { pub name: String, pub calls: u64, pub errors: u64, pub result_bytes: u64 }
  /// A partial cost: what we could price, and how many turns we could not.
  pub struct Cost { pub micro_usd: u64, pub unpriced_turns: u64 }
  pub fn aggregate(members: &[SessionStats]) -> SessionStats;
  impl SessionStats { pub fn cache_hit_pct(&self) -> Option<u8>; pub fn tool_error_pct(&self) -> Option<u8>; }
  ```

**Changed.**

- `SessionStats` (`stats.rs:37`) — fields:
  - `pub tools: Vec<(String, u64)>` → `pub tools: Vec<ToolStat>` (`stats.rs:48`), keeping the documented sort (count desc, name asc).
  - `+ pub cache_turns: u64` — how many turns reported any cache field (the honest denominator for a rate).
  - `+ pub cost: Option<Cost>` — `None` when nothing could be priced.
  - `+ pub tool_errors: u64` — the headline error sum.
  - Derives gain `Serialize, Deserialize` (`stats.rs:36`) so the type can ride `AgentEvent::Usage`.
- `session_stats` (`stats.rs:66`) — the fold additionally: builds a `tool_call_id → (name, index)` map from `ToolCall` blocks, then walks `ToolResult` messages to bump `errors` and add `output.len()` to `result_bytes`. Contract additions:
  - a `ToolCall` with **no** matching `ToolResult` counts `calls += 1, errors += 0` (a mid-run snapshot can catch this state);
  - `is_error == true` includes the loop's **synthesized** results for blocked/aborted calls (`loop_.rs:688`/`:798`), so "error" means *the call did not succeed*, not *the tool failed* — named in the doc and in §4b Q3.
  - `cache_turns` counts assistant messages with `Some(usage)` and ≥1 cache field `Some`.
- `summary()` (`stats.rs:108`) — appends `, 87% cache` (when `cache_hit_pct()` is `Some`), `, 4 tool errors` (when > 0), and `, ~$0.0123` (when `cost` is `Some` and `unpriced_turns == 0`, else `, ~$0.0087 (3 turns unpriced)`). Existing clauses keep their order and omission rules.
- `protocol.rs` — `Request` (`protocol.rs:230`) gains `Usage,` and `AgentEvent` (`event.rs:61`) gains `Usage { stats: SessionStats }`. Additive ⇒ `PROTOCOL_VERSION` stays 1 (`event.rs:117`).
- `actor.rs` — two new arms, mirroring `Status` exactly: idle arm beside `Request::Status =>` (`actor.rs:438`), mid-run arm beside `Request::GetHistory` (`actor.rs:569`). Both call one shared helper that folds `history()`, then prices it via `pricing::model_price(llm.base_url.as_deref(), &llm.model)` (the `llm` the actor already holds, `agent.rs:410`).
- `app.rs` — `Action` (`app.rs:116`) gains `AskTo(SessionId, Request)`; the run loop's action fan-out (`lib.rs:582`) resolves the backend by id rather than by focus; `App` gains a `/usage` collector (`usage_pending: Option<HashSet<SessionId>>` beside `plan_pending` at `app.rs:1282`, drained in the `AppEvent::Agent` arm at `app.rs:3570`) that assembles the **one** ` usage ` modal via `open_info_rows` (`app.rs:2691`, `InfoRow::Item` at `app.rs:494`) once every member has answered or the ask times out.
- `repl.rs` (`Some(Command::Usage)` at `repl.rs:1191`) — after the focused line, iterate the registry's locals and ask each `Request::Usage`, printing one line per member; teamless output stays **byte-identical** to today.

**Integration points** (verify before coding — a line number moves).

- `stats.rs:37` `pub struct SessionStats {` — the type both front-ends consume; only `stats.rs`'s tests read a field (`repl.rs:1196`/`app.rs:2026` read `last_input_tokens`).
- `stats.rs:17` the 1:1 pairing invariant the error rate rests on; its `loop_.rs:683` anchor has drifted to `:688`.
- `loop_.rs:669` `let result = AgentMessage::ToolResult {` — proves `is_error` and the model-visible `output` reach the transcript.
- `tool.rs:23` `pub struct ToolOutput {` — proves there is no per-tool token field to read.
- `event.rs:101` `duration_ms: Option<u64>,` + `loop_.rs:634` — why aggregate timing is out of scope.
- `agent.rs:410` `llm: LlmOpts,` (inside `SideHandle`) + `agent.rs:417` `pub fn history(&self) -> Vec<AgentMessage> {` — the actor can fold **and** price a mid-run read.
- `actor.rs:438` / `actor.rs:569` — the two arms `Request::Usage` must join.
- `limits.rs:57` `pub fn is_opencode_go(base_url: Option<&str>) -> bool {` + `limits.rs:119` `static OPENCODE_GO: &[(&str, u64, u64)] = &[` — the gating rule and the table shape the pricing module copies.
- `protocol.rs:175` `pub struct SessionInfo {` / `registry.rs:238` / `main.rs:1028` — the *rejected* route's cost (Q1).
- `lib.rs:568` `// Actions target the focused surface's backend.` — why the TUI needs `AskTo`.
- `app.rs:3570` `AppEvent::Agent(id, event) => {` + `app.rs:3581` — replies are already per-session routed; a *multi*-member modal must be assembled at `App` level.
- `app.rs:3877` `self.actions.push(Action::Ask(Request::GetHistory));` — the call site that changes; **two live tests assert its exact value** (`app.rs:5830`, `app.rs:7449`) and must be updated, not deleted.
- `repl.rs:1191-1198` — the REPL formatting site (the context line already lives here, `summary()` deliberately does not — `stats.rs:105-107`).
- `event.rs:117` — the "additive ⇒ no version bump" precedent.

---

## 4b. The six decisions (recommendation + open question)

**Q1 — "All member usage": how? → per-session `Request::Usage` + front-end aggregation.** *(accepted)*
- `SessionInfo` push **rejected** (registry has no messages; re-push per `MessageEnd`; the local TUI never reads it).
- Server-intercepted aggregate **rejected** (no server in the in-process path → `Ack`; an aggregate is not a per-session concept).
- Client-side event folding **rejected as primary** (a mid-session attach has no baseline; compaction invalidates a folded transcript).
- **Chosen:** `Request::Usage` per session (ships folded stats, not the transcript — the `Status` rationale), summed by a kernel `aggregate()`, computed per front-end. Works uniformly for a local handle and a remote socket peer (both go through `Backend::ask`).
- Every surface the front-end holds a backend for; an older remote peer answers `Ack`, treated as "unavailable".

**Q2 — Cache hit rate → `cache_read / input_tokens`, integer percent, `None` when nothing reported.** *(accepted)*
- Cached is a **subset of `input_tokens`** on this wire (verified in rig), so the denominator is `input_tokens` alone; adding `cache_read` back would double-count.
- `Some((cache_read * 100 + input / 2) / input)` when `input > 0 && cache_turns > 0`, else `None`. Rendered `87% cache`.
- `cache_read_tokens: Option<u64>` is `None` when the provider reports nothing (zero is rig's sentinel, `streamfn.rs:1255`) → the clause is **omitted**, never `0%`.
- `cache_write` is not a "hit"; reported separately and priced.
- An Anthropic-wire provider (where `input_tokens` excludes cache read/write) would understate; wcode speaks the OpenAI-compatible wire only — the doc comment must say so.

**Q3 — Tool usage / error rate → pair by `tool_call_id` in the same fold; `ToolStat { calls, errors, result_bytes }`.** *(accepted)*
- First pass maps `tool_call_id → name` from `ToolCall` blocks; then bump `errors`/`result_bytes` on each `ToolResult`.
- Sorting unchanged (calls desc, name asc). Headline `tool_error_pct()` = `Sum(errors) * 100 / Sum(calls)`.
- **Documented caveat:** `is_error` includes hook-blocked calls and the loop's synthesized aborts (`loop_.rs:688`, `:798`) — "errors", not "failures".
- v1 does NOT break out "blocked by a hook" from "the tool failed" (would need a new transcript flag).

**Q4 — Per-tool token consumption → NOT possible; ship `result_bytes` instead.** *(accepted)*
- Usage is per turn (`message.rs:21`, attached once at `loop_.rs:379`); `ToolOutput`/`ToolResult` carry no token counts; the provider bills prompt tokens per request, not per tool. A "delta of consecutive turns' `input_tokens`" would absorb the reply, steering, and compaction.
- Substitute: `result_bytes` = summed `ToolResult.output.len()` — exact, folded from the transcript, answers "which tool is eating my window?".
- Name it `result_bytes` (not a `/4` token estimate — an invented number in a cost report is worse than an absent one).

**Q5 — Total $ cost → a static in-kernel price table, `Option`-typed, integer micro-USD, computed by the actor.** *(accepted; base rate only in v1)*
- **Source:** `models.dev`, provider key `opencode-go` — `cost { input, output, cache_read, cache_write }` per Mtok for 32 models, incl. `deepseek-v4.1-flash` (`0.15 / 0.6 / 0.003`). Same source and `is_opencode_go` gate as `limits.rs`.
- **Where:** `crates/wcode-harness/src/pricing.rs` — a Rust const table, **not** config. Precedent: `limits.rs:119`, incl. its "cite the source in the doc comment" discipline.
- **Shape:** integers in micro-USD per Mtok → `cost = (input*in + output*out + cache_read*read + cache_write*write) / 1e6`, truncated. No floats.
- **Cache accounting:** cached read is a **subset of `input_tokens`**, so price `(input - cache_read) * input_rate + cache_read * read_rate`; a provider that reports no cache field pays the full input rate.
- **Unknown model → omit, never `$0`.** `None` prints nothing; a known-free model is `Some(0)` → `~$0.0000`.
- **Partial coverage:** `Cost { micro_usd, unpriced_turns }` — a mid-run model switch reports `~$x (n turns unpriced)`. Per-message attribution uses `AgentMessage::Assistant { model: Option<String> }` when set, else the session's effective model.
- **Who:** the actor, in the `Request::Usage` arm, from the `llm` it holds (`agent.rs:410`); `session_stats()` stays a pure fold, but its doc "Pure data — no timing, no cost" is **amended** (cost is no longer pure over messages alone).
- **v1 = base rate only**, documented as an undercount (up to ~5× on the 6 tiered models); tier support is a named follow-on. *(human signed off on base-rate-first)*
- `cache_write`-bearing and tiered models are still priced (flat 4th rate / documented base rate).

**Q6 — Presentation → kernel-computes / front-end-styles; two surfaces change.** *(accepted)*
- **Kernel:** `summary()` carries the shared body (extended with `%`, errors, `~$`); `aggregate()` supplies totals. No layout.
- **REPL `/usage`:** focused line + one line per local member when a team exists; teamless byte-identical.
- **TUI `/usage`:** the ` usage ` Info modal gains a `total` row plus one `InfoRow::Item` per member. One modal, not N lines.
- **Unchanged:** TUI status-line gauge, VS Code panel, `-p` one-shot.
- Order: focused first, then members, then `total`.

---

## 5. Plan

1. **WS-A · kernel stats.** `SessionStats` with `tools: Vec<ToolStat>` (+`errors`, `result_bytes`), `tool_errors`, `cache_turns`, `cache_hit_pct()`, `tool_error_pct()`, `aggregate()`, and a `summary()` carrying `% cache`/`tool errors`/`~$`; `session_stats` amended (never weakened) to pair by `tool_call_id`. Gate: `cargo test -p wcode-harness stats` — new §7.1 tests + all eight existing `stats.rs` tests green **except** the two tuple-shaped `tools` comparisons, rewritten to the new shape with assertions intact.
2. **WS-B · kernel pricing.** `pricing.rs` with `ModelPrice`, `model_price()`, the `opencode-go` snapshot (each row's source cited), and `SessionStats::cost_of(price)`. Gate: unit tests pinning `deepseek-v4.1-flash` to `0.15/0.60/0.003`, a free model to `Some(0)`, an unknown id / non-zen `base_url` to `None`; clippy clean.
3. **WS-C · protocol + actor.** `Serialize`/`Deserialize` on the stats types; `Request::Usage` + `AgentEvent::Usage { stats }`; both actor arms sharing one fold+price+reply helper. Gate: roundtrip tests extended (`usage` tag), a mid-run `Usage` test (mirroring `actor.rs:1500`), and an `Ack`-as-unavailable test.
4. **WS-D · REPL.** `/usage` prints the focused line then one line per local member and a `total`; teamless byte-identical. Gate: a pure `usage_lines(...)` formatting unit + a live run.
5. **WS-E · TUI.** `Action::AskTo(SessionId, Request)`, the fan-out resolving by id, and a ` usage ` collector opening the modal when all answer or the ask times out. Gate: updated `a_usage_reply_opens_the_usage_modal`, a new multi-member modal test, the two `/usage`-action tests updated, a dump-frame inspection.
6. **WS-F · `member op="usage"`.** A new op in `crates/wcode-cli/src/tools/member.rs` returning the member's `summary()`. Gate: a tool test + a live `member` call.
7. **WS-G · docs.** README `/usage` row, `tui-design.md` §4, `docs/decisions/D028-usage-telemetry.md` locking Q2/Q4/Q5 semantics, a tracker row. Gate: cited anchors resolve.

Ordering: 1 → 2 → 3 → (4 ∥ 5) → 6 → 7.

---

## 6. Quality gates

- `cargo test --workspace` → all pass (new tests additive; counts rise).
- `cargo clippy --workspace --all-targets` → clean, no warnings.
- Live: with the zen config —
  ```sh
  cargo run -p wcode-cli -- -p "say hi in three words"
  cargo run -p wcode-cli --                    # REPL, then: /usage
  ```
  expect `usage: 1 turn, … in, … out, …% cache, ~$0.0001`.
- Error/retry path: `WCODE_RETRY_MAX=2 cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1 -p hi` → run fails, `/usage` still works (no panic on an unpriced/empty session).
- **Regression tests kept green, by name:** `empty_is_default`, `counts_usage_only_on_reported_turns`, `last_input_tokens_is_most_recent`, `summary_empty_and_full_line`, `summary_caps_tools_and_says_more`, `summary_omits_empty_tools_segment` (`stats.rs`); `a_turn_end_records_context_usage`, `a_history_reply_updates_usage`, `a_history_reply_without_usage_reports_none`, `a_usage_reply_opens_the_usage_modal` (`app.rs`); `agent_event_roundtrip_all_variants`, `agent_event_shape_matches_tagged_payload` (`event.rs`); `request_roundtrip_all_variants`, `request_unknown_tag_is_caught_not_fatal`, `frame_flattens_body_and_omits_absent_reply_to` (`protocol.rs`); the `ask_is_answered_mid_run_*`-class mid-run reads (`actor.rs:1494-1506`, `:1615-1627`). The only assertions this change may *rewrite* are the two tuple-shaped `tools` comparisons in `stats.rs` and the two `/usage`-action payloads in `app.rs:5830`/`:7449`.

---

## 7. Testing

### 7.1 Automated

Kernel (`stats.rs`): `tool_errors_pair_by_call_id`, `a_call_without_a_result_counts_as_a_call_not_an_error`, `a_synthesized_abort_result_is_counted_as_an_error`, `result_bytes_sum_per_tool`, `cache_hit_pct_is_none_without_cache_reporting`, `cache_hit_pct_rounds_to_nearest_percent`, `tool_error_pct_is_none_with_no_calls`, `aggregate_sums_two_members_and_sorts_tools`, `summary_reports_cost_and_unpriced_turns`.

Kernel (`pricing.rs`): `deepseek_v4_1_flash_prices_match_models_dev`, `a_free_model_is_some_zero_not_none`, `unknown_model_or_provider_is_none`, `cost_charges_the_cached_subset_at_the_cache_rate`.

Protocol/actor: `usage_request_roundtrips_with_costs`, `usage_reply_is_answered_mid_run_without_waiting`, `an_ack_from_an_older_peer_reads_as_unavailable`.

Front-ends: `usage_lines_totals_every_local_member` (`repl.rs`), `all_members_usage_opens_one_modal_with_a_total_row` (`app.rs`), `usage_modal_opens_from_an_ask_to_every_surface` (`app.rs`).

### 7.2 How a human verifies it

```sh
cd /home/wei/Work/wcode
cargo build --bin wcode
cargo run -p wcode-cli -- -p "list the files in crates/ in one line"
cargo run -p wcode-cli --                    # REPL; type: /usage
```

Check the money arithmetic. For `deepseek-v4.1-flash` (`$0.15 / $0.60 / $0.003` per Mtok, in/out/cache-read):
`expected ≈ (in − cache_read) * 0.15 + cache_read * 0.003 + out * 0.60`, all ÷ 1e6 in dollars — the printed `~$` must agree to the truncated micro-USD.
Then, with a team, the `/usage` modal shows a row per member plus `total`. Negative: no endpoint — `WCODE_RETRY_MAX=2 cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1 -p hi` then `/usage` → no panic, no `$` on a session that reported no usage.

---

## 8. Expected outcome

- `SessionStats` carries per session: `turns`, input/output tokens, cache read/write tokens, **`cache_hit_pct()`**, **per-tool `calls`/`errors`/`result_bytes`** + a headline error rate, and an **`Option<Cost>`** (micro-USD + `unpriced_turns`).
- The REPL `/usage` prints the focused line, one line per local member, and a `total`; a teamless REPL prints exactly today's line.
- The TUI `/usage` modal shows a `total` row and one row per member (root, workers, reachable remote peers) from `Request::Usage` replies.
- A zen session prints `~$0.00xx` that matches `(in − cache_read)·0.15 + cache_read·0.003 + out·0.60` per Mtok; an unknown model prints **no** `$`.
- A provider that reports no cache fields prints no `% cache` clause (never `0%`); a blocked call is visible in the error count and documented as "not a success".
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` green, `PROTOCOL_VERSION` still `1`, no config key added.

---

## 9. References

- `crates/wcode-harness/src/stats.rs` — the type and fold this change extends; doc `:34`, `:57-64`, `:98-107` is the contract to amend.
- `crates/wcode-harness/src/limits.rs` — the static-table precedent (`:57`, `:117-119`).
- `crates/wcode-harness/src/message.rs` — `Usage` (`:21`), `ToolResult` (`:58`), `Assistant { model }` (`:56`).
- `crates/wcode-harness/src/actor.rs` — the `Status`/`GetHistory` reply pattern (`:435-440`, `:569-583`).
- `crates/wcode-harness/src/event.rs` — additive protocol evolution (`:116-119`).
- `crates/wcode-protocol/src/registry.rs` + `server.rs` + `main.rs:1028/1040` — why the `SessionInfo` route is rejected.
- `crates/wcode-tui/src/app.rs` (`Action` `:116`, `Overlay::Info` `:478`, `InfoRow::Item` `:494`, routing `:3570`) and `lib.rs:568-582`.
- `editors/vscode/src/reducer.ts:270` — the second client's usage handling (out of scope).
- `docs/tui-design.md` §4 — the info-modal spec.
- `docs/next-steps.md` — row 48 (per-member metadata prior art), row 32 (tool timing).
- External: `https://models.dev/api.json` → `opencode-go` (32 models, `cost` per Mtok, `tiers`, `cache_write`, 4 zero-priced).

---

**Ends with a brief.** Recorded by the orchestrator; approved by the human before any code.
