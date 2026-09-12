# Next steps — plan & progress tracker

Five follow-ups identified after the compaction work landed. Ordered
low-risk → high-risk; tick the boxes as each task completes and keep the
status table current.

| # | item | status |
|---|------|--------|
| 5 | README duplicated line | ☑ done |
| 2 | `bash` output cap | ☑ done |
| 1 | Project instructions (`AGENTS.md`) | ☑ done |
| 4 | Retry / backoff on transient errors | ☑ done |
| 3 | REPL line editing (history, completion, multiline) | ☐ todo |

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

## 3. REPL line editing (history, completion, multiline)

**Problem.** `repl.rs` reads input with
`tokio::io::BufReader::new(tokio::io::stdin()).lines()` — no up-arrow
history, no tab completion, no multiline. Editing is limited to the
terminal's cooked-mode line discipline. A pasted multi-line block is split
into separate prompts; a multi-line prompt submits on the first `Enter`.

**Example.**
- Re-running the previous prompt needs a full retype (no history).
- Pasting a 5-line snippet to discuss fires 5 independent turns.
- Typing a 3-line prompt executes the first line before the rest is typed.

**Proposed fix.** Introduce a real line editor, **branched on
`std::io::stdin().is_terminal()`**: TTY → editor, non-TTY (pipes, `-p`,
tests) → the current `BufReader` loop unchanged. The non-TTY branch is
non-negotiable — CI and piping depend on it.

Options considered:
- **(a) `rustyline`** — mature, history file, completion, hints,
  multiline via validator. Sync API → run each prompt in
  `tokio::task::spawn_blocking`, editor behind `Arc<Mutex<_>>`. *Recommended*:
  smallest well-scoped dep with the features we want.
- **(b) `reedline`** — richer (menus, hilite); heavier.
- **(c) hand-rolled raw mode** — zero deps, fits "minimal", but ~200+
  lines of ANSI/termios edge cases (wcode already touches `libc` for
  process groups, so not alien).

Behaviors:
- **History**: persist to `~/.local/share/wcode/history` (beside
  sessions); dedup consecutive, bounded length.
- **Completion**: start with `/`-command completion at line start; later
  `/model <tab>` (model ids) and `/resume <tab>` (session files).
- **Multiline**: trailing `\` continues the line (cheap, predictable);
  rely on rustyline bracketed-paste for pasted blocks.
- **Ctrl-C / Ctrl-D**: map `ReadlineError::Interrupted` / `Eof` to
  today's semantics — in-flight → cancel; idle → exit.
- Output during a run happens *after* the line is submitted, so prompt
  clobbering isn't a concern.

**Tasks**
- [ ] Add the dep + `is_terminal()` branch; keep the piped path covered.
- [ ] History file + load/save; dedup + cap.
- [ ] `/`-command completion.
- [ ] Backslash multiline.
- [ ] Ctrl-C / Ctrl-D semantics preserved.
- [ ] Manual TTY smoke + README note.

**Open questions.**
- rustyline vs hand-rolled — the one real dependency decision; confirm
  before starting.
- History file location/name and retention.
- Extend completion to model ids / session paths now or later?

---

## Sequencing

1. **5** README (minutes) — clear the deck.
2. **2** bash cap — self-contained tool fix; protects the context that
   compaction manages.
3. **1** AGENTS.md — self-contained CLI + config feature; highest value.
4. **4** retry — kernel robustness, no deps.
5. **3** line editor — new dependency and interactive surface; do last,
   confirm the rustyline-vs-hand-rolled call first.
