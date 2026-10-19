# W028 — workflow hardening — brief

- **Status:** draft — awaiting human sign-off
- **Work item:** W028
- **Decisions:** locked by the human (this brief is their contract) → to be recorded as **D033** — "workflow node timeout = `fail`; edges and judge edges share a bound; headless log always on (`--quiet`); `--status` is file-based read-only".
- **Author:** session `agent:brainstormer`
- **Date:** 2026-07-05
- **Companion:** W029 (`docs/work/W029-queue-input-while-running.md`) — independent, TUI-only.

## 1. The ask

A headless `[workflow]` run is today an opaque, unbounded, one-shot batch job. Five gaps: (1) a `script` edge/node timeout is a hard-coded shared const (`scheduler.rs:39`); (2) the only deadline is the CLI `--timeout`, which requires `--task` — no per-node deadline and no config default; (3) in headless mode every `AgentEvent` is discarded (`main.rs:2467-2468`) and only a final summary prints (`main.rs:2536-2539`); (4) a cancelled/errored run prints no way back and the task text is persisted nowhere; (5) there is no non-TUI way to read a run's progress. The request: make the run **observable, bounded, and resumable** — without touching the kernel.

## 2. Scope / Non-scope

**In scope.**
- **Timeouts (three knobs, config).** `[[workflow.node]] timeout_secs` (per-node deadline, default 300 s = today's const), `[workflow] timeout_secs` (total-run default when `--timeout` is absent), `[[workflow.edge]] condition.timeout_secs` (per-edge bound for **both** `script` and `judge` edges, default 30 s = `tools/bash.rs:13` `DEFAULT_TIMEOUT_SECS`). A `script` edge timeout stays `Unjudgeable`; a `script`/`Session` node timeout is terminal `fail`.
- **A real cancel owner.** Mint the scheduler `CancellationToken` so a run deadline / exit / SIGINT actually cancels in-flight `Script` nodes and worker turns, and leaves the plan resumable.
- **Headless log, always on.** Render the root's node→root reports (`AgentEvent::MessageReceived`), the root's own text, and node state transitions on stderr. `--quiet` suppresses the per-event log.
- **Resume.** Persist the task text (plus config path/timeout) in the run's group dir; print a copy-pasteable `resume:` line on any non-zero exit; install a headless SIGINT handler.
- **`--status [group|latest]`.** Read-only, file-based dump of a run's plan (no protocol request, no socket).
- **Two guards.** `run_workflow` refuses an empty plan (exit 2, "no plan loaded"); `check_task_args` refuses `--serve` + `--task`.

**Non-scope.**
- **A worker's internal tool stream in the headless log** — `Backend::from(root_handle).subscribe()` yields the **root's** events only; per-worker internals would need one backend subscription per worker (the TUI's `worker_backend` path, `main.rs:1951`). Phase 1 is the root view; per-worker logs are a separate change.
- **Reopening on timeout (the `reopen` path).** Locked against — a node timeout is `fail`, terminal.
- **A new protocol request / socket surface for status.** Locked against — `--status` reads files. `Request::Status` exists (`crates/wcode-harness/src/protocol.rs:286`) but is session-scoped, not plan-scoped, and stays untouched.
- **`--verbose` / a log-level config.** Locked against — the log is always on; `--quiet` is the only knob.
- **Widening the AND-fold (F1, `docs/next-steps.md` item 83).** A per-edge timeout does not touch the fold; it only bounds one edge's evaluation. F1 waits (see §4.4 item 9).
- **MCP / permission prompts / approval flows** — absent by design (`AGENTS.md`); the policy seam is `Hooks`, unchanged.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P1.1 | Headless root-event log (`repl::print_workflow_events`) | high | low | low | reuses the existing `MessagePrinter`/`emit` renderer (`repl.rs:1499-1578`); no kernel change |
| P1.2 | `--quiet` flag + wiring | med | low | low | one `Args` bool threaded into the log task |
| P1.3 | `--status [group\|latest]` read-only | high | med | med | needs a replay-only, **non-writing** loader — `load_at` writes (`tasks.rs:577`) |
| P1.4 | Guards: empty plan; `--serve`+`--task` | med | low | low | one pure predicate each; breaks one existing test (see §4.4 item 1) |
| P2.1 | Three timeout knobs + Session-node timer arm + judge bound | high | high | high | a new `select!` arm in the loop + journaled per-node deadlines + a `&mut self` signature change (§11 A6) |
| P2.2 | Real cancel owner (token + SIGINT + resume-safe state) | high | med | high | a cancel must NOT `fail` a node, else resume is stuck (§4.4 item 2) |
| P2.3 | Resume: persist task/run, print `resume:` line | high | med | med | a new group-dir sidecar + a printer mirroring `repl::print_relaunch` |

## 4. Interface & structure

### 4.1 Added

- **`<groupdir>/run.json`** — the launch record, written once at the start of a headless run: `{ "task": String, "config": Option<String>, "timeout_secs": Option<u64> }`. Home = the group dir alongside `plan.ndjson` (`session_groups.rs:83-85` `"<dir>/plan.ndjson — the task-DAG journal"`). Read by `--status` and the resume printer. New helper pair in `session_groups.rs` (`write_run`/`load_run`), mirroring `write_manifest`/`load_manifest` (`session_groups.rs:448` `pub fn load_manifest(group: &SessionGroup) -> Option<Manifest>`) — **sidecar confirmed, see §11 A2**.
- **`TaskList::read_at(path, cap) -> Result<TaskList, String>`** — a replay-only loader (sink `None`, **no** `reconcile`), for `--status`.
- **New `--quiet` and `--status` CLI flags** (see §4.3).

### 4.2 Changed

**`crates/wcode-cli/src/main.rs`**

- `USAGE` (`main.rs:46`) — add `--quiet` and `--status [group|latest]` lines; extend the `--timeout` line (`main.rs:79` `"  --timeout <secs>   cap the headless run (0/absent = no cap)"`) to mention the `[workflow] timeout_secs` default.
- `struct Args` (`main.rs:130`) — add `quiet: bool` and `status: Option<Option<String>>` (the `Option<Option<_>>` shape mirrors `resume` at `main.rs:139` `/// None = flag absent; Some(None) = latest; Some(Some(path)) = that file.`).
- `parse_args` (`main.rs:~227`) — parse `--quiet` and `--status` (with an optional value, like `--resume` at `main.rs:239`).
- `check_task_args` (`main.rs:1368-1388`) — add two guards: `args.serve && args.task.is_some()` → `Err("--task cannot be combined with serve")`; `args.status.is_some()` combined with `-p`/`--task`/`serve` → `Err(...)`.
- `main` (`main.rs:2134`) — handle `--status` immediately after `parse_cli`/`WCODE_TASK` resolution and **before** `load_config_raw`, so it needs no model. `session_dir()` is pure (`repl.rs:278-282`).
- `dispatch` (`main.rs:1856-1877`) — in the `--task` branch: read the cancel token from the orchestrator, feed `[workflow] timeout_secs` as the timeout default, pass the group dir + task into `run_workflow`.
- `build_runtime` (`main.rs:1613-1629`) — mint the token once, hand a clone to `spawn_scheduler` (`main.rs:1626` `tokio_util::sync::CancellationToken::new(),`) and keep a clone reachable from `dispatch`.
- `run_workflow` (`main.rs:2457-2533`) — replace the discard-drain, add the empty-plan guard, accept the cancel token + group/task, cancel on the deadline arms, print the resume line, render the log.
- `should_instantiate_workflow` (`main.rs:2582-2588`) — **unchanged**; a resume must still not re-seed.

**`crates/wcode-cli/src/scheduler.rs`**

- `SCRIPT_TIMEOUT` (`scheduler.rs:39` `const SCRIPT_TIMEOUT: Duration = Duration::from_secs(300);`) — becomes two named consts: `EDGE_TIMEOUT` (30 s) and `NODE_TIMEOUT` (300 s), both overridable per node/edge.
- `Scheduler` (`scheduler.rs:46-95`) — add `started: HashMap<u32, Instant>` (deadline bookkeeping) and reuse the `cancel` field (`scheduler.rs:70-72`). See §11 A6 for the `&mut self` consequence.
- `run` (`scheduler.rs:150-197`) — add a **timer arm** to the `select!` (`scheduler.rs:187-195`) and set `started` on dispatch.
- `edges_admit` (`scheduler.rs:332-408`) — the edge `run_command` call (`scheduler.rs:363-371`) takes `cond.timeout_secs` (default `EDGE_TIMEOUT`); the judge spawn (`scheduler.rs:397-401`) is wrapped in `tokio::time::timeout(…)`, mapping an elapsed deadline to the **existing** `Verdict::None` path.
- `dispatch_one` (`scheduler.rs:415-492`) — the `Script` arm's `run_command` (`scheduler.rs:462-470`) takes `run.timeout_secs` (default `NODE_TIMEOUT`); the spawned task (`scheduler.rs:461-489`) returns early (leaves `Doing`) when `cancel.is_cancelled()`.

**`crates/wcode-cli/src/judge.rs`** — doc only: `judge.rs:67-68` (`"a stalled judge is bounded only by the run's --timeout (main.rs)."`) is now false; the bound is the edge's `timeout_secs`.

**`crates/wcode-cli/src/config.rs`**

- `Workflow` (`config.rs:234-246`) — add `pub timeout_secs: Option<u64>` (the **total-run** default — §11 A1).
- `WorkflowNode` (`config.rs:351-368`) — add `pub timeout_secs: Option<u64>` (`deny_unknown_fields` stays; a new **optional** field is back-compat for an old config).
- `EdgeCondition` (`config.rs:273-286`) — add `pub timeout_secs: Option<u64>` (`Serialize` already; journaled via `Dep::Edged`, `tasks.rs:124-129`).
- `validate_workflow_structure` (`config.rs:969`) — `0` = disabled (mirrors `--timeout 0`, `main.rs:135`); no new rejections (a `u64` cannot be negative).

**`crates/wcode-cli/src/tasks.rs`**

- `RunSpec` (`tasks.rs:98-110`) — add `#[serde(default, skip_serializing_if = "Option::is_none")] timeout_secs: Option<u64>` to **both** `Session` and `Script` variants, so the per-node deadline is journaled (back-compat: a serde default decodes an old line).

**`crates/wcode-cli/src/repl.rs`**

- New `pub async fn print_workflow_events(rx: &mut broadcast::Receiver<AgentEvent>, quiet: bool)` — render until `RecvError::Closed` (not until `AgentEnd`; a run has many turns), reusing `MessagePrinter`/`emit`/`close_blocks` (`repl.rs:1499-1599`). All output to **stderr**.
- New `pub fn print_workflow_resume(group: &Path, task: &str, config: Option<&str>)` — reuse `relaunch_line`/`shell_quote` (`repl.rs:625-652`), mirroring `print_relaunch` (`repl.rs:607-621`).

**`crates/wcode-cli/src/session_groups.rs`** — `write_run`/`load_run` for `<groupdir>/run.json` (mirroring `Manifest`, `session_groups.rs:352-362`).

### 4.3 CLI surface (exact)

```
--quiet                 headless run: suppress the per-event log (the final
                        summary + any `resume:` line still print)
--status [group|latest] read-only: print <group>'s plan (#id [state] title),
                        or the latest group; exit 0. Needs no model.
```

Precedence for the total-run deadline: `--timeout` flag → `[workflow] timeout_secs` → unbounded. For a node deadline: `[[workflow.node]] timeout_secs` → 300 s. For an edge: `condition.timeout_secs` → 30 s. `0` = disabled at every level.

### 4.4 Where the design fights the current code (flagged)

1. **The empty-plan guard breaks an existing test.** `run_workflow_bounds_the_in_flight_turn` (`main.rs:2732`) drives `run_workflow` with an **empty** `TaskList` (`main.rs:2748` `let tasks = crate::tasks::TaskList::new();`) and asserts `assert_eq!(code, 1, ...)` (`main.rs:2758`). A top-of-function empty-plan guard returning 2 fails it. Fix: seed one node (`tasks.create("n", vec![], false)`) so the deadline arm is what returns, and add a sibling test `run_workflow_refuses_an_empty_plan`. (Alternative: put the guard in `dispatch` before the call at `main.rs:1873` — but the locked text says "in `run_workflow`", so the test change is preferred.)
2. **A naive cancel `fail`s nodes and wedges resume.** `run_command` returns `(None, "cancelled")` on cancel (`tools/bash.rs:453-456`), and the detached `Script` task fails the node on `None` (`scheduler.rs:479-487`) → a `Failed` node is not repaired by `reconcile` (`tasks.rs:564-580` only flips `Doing → Todo`) → the resumed run is stuck. So the spawned task must check `cancel.is_cancelled()` and leave the node `Doing`. Load-bearing for gap 4.
3. **`load_at` writes.** `TaskList::load_at` calls `reconcile` → `record(Restart)` (`tasks.rs:577` `let _ = self.inner.record(&PlanOp::Restart { id });`) → a live journal gains lines. `--status` must use the replay-only `read_at`, never `load_at`. Also note `reconcile` would display a mid-run `Doing` node as `todo` — wrong for monitoring.
4. **The scheduler loop has no timer arm** (`scheduler.rs:187-195` selects only on `updates.changed()` and `judge_rx.recv()`), and `scheduler.rs:172` admits the hole: `"A Doing node that never reports is a crash — reconciled in P4, not here."` The per-node Session deadline needs a third arm plus `started` bookkeeping.
5. **The cancel token has no owner today.** `main.rs:1626` mints it fresh and it is never cancelled; `scheduler.rs:70-72` says so (`"A fresh, never-cancelled token is fine for P3"`). `spawn_scheduler` (`agents.rs:813-819`) takes it by value, so a clone must be retained (on the `Orchestrator`).
6. **`--serve` silently swallows `--task`.** `serve(…)` owns the session and diverges at `main.rs:2211-2215` before `dispatch`; the guard is the fix.
7. **Adding `RunSpec.timeout_secs`** touches ~13 construction sites (mostly tests): `tools/task.rs:436,557,572`, `main.rs:2667`, `scheduler.rs:1073,1093,1108,1127,1145,1518`, `tasks.rs:1396,1561`, `verify_gate.rs:341`. Accepted — it is the only journal-robust home for a per-node deadline (a side-map cannot be rebuilt on resume, because `should_instantiate_workflow` is false and node ids come from the journal).
8. **The escalation path is reused unchanged.** A `fail`ed node leaves `failed_ids()` non-empty (`scheduler.rs:173`) and the existing arm escalates once (`scheduler.rs:179-186`) — a timed-out node rides it with no new code.
9. **F1 (the AND-fold) is unaffected but adjacent.** `edges_admit` returns `Pending` at the first `judge` edge (`scheduler.rs:397-401`; `docs/next-steps.md` item 83 F1), so a later conditional edge on the same target is not evaluated. A per-edge `timeout_secs` only bounds the edge that *is* evaluated; it neither widens nor depends on F1 — **no wait required**.

## 5. Plan

### Phase 1 — observability (steps 1–5)

1. **Replay-only plan reader.** Deliverable: `TaskList::read_at(path, cap)` that replays with sink `None` and never calls `reconcile`. Gate: a unit test that reads a fixture journal and asserts the file's bytes are byte-for-byte unchanged (and that a `Doing` node is reported as `doing`, not `todo`).
2. **`--status` command.** Deliverable: `wcode --status latest` prints `#id [state] title` per node (reusing `print_workflow_summary`'s shape); `--status <group>` targets one. Gate: a unit test for the group selector (latest = `list_groups(&session_dir())` head, descending at `session_groups.rs:267`) plus a live check (§7.2).
3. **Headless root log.** Deliverable: `repl::print_workflow_events(rx, quiet)` rendering `MessageReceived` (`event.rs:133-136`) as `«from» content`, the root's `MessageEnd` text, and — from the `TaskList` watch the driver already holds (`main.rs:2502`) — `#id [state] title` on every change; all to **stderr**. The discard-drain (`main.rs:2467-2468`) is replaced. Gate: `--quiet` suppresses the log but not the summary (a unit test on the renderer's gating + a live check).
4. **`--quiet` flag.** Deliverable: `Args.quiet` parsed and threaded into `run_workflow`. Gate: `parse_args` test `parse_quiet`.
5. **Guards.** Deliverable: `run_workflow` returns 2 + `eprintln!("error: no plan loaded")` on an empty snapshot; `check_task_args` rejects `--serve`+`--task` and `--status` with a run/prompt. Gate: tests `run_workflow_refuses_an_empty_plan`, extended `task_guards_reject_misconfiguration`, and the fix to `run_workflow_bounds_the_in_flight_turn` (§4.4 item 1).

### Phase 2 — bounds + resume (steps 6–12)

6. **Three timeout knobs in config.** Deliverable: `Workflow.timeout_secs`, `WorkflowNode.timeout_secs`, `EdgeCondition.timeout_secs` with the defaults in §4.3. Gate: config tests proving an **old config** and an **old journal line** still parse.
7. **Journal the per-node deadline.** Deliverable: `RunSpec::{Session,Script}.timeout_secs`, set by `instantiate_workflow` via `configure`. Gate: a journal round-trip test — a `Create`/`Configure` with `timeout_secs` survives `load_at`, and an old line without it decodes to `None`.
8. **Edge + judge bound.** Deliverable: `edges_admit` uses `cond.timeout_secs` (default `EDGE_TIMEOUT`); a timed-out `script` edge stays `Unjudgeable` (`scheduler.rs:377-380` unchanged); a timed-out judge maps to `Verdict::None` → block-and-escalate. Gate: scheduler tests `a_timed_out_script_edge_blocks_and_escalates`, `a_timed_out_judge_blocks_and_escalates`.
9. **Script node + Session node deadline.** Deliverable: `Script` uses `run.timeout_secs` in `run_command` and keeps the existing `fail` outcome; a new timer arm in `Scheduler::run` `fail`s a `Doing` **Session** node past its deadline with a reason naming the limit (`fail(id, "node timeout: exceeded {n}s")`). The reason shape already exists for scripts: `tools/bash.rs:459` `format!("timed out after {}s (command killed)", timeout.as_secs())`. Gate: scheduler tests `a_session_node_past_its_deadline_fails`, `a_script_node_timeout_names_the_limit`.
10. **Real cancel owner.** Deliverable: the token minted in `build_runtime`, a clone on the `Orchestrator` (`agents.rs`), `run_workflow` cancels it on both deadline arms (`main.rs:2484`, `main.rs:2518`), and the detached `Script` task leaves the node `Doing` when `cancel.is_cancelled()` (§4.4 item 2). Gate: a scheduler test `a_cancelled_script_node_stays_doing` (resume-safe) and a live SIGINT check (§7.2).
11. **Persist the launch + print the resume line.** Deliverable: `run.json` written at run start; `repl::print_workflow_resume` printed on any non-zero exit; a headless SIGINT handler (mirroring `repl.rs:769-779`) that cancels, prints `resume:`, and exits. Gate: a test that a cancelled run leaves `<groupdir>/run.json` with the task text, and the printed line round-trips through `relaunch_line`.
12. **Docs + tracker.** Deliverable: this work item, `docs/decisions/D033-…md`, a `docs/next-steps.md` row, and a `README.md` flag note. Gate: `--dump-config`/USAGE reflect the new keys; the tracker row cites the commits.

## 6. Quality gates

- `cargo test --workspace` → all pass.
- `cargo clippy --workspace --all-targets` → clean.
- **Known-flaky, diff-orthogonal:** `markdown::tests::a_rust_fence_has_multiple_distinct_styles` and `markdown::tests::install_syntax_theme_swaps_the_process_theme` (`docs/next-steps.md` W021 residuals) — a process-global `SYNTAX_THEME`; they pass 10/10 in isolation. Re-run before reporting a failure; do not "fix" them here.
- **Live checks** (§7.2): a keyless endpoint `--base-url http://localhost:11434/v1`; a refused port `http://127.0.0.1:9/v1` with `WCODE_RETRY_MAX=2` for the error/resume path (`.wcode/skills/live-verification/SKILL.md` §1–§2).
- **Regression tests kept green:** `parse_task_and_timeout`, `timeout_rejects_junk`, `task_guards_reject_misconfiguration`, `task_guards_accept_a_well_formed_run`, `f1_should_instantiate_only_with_a_task`, `edge_parses_an_unconditional_and_a_script_condition`, `the_repo_workflow_toml_migrated_to_edges`, `seed_names_the_task_and_forbids_new_nodes`. (`run_workflow_bounds_the_in_flight_turn` is **updated**, not kept — §11 A4.)

## 7. Testing

### 7.1 Automated

- `run_workflow_refuses_an_empty_plan` — an empty `TaskList` returns 2 and prints `no plan loaded`.
- `run_workflow_bounds_the_in_flight_turn` (updated) — a seeded node + a hanging stream: the deadline arm cancels and returns 1.
- `read_at_never_writes` — the plan file's bytes are identical before/after `TaskList::read_at`; a `Doing` node reads back `doing`.
- `status_latest_picks_the_newest_group` — `list_groups` head is selected (`session_groups.rs:267` descending).
- `parse_quiet_and_status` — `--quiet`, `--status`, `--status g1` parse; `--status` with `-p` errors.
- `task_guards_reject_serve_task` — `--task`+`serve` → `Err("--task cannot be combined with serve")`.
- `print_workflow_events_shows_node_reports_and_quiet_suppresses` — a `MessageReceived` renders under `quiet=false` and not under `quiet=true` (drive `print_workflow_events` over a fed `broadcast`, like `print_events_caches_the_latest_todo_event`, `repl.rs:2549`).
- `old_config_and_old_journal_still_parse` — a `[workflow]` with no `timeout_secs` and an `EdgeCondition` line with no `timeout_secs` both decode.
- `node_and_edge_timeouts_reach_the_scheduler` — a `RunSpec::Script { timeout_secs: Some(1) }` and an `EdgeCondition { timeout_secs: Some(1), .. }` survive `load_at` and drive the right `run_command`/`timeout` argument.
- `a_timed_out_script_edge_blocks_and_escalates` / `a_timed_out_judge_blocks_and_escalates` — outcome is `Unjudgeable`, never `Skip`.
- `a_session_node_past_its_deadline_fails` — the timer arm `fail`s a `Doing` Session node; the reason names the limit; dependents stay blocked.
- `a_cancelled_script_node_stays_doing` — on `cancel.cancel()`, the node is left `Doing`.
- `cancelled_run_persists_the_task_and_prints_resume` — `<groupdir>/run.json` holds the task text; `print_workflow_resume` output round-trips through `relaunch_line`.

### 7.2 How a human verifies it

The repo's pipeline needs `gh` + a git remote; use a **tiny local workflow** instead. Write `/tmp/wf.toml`:

```toml
model = "llama3"                      # any id your keyless endpoint serves
[workflow]
max_attempts = 1
timeout_secs = 120                    # (phase 2) total-run default, no --timeout flag
[[workflow.node]]
id = "a"
member = "w1"
title = "Say hello for: {{task}}"
[[workflow.node]]
id = "b"
member = "w1"
title = "Say done for: {{task}}"
[[workflow.edge]]
from = "a"
to = "b"
condition = { script = "sleep 5", timeout_secs = 1 }   # (phase 2) bounded edge
[[team]]
name = "w1"
```
plus a `.wcode/agents/w1.md` role (or a `[[team]]` with a model). Then:

```sh
# Phase 1 — the log is visible (stderr), the summary is the result (stdout)
wcode --agents --config /tmp/wf.toml --task "smoke test" 2>/tmp/log
# look for:  «agent:w1» …          (a node→root report, from MessageReceived)
#            #1 [doing] …  #1 [done] …   (node transitions)

# Phase 1 — --quiet keeps the summary, drops the log
wcode --agents --config /tmp/wf.toml --task "smoke test" --quiet 2>/tmp/log2
# look for: /tmp/log2 has no «…» report lines.

# Phase 1 — --status is read-only
G=$(ls -t ~/.local/share/wcode/sessions | head -1)
md5sum ~/.local/share/wcode/sessions/$G/plan.ndjson
wcode --status $G        # or: wcode --status latest
md5sum ~/.local/share/wcode/sessions/$G/plan.ndjson   # SAME hash → no writes
# look for: #id [state] title lines naming the plan's nodes.

# Phase 2 — a cancelled/errored run prints a resume line
WCODE_RETRY_MAX=2 wcode --agents --config /tmp/wf.toml \
  --base-url http://127.0.0.1:9/v1 --task "resume me"
# look for:  error: …  then  resume: wcode --agents --config /tmp/wf.toml --resume <dir> --task 'resume me'

# Phase 2 — the resume is real: re-run the printed line; the plan must NOT re-seed
# (should_instantiate_workflow is false on resume) and must progress from where it stopped.

# Phase 2 — SIGINT prints the resume line
wcode --agents --config /tmp/wf.toml --task "interrupt me"   # Ctrl-C mid-run
# look for: a `resume:` line and a clean exit.

# Phase 2 — a bounded edge blocks-and-escalates (never a silent skip)
wcode --agents --config /tmp/wf.toml --task "edge timeout"
# look for: node #2 stays todo / blocked, and the root is woken with an escalation naming #2.
```

Quote the observed line (`«agent:w1» …`, the `resume:` line, the unchanged `md5sum`).

## 8. Expected outcome

- A headless `--task` run prints, to stderr, each node→root report (`«agent:<member>» …`), the root's own text, and every `#id [state] title` transition — and `--quiet` silences that while keeping the stdout summary.
- `wcode --status latest` prints a run's plan and provably does not write (`md5sum` unchanged).
- An interrupted or failed run prints a `resume: …` line whose task text came from `<groupdir>/run.json`; re-running it continues the plan (no re-seed) from the reconciled state.
- A hung `script` edge (default 30 s), a hung `judge` edge, a hung `script` node, and a hung `Session` node each hit a configurable bound; a node timeout lands as `fail` naming the limit and blocks its dependents; an edge timeout stays `Unjudgeable`.
- `wcode serve --task x` exits 2 with `--task cannot be combined with serve`; a `--task` run with no plan exits 2 with `no plan loaded`.
- An old `config.toml` and an old `plan.ndjson` still load unchanged.

## 9. References

- `crates/wcode-cli/src/scheduler.rs` — the loop, `SCRIPT_TIMEOUT`, `edges_admit`, `dispatch_one`, the escalation arm (`:179-186`).
- `crates/wcode-cli/src/main.rs` — `run_workflow`, `dispatch`, `check_task_args`, `print_workflow_summary`, `should_instantiate_workflow`.
- `crates/wcode-cli/src/config.rs` — `Workflow`/`WorkflowNode`/`WorkflowEdge`/`EdgeCondition`.
- `crates/wcode-cli/src/tasks.rs` — `PlanOp`, `load_at`/`reconcile` (the write hazard), `terminal_code`, `RunSpec`.
- `crates/wcode-cli/src/session_groups.rs` — `plan_path`, `manifest`, `list_groups`.
- `crates/wcode-cli/src/repl.rs` — `print_events` (the renderer to reuse), `print_relaunch`/`relaunch_line`, `session_dir`, the `ctrl_c` pattern.
- `crates/wcode-cli/src/tools/bash.rs` — `DEFAULT_TIMEOUT_SECS` (`:13`, `= 30`; used at `:498`) and `run_command`'s timeout/cancel return shapes.
- [`docs/task-dag-plan.md`](../task-dag-plan.md) — resume design (`:232-248`) and the journal home (`:344`), designed but never driven end-to-end.
- [`docs/work/W025-workflow-edges-and-lateral-messaging.md`](W025-workflow-edges-and-lateral-messaging.md) — edges/judge; `:11` the F1 follow-up.
- [`docs/next-steps.md`](../next-steps.md) — item 83 (the F1 follow-up), the W021 markdown flake.
- [`.wcode/workflow.toml`](../../.wcode/workflow.toml) — the shipped pipeline; its `ci` node title (`:144`) already says "run via bash with a long `timeout_secs` such as 900" (the knob is anticipated, not invented).
- [`.wcode/skills/live-verification/SKILL.md`](../../.wcode/skills/live-verification/SKILL.md) — the keyless-endpoint and refused-port recipes.

## 10. Facts the author did NOT verify

- **`run.json` vs. widening `manifest.json`** — resolved by the human as a sidecar (§11 A2).
- **Exact `parse_args` line for the `--task`/`--timeout` arms** — recon says `main.rs:227-238`; read at the `grep` level, not byte-verified for where `--status`/`--quiet` would slot.
- **Whether a full pipeline E2E works today** — `docs/task-dag-plan.md:232-246`/`:344` say resume was designed but never driven; the same is true of a complete headless run (needs `gh` + a remote). Not run.
- **The `SCRIPT_TIMEOUT` comment line** — the const at `scheduler.rs:39` is verified; the comment's exact line split may differ from the recon's phrasing.
- **`Manifest` field line range** — the struct is at `session_groups.rs:352-362`; cite the snippet, not the range.
- **Timing of `--status` before `load_config_raw`** — reasoned from `session_dir()`'s purity (`repl.rs:278-282`), not executed.

## 11. Orchestrator pre-review notes (answers to §10, plus a friction the brief missed)

All four facts below were re-verified against the tree before recording.

**A1 — `[workflow] timeout_secs` is the TOTAL-RUN default.** The brief's §4.3 precedence stands (flag → `[workflow] timeout_secs` → unbounded); it is **not** a node default. The node default stays the 300 s const, the edge default the 30 s const. The three keys keep the same name `timeout_secs` and differ only in scope, which is the repo's own vocabulary (`bash { timeout_secs }`, `bg { wait, timeout_secs }`, `webfetch { timeout }`) and is already anticipated in the shipped config: `.wcode/workflow.toml:144` "run via bash with a long `timeout_secs` such as 900". Do not rename per-scope — `--dump-config` and the README can disambiguate.

**A2 — `run.json` sidecar confirmed**, not a widened `manifest.json`. `Manifest`'s doc scopes it deliberately: `session_groups.rs:346-347` "team topology, written as a fast path only (D5)". A task text, a config path and a timeout are *run launch* parameters, not team topology; folding them in would blunt that doc's contract and every reader of the manifest.

**A3 — §4.4 item 1 is correct and is a required test edit.** `run_workflow_bounds_the_in_flight_turn` (`main.rs:2732`) builds `let tasks = crate::tasks::TaskList::new();` (`:2748`) — empty — and asserts `code == 1`. The empty-plan guard makes it 2. Seed one node and add the sibling refusal test, as the brief says.

**A4 — §4.4 item 2 is correct and is the load-bearing one.** `tools/bash.rs:453-456` returns `(None, "cancelled")`, and the `Script` arm fails on `None` (`scheduler.rs:479-487`); `reconcile` only flips `Doing → Todo` (`tasks.rs:564-580`), so a cancel-induced `Failed` is permanent and the resumed run stalls on a node nobody can reset. The `cancel.is_cancelled()` early return is not an optimisation — without it, gap 4 (resume) is a promise the code cannot keep.

**A5 — `list_groups` is descending as assumed.** `session_groups.rs:267` `entries.sort_by(|a, b| b.id().cmp(a.id()));` with the comment "Q(D): name-lexicographic descending … the `{millis}_` prefix makes name order chronological". `--status latest` = `.first()`.

**A6 — NEW friction: `dispatch_one` takes `&self`.** `scheduler.rs:415` `async fn dispatch_one(&self, id: u32)`. `started: HashMap<u32, Instant>` cannot be written through `&self`, so step 9 must change the signature to `&mut self` (its caller `dispatch` is already `&mut self`, `scheduler.rs:294`, so the change is local). Two consequences the plan must state:

- `started` must be **pruned against the snapshot** each loop iteration, not just on `fail`: a node that completes, is rejected, reset or reopened leaves a stale entry that would fire a bogus deadline later.
- `started` is **in-memory only**, deliberately. `reconcile` turns `Doing → Todo` on load (`tasks.rs:564`), so a crash restarts the deadline — correct and consistent with "a crash is not the node's fault", but it must be written down, because a journaled deadline would survive a crash and time out a node that has not run yet.

**A7 — one rendering risk the brief leaves open.** §4.2 says the log renders "the root's `MessageEnd` text". The root's turn also emits `MessageStart`/`MessageUpdate` while streaming; printing only `MessageEnd` gives one block per turn (right) but a run whose root turn is long still looks frozen between turns. Phase 1's gate should pin whichever choice is made — one line per completed message, not a stream — and the live check should confirm a multi-turn root shows several lines, not one.

## 12. First-layer review of the sketch — APPROVED (amendments are BINDING)

**Verdict:** APPROVE. 25 SKETCH blocks (config 3, main 8, repl 1, scheduler 8, session_groups 2, tasks 3), marker-balanced; `git diff -U0` shows **zero non-comment deletions**; `cargo check -p wcode-cli` clean. Fidelity to §4/§11/D033 verified line-by-line against HEAD (A3/A4/A6/A7 all landed).

The developer must apply **all** of the following. Line numbers are HEAD (committed).

### The eight frictions — settled

- **A1 (F1) — REQUIRED: `instantiate_workflow` must `configure` the member node.** `main.rs:2658-2671` only `assign`s the member arm, so `RunSpec::Session.timeout_secs` is `None` forever and the per-node knob is **dead for Session nodes**. Add, after the existing `assign`, a `tasks.configure(t.id, RunSpec::Session { member: None, timeout_secs: node.timeout_secs }, node.gate)` with the same `workflow_fail` error shape. Keep `member: None` — `RunSpec::Session.member` is read nowhere (`scheduler.rs:421`, `verify_gate.rs:173`; dispatch uses `task.owner`, `scheduler.rs:422-423`), so `None` is status quo. Safe: runs before `spawn_scheduler` (`main.rs:1593` vs `:1626`), no subscriber.
- **A2 (F2) — REQUIRED: `Some(0)` must mean DISABLED, not instant-kill.** `tools/bash.rs:457` turns `Duration::ZERO` into an immediate kill, and the sketch's `.filter(|d| !d.is_zero()).unwrap_or(EDGE_TIMEOUT)` silently renames "disabled" to 30 s. Add ONE pure helper used by all five bound sites (script node, session node, script edge, judge edge, session timer): `workflow_bound(secs, default) -> Option<Duration>` with `None→Some(default)`, `Some(0)→None`, `Some(n)→Some(n s)`. Widen `run_command`'s `timeout` to `Option<Duration>` with an optional timer arm. Delete the sketch's 30-s-when-zero fallback.
- **A3 (F3) — renderer vs stderr: confirm the resolution, with a correction.** `out`/`close_blocks`/`emit` are stdout and `emit` streams partials — irreconcilable with A7's stderr. Do **not** touch the sink functions. Correction: `MessagePrinter::update` (`repl.rs:216`) returns a **delta** (`""` when the text did not grow), so it is an extractor ONLY if the printer is **fresh per message** (`MessagePrinter::default()` per `MessageEnd`, or `reset()` first) and `update` is called **once**. State that, and make the gate prove it (feed `MessageStart`+2×`MessageUpdate`+`MessageEnd`; assert the full text appears **exactly once** at `quiet=false`, absent at `quiet=true`). Mirror `print_events`' lag handling (`Lagged→continue`, `Closed→break`). Keep `quiet` on `print_workflow_events`; delete the caller-side `if !quiet` branch in `run_workflow`.
- **A4 (F4) — confirm: lift the `--status` guard out of `check_task_args`.** `check_task_args` runs at `main.rs:1475` (inside `build_runtime`), after `load_config_raw` (`:2149`), but `--status` early-exits before it — so a guard there is dead code. Add a pure `check_status_args(&Args)` called only by the early exit. Keep **only** `--serve`+`--task` in `check_task_args`.
- **A5 (F5) — census is 17 literal sites, not 13; plus a pattern.** Script (8): `main.rs:2667`, `scheduler.rs:1073,1093,1108,1127,1145,1518`, `verify_gate.rs:341`. Session (9): `tasks.rs:500` (production `apply`/`Create` default — **must be `None`**), `tasks.rs:625,1396,1530,1561`, `scheduler.rs:629`, `tools/task.rs:436,557,572`. Plus the **pattern** `scheduler.rs:439` `RunSpec::Script { command } =>` → `{ command, .. }`.
- **A6 (F6) — confirm: enforce non-double-booking in the accessor.** Insert into `started` **only** in the Session arm of `dispatch_one`; make `session_deadline` return `None` for a non-`Session` `RunSpec`, so the invariant is structural. A `Script` node's bound stays in `run_command`.
- **A7 (F7) — confirm harmless; build `read_at` with `with_sink(None, true, 3)`.** `max_attempts` is read only by `reopen_inner` (`tasks.rs:795-800`), never by a read-only replay; the built-in `3` matches `load`/`with_journal` (`tasks.rs:445,461`). Keep the `#id [state] title` print shape — do not render `attempts`.
- **A8 (F8) — REQUIRED: the timer arm must re-check `Doing` on the LIVE snapshot.** `TaskList::fail` has **no state guard** (`tasks.rs:876` sets `Failed` and journals unconditionally). A worker's `complete` landing between the snapshot and the arm would flip `Done → Failed` and journal a `Fail` into the plan, **poisoning the resume**. Re-check `state == Doing` on a fresh snapshot before `fail`ing. Documented consequence (do NOT "fix"): a node timeout does not cancel the worker's in-flight turn — D033 reuses the escalation path unchanged.

### Mandatory corrections beyond the eight

- **C1 — no `FAR` sentinel; REPLACE the `select!`.** `sleep_until(next.map_or(FAR, …))` leaves `FAR` undefined and its overflow claim is wrong (tokio clamps a large `sleep(Duration)`; the panic is on `Instant + Duration`). Use `async fn sleep_until_opt(when: Option<(u32, Instant)>) -> Option<u32>` with `std::future::pending()` for `None`. The new `select!` must **replace** `scheduler.rs:224-232`, with the prune + `next` computed above it — two `select!`s would double-handle `judge_rx`/`updates`.
- **C2 — `USAGE` is missing from the sketch.** Add the `--quiet` and `--status [group|latest]` lines and extend the `--timeout` line (`main.rs:46`, `:79-80`) per §4.3.
- **C3 — the empty-plan `return 2` writes no `run.json` and prints no resume line.** It is the first statement. Refine §2's "any non-zero exit" to "any exit from a run that was created" — a resume whose plan never loaded would re-check `should_instantiate_workflow` (false) and stay empty forever: a trap dressed as help.
- **C4 — `#[serde(default)]` on an `Option` is harmless but not load-bearing.** serde's `deserialize_option` already visits `None` for a missing field. Keep the attribute (matches `RunSpec::Session.member`), but prove back-compat by the **regression test**, not by the attribute.
- **C5 — `--status latest`'s head can be a `GroupEntry::Legacy`** (`session_groups.rs:108-133`) — a `.jsonl` file, so no `plan.ndjson`. `read_at` must take the NotFound→empty path (create nothing, do not panic); `run_status` exits 1. Resolution order for `--status <sel>`: (1) an existing dir; (2) else `session_dir().join(sel)`; (3) else exit 1 `no such group`. Empty sessions dir → exit 1. Success → `print_workflow_summary` and exit 0.
- **C6 — the transition log needs an initial print + a diff map.** Print the initial snapshot once (right after the empty-plan guard), then `HashMap<u32, TaskState>` and print only on change. All via `eprintln!`.
- **C7 — completing A3's `run_workflow_bounds_the_in_flight_turn`.** Its call (`main.rs:2753-2756`) becomes `run_workflow(backend, tasks, "go".into(), timeout, CancellationToken::new(), None, "go", None, false)` with `tasks.create("go", vec![], false)` seeded first.

### Test homes for the two un-sketched §7.1 gates

- `cancelled_run_persists_the_task_and_prints_resume` → split: `main.rs::tests::a_cancelled_run_persists_run_json` (integration: real `run_workflow`, tempdir group, `task = "resume me"`, assert `load_run` == the record and `code == 1`) + `repl.rs::tests::workflow_log_tests::resume_line_round_trips_through_relaunch_line` (pure string). Requires exposing `pub(crate) fn workflow_resume_line(...) -> String` with `print_workflow_resume` a thin wrapper — mirroring `relaunch_line`/`print_relaunch` (`repl.rs:607-621`).
- `node_and_edge_timeouts_reach_the_scheduler` → split: `tasks.rs::tests::a_configured_timeout_survives_load_at` (journal) + `scheduler.rs::tests::the_bound_maps_zero_to_disabled` (pure `workflow_bound`) + the behaviour tests as sketched. Mark the timer tests `#[tokio::test(start_paused = true)]` (test-util is already a dev-dep, `crates/wcode-cli/Cargo.toml:41`). A8 needs its own arm: land a `complete`, then fire the deadline, assert the node is still `Done` and **no** `Fail` line was journalled.

### Reviewer's not-checked

No live end-to-end headless run; `--status latest` against a real sessions tree (Legacy head, torn journal, absent dir); whether `--dump-config` prints the new keys; a served session's cancel owner; the judge's cancellability beyond the existing spawn shape.
