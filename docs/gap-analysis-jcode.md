# wcode vs jcode — feature gap analysis

Status: **review snapshot** (docs only); produced by reading both trees, no build or live run. Companion to [`next-steps.md`](next-steps.md) §24 and [`swarm-comparison-plan.md`](swarm-comparison-plan.md) (the multi-agent slice — already tracked).

## 1. Method
This is a *feature* diff, not a line diff: `../jcode` is ~670k LOC over 80+ crates (measured: 672,565 lines across 82 workspace members); wcode is ~39k over 4 (measured: 38,722). The question is not "what differs" but "what is worth **adopting**": real on jcode's side, **not** excluded by wcode's doctrine (`AGENTS.md`: no MCP, no permission prompts, no behavior config — policy lives in code via `Hooks`), and cheap for a small codebase. Evidence: wcode cited by symbol; jcode by crate/symbol. Nothing was verified by building or running either binary.

## 2. Worth implementing — Tier 1 (small, on-doctrine)
| # | gap | why | size |
|---|-----|-----|------|
| 1 | **Honor `Retry-After`** on 429/503 | wcode's retry path retried on blind backoff and ignored the header (`streamfn.rs::backoff`, `is_transient_status`). **Done** (`b3b3240`): `retry_after` reads the header and clamps it to `policy.cap`. | S |
| 2 | **Socket `0600` + frame cap** | `socket::bind` set no mode (followed umask) and the NDJSON reader (`frame::read_frame`, `read_line` into an unbounded `String`) was unbounded. **Done** (`9705f06`): `bind` chmods `0600` (fails on error); `read_frame` caps at `MAX_FRAME_BYTES` (16 MiB). | S |
| 3 | **Cross-session `session_search`** | sessions are already full JSONL under `~/.local/share/wcode/sessions` (`repl::session_dir`, `Session::append`); only listing existed (`repl::list_sessions`). **Done** — shipped as ONE tool, `session_search` with `scope:"all"` (`7ef9084`, tightened in `4078db1`); it walks groups + legacy files, grouping members under their root. jcode's is keyword + Bloom index, no embeddings. | S |
| 4 | **Within-session `conversation_search`** | after compaction `compaction::compact_ctx` summarizes and drops the summarized prefix from the model's view with no retrieval path. **Done** — folded into the SAME tool as #3: `session_search` with `scope:"current"` reads `Session::entries()` (every `Message`, pre-compaction included) via `ToolContext.session_path` (`5eb7d78`/`7ef9084`), and `SUMMARY_PROMPT` now points the model at it (`0a1bce3`). | S |
| 5 | **`todo` tool** | no model-facing checklist; `task` is root/team-scoped (`tools/task.rs` "Root-only"). **Done** — shipped as `todo` (`59a2cd7`; the `AgentEvent::Todo` event in `cc6d5d6`, the TUI notice in `18bc454`): a session-local, event-sourced checklist every session gets. jcode's is a plain `{content,status}` list (its goal/gate machinery was skipped). | S |
| 6 | **Widen the per-model context catalog** | `limits::model_limit` returns real windows only when `is_opencode_go` holds; every other endpoint falls back to `DEFAULT_CONTEXT_WINDOW = 128_000`. The table already exists (`OPENCODE_GO`) — key it off the model, not one provider. **Deferred** — needs a models.dev-style catalog to be worth the maintenance. | S |
| 7 | **Soft-interrupt point B** | `Agent::steer`/`follow_up` drained at turn start and after the inner loop, so a steer arriving during a tool-free turn was not injected. **Done** (`a3a5705`): the outer tail drains a follow-up **or** a steer via `next_injected` (follow-up first), injecting it and starting the next turn. | S–M |
| 8 | **Dedicated reasoning + usage events** | **Dropped — cosmetic, already covered.** `AgentEvent` has no *dedicated* reasoning-delta or usage variant, but thinking already rides `MessageUpdate` (`ContentBlock::Thinking`, rendered live by `ui.rs::content_lines`) and usage rides `MessageEnd`/`TurnEnd` (`app.rs::record_usage`, `/usage`). | — |

**Correction to the draft.** Row 8 was originally scoped as "reasoning + usage: the TUI can't show thinking deltas or live cost". Reading the tree, both are *already* surfaced (see the row); the residual is cosmetic. **Dropped** — not worth a dedicated event.

## 3. Tier 2 (medium, net-positive)
- **Native Anthropic (+Gemini) provider** (S–M) — `rig` is already a dep at `streamfn` (workspace `rig = "0.42"`) and `rig-core` 0.42 ships `providers::anthropic` and `providers::gemini` as ungated modules behind the same `CompletionClient`/`CompletionModel` traits `open_stream()` uses; plumbing a client enum is small. (Bedrock: N — not in rig-core.)
- **`bash` command-risk gate** (M) — `bash` is entirely ungated. Build the **catastrophic-only** subset as a `Hooks` policy (`before_tool_call` → reason), i.e. code, not config — this is the rare jcode feature that fits wcode's doctrine. jcode's crate exists *because* a user lost a home dir (its issue #604; `jcode-command-risk`).
- **Markdown maturity** (M) — the hand-rolled parser (`crates/wcode-tui/src/markdown.rs`) has no syntax highlight, no ordered lists/blockquotes/nested indent, and re-parses the whole transcript every frame. Add `syntect` + a couple of block kinds + a per-block cache; do not migrate to cmark.
- **`webfetch`** (S) — one GET; closes the URL gap. (`websearch` needs a backend: M, lower value.)
- **Skill authoring (`skill_manage`)** (S–M) — wcode already discovers `SKILL.md`; a write path is coherent.
- **Plan card + `/plan`** (S) — one-shot plan-only turn + a bordered `plan` fenced block; skip the *mode* (that belongs in `Hooks`).
- **Live theme reload** (S) — theme is `OnceLock`-frozen after first draw (`theme::THEME`, install-once); an `RwLock` swap enables runtime recolor. Do not port jcode's literal-remap machinery.
- **Side panel (narrow)** (M) — no way to pin a file/diff beside the chat; reuse the inline diff renderer as a UI-only pane. jcode has `jcode-side-panel-types`. (Persisted pages: L.)
- **Tool backgrounding** (M) — let a long `bash` move to background and keep the turn going; needs a bg registry + wait/status tool.

## 4. Tier 3 (larger / only if wanted)
- **Mermaid/diagram rendering** (M–L) — wcode cannot render a diagram at all; a halfblock-only, feature-gated slice is the sane one (jcode's `jcode-tui-mermaid` is heavy). (Documented as TUI P4.)
- **Info widgets** (M) — dock aux info into transcript *negative space*; one widget, not jcode's 15-kind registry (`info_widget_*` × 15).
- **Task DAG (C3)** — already wcode's documented north star (`swarm-comparison-plan.md`), not re-proposed here.

## 5. Deliberate non-goals (do NOT build)
Where jcode is heavy and wcode is right: **permission prompts / approval queue / HITL UI** (stated doctrine; the `Hooks` seam replaces it); **OAuth / login flows / credential store / provider picker / full provider-doctor**; **MCP / `integration_tools` / browser / computer-use / ACP / Windows pipes / remote handoff / sandbox**; **embedding memory graph + extraction/consolidation sideagents + ambient mode** (ONNX + a ~90 MB model + storage/decay + background agents — the `Hooks::transform_context` seam already covers a user-built variant); **external config-driven hooks / spawn hooks / scheduled wakeup** (need a long-lived daemon wcode lacks; policy belongs in compiled `Hooks`); **semantic/proactive compaction** (wcode's absolute-budget policy is arguably more portable).

## 6. Already tracked — multi-agent
wcode's team (star: `spawn`/`message`/`peers`/`task`) is already compared against jcode's swarm in `swarm-comparison-plan.md`; C1 (typed report) + C2 (task list) landed, C3 (task DAG) is the north star. Not a new finding.

### Settled — the `todo`/`task` injection question

wcode's `todo` (Tier-1 #5, `tools/todo.rs`) and `task` (C2, `tools/task.rs`) are **tool-driven with zero injection**: the list enters the model's context only when the model calls the tool (an append-only tool result), and `AgentEvent::Todo` is a UI-only event. Nothing splices it into the prompt — there is no `transform_context` impl, and `system_prompt()` (`repl.rs`) is composed once at startup and never rebuilt per turn. jcode goes further: besides the tool result, a **client-side turn-end "gate"** synthesizes a **user-role message** from the persisted todo state and pushes it onto `queued_messages` as the next turn's prompt (`jcode-tui/src/tui/app/input.rs::deliver_deferred_gate_digest_if_needed` → `build_gate_digest`; the ownership / confidence / final-response reminders and the `auto_poke` nudge). That is a behavior loop, not a data-path requirement, and it is **deliberately not imported** — it is append-only (so it buys no cache win) and it belongs to the agent-quality scaffolding §5 excludes. Consequence worth remembering: in wcode the todo/task state can never break the prompt prefix cache, because it is never spliced into the prompt, only appended through tool calls.

## 7. Incidental findings
- **Security parity gaps** (Tier 1 #2): socket mode + frame bound, and no `0600` on session files (`Session::append` opens without a mode; jcode's storage forces owner-only via `set_permissions_owner_only`).
- **Stale doc**: `docs/tui-sidebar-plan.md` claims "S1/S2 landed" and cites `ui.rs:draw_sidebar` — that symbol is gone (the sidebar was replaced by `draw_team_strip`, `ui.rs`). Its data source survives: `app.rs::member_rows` (pub fn) now feeds `draw_team_strip` (`ui.rs` calls `app.member_rows()`) and backs `/team` (`app.rs`). Only the `draw_sidebar` citation is stale.

## 8. Recommendation
Do the **Tier 1 sweep** first — eight items, each S (one S–M), none crossing a subsystem boundary or the kernel/CLI split (though #8 is cosmetic and can be skipped). If a second wave is wanted, **`bash` command-risk as a `Hooks` policy** is the most defensible behavioral improvement.

## 9. References
- wcode: `crates/wcode-harness/src/{streamfn,limits,compaction,event,loop_,hooks}.rs`, `crates/wcode-protocol/src/{socket,frame}.rs`, `crates/wcode-cli/src/{repl,instructions}.rs`, `crates/wcode-cli/src/tools/mod.rs`, `docs/swarm-comparison-plan.md`, `AGENTS.md`.
- jcode: `crates/jcode-command-risk/`, `crates/jcode-provider-*`, `crates/jcode-side-panel-types/`, `crates/jcode-tui-mermaid/`, `crates/jcode-import-core/`, `docs/{SOFT_INTERRUPT,HOOKS,SAFETY_SYSTEM,MEMORY_ARCHITECTURE}.md`.
