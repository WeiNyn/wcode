# wcode vs jcode — feature gap analysis

Status: **review snapshot** (docs only); produced by reading both trees, no build or live run. Companion to [`next-steps.md`](next-steps.md) §24 and [`swarm-comparison-plan.md`](swarm-comparison-plan.md) (the multi-agent slice — already tracked).

## 1. Method
This is a *feature* diff, not a line diff: `../jcode` is ~670k LOC over 80+ crates (measured: 672,565 lines across 82 workspace members); wcode is ~39k over 4 (measured: 38,722). The question is not "what differs" but "what is worth **adopting**": real on jcode's side, **not** excluded by wcode's doctrine (`AGENTS.md`: no MCP, no permission prompts, no behavior config — policy lives in code via `Hooks`), and cheap for a small codebase. Evidence: wcode cited by symbol; jcode by crate/symbol. Nothing was verified by building or running either binary.

## 2. Worth implementing — Tier 1 (small, on-doctrine)
| # | gap | why | size |
|---|-----|-----|------|
| 1 | **Honor `Retry-After`** on 429/503 | wcode's retry path retries on blind backoff and ignores the header (`streamfn.rs::backoff`, `is_transient_status` — no `Retry-After` anywhere in the tree). Isolated change. | S |
| 2 | **Socket `0600` + frame cap** | `socket::bind` sets no mode (follows umask) and the NDJSON reader (`frame::read_frame`, `read_line` into an unbounded `String`) is unbounded: any local user may connect, and a large frame grows memory. | S |
| 3 | **Cross-session `session_search`** | sessions are already full JSONL under `~/.local/share/wcode/sessions` (`repl::session_dir`, `Session::append`); only listing exists (`repl::list_sessions`). jcode's is keyword + Bloom index, no embeddings. | S |
| 4 | **Within-session `conversation_search`** | after compaction `compaction::compact_ctx` summarizes and drops the summarized prefix from the model's view with no retrieval path; same shape as #3 over `Agent::session_path`. | S |
| 5 | **`todo` tool** | no model-facing checklist; `task` is root/team-scoped (`tools/task.rs` "Root-only"). jcode's is a plain `{content,status}` list (skip its goal/gate machinery). | S |
| 6 | **Widen the per-model context catalog** | `limits::model_limit` returns real windows only when `is_opencode_go` holds; every other endpoint falls back to `DEFAULT_CONTEXT_WINDOW = 128_000`. The table already exists (`OPENCODE_GO`) — key it off the model, not one provider. | S |
| 7 | **Soft-interrupt point B** | `Agent::steer`/`follow_up` drain at turn start (`loop_.rs` steering drain) and after the inner loop (`follow_ups.try_recv`), so a steer arriving during a tool-free turn is not injected. Inject before the `calls.is_empty()` break; tag a `source`. | S–M |
| 8 | **Dedicated reasoning + usage events** | `AgentEvent` has no *dedicated* reasoning-delta or token-usage variant — but there is **no functional gap**: thinking already rides `AgentEvent::MessageUpdate` as `ContentBlock::Thinking` (rendered live by `ui.rs::content_lines` / `theme.thinking`) and usage rides `AgentMessage::Assistant.usage` on `MessageEnd`/`TurnEnd` (`app.rs::record_usage`, surfaced by `/usage`). A separate event would only tidy the seam. | S (cosmetic) |

**Correction to the draft.** Row 8 was originally scoped as "reasoning + usage: the TUI can't show thinking deltas or live cost". Reading the tree, both are *already* surfaced (see the row); the true residual is cosmetic. Treat #8 as optional, not a gap.

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

## 7. Incidental findings
- **Security parity gaps** (Tier 1 #2): socket mode + frame bound, and no `0600` on session files (`Session::append` opens without a mode; jcode's storage forces owner-only via `set_permissions_owner_only`).
- **Stale doc**: `docs/tui-sidebar-plan.md` claims "S1/S2 landed" and cites `ui.rs:draw_sidebar`, but the sidebar was replaced by `draw_team_strip` (`ui.rs`); no `draw_sidebar`/`member_rows` remain.

## 8. Recommendation
Do the **Tier 1 sweep** first — eight items, each S (one S–M), none crossing a subsystem boundary or the kernel/CLI split (though #8 is cosmetic and can be skipped). If a second wave is wanted, **`bash` command-risk as a `Hooks` policy** is the most defensible behavioral improvement.

## 9. References
- wcode: `crates/wcode-harness/src/{streamfn,limits,compaction,event,loop_,hooks}.rs`, `crates/wcode-protocol/src/{socket,frame}.rs`, `crates/wcode-cli/src/{repl,instructions}.rs`, `crates/wcode-cli/src/tools/mod.rs`, `docs/swarm-comparison-plan.md`, `AGENTS.md`.
- jcode: `crates/jcode-command-risk/`, `crates/jcode-provider-*`, `crates/jcode-side-panel-types/`, `crates/jcode-tui-mermaid/`, `crates/jcode-import-core/`, `docs/{SOFT_INTERRUPT,HOOKS,SAFETY_SYSTEM,MEMORY_ARCHITECTURE}.md`.
