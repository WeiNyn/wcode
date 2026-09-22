# wcode plan mode — design & phases

Status: **design locked; P1–P2 shipped, P3 not started.** A project, not a task — the detail
lives here so it can be picked up as its own workstream. Companion row in
[`next-steps.md`](next-steps.md).

Plan mode makes the agent **explore and plan/brainstorm, and NOT mutate** until
the user approves. On approval the plan **comes with a todo list**; the user can
then **verify it finished**.

## 1. Problem

Today wcode has one mode: act. The agent will `edit`/`write`/`bash` as soon as it
thinks a change is warranted, and the user reviews *after the fact*. There is no
cheap way to say "look, discuss, propose — but change nothing yet." Borrowing the
shape from other agents' "plan mode": a runtime toggle that (a) tells the model to
plan, not mutate, via the prompt, and (b) *enforces* that by blocking mutating
tools, so an over-eager model cannot jump the gun. The plan it produces is recorded
as the existing `todo` checklist, which the user approves by toggling the mode off.

Two invariants drive the whole design:

- **The mode is code, not config.** A `Hooks` impl + a prompt section, flipped at
  runtime. No `[plan]` table, no approval-flow machinery, no sandbox.
- **The kernel stays presentation-free.** The hook lives in `wcode-harness`; the
  indicator and `/plan` command are front-end.

## 2. Design

### 2.1 The mode and its toggle (D1)

A boolean, on/off, toggled by `/plan` (bare = toggle; `/plan on|off` = set). No
config key. The flag lives in a shared `Arc<AtomicBool>` wrapped by a
`PlanModeHandle` (sketched in `hooks.rs`), so the toggle flips it live without
rebuilding the agent.

**Toggle plumbing (flagged — reviewer settles).** Both front-ends are `Backend`
clients (`repl.rs:673` `Backend::from(handle)`; the TUI speaks the same seam), so
neither can flip the hook's Arc directly. The recommended path is a new
`Request::SetPlanMode { on: bool }` whose actor arm (`actor.rs:327` `dispatch`)
both flips the shared handle and swaps the system prompt (§2.2). The REPL *could*
flip an Arc it holds in-process, but that breaks over a socket — so route it
through the request for uniformity. The actor needs the same `PlanModeHandle`;
thread it in alongside the agent (a new field on the actor/config). **This is the
one piece of new plumbing the design needs** beyond a hook and a prompt.

### 2.2 The prompt carries the behavior (D2)

**The kernel owns the section.** `PLAN_SECTION` lives in `wcode-harness`
(`agent.rs`) — not in a front-end: a socket client never composes the server's
prompt, so `SetPlanMode { on }` alone must suffice. The `Agent` tracks its
`base_system` (the composed prompt as passed) and exposes
`Agent::set_plan_mode(&mut self, on: bool)`, which flips the shared
`PlanModeHandle` and recomposes
`self.system = if on { format!("{base_system}\n\n{PLAN_SECTION}") } else { base_system }`.
A rebuild (`build_agent`) is rejected: it drops the live `ctx`, the open
`Session`, and the cancel token. (The sketched `Agent::set_system` is subsumed by
`set_plan_mode`.) Takes effect on the next run. The handle is threaded in via
`AgentConfig::plan_mode`, shared with the `PlanModeHooks`.

`PLAN_SECTION` (P1 text + the P2 plan→todo sentence):

```
# Plan mode
You are planning, not executing. Explore and propose only — do NOT modify the
workspace: the editing tools are disabled and mutating shell commands are refused.
Fine-tune the plan with the user first. When the plan is final, record it as a
todo list (status pending), then tell the user to run `/plan off` to execute.
```

### 2.3 Enforcement is a `Hooks` impl (D3)

The tool set is fixed at construction (`default_tools`, `tools/mod.rs:36`), so we
cannot add/remove tools per mode (recon C5/C6). Instead `PlanModeHooks`
(`hooks.rs:24` `before_tool_call`) returns `Some(reason)` for a **denylist of
workspace-mutating tools**; the loop blocks the call and feeds
`blocked: {reason}` back to the model (`loop_.rs:461`).

- **Blocked:** `edit`, `edits`, `write`, `replace`, `ast_edit`.
- **Allowed:** `read`, `grep`, `find`, `ast_search`, `session_search`, and —
  crucially — **`todo`** (it records the plan; it does not touch the workspace),
  plus `task`/`peers`/`message` (team coordination, not workspace mutation).

A denylist (not an allowlist) is deliberate: a new read-only tool is allowed by
default; only a new *mutator* must remember to join the list. The cost is that a
future mutator silently slips through plan mode. To keep the flag and the
denylist one source of truth, a mutator opts in via **`TypedTool::mutating()`**
(default `false`; forwarded through `Tool`/`erased()`), and a test asserts every
`mutating()` tool in `default_tools` is in `MUTATING_TOOLS`.

### 2.4 The bash read-gate (D4)

`bash` is a mutator by nature, but the agent needs it to explore (`git status`,
`ls`, `cat`, test/typecheck *reads*). The same `before_tool_call` inspects
`ToolCall { name: "bash", arguments }` and blocks when the command looks mutating.
Raw extraction mirrors `rtk.rs:112-118`
(`call.arguments.get("command").and_then(|c| c.as_str())`); `BashArgs { command,
timeout_secs }` (`bash.rs:13`). The rule is a **string scan** for a mutating verb /
redirection — **a guardrail, not a sandbox.**

| Input | Verdict | Why |
|---|---|---|
| `ls -la`, `cat x`, `git status`, `rg foo` | allow | reads |
| `cargo build`, `cargo test`, `cargo check`, `cargo run`, `make`, `tsc --noEmit` | allow | read-only builds (artifacts are not a workspace mutation) |
| `rm -rf build`, `mv a b`, `sed -i`, `tee f`, `>f` | block | mutate |
| `git commit -m x`, `git checkout -- .`, `git push` | block | mutate history/remote |
| `cargo fix/fmt/install/add/remove`, `npm/pnpm/yarn install/ci/add/remove/update`, `go mod tidy`, `go get`, `pip install` | block | mutate the source tree / lockfiles |
| `echo hi > /tmp/x` | **false positive** | writes outside the workspace (blocked anyway — acceptable) |
| `python -c 'open("f","w")'` | **false negative** | arbitrary mutator hidden in an interpreter |
| `./build.sh` | **false negative** | side effects unknown |

The FP/FN table is the honest boundary: bash can always escape a string scan, so
plan mode's *guarantee* is "the model must actively disguise a mutation to bypass
it", not "mutation is impossible". The reason string tells the model the same.

### 2.5 Halting at the plan (D5)

No hook. The loop ends naturally when the model stops calling tools (the run ends
on the tool-free turn). Plan mode = prompt + block-hook; the user reads the plan,
toggles `/plan off`, and the next turn executes. **Do not** add a
`should_stop_after_turn` for this — it would truncate the model's own reasoning
about when it is done.

### 2.6 Plan → todo (D6)

A prompt instruction, not plumbing. `PLAN_SECTION` (`agent.rs`, §2.2) now ends
"When the plan is final, record it as a todo list (status pending), then tell the
user to run `/plan off` to execute." The `todo` tool already exists and is allowed
(§2.3); a write emits `AgentEvent::Todo { todos }` (`event.rs`; `TodoItem`,
`TodoStatus`) on the run's sink, so the plan shows up in the transcript for free.
No new plumbing — the sentence rides the same prompt section P1 added.

### 2.7 Todo persistence (D7)

Plan → todo makes the checklist load-bearing, so the in-memory list (`todo.rs`) is
persisted:

- **`SessionEntry::Todo { id, todos }`** (`session.rs`), appended by the `todo`
  tool on **every write** (Q4; mirror `ModelChange`), with a last-wins
  `Session::todo()` accessor (mirror `model()`/`effort()`). `#[serde(other)]
  Unknown` keeps old files forward-compatible; the wire tag is `todo`.
- **Seed on first access:** `default_tools` has no session handle (recon E8), so
  the tool seeds from `ToolContext.session_path` on its first `execute` — a bare
  READ reflects the restored list. An explicit `seeded: AtomicBool` guards it, so
  a legitimately empty/cleared list is not re-read every call.
- **Two writers, one file:** the tool opens its OWN `Session::open(path)` to
  append, so the agent's in-memory `entries` never sees the `Todo` entry. Both
  writers open `O_APPEND` and write one full JSON line per call, and a turn is
  serialized around its tool calls, so appends cannot interleave — the
  `SessionEntry::Todo` route is safe. (A sidecar would avoid the second writer,
  but D7 chose the entry: one transcript, replayed by `/verify` in P3.)

### 2.8 Verify if we finished (D8)

"When the plan is done, prove it." Three candidate shapes:

- **(a) `/verify` command** (REPL + TUI): prints the persisted checklist with
  incomplete items flagged, plus the run's changed files (TUI already tracks a
  per-run changeset) and the final assistant text. **No hook, no loop risk,
  user-driven.** Recommended for v1.
- **(b) Soft reminder** (`Hooks::transform_context`, `hooks.rs`, runs at every
  turn start): append "the plan has N unfinished items: …" so the model
  self-corrects. Cheap, but can nag; no forcing.
- **(c) Hard block** (`Hooks::should_stop_after_turn`): end the run when
  incomplete todos remain. **Flagged risk:** the model, told it is not done,
  may re-run and thrash trying to satisfy an item it cannot — a near-infinite
  loop, and it fights D5's natural stop. Do not ship in v1.

Recommend **(a)** now, and **(b)** as an optional later nag, with **(c)** rejected.

## 3. Locked decisions

| # | decision |
|---|----------|
| D1 | Mode is **runtime, not config**: a `/plan` toggle (REPL + TUI). No `[plan]` table. Toggle reaches the agent via `Request::SetPlanMode` (flagged, §2.1). |
| D2 | **Prompt carries the behavior**: `PLAN_SECTION` lives in the kernel; `Agent::set_plan_mode` recomposes from `base_system` for a live swap; reject rebuild. |
| D3 | **Enforcement is a `Hooks` impl** (`PlanModeHooks`): `before_tool_call` blocks `edit`/`edits`/`write`/`replace`/`ast_edit`; read-only tools and `todo` stay allowed. |
| D4 | **bash read-gate**: a heuristic `bash` block (guardrail, not sandbox); FP/FN table in §2.4. |
| D5 | **Halting needs no hook**: the run ends on the tool-free turn; `/plan off` executes. No `should_stop_after_turn`. |
| D6 | **Plan→todo is a prompt instruction**; uses the existing `todo` tool + `AgentEvent::Todo`. |
| D7 | **Persist the todo list**: `SessionEntry::Todo` + last-wins `todo()`; seed the tool on first access from `ToolContext.session_path` (guarded by `seeded`). |
| D8 | **Verify**: `/verify` command (recommended); optional soft reminder; hard-block rejected (loop risk). |

## 4. Phases

| phase | scope | status |
|-------|-------|--------|
| P1 | **Core mode**: `PlanModeHandle` + `PlanModeHooks` (D3/D4); `Agent::set_plan_mode` + kernel `PLAN_SECTION` (D2); `/plan` toggle in REPL + TUI (`Request::SetPlanMode`); status-line chip. | ✅ shipped |
| P2 | **Todo integration + persistence**: plan→todo prompt (D6); `SessionEntry::Todo` + `todo()` + seed (D7); `todo` tool writes the entry. | ✅ shipped |
| P3 | **Verify**: `/verify` command (D8); optional soft reminder. | ☐ todo |

## 5. Where the design fights the code

- **A1 — `Agent.system` is private.** No setter; forcing a live swap, now
  `set_plan_mode` (D2), which recomposes from the tracked `base_system`. A rebuild
  is the only alternative and is too heavy.
- **A2 — the status line is CLI-composed.** `Status` (`app.rs:666`) is built in
  `main.rs` and mutated in-app; the plan chip is one more field + a `draw_status`
  span (`ui.rs:876`) reusing `accent()` (no `theme.rs` role), placed next to the
  model so it is not the first field dropped when space is tight. The flag is
  optimistic on the client and reverts on `AgentEvent::Error`.
- **C5/C6 — the tool set is fixed at construction.** No per-mode tool filtering;
  enforcement must be a hook (§2.3).
- **E8 — `default_tools` has no session handle.** The todo seed must go through
  `ToolContext.session_path` (§2.7).
- **Toggle plumbing (§2.1)** — the only genuinely new seam: both front-ends are
  `Backend` clients, so the flip needs a `Request::SetPlanMode`, and the actor
  needs the same `PlanModeHandle`.

## 6. Resolved questions (P1–P2)

The P1/P2 reviews settled these; kept here so the design record matches the code.

1. **Toggle transport — settled.** A `Request::SetPlanMode { on }` whose actor arm
   flips the agent's shared `PlanModeHandle` and recomposes the prompt (§2.1). No
   front-end holds the handle; a socket client just sends the request.
2. **Denylist, not allowlist — settled.** `MUTATING_TOOLS` (§2.3); each mutator
   also opts in via `TypedTool::mutating()`, and a test keeps the two in sync.
3. **bash heuristic — settled.** §2.4: `cargo build/test/check/run`, `make`, and
   `tsc` are allowed (artifacts are not a workspace mutation); the build
   subcommands that mutate the source tree/lockfiles (`cargo fmt`, `npm install`,
   `go mod tidy`, `pip install`, …) are blocked.
4. **Plan session entries — shipped (D7, P2).** Every `todo` write appends a
   `SessionEntry::Todo`; the tool seeds from it on first access (§2.7).
5. **`/verify` output shape — deferred to P3 (D8).** Recommended: checklist +
   changed files + last reply.
6. **Live mid-run toggle — settled.** Takes effect on the *next* run; the
   in-flight one keeps its cloned prompt (`set_plan_mode`, §2.2).
7. **Chip styling & team tools — settled.** The chip reuses the existing
   `accent()` role; no `theme.rs` field (§5 A2). `task`/`peers`/`message` stay
   allowed (§2.3).
