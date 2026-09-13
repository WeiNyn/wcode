# Next steps — plan & progress tracker

Follow-ups identified after the compaction work landed, plus pointers to two
larger projects (items 3 and 6). Ordered low-risk → high-risk; tick the boxes as
each task completes and keep the status table current.

| # | item | status |
|---|------|--------|
| 5 | README duplicated line | ☑ done |
| 2 | `bash` output cap | ☑ done |
| 1 | Project instructions (`AGENTS.md`) | ☑ done |
| 4 | Retry / backoff on transient errors | ☑ done |
| 3 | Interface → full-screen TUI (see [`tui-plan.md`](tui-plan.md)) | ☐ todo |
| 6 | Skills & references (see [`skills-references-plan.md`](skills-references-plan.md)) | ☑ done |
| 7 | Parallel tool execution & `.gitignore` awareness (see [`parallel-and-gitignore-plan.md`](parallel-and-gitignore-plan.md)) | ☑ done |
| 8 | Unified interface & protocol — TUI + multi-agent (see [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)) | ◐ S0–S2 landed; S3 TUI P0–P2; S4 (A2A) next |

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
- [ ] (later, with each item) document the new `[instructions]` / `[retry]`
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
- Cleanup: v1 leans on the OS temp reaper. Follow-ups: a startup sweep of
  stale `{temp}/wcode`, or a `ToolContext` scratch-dir field for
  session-scoped lifetime.
- A single pathological no-newline line still buffers one line in memory
  (`read_until`); add a per-line byte cap in a follow-up if it bites.
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
another `wcode-protocol` client, and has landed P0–P2: `wcode-tui` runs a session
through a `Backend` (local or `--socket`), and `wcode` picks it on a TTY
(`--tui`/`--no-tui`) — three bands, streaming, scrollback, `/`-commands, prompt
history, multiline, markdown (tables included), a context bar, resize, `/copy`.
Next: P3 (side panel, diff pane, pickers).

This entry stays as the pointer plus status only, like items 3, 6, and 7.

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
