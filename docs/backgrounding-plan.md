# wcode — tool backgrounding (`bash` → background + `bg` + completion push)

Status: **design — decisions D1–D17 locked, Q1–Q6 settled by the first-layer review
(verdict: BLOCK → fixes folded); ready to sketch.** Tracker item 36; companion to
[`gap-analysis-jcode.md`](gap-analysis-jcode.md) §3.

## 1. Why

`bash` runs synchronously and kills the command after a default 30 s
(`crates/wcode-cli/src/tools/bash.rs`, `DEFAULT_TIMEOUT_SECS`). So a long build, a
full test suite, or a dev server either blocks the turn until it times out or gets
killed — there is no way to start work, keep reasoning, and collect the result
later. Backgrounding lets a command outlive the turn: start it, return a handle,
poll it with a `bg` tool, **and — the point of this revision — have the harness
push a completion event back to the agent**, so a finished build wakes an idle agent
or lands in a running agent's next-turn context. That push reuses the exact seam a
peer `message` already uses (`SessionActor`, `Request::Wake`).

## 2. Ground truth

- **The spawn path already exists.** `Bash::execute` builds `sh -c`, sets
  `process_group(0)`, drains stdout/stderr through `drain_lines` into a spill file
  under `spill_root()` (= `$TMPDIR/wcode`) with a head+tail preview, and races the
  work against `ctx.cancel` / `sleep(timeout)` with `tokio::select!`. `kill_group`
  signals `libc::kill(-pid, SIGKILL)` (`#[cfg(unix)]`, with a `child.kill()`
  fallback). Backgrounding **reuses** these pieces.
- **The push seam already exists — this is the key fact.** Every interactive agent
  runs behind a `SessionActor` (`crates/wcode-harness/src/actor.rs`). A
  `SessionHandle` sends `Request`s into its inbox, and the actor's delivery rules
  are exactly "wake if idle, else append":
  - **idle** (`serve()`): `Wake { content }` → `run(agent, &tag(from, content))` — a
    full turn **even when idle**; `Interrupt` → `agent.steer`; `Notify` →
    `agent.notify` (append, no turn).
  - **mid-run**: `Wake { content }` → `follow_up.send(...)` (injected **after** the
    current run — i.e. the next-turn context); `Interrupt`/`Notify` → `steer.send`
    (next turn *boundary* inside the run); `Cancel` → cancel.
  - Both paths emit `AgentEvent::MessageReceived { from, content }`.
  - `tag(sender, content)` = `"[message from {sender}]\n{content}"`
    (the `tag` fn in `actor.rs`); `is_inbound` marks `Notify`/`Interrupt`/`Wake` as the
    inbound (policy-checked) verbs (the `is_inbound` fn in `protocol.rs`).
  So **`Request::Wake` is precisely "wake it up, or append to the next turn if it's
  working"** — no new delivery verb is needed.
- **Tools are built before the actor spawns.** `build_agent` calls
  `default_tools` and returns an `Agent`; `SessionActor::spawn(agent)`
  (`repl::run`'s local source; `main::serve`/`main::dispatch`; `SessionFactory::build_with`) happens *after* and
  returns the `SessionHandle`. So a tool cannot hold the handle at construction — the
  notification target must be **late-bound** (D16).
- **`default_tools(cfg, sessions_dir)`** is the registration seam; the mutation
  `lock` is built **inside** it, not injected (so "inject it like the lock" is
  self-contradictory — D2). 7+ call sites (`repl.rs`, `agents.rs`, tests).
- **`bash` is a unit struct** today; holding state makes it
  `pub struct Bash { bg: Arc<Background> }` with a `new(bg)`.
- **Teardown never runs `Drop`.** `/reload`/`/resume` replace the image via
  `CommandExt::exec` (`repl.rs::exec_self`), and there are **29** `std::process::exit`
  sites — neither runs destructors (D8). The precedent for explicit teardown is
  `wcode_protocol::flush_all(FLUSH_TIMEOUT)` at the central exit points + before
  `exec_self`.
- **jcode's `bg`** action set is `{list, status, output, tail, cancel, cleanup,
  watch, delivery, subscribe, wait}` plus `notify`/`wake` flags. Its
  watch/delivery/subscribe/notify/wake machinery wakes an idle *server-side* agent
  (a `WakeRequested` server event); wcode already has the wake seam, so it only
  needs the completion→`Wake` wiring, not that machinery.

## 3. Decisions (locked)

- **D1 — Start via `bash { background: true }`; manage via a new `bg` tool.**
  `bash` already owns spawning, so a boolean on it beats a second spawn surface.
- **D2 — One `Background` handle per agent, CLI-owned, threaded in as a parameter.**
  `struct Background { registry, notify: OnceLock<SessionHandle>, next_id }`, an
  `Arc` built by the CLI **where it builds the agent** and passed to
  `default_tools(cfg, sessions_dir, bg)` (a real signature change; the 7+ call sites
  update, tests pass `Background::new()`). It carries both the task registry *and*
  the late-bound notifier, so the CLI holds one handle for `bind` (D16) and
  `shutdown` (D8). Retract "exactly like the lock". Per agent, so a worker's
  `bg list` shows only its own tasks. *(Q5.)*
- **D3 — One supervisor task per process.** `bash { background: true }`
  `tokio::spawn`s a supervisor that **owns the un-reaped `Child`**, drains both pipes
  into a bounded buffer, and `wait()`s to record the exit. The registry entry stores
  `id`, `command`, `cwd`, `started`, `pid`, `state` (`Running | Exited{code} |
  Killed`), a `kill_tx` channel, and the buffered output.
- **D4 — Bounded output, spill-and-page.** A capped inline head+tail (reuse
  `bash`'s `MAX_INLINE_BYTES` idea) + the full stream spilled to
  `spill_root()/bash-bg-<id>-<stream>.log` (the `bash-*.log` name matches
  `sweep_stale_spills`'s filter). `bg output` returns the tail **and the spill path**,
  so the model can `read` the rest. *(Q2.)*
- **D5 — The supervisor is the sole signaler.** `bg kill <id>` sends a request on the
  entry's `kill_tx`; the supervisor runs
  `tokio::select! { st = child.wait() => record Exited, _ = kill_rx.recv() => signal_group(pid) then child.wait() => record Killed }`.
  **Immediately after `child.wait()` resolves — in one lock scope, before the
  (bounded) drain — the supervisor flips the state to terminal AND clears `pid` to
  `None`**, so `Running ⇒ pid Some ⇒ un-reaped` holds for the entry's whole life.
  Signalling `-pid` while the leader is an un-reaped zombie is race-free (the pid is
  not reusable until reaped); a signal on a `pid: None` entry is skipped. `kill` on a
  terminal task is a no-op returning the state. Off-unix, mirror `kill_group`'s
  `#[cfg(unix)]` + `child.kill()` fallback. (A pidfd close to remove the residual
  TOCTOU is a follow-up.) *(Q4.)*
- **D6 — Action set: `list`, `status`, `output`, `wait`, `kill`.** These stay for
  on-demand polling. `wait <id> [timeout_secs]` blocks until terminal or the cap
  (default 60, max 300). *(Q3.)*
- **D7 — Barrier, non-mutating.** Both tools keep `parallel_safe() == false` and
  `mutating() == false` — registry state is not workspace mutation, so they must
  **not** join `MUTATING_TOOLS` (the CLI test
  `every_mutating_tool_is_in_the_plan_mode_denylist` stays green). Plan mode's gate
  keys on the tool **name**, so `bash { background: true }` is governed like a
  foreground `bash`.
- **D8 — Explicit teardown at the seams, not `Drop`.** `Background::shutdown()`
  (sync: signal every running group, no wait; a no-op when empty) is called at the
  same central points as `flush_all(FLUSH_TIMEOUT)` (the one-shot exit paths in
  `main.rs`) and before `exec_self` (`repl.rs`). A process-global `Weak` list of live
  `Background`s (registered in `new()`) lets one `shutdown_all()` cover every agent;
  the ~29 early-exit error paths need no individual wiring. v1 kills, it does not
  persist/reconcile across `/reload`.
  **Teardown seams (pinned):** the one-shot `flush_all` sites (`main.rs`), the
  `exec_self` handoff (`repl.rs`, which covers `/reload` and the TUI `Reload`/`Resume`
  arms), **and the TUI `Outcome::Quit` arms** — local (`main.rs`) and the socket path —
  which must call `shutdown_all()` before `process::exit(0)`.
- **D9 — Failures are `ToolOutput { is_error: true }`, never a panic** (bad id, kill
  on a terminal task, already-exited wait).
- **D10 — Shared state under a `std::sync::Mutex<Registry>`.** Short critical
  sections (insert / lookup / state flip); a supervisor never holds the lock across
  an `.await`.
- **D11 — `bg wait` is cancellable.** It races task completion against `ctx.cancel`
  (bash parity), returning `ToolOutput { output: "cancelled", is_error: true }` — a
  60 s wait must not be un-abortable.
- **D12 — `timeout_secs` under `background: true`.** Ignored (the task runs until it
  exits or is killed), and the `bash` tool **description** says so — documented, not
  a silent no-op.
- **D13 — ID scheme: `bgN`, per-registry monotonic.** Readable; collisions
  impossible within a registry. *(Q6.)*
- **D14 — Registration: always on.** Pairs with `bash` (core); no `[tools]` flag.
  *(Q5.)*
- **D15 — Completion push (the point of this revision).** When a task reaches a
  terminal state (`Exited{code}` / `Killed`), the supervisor delivers a notification
  through the **existing peer-message seam**:
  `handle.send_from(SessionId::new("bg"), Request::Wake { content })`. By the actor's
  rules this **wakes an idle agent into a turn**, and when the agent is running it
  becomes a `follow_up` — **appended to the next-turn context**. Content is framed
  like a peer message:
  `[message from bg]\nbg3 exited (0) after 12s\n<tail…>` (or `bg3 killed`, or
  `bg3 failed (exit 1)`). No new `Request` variant, no kernel change.
- **D16 — The notifier is late-bound.** `Background.notify` is a
  `OnceLock<SessionHandle>`, filled by the CLI immediately after
  `SessionActor::spawn` (`bg.bind(handle)`). A task whose owner never bound, or whose
  `send_from` fails (session shut down — e.g. a one-shot `-p` that already exited),
  is silently dropped. This is the only reason the handle is threaded through
  `default_tools` rather than built inside it.
- **D17 — A UI event — deferred (out of v1).** `AgentEvent::BackgroundTask` would be a
  kernel (`event.rs`) change, which the design keeps out of scope; the UI signal is
  the `MessageReceived` the `Wake` already emits. Revisit after v1.

## 4. Interface

```
bash { command, timeout_secs?, background?: bool }      # background:true → returns a handle
bg   { action: "list"|"status"|"output"|"wait"|"kill", task_id?, tail_lines?, timeout_secs? }
```

- `bash { background: true }` → `started bg3: <command>` (or an immediate
  `bg3 exited (0): <preview>` if it finished before the call returned).
- `bg list` → `bg3 running 12s  cargo test  (2.1 MB output)`.
- `bg wait bg3` → `bg3 exited (0)\n<tail…>` (or `still running after 60s`).
- `bg output bg3` → the tail + `full output: <spill path>`.
- `bg kill bg3` → `bg3 killed` (or `bg3 already exited (0)`).
- **On terminal state (push, D15):** the agent receives
  `[message from bg]\nbg3 exited (0) after 12s\n<tail…>` — a turn if it was idle,
  next-turn context if it was working.

## 5. Non-goals (v1)

jcode's `watch`/`delivery`/`subscribe`/`notify`/`wake` *server* machinery (wcode's
wake seam is the actor, not a server event); `cleanup`/`superseded`;
cross-`/reload` persistence or orphan-reconciliation (D8 kills, it does not adopt);
**interim** progress streaming mid-task (the push is at the terminal state only —
`bg status`/`output` cover on-demand peeks). A start-time opt-out
(`background: true, notify: "silent"`) is a possible follow-up.

## 6. First-layer review — resolutions

Verdict **BLOCK** (three defects + missing decisions), now folded:

- **D8 rewritten** — `Drop` never runs on `exec_self` or the 29 `process::exit`
  sites; teardown is an explicit `shutdown()` at the `flush_all` seams.
- **D5 rewritten** — the supervisor is the sole signaler (owns the un-reaped
  `Child`); `bg kill` requests, it does not signal by a stored pid.
- **D2 rewritten** — a real `default_tools` parameter (CLI-owned), not "like the
  lock"; the "exactly like the lock" claim is retracted.
- **Added** — D10 (synchronization), D11 (`wait` cancel), D12 (`timeout_secs`),
  D13 (id scheme); D7 confirmed correct.

**Revision (this round, per the requester):** the model is now **push, not poll** —
D15/D16/D17 add the completion→`Wake` delivery through the actor seam. The former
non-goal "auto-injecting completion into a later turn" is **now the feature**.

**Sketch gate (first-layer on the sketch, verdict BLOCK → folded).** Two process
fixes: the supervisor flips state + clears `pid` before the drain (D5), and the drain
is bounded (`DRAIN_GRACE`; drop the pipe readers on expiry) so a daemonized
grandchild can't stall the completion push. Completeness: the 12 `BashArgs` test
literals gain `background: None`; `bg` threads through `serve`/`dispatch`/
`SessionSource::Local`; the bash chunking consts become `pub(crate)`; the worker
allow-list keeps `bg` iff `bash` is retained.

Q1–Q6 settled: **Q1** start via `bash { background: true }`; **Q2** spill-and-page;
**Q3** `wait` 60 s / 300 s; **Q4** `bg kill` allowed in plan mode (no hook change);
**Q5** always on; **Q6** `bgN`.

**Doc/test touchpoints:** add `bg` to the README **Tools** table; the parallel-safe
list is **unaffected** (both tools are barriers); add `bg` to the core-tool assertion
list (`default_tools_omit_grep_and_find`). **Before sketching**, grep whether any
test pins the exact `bash` schema (adding `background` changes it).

## 7. Sizing & test plan

Registry + supervisor + `bash` flag + `bg` tool + the `Wake` wiring **M–L**;
tests **M**; docs **S**. Call it **M**, leaning **L**.

**Tests:** start→list→kill; `wait` on a short task (exit code); output cap + spill
path; two concurrent tasks; **kill-after-exit is a no-op** (no signal to a reused
pid); **no orphaned group after kill** (`kill(pid,0)` → ESRCH); `shutdown()` kills a
running group; `wait` aborts on `ctx.cancel`; **a finished task wakes an idle actor**
(assert the next turn starts with the `[message from bg]` content); **a finished task
mid-run is delivered as a follow-up** (assert it lands in the next turn's context).

## 8. References

- wcode: `crates/wcode-cli/src/tools/{mod.rs,bash.rs}`, `crates/wcode-harness/src/tool.rs`,
  `crates/wcode-harness/src/actor.rs` (delivery rules, `SessionHandle`, `tag`),
  `crates/wcode-harness/src/protocol.rs` (`Request`, `is_inbound`),
  `crates/wcode-cli/src/{main.rs,repl.rs,agents.rs}`.
- jcode: `crates/jcode-app-core/src/tool/bg.rs`,
  `crates/jcode-app-core/src/server/background_tasks.rs`,
  `crates/jcode-base/src/background/tests.rs`, `crates/jcode-base/src/platform.rs`.
