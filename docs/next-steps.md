# Next steps — plan & progress tracker

Follow-ups identified after the compaction work landed, plus pointers to the
larger projects (items 3, 6, 7, 8, 9, 10). Ordered low-risk → high-risk; tick
the boxes as each task completes and keep the status table current.

| # | item | status |
|---|------|--------|
| 5 | README duplicated line | ☑ done |
| 2 | `bash` output cap | ☑ done |
| 1 | Project instructions (`AGENTS.md`) | ☑ done |
| 4 | Retry / backoff on transient errors | ☑ done |
| 3 | Interface → full-screen TUI (see [`tui-plan.md`](tui-plan.md)) | ☑ TUI shipped (P0–P3); P4 stretch open |
| 6 | Skills & references (see [`skills-references-plan.md`](skills-references-plan.md)) | ☑ done |
| 7 | Parallel tool execution & `.gitignore` awareness (see [`parallel-and-gitignore-plan.md`](parallel-and-gitignore-plan.md)) | ☑ done |
| 8 | Unified interface & protocol — TUI + multi-agent (see [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)) | ☑ S0–S2 landed; S3 TUI P0–P3d (+ multiplexed multi-surface); S4 (A2A) complete — S4-1…S4-5 landed; socket multiplexing + live roster, per-agent provider, and remote agent definition (R1/R2) landed |
| 9 | TUI input: wrap & paste (see [`tui-input-plan.md`](tui-input-plan.md)) | ☑ done |
| 10 | Team: define, declare, see (see [`team-and-tui-plan.md`](team-and-tui-plan.md)) | ☑ F1–F3 (+F3b/F3c) + F4a/F4b landed — complete |
| 11 | One-shot `-p` drops fire-and-forget remote deliveries (A2A residual) | ☑ done |
| 12 | TUI polish: tool output, theme, keys (see [`tui-polish-plan.md`](tui-polish-plan.md)) | ☑ done |
| 13 | TUI browse mode: select a transcript block (see [`tui-browse-plan.md`](tui-browse-plan.md)) | ☑ phases 1–3 |
| 14 | TUI palette & theming (see [`tui-theming-plan.md`](tui-theming-plan.md)) | ☑ T1 (palette B) + T2 (color-mode ladder, hex-gated truecolor) + T3 (`[theme]` overlay) landed; tier palettes open |
| 15 | TUI team sidebar: status + live action (see [`tui-sidebar-plan.md`](tui-sidebar-plan.md)) | ☑ S1 + S2 (socket roster model) landed |
| 16 | Session groups: team session persistence & resume (see [`session-groups.md`](session-groups.md)) | ☑ done — 2 commits (`fe95cf8`, `1408550`); second-layer review APPROVED |
| 17 | Session-groups follow-ups (reviewer's non-blocking list) | ☑ done — `3342073`/`b58a6f4`/`ef5e65b`/`5d272c9`; second-layer APPROVED; manual `/reload` sign-off pending |
| 18 | Workspace concurrency: whole-file **digest CAS** (detail: [`workspace-concurrency.md`](workspace-concurrency.md)) | ☑ done — `94069f0`; **claims dropped** (orchestrator owns targeting/scope by prompt); second-layer review APPROVED |
| 19 | Digest-CAS follow-ups (item 18 review debt) | ☑ done — `3886148`; chain test has teeth (reviewer reproduced); second-layer APPROVED |
| 20 | Digest-CAS chain-test residuals (item 19 review debt) | ☑ done — `1b2ca6b` + `28fde3e`; real-seam chain (edit + HooksSet order + plain negative); second-layer APPROVED |
| 21 | Session relaunch UX: TUI `/reload` + a relaunch line on exit | ☑ done — `85911d5`/`2f920f8`/`7c55844`/`0765661`; second-layer APPROVED |
| 22 | Team vs jcode swarm: comparison & candidate features (see [`swarm-comparison-plan.md`](swarm-comparison-plan.md)) | ☑ done — C1 (`d99ab65`) + C2 (`7bd24bb`/`f1338c2`/`ca12f16`) landed; second-layer APPROVED; C3 (DAG) is the north star |
| 23 | LLM stream stall hangs the run — no idle timeout | ☑ done — ttft/idle timeouts in the adapter + a kernel backstop; default on, `0` disables |
| 24 | jcode feature-gap analysis (see [gap-analysis-jcode.md](gap-analysis-jcode.md)) | ☑ review snapshot; Tier-1 sweep complete — see the summary below |
| 25 | Tier-1 gap sweep: Retry-After + socket hardening (see [gap-analysis-jcode.md](gap-analysis-jcode.md)) | ☑ done — b3b3240, 9705f06 |
| 26 | Session search: cross-session + in-session (see [session-search-plan.md](session-search-plan.md)) | ☑ done — `5eb7d78`/`7ef9084`/`0a1bce3`/`4078db1`; second-layer review tightened it |
| 27 | Tier-1 gap sweep 2: `todo` tool + steer at the tool-free boundary (see [gap-analysis-jcode.md](gap-analysis-jcode.md) §2 rows 5/7) | ☑ done — `cc6d5d6`/`59a2cd7`/`18bc454` (todo), `a3a5705` (steer point B) |
| 28 | Plan mode: explore + plan, don't mutate until approved (see [plan-mode.md](plan-mode.md)) | ☑ P1–P3 complete (mode/prompt/hook/bash-gate/`/plan`/chip + todo persistence & seed + `/verify`) |
| 29 | Stream truncation ends a turn silently (no terminal record) | ☑ done — see [stream-truncation-plan.md](stream-truncation-plan.md) |
| 30 | TUI markdown maturity: per-block render cache + parser features (see [`tui-markdown-plan.md`](tui-markdown-plan.md)) | ☐ planned |

Legend: ☑ done · ◐ in progress · ☐ todo.

Suggested commit per item. Reference files by function, not line number
(they drift).

---

## 5. README duplicated line

**Problem.** The env-var paragraph in `README.md` repeats a fragment,
left over from the compaction-config edit:

```
`WCODE_RTK` and `WCODE_GREP`/`WCODE_FIND` (which override `tools.grep`/
`WCODE_RTK` and `WCODE_GREP`/`WCODE_FIND` (which override `tools.grep`/
`tools.find`), and `WCODE_COMPACT_BUDGET`, …
```

**Example.** Reading the "Configure" section, the same sentence starts
twice mid-paragraph.

**Proposed fix.** Delete the duplicated line so it reads once. While
editing, add the entries for items 1–4 as they land (instructions file,
bash cap, retry knobs).

**Tasks**
- [x] Remove the duplicated `WCODE_RTK … tools.grep/` line.
- [x] (later, with each item) document the new `[instructions]` / `[retry]`
      tables and their env vars.

**Open questions.** None.

---

## 2. `bash` output cap

**Problem.** `drain_lines()` in `crates/wcode-cli/src/tools/bash.rs`
accumulates every line into an unbounded `buf`, and emits every line as a
live `ToolExecutionUpdate` on an unbounded channel. `grep`/`find`/`read`
all truncate; `bash` is the outlier. One verbose command floods the
model's context (a single tool result can be megabytes), spiking cost or
producing an outright request failure — and compaction then has to chew
through it.

**Example.** `bash: cat /var/log/big.log` (or `yes`, or a noisy build) →
the whole file rides into the next request; the tail (where errors live)
is drowned by the head.

**Proposed fix.** Spill to a file rather than discard. The full stream is
written to a temp file, and the model gets a bounded preview **plus the
path**, so it can `read` the rest (paging with `offset`/`limit`/`from`) —
nothing is lost, and the context stays small. (`read` resolves absolute
paths, so a `/tmp` file is reachable.)

- Per stream (stdout, stderr independently): buffer in memory while the
  output is small. On crossing `MAX_INLINE_BYTES`, open a temp file, flush
  the buffer, and write everything after there — **created lazily**, so a
  small result never touches disk.
- Return a bounded preview: first `PREVIEW_HEAD_CHARS` + last
  `PREVIEW_TAIL_CHARS` (both ends — errors live at the tail), joined by an
  elision notice carrying the byte count, the stream name, and the path. The
  notice is part of the model-visible `output`.
- **Location**: `{temp_dir}/wcode/bash-<pid>-<n>-<stdout|stderr>.log` —
  absolute; written as UTF-8 (lossy) so `read` (`read_to_string`) can open
  it. `read`'s own 1000-line page cap keeps paging cheap.
- **Live updates**: keep the first `LIVE_LINE_CAP` lines as
  `ToolExecutionUpdate`, then stop (UI flood guard) while still spilling.
- Update the `bash` description so the model knows large output lands in a
  file it can read.

Starting constants (module-level; config knob later if wanted):
`MAX_INLINE_BYTES = 24_000`, `PREVIEW_HEAD_CHARS = 12_000`,
`PREVIEW_TAIL_CHARS = 6_000`, `LIVE_LINE_CAP = 200`.

Output shape (stdout spilled, stderr small):

```
<first 12k of stdout>
… [bash: 15164321 bytes of stdout elided; full output at
   /tmp/wcode/bash-1234-3-stdout.log — use read (offset/limit or from) to
   page it] …
<last 6k of stdout>

[stderr]
build failed

exit code: 1
```

**Tasks**
- [x] Lazy spill: buffer → on `MAX_INLINE_BYTES`, create file, flush, stream on.
- [x] Head+tail preview + elision notice (path, byte count, stream name).
- [x] Per-stream (stdout/stderr) independent files + notices.
- [x] Live-update line cap.
- [x] Update the `bash` description; note the spill behavior.
- [x] Tests: over-cap output spills (notice carries a path; the file holds
      the full output; inline text is bounded); small output stays inline
      (no file); stderr spills independently; live updates capped.

**Open questions.**
- Cleanup: both follow-ups landed — `85ee290` (a startup sweep of stale
  `{temp}/wcode` spills, age-gated and name-filtered) and `e4d6587` (a
  per-line byte cap so a pathological no-newline stream chunks instead of
  buffering unboundedly). The `ToolContext` scratch-dir alternative stays
  unmade (session-scoped lifetime; fields in `tool.rs`, constructed per
  call at `loop_.rs:465`).
- A spill path noted in an early turn only survives as long as the temp
  file (fine within a session).

---

## 1. Project instructions (`AGENTS.md`)

**Problem.** The system prompt is a fixed `const` string
(`system_prompt()` in `crates/wcode-cli/src/repl.rs`); nothing reads a
project instruction file. Users must retype build/test/convention guidance
every session, and the model can't learn "this repo forbids `anyhow`",
"run clippy before committing", etc. Every peer (pi's `AGENTS.md`, jcode's
Memory) loads it. This is prompt *content*, not behavior config, so it
sits outside the "no behavior config" stance.

**Example.** In a repo root:

```markdown
# AGENTS.md (in the repo)
Build: cargo build --workspace
Test:  cargo test --workspace; clippy must be clean.
Never add `anyhow`. Prefer anchored `edit` over `replace`.
```

Today the agent never sees this. With the fix, the system prompt carries
it and the model follows the conventions without being told again.

**Proposed fix.**
- **Discovery**: from the working dir, walk up to the repo root (stop at
  the first `.git` boundary or filesystem root) and load the nearest
  `AGENTS.md`. Explicit override wins over discovery.
- **Injection**: `system_prompt(tools, instructions: Option<&str>)` appends
  a clearly framed block after the base prompt:
  `\n\n# Project instructions (<path>)\n<contents>`.
- **Size cap** (e.g. 32 KiB) with a truncation marker — a giant file is
  counterproductive.
- **Banner**: print `instructions: <path>` at startup (like `session:`).
- **Config / env / flag** (mirror the existing precedence: flag > env >
  file > default):
  - `[instructions] file = "AGENTS.md"` (relative to cwd; `"off"`/empty
    disables).
  - `WCODE_INSTRUCTIONS` (path, or `off`).
  - `--no-instructions` flag (force off; optionally `--instructions PATH`).
- Loaded once at startup; `/new` keeps it (same cwd); `/reload` re-execs,
  so it re-reads.

**Tasks**
- [x] `load_instructions(cwd, spec) -> Option<Instructions>` +
      size cap (pure, unit-tested: nearest wins, `.git` boundary, missing
      file, `off`).
- [x] `system_prompt(tools, instructions)` + test (block appended when
      `Some`, absent when `None`).
- [x] `build_agent` threads the loaded contents; banner line.
- [x] Config `[instructions]` + `WCODE_INSTRUCTIONS` + `--no-instructions`.
- [x] README.

**Open questions.**
- Walk-up boundary: `.git` root only, or all the way to `/`?
- Single nearest file, or a chain (nested `AGENTS.md` concatenated)?
  Start single; note chain as a follow-up.
- Also honor `CLAUDE.md`? Start `AGENTS.md` only; config makes it swappable.

---

## 4. Retry / backoff on transient errors

**Problem.** `adapt()` in `crates/wcode-harness/src/streamfn.rs` awaits
`model.stream(request)` exactly once. Any error becomes a single
`LlmStreamEvent::Error`, which ends the run with `StopReason::Error`. A
transient `429`/`503` or a dropped connection kills an in-progress task.

**Example.** The provider returns `429 Too Many Requests` (rate limit) or
a transient `503`; the turn aborts and the user must re-prompt from
scratch. A bounded retry with backoff would ride it out silently.

**Proposed fix.**
- Retry **only the pre-first-event phase** (`model.stream(req).await` —
  the connect + response handshake). Once events are flowing, a
  mid-stream error is *not* retried (it would duplicate partial text);
  surface it as today.
- **Classification** (rig 0.42):
  - retry on `CompletionError::HttpError` (transport: connect/timeout/
    reset), and
  - retry on provider status ∈ {408, 409, 429, 500, 502, 503, 504} via
    rig's `provider_response_status()` (fallback: string match);
  - fatal (no retry): 400/401/403/404/422, `JsonError`, `UrlError`,
    `RequestError`, `ResponseError`.
- **Backoff**: exponential + jitter — `base * 2^(n-1)`, capped
  (e.g. base 500 ms, cap 8 s), `max_retries = 3`. Honor `Retry-After`
  when present (nice-to-have).
- **Rebuild** the stream future each attempt from a cloned
  `CompletionRequest` (restructure `adapt` so the "make a future" step is
  a closure).
- **Cancellation**: the backoff sleep must stay cancellable —
  `tokio::select! { _ = tx.closed() => return, _ = sleep => {} }`, so
  Ctrl-C / stream-drop still aborts promptly.
- **Config**: `[retry] max = 3, base_ms = 500, cap_ms = 8000`
  (+ `WCODE_RETRY_*`); `max = 0` disables.
- **Observability**: emit a retry notice so the UI can show
  `retrying (2/3) after 503 …` dimmed.

**Tasks**
- [x] Extract a generic `with_retry(policy, classify, op)` helper
      (rig-free; unit-tested with synthetic errors: succeeds after N
      transient failures, gives up after `max`, ignores fatal).
- [x] Restructure `adapt` to rebuild the future per attempt; wire the
      helper around the connect step.
- [x] Classifier from rig's `provider_response_status()` + `HttpError`.
- [x] Cancellable backoff.
- [x] Config `[retry]` + `WCODE_RETRY_*` (+ `to_policy`-style fold).
- [x] Retry notice event → dim REPL line.
- [x] README.

**Result.** `RetryPolicy` + `retryable`/`is_transient_status`/`backoff` +
`connect_with_retry` in `streamfn.rs`, wired into `adapt`; config in the
`[retry]` table (+ `WCODE_RETRY_*`); `LlmStreamEvent::Retrying` /
`AgentEvent::Retrying` → a dim REPL line. Two findings during the build:

- **Retry must cover the first streamed item, not just the connect.** rig's
  OpenAI path defers the HTTP request *into* the stream, so a connection
  failure surfaces on the first poll — confirmed live (the error arrived from
  the stream phase, not `stream().await`). `connect_with_retry` peeks the first
  item and retries it; after any content is forwarded, mid-stream errors pass
  through unchanged.
- **Transport failures arrive as `ProviderError(String)`, not `HttpError`** on
  the OpenAI path, so the classifier also matches connect/DNS/timeout wording
  (`is_transient_message`) when no status is preserved.

**Open questions.**
- Is `provider_response_status()` publicly reachable from our rig version
  (trait import path)? Fallback to string matching if not.
- Surface `Retry-After` from `InvalidStatusCodeWithDetails` headers now,
  or later?

---

## 3. Interface redesign → full-screen TUI

**Reframed.** This was scoped as a line editor (rustyline / hand-rolled). The
actual goal is larger: a **complete TUI** — full-screen transcript, streaming
render, input box, status line, scrollback — consuming the same `AgentEvent`
stream the REPL prints today.

That is its own project, so the detail moved out of this list:

> ➡️ **[`docs/tui-plan.md`](tui-plan.md)** — problem, architecture, crate
> layout, terminal handling, phased tasks, open questions, progress tracker.

This entry stays as the pointer plus status only.

---

## 6. Skills & references

**New.** Two related capabilities: **skills** (`SKILL.md` packages whose
name/description go into the system prompt, the body loaded on demand via
`read`) and **references** (context files generalized from a single `AGENTS.md`
to a global + ancestor set, plus the `references/` convention inside a skill).

Agents/subagents are **not** part of this — they wait on an inter-agent
communication protocol.

> ➡️ **[`docs/skills-references-plan.md`](skills-references-plan.md)** — scope,
> pi/jcode reference, design, phased tasks, open questions, progress.

**Status.** All three phases shipped — references as a set (`instructions.rs`),
the skills core (`skills.rs`), and the phase-3 polish (`/skills`,
`/skill <name>`). Verified end-to-end against a live model at each stage. The
optional `<root>/<name>.md` form was declined (see the plan's §7).

This entry stays as the pointer plus status only, like item 3.

---

## 7. Parallel tool execution & `.gitignore` awareness

**New.** Two independent harness gaps found by comparing against pi and jcode:
our tool loop runs calls **sequentially** (pi defaults to parallel, with per-tool
opt-in), and `grep`/`find` ignore files by a **hardcoded** list rather than the
project's `.gitignore` (both references respect it).

> ➡️ **[`docs/parallel-and-gitignore-plan.md`](parallel-and-gitignore-plan.md)** —
> scope, pi/jcode reference, design, phased tasks, open questions, progress.

This entry stays as the pointer plus status only, like items 3 and 6.

---

## 8. Unified interface & protocol

**New.** The TUI's client/kernel seam and multi-agent communication are the
same seam, so they should be one protocol: a session is a peer with a mailbox —
a `Request` in, an `AgentEvent` out — and the TUI, a script, and another session
differ only in the address they hold. This reframes the TUI rework (item 3) and
the deferred inter-agent protocol as one workstream.

> ➡️ **[`docs/interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)** —
> thesis, jcode reference, the `Request`/`Frame` shape, a staged roadmap
> (S0 types → S1 actor → S1b read-back + repoint → S2 transport → S3 TUI → S4 A2A),
> open questions, decisions.

**Status.** S0 (`protocol.rs`: `Request`, `Frame`, `SessionId`) and the S1 actor
core (`actor.rs`: `SessionActor`/`SessionHandle`, inbox + broadcast outbox) have
landed, plus the reply path (`GetHistory` + `ask`) and the CLI repoint: the REPL
and `-p` now drive a `SessionHandle`. S2 adds the `wcode-protocol` crate (NDJSON
frames, a socket server, a remote `Client`, a local-or-remote `Backend`) and wires
`wcode serve` + `wcode --socket` (one-shot and interactive). The TUI (S3) is
another `wcode-protocol` client, and has landed P0–P3d: `wcode-tui` runs a session
through a `Backend` (local or `--socket`), and `wcode` picks it on a TTY
(`--tui`/`--no-tui`) — three bands, streaming, scrollback, `/`-commands, prompt
history, multiline, markdown (tables included), a context bar, resize, `/copy`,
then overlays (the model, `/changes`, and `/resume` pickers), tool-diff
rendering, a per-run changeset, and a session picker that hands off by re-exec.
S4 (agent-to-agent) is complete (S4-1…S4-5); the socket layer multiplexes several surfaces over one connection and serves a **live** roster — a runtime-spawned worker reaches a `--socket` client via the pushed roster (`6f3ef23`, `c3eb261`); a worker may run on its own provider (`9c02e96`), and a root may define a worker on a served peer (`Request::Define` → `AgentEvent::Spawned` — `49b227e`, `7627b9e`). Remaining: the optional S5 (task-DAG) only; one transport residual is tracked separately as item 11 (one-shot `-p` can drop fire-and-forget remote deliveries).

This entry stays as the pointer plus status only, like items 3, 6, and 7.

---

## 9. TUI input: wrap & paste

**New.** Two composer defects: the input box does not **wrap** (a logical line
wider than the band is truncated; more than six lines hides the cursor), and a
**paste** is inserted verbatim, flooding the box. Fix the first with wrapping +
a scrolled, height-capped band; the second with a paste **placeholder** — a big
paste collapses to a `❰ pasted N lines · M chars ❱` chip that expands on submit.

> ➡️ **[`tui-input-plan.md`](tui-input-plan.md)** — problem, design (Phase 1 wrap
> · Phase 2 paste-as-placeholder), locked decisions, phased tasks, open
> questions, progress.

This entry stays as the pointer plus status only, like items 3, 6, 7, and 8.

---

## 10. Team: define, declare, see

**New.** The A2A/team capability (item 8, S4) is complete at the protocol level but
remains **manual** (the team forms only when the model decides to `spawn`),
**uncustomizable** (`WorkerSpec` carries only a name, so every worker inherits the
parent wholesale), and **invisible** (the TUI holds one session and knows nothing
of peers or workers). Four features share one spine: a TUI **command table +
completion** (F1), **customizable spawned agents** — `model`/`role`/`tools`
(F2), a **`[team]` preset** that forms the team at startup (F3), and a **team
status sidebar + in-process multi-surface** (F4). F2 is the hinge: F3 depends on
it, and F4 shows its results; F1 is the orthogonal command surface the others
extend.

> ➡️ **[`team-and-tui-plan.md`](team-and-tui-plan.md)** — problem, the dependency
> spine, per-feature design, locked decisions, phased tasks, open questions,
> progress tracker.

**Status.** Complete — all phases landed (F1 → F2 → F3 → F4, including F4b-1/F4b-2),
and tier-2 socket multiplexing has landed (`6f3ef23`, `c3eb261`): a `serve` holds a
**live** roster and pushes growth, so a runtime-spawned worker reaches a `--socket`
client. The two follow-ups this item tracked have also landed: the per-agent provider
(a worker on its own `base_url`/key — `9c02e96`) and remote agent definition (a
served peer answers `Request::Define` with `AgentEvent::Spawned`, and `spawn { to }`
targets one — `49b227e`, `7627b9e`). Nothing remains in this item. Design and progress
tracker: [`team-and-tui-plan.md`](team-and-tui-plan.md).

This entry stays as the pointer plus status only, like items 3, 6, 7, 8, and 9.

---

## 11. One-shot mode drops fire-and-forget remote deliveries

**Problem.** `Registry::deliver` queues and returns — it does **not** await the
flush. `deliver` → `Backend::send_from` (documented "Fire-and-forget") →
`Backend::Remote` → `Client::send_from` (`crates/wcode-protocol/src/client.rs`),
which pushes onto an mpsc that a background supervisor task drains. In one-shot
`-p` mode, `one_shot` (`crates/wcode-cli/src/main.rs`) returns after `AgentEnd`
and `main` calls `std::process::exit(...)`, dropping the runtime with the
outbound queue possibly unwritten.

**Impact.** `wcode --socket … -p "message { to: <remote>, … }"` (the `message`
tool, `crates/wcode-cli/src/tools/message.rs`) and R2's remote `spawn { to }`
(its `Wake`, `crates/wcode-cli/src/tools/spawn.rs`) can be lost. The interactive
REPL is unaffected. `Define` itself is safe — it uses `ask`, which round-trips.

**Fixed.** `Client` gained a flush **barrier**: the outbound queue now carries an
`Out` enum (`Frame` | `Flush(oneshot::Sender<()>)`), and the supervisor completes
the oneshot the moment it reaches it — the queue is FIFO and frames are written in
order, so every earlier frame is already written. `Client::flush(timeout)` enqueues
the barrier and awaits it, **bounded** (a dead or reconnecting peer simply times
out). `wcode_protocol::flush_all(timeout)` flushes **every** live connection the
process holds concurrently — weak handles registered in `Client::adopt`, and the
`Client` supervisor holds only a `Weak`, so a dropped client's handle dies and is
pruned on the next `register`/`flush_all`. Both one-shot `-p` exit sites
(`crates/wcode-cli/src/main.rs`) call it before `std::process::exit`. The hot path
stays fire-and-forget.

**Tasks**
- [x] Flush the client's outbound queue before `main` exits `-p` (barrier +
      `flush_all`), bounded so an unreachable peer never hangs the exit.

**Open questions.** None — the reviewer classified this as a pre-existing
transport property, not an R2 regression. Nothing is left open for the one-shot
`-p` path: the barrier is best-effort by design, and the REPL keeps its connection
open. (The TUI also exits via `process::exit` on `/quit`, so a quit mid-run could
still drop an in-flight remote frame — out of scope here.)

---

## 12. TUI polish: tool output, theme, keys

**New.** Three presentation gaps in the shipped TUI: a finished tool collapses to
one line (its output is unreadable), colors are hardcoded across eight ad-hoc
style fns with no `Theme`, and the keymap is undiscoverable (no help overlay, no
word/line editing in the composer).

> ➡️ **[`tui-polish-plan.md`](tui-polish-plan.md)** — problem, design
> (expand/collapse, a named-color theme, the keys), commit list, status.

This entry stays as the pointer plus status only, like items 3, 6, 7, 8, and 9.

---

## 13. TUI browse mode

**New.** The transcript has no cursor: every shortcut is position-blind
(`Ctrl-T` acts on *every* tool, `Ctrl-Y` on "the last reply"), so a
long scrollback cannot be *pointed at*. A browse mode moves a `▌` selection over
the committed blocks, in the gutter the design already reserved for it, and
unlocks block-wise actions (expand, copy, a `$PAGER`) one phase at a time. (Phase
2 retired the old `Ctrl-O` last-tool shortcut in favour of pointing at a block;
phase 3 added `Home`/`End`, block-wise `{`/`}`, and `/` search.)

> ➡️ **[`tui-browse-plan.md`](tui-browse-plan.md)** — problem, design (a mode,
> a per-surface selection, the second-pass bar, scroll-to-reveal), phases, status.

**Status.** Phases 1–3 landed (`f3b045a` — selection + actions; phase 3 added
`Home`/`End` viewport-anchored, block-wise `{`/`}`, and `/` search with `n`/`N`)
plus the routing hardening (`13e3977` Alt-delivered braces, `dcc8605`
paste-to-search). Phase 4 ($PAGER viewer + structured jumps) is parked: the
`$PAGER` needs a suspend/resume seam `terminal.rs` lacks (`TerminalGuard` only
restores on drop) — its own design.

This entry stays as the pointer plus status only, like items 3, 6, 7, 8, 9, and 10.

---

## 14. TUI palette & theming

**New.** The palette names 16 roles but several share a look — `muted`==`border`
(both `DarkGray`), `warn`==`code` (both `Yellow`), and `tool_name`/`link` share one
`Blue` fg — so distinct components are hard to tell apart (the sidebar's `done` row
matches the pane border). Adopt
palette **B** (semantic families: meta=magenta, tools=blue+bold, code=yellow,
warn=light-yellow), then add capability detection (P2) and an opt-in truecolor
`[theme]` overlay (P4) behind a `Plain/Named/Indexed/Rgb` ladder.

> ➡️ **[`tui-theming-plan.md`](tui-theming-plan.md)** — problem, the B role
table, the theming tiers, phases, non-goals.

**Status.** T1 (palette B), T3 (`[theme]` overlay), and T2 (`e602c77` — the
`Plain/Named/Indexed/Rgb` color-mode ladder; hex `#rrggbb` overrides honored
only under Rgb, else degrade to the role default) have landed. The tier
palettes themselves (Basic 8-color / 256 / truecolor) remain open — land only
alongside a tier that needs them.

This entry stays as the pointer plus status only, like items 3, 6, 7, 8, 9, 10, and 13.

---

## 17. Session-groups follow-ups (item 16 review debt)

**Done** (except the manual sign-off). Tracker item 16 (session groups) shipped
and passed second-layer review with a reviewer's non-blocking list; four of five
landed (`3342073`, `b58a6f4`, `ef5e65b`, `5d272c9`) and passed second-layer review:

- [x] `group_dir_of` unit test (dir + `<dir>/root.jsonl` + bare-file `None`) — the
      6b linchpin, previously verified only live and by call-site reading.
- [x] Document the phantom-member window: a crash between member-file creation
      and the manifest write resumes a header-only member as "Restored, 0
      messages" (by design, differs from the corrupt→Skipped story).
- [x] Fix the `Manifest.group` doc comment that claimed it is "checked against
      the dir name" — it is informational only (scan is truth); validation on
      open was **not** added.
- [ ] One real `/reload` in a group session (full exec re-invocation) — **manual
      final sign-off**, not automatable.
- [x] Drop the unused `WorkerSpec` serde derive (kept `Clone, Default`).

## 19. Digest-CAS follow-ups (item 18 review debt)

**Done.** Tracker item 18 (whole-file digest CAS) shipped (`94069f0`) and passed
second-layer review with a reviewer's non-blocking list; all three landed in
`3886148` and passed second-layer review:

- [x] Automated integration test wiring the *real* `read` output →
      `WorkspaceHooks::after_tool_call` → `transform_tool_input` → a *real*
      mutator `execute` — the one link covered only by unit seams + the live
      check.
- [x] Strengthen `build_agent_leaves_the_shared_hook_set_untouched` with
      `assert!(!agent.hooks().is_empty())` — asserts the push actually happened,
      not just that the shared input set was left unmutated.
- [x] Mention in the `read` tool description that `--plain` does not arm the
      digest cache (a model that reads plain before writing gets no protection).

## 20. Digest-CAS chain-test residuals (item 19 review debt)

**Done.** Item 19 (`3886148`) passed second-layer review; the three non-blocking
test-coverage extensions landed (`1b2ca6b`, `28fde3e`) and passed second-layer
review:

- [x] A chain step through `edit` (not just `write`) — it also exercises the
      anchor-resolution path *after* the CAS check.
- [x] Drive the chain through the real `HooksSet` (both hooks composed, in
      insertion order, as the loop invokes them).
- [x] A negative test that a `--plain` read output parses to `None` and arms
      nothing (pairs with the description note).

Optional follow-ons (not tracked as an item): the `edit` chain derives its
`from` anchor via `anchor::anchor(..)` rather than parsing it out of the read
output; `edits`/`replace` have unit tests but no full chain.

## 21. Session relaunch UX

**Done.** Two gaps found while signing off §17.4:

- **`/reload` was REPL-only.** The TUI's `COMMANDS` table (`app.rs`) now carries
  `/reload [--no-session]`; it mirrors the REPL (rebuild + re-exec into the
  current session) via a new `Outcome::Reload`, refused over a socket.
- **No relaunch command on exit.** Closing (REPL `/exit`, EOF, idle Ctrl-C; TUI
  `/exit`) now prints the exact relaunch command built from `reload_args` — flat
  `wcode --resume <file.jsonl> …`, team
  `wcode --resume <groupdir>/root.jsonl --agents …`.

**Tasks**
- [x] TUI: `/reload [--no-session]` in the command table, mirroring the REPL
      (rebuild + re-exec into the current session); refused over a socket.
- [x] Print the exact relaunch command on exit (REPL + TUI): flat and team/group.
- [x] README + `team-and-tui-plan.md` note.

Commits `85911d5` (TUI `/reload`), `2f920f8` (relaunch line), `7c55844` (Ctrl-C
parity), `0765661` (docs); second-layer APPROVED.

**Out of scope (stated, not papered over):** a terminal **window-close
(`SIGTERM`/`SIGHUP`)** has no handler, so it exits without printing the line.

## 15. TUI team sidebar: status + live action

**New.** The sidebar's `name · model · state` row wastes the model column (uniform
across members, and *wrong* over `--socket`) and shows nothing about what a teammate
is doing. Replace it with a status glyph (`● ○ ✓`) + name + a dim **live action**,
dropping the model from the sidebar (revising D28); the action comes from
`ToolExecutionStart`, its target resolved UI-side from the assistant tool-call
arguments. A separate fix (S2) plumbs the member model through the socket roster
(landed).

> ➡️ **[`tui-sidebar-plan.md`](tui-sidebar-plan.md)** — problem, the row design, the
action data path, S1/S2, non-goals.

This entry stays as the pointer plus status only, like items 3, 6, 7, 8, and 13.

---

## 22. Team vs jcode swarm — comparison & candidates

**Revived.** The parked comparison of wcode's agent-team against jcode's swarm
(brainstorm §10/§10.1, roadmap S5, decision #11) is re-opened against **as-built**
code on both sides: wcode's shipped star (F1–F4) vs jcode's DAG-first model. The
capability table, gap analysis, and candidate features live in
[`swarm-comparison-plan.md`](swarm-comparison-plan.md).

**Recommendation.** Take the minimal subset: **C1 — typed report** now (improves
the shipped star with no new subsystem); **C2 — shared task list** if the team
needs explicit task tracking. Treat **C3 — the task DAG + scheduler (S5)** as the
documented north star, built only if coverage-first exploration becomes a goal
(then with C4's gates). Do **not** import jcode's channels / shared-context KV /
worktree managers / coordinator entity / ACLs.

**Tasks**
- [x] C1 — typed report: the report shape is in the worker blurb; the
auto-forward is tested. `cli:` `d99ab65`.
- [x] C2 — shared task list: root-owned list + `task` tool + TUI `/tasks`.
      `cli:` `7bd24bb`/`ca12f16`, `tui:` `f1338c2`.
- [ ] C3 — task DAG + scheduler (decision gate; needs its own plan doc).
- [ ] C4 — verify gate as a `Hooks` policy (rides on C2/C3).
- [ ] C5 — lifecycle footer (partial: `failed` shipped, `5911eab`; `blocked`/`waiting-on-detail` deferred).

---

## 23. LLM stream stall hangs the run (no idle timeout)

**Problem.** The stream is consumed with **no stall/idle timeout** at any layer.
The kernel turn loop selects only on `cancel` and `stream.next()` — the
`tokio::select!` at `crates/wcode-harness/src/loop_.rs:206` has exactly two arms,
`cfg.cancel.cancelled()` and `item = stream.next()`. The rig adapter is the same:
`crates/wcode-harness/src/streamfn.rs:239` (the `connect_with_retry` first-item
peek) and `streamfn.rs:241` (the main forward loop) select only on `tx.closed()`
and `stream.next()`. So if a provider accepts the connection but then goes
silent — no deltas, no error, no EOF (a half-open TCP / idle upstream / a proxy
that keeps the socket open) — `stream.next()` never resolves and the whole turn
awaits forever. This is a common provider failure mode.

**Impact.** The agent hangs indefinitely mid-turn: the UI shows a spinner and
never finishes, and one-shot `-p` never returns. Only a manual cancel (Ctrl-C →
`cfg.cancel`) unwedges it.

**Example.** A provider (or an overloaded local endpoint) stalls after a `200`
with headers: the run spins on `stream.next()` with no event, no error, no
timeout, until the user gives up and cancels.

**Proposed fix.**
- Add an **idle deadline** around `stream.next()` at both layers: a shorter
  **time-to-first-token** deadline before the first event, and a longer
  **inter-token** deadline after (each received event resets it). On expiry,
  surface a *transient* stream error so the existing recovery path handles it —
  the retry/backoff in `streamfn.rs` for the pre-content case, or the fed-back
  stream error in `loop_.rs` once content has flowed.
- Keep it **cancellable**: the deadline is just another `select!` arm beside
  `cancel`/`tx.closed()` — no new cancellation path.
- Cover the `connect_with_retry` peek (`streamfn.rs:239`) too, so a stall
  *before* the first token also retries rather than hanging.
- **Config**: `[retry]` (or a new `[timeout]` table) gains `ttft_ms` / `idle_ms`
  (+ `WCODE_*`); sane defaults on, `0` disables.
- **Observability**: emit a stall notice on expiry, reusing the existing
  `LlmStreamEvent::Retrying` / `AgentEvent::Retrying` → a dim line.

**Tasks**
- [x] Idle / time-to-first-token timeout around `stream.next()` in `loop_.rs`.
- [x] The same in the adapter (`streamfn.rs`: the peek and the forward loop).
- [x] Config + env; default on; `0` disables.
- [x] Stall notice → dim line (reuse the retry notice).
- [x] Tests: a stream that hangs after N events trips a time-out (short injected
      duration); a clean stream is unaffected.

**Open questions.**
- One deadline on the raw stream item, or also a keepalive/heartbeat probe?
- Default values (e.g. ttft 60 s, idle 120 s) — provider-dependent; make it a knob.

---

## 24. jcode feature-gap analysis

**Summary.** A feature diff of wcode against `../jcode` (~670k LOC, 80+ crates) —
what is worth *adopting*, filtered by wcode's doctrine (no MCP, no permission
prompts, no behavior config; policy lives in `Hooks`). Full analysis:
[`gap-analysis-jcode.md`](gap-analysis-jcode.md) (companion to
[`swarm-comparison-plan.md`](swarm-comparison-plan.md), the already-tracked
multi-agent slice). Status: **review snapshot** — read-only, no build/run.

**Tier 1 (small, on-doctrine) — the recommended first sweep.**
- Honor `Retry-After` on 429/503 (the retry path is blind backoff today).
- Socket `0600` + an NDJSON frame cap (`socket::bind` sets no mode;
  `read_frame` is unbounded).
- Cross-session `session_search` (sessions are already JSONL; only listing
  exists).
- Within-session `conversation_search` (compaction drops the summarized prefix
  with no retrieval path).
- A `todo` tool (no model-facing checklist; `task` is root/team-scoped).
- Widen the context catalog — `limits::model_limit` keys off one provider only.
- Soft-interrupt point B — a steer during a tool-free turn is lost.
- Dedicated reasoning/usage `AgentEvent`s — *cosmetic only* (both are already
  surfaced).

**Second wave (Tier 2).** The most defensible — now **shipped** as
`harness::hooks::BashRiskHooks`, an always-on `Hooks` policy (code, not config)
for the catastrophic-only subset — was a **`bash` command-risk gate**, the one
jcode feature that fits wcode's doctrine. Also: native Anthropic/Gemini
provider, markdown maturity, `webfetch`, skill authoring, plan card, live theme
reload, a narrow side panel, tool backgrounding.

**Non-goals (do NOT build).** Permission prompts/approval/HITL,
OAuth/credential store/provider picker, MCP/browser/computer-use/sandbox, the
embedding memory graph + sideagents, config-driven/spawn/scheduled hooks,
semantic compaction — all excluded by doctrine or by the missing-daemon
constraint.

---

## 29. Stream truncation ends a turn silently (no terminal record)

**Problem.** A turn ended when the stream yielded `None`, not when it yielded a
terminal record (`Final` → `Done`). A provider SSE body that ends early — a
proxy idle-close / half-close / EOF, no `[DONE]`/`finish_reason` — makes rig
emit neither a `Final` nor an `Err`, so `forward_stream`'s `None` arm `break`
treated the truncation as a clean stop: no `Done`, no `Error`, and the kernel
defaulted the missing `Done` to `StopReason::Stop`. A partial thinking-only
assistant, or a silent empty turn, went unnoticed.

**Fix.** Two layers. The adapter tracks `done_seen` (set on a mapped `Final`)
and turns the `None` arm into the real terminal condition: pre-content it
retries on the shared budget; post-content it surfaces a non-fatal `Error`. The
kernel adds a backstop guard so a custom `StreamFn` that ends with no terminal
record is fed back (bounded by `DEFAULT_MAX_STREAM_ERROR_TURNS`) instead of
silently stopping. Detail & status: [`stream-truncation-plan.md`](stream-truncation-plan.md)
(☑ done).

**Out of scope (follow-up).** A `Done` with empty **non-tool** content is a
completed turn, not a truncation — left as-is.

## 30. TUI markdown maturity

**New.** The transcript re-parses every committed block every frame
(`ui.rs::draw_transcript` → `markdown::render`), and the parser lacks ordered
lists, blockquotes, nested indent, heading levels, italic, and syntax
highlighting. Phase **1a** adds a per-block render cache keyed by
`(revision, width)` — the design [`tui-design.md:147`](tui-design.md) already
names — and phase **1b** matures the parser (+ `syntect`). Mermaid stays a gated
stretch (TUI P4); the side panel is a separate later layer.

> ➡️ **[`tui-markdown-plan.md`](tui-markdown-plan.md)** — problem, the cache
> design (revision/width + the mutable-`Tool`-block crux), the parser features,
> phases, open questions.

**Status.** Planned — 1a (cache) then 1b (features). Pointer only, like items 3,
6, 7, 8, 9, 10, and 13.

---

## Tier-1 gap sweep — status

The jcode feature-gap analysis ([`gap-analysis-jcode.md`](gap-analysis-jcode.md))
produced eight Tier-1 candidates. Their disposition:

| gap (gap-analysis §2) | status |
|---|---|
| 1. Honor `Retry-After` on 429/503 | ☑ done — `b3b3240` |
| 2. Socket `0600` + bounded frame read | ☑ done — `9705f06` |
| 3. Cross-session `session_search` | ☑ done — `5eb7d78`/`7ef9084`/`0a1bce3`/`4078db1` (one tool; `scope` covers rows 3+4) |
| 4. In-session `conversation_search` | ☑ done — same tool; `scope:"current"` recovers turns a compaction hid |
| 5. `todo` tool | ☑ done — `cc6d5d6`/`59a2cd7`/`18bc454` |
| 6. Widen the per-model context catalog | ☐ deferred — `limits::model_limit` still returns a window only for `is_opencode_go` |
| 7. Soft-interrupt point B | ☑ done — `a3a5705` |
| 8. Reasoning/usage events | — dropped: cosmetic (`AgentEvent::MessageUpdate` already streams thinking; usage rides `MessageEnd`) |

**6 of 8 done, #8 dropped, #6 deferred.** Follow-ups landed alongside:
`70a14cd`/`7bed04e` (gap-doc corrections), `4078db1` (session-search tighten),
`25749bd` (todo resume wart). The session-search design lives in
[`session-search-plan.md`](session-search-plan.md). Tier-2 (medium) and Tier-3
(larger) candidates are in `gap-analysis-jcode.md` §3–§4; none started.

---

## Sequencing

1. **5** README (minutes) — clear the deck.
2. **2** bash cap — self-contained tool fix; protects the context that
   compaction manages.
3. **1** AGENTS.md — self-contained CLI + config feature; highest value.
4. **4** retry — kernel robustness, no deps.
5. **3** interface → TUI — its own project; see [`tui-plan.md`](tui-plan.md).
6. **6** skills & references — prompt-surface feature; see
   [`skills-references-plan.md`](skills-references-plan.md).
7. **7** parallel exec & `.gitignore` — harness/tool gaps; see
   [`parallel-and-gitignore-plan.md`](parallel-and-gitignore-plan.md). Phase 2
   (ignore) is the smaller independent win; Phase 1 (parallel) touches the loop.
8. **8** interface & protocol — subsumes item 3 (the TUI is one client of it);
   see [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md).
9. **9** TUI input: wrap & paste — see [`tui-input-plan.md`](tui-input-plan.md).
10. **10** Team: define, declare, see — see
    [`team-and-tui-plan.md`](team-and-tui-plan.md).
11. **11** one-shot remote-delivery flush — a small transport fix that also
    hardens the `message` tool.
12. **12** TUI polish: tool output, theme, keys — presentation-only; see
12. **12** TUI polish: tool output, theme, keys — presentation-only; see
12. **12** TUI polish: tool output, theme, keys — presentation-only; see
    [`tui-polish-plan.md`](tui-polish-plan.md).
13. **13** TUI browse mode — select a block, then act on it; see
    [`tui-browse-plan.md`](tui-browse-plan.md).
14. **14** TUI palette & theming — presentation-only; see
    [`tui-theming-plan.md`](tui-theming-plan.md).
15. **15** TUI team sidebar: status + live action — presentation-only (+ a socket fix);
15. **15** TUI team sidebar: status + live action — presentation-only (+ a socket fix);
    see [`tui-sidebar-plan.md`](tui-sidebar-plan.md).
16. **16** session groups — team persistence/resume; see [`session-groups.md`](session-groups.md). Done.
17. **17** session-groups follow-ups. Done (`3342073`…`5d272c9`); manual `/reload` sign-off pending.
18. **18** workspace concurrency — whole-file digest CAS. Done (`94069f0`).
19. **19** digest-CAS follow-ups. Done (`3886148`).
20. **20** digest-CAS chain-test residuals. Done (`1b2ca6b`, `28fde3e`).
21. **21** session relaunch UX — TUI `/reload` + relaunch line on exit. Done (`85911d5`…`0765661`).
22. **22** team vs jcode swarm — comparison revived; see [`swarm-comparison-plan.md`](swarm-comparison-plan.md).
23. **23** LLM stream stall hangs the run — no idle timeout (see §23).
