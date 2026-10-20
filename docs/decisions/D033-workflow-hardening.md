# D033 — Workflow hardening: bounded, observable, resumable headless runs

- **Status:** accepted & **shipped** (W028 closed — Phase 1 `d251c40` + the
  `5c8fbd4` review fix; Phase 2a (bounds) `7b86f80` + the `dd5fe8b` review fix;
  Phase 2b (cancel + resume) `b32deee` + `d791944`, which carries the recorded cap
  into the `resume:` line; the boundary reversal in this Amendment is `47dbad0`;
  the brief's §12 amendments are all landed)
- **Date:** this session
- **Relates to:** [`crates/wcode-cli/src/scheduler.rs`](../../crates/wcode-cli/src/scheduler.rs)
  (`SCRIPT_TIMEOUT`, `edges_admit`, `dispatch_one`, `Scheduler::run`),
  [`crates/wcode-cli/src/main.rs`](../../crates/wcode-cli/src/main.rs)
  (`run_workflow`, `dispatch`, `check_task_args`, `build_runtime`),
  [`crates/wcode-cli/src/config.rs`](../../crates/wcode-cli/src/config.rs)
  (`Workflow`/`WorkflowNode`/`EdgeCondition`),
  [`crates/wcode-cli/src/tasks.rs`](../../crates/wcode-cli/src/tasks.rs)
  (`RunSpec`, `load_at`/`reconcile`),
  [`crates/wcode-cli/src/session_groups.rs`](../../crates/wcode-cli/src/session_groups.rs)
  (`plan_path`)
- **Amends:** the `run_workflow` body contract in
  [`workflow-task-injection-plan.md`](../workflow-task-injection-plan.md) §4.6/§11
- **Work item:** [`work/W028-workflow-hardening.md`](../work/W028-workflow-hardening.md)

## Context

A headless `[workflow]` run (`--task`) was an **opaque, unbounded, one-shot
batch job**:

- **Unbounded in three places.** One hard-coded const `SCRIPT_TIMEOUT = 300s`
  (`scheduler.rs:39`, its own comment claiming "P5 makes it configurable" — it
  never did) bounded *both* a `script` edge condition and a `Script` node; a
  `Session` node had **no wall clock at all** (only `DEFAULT_MAX_TURNS` plus
  per-stream stall timers), so a worker that never reported left the node `Doing`
  forever — `scheduler.rs:172` admits it ("a crash — reconciled in P4, not
  here"); and a `judge` edge had no bound either (`judge.rs:68`).
- **Opaque.** `run_workflow` subscribed only to *drain and discard* every
  `AgentEvent` (`main.rs:2467-2468`); the only output was a final per-node table
  (`print_workflow_summary`). The node→root reports were already in flight as
  `AgentEvent::MessageReceived` — and thrown away.
- **Unresumable in practice.** The plan *was* journaled (`plan.ndjson`,
  reconciled `Doing → Todo` on load) and `--resume <groupdir>` worked — but the
  task text was persisted nowhere, no resume command was printed, and no SIGINT
  handler existed, so a cancelled run left the operator with no way back.
- **Unobservable.** No non-TUI way to read a run's progress: `/tasks` is TUI-only,
  `task list` is an LLM tool op, no `--dump-*` covers the plan, and there is no
  plan request on the wire.

Two hazards were load-bearing, not cosmetic:

- The scheduler's `CancellationToken` was **minted and never cancelled**
  (`main.rs:1626`), so a run deadline's `Request::Cancel` reached only the root —
  a detached `Script` node ran to its own cap regardless.
- `TaskList::load_at` → `reconcile()` **writes** `PlanOp::Restart` lines
  (`tasks.rs:577`), so any naive `--status` reader would mutate the journal it
  was observing.

## Decision

Make the run **bounded, observable, and resumable**, in two separately-gated
phases, without touching the kernel.

**Bounds — three config knobs, same key name, different scope.**
`[[workflow.node]] timeout_secs` (default 300 s), `[[workflow.edge]] condition.timeout_secs`
(default **30 s** — bash's own default; an edge runs *synchronously on the
scheduler loop thread*, `scheduler.rs:329`, so a generous edge default would
block the whole frontier), and `[workflow] timeout_secs` (**the total-run
default**, used when `--timeout` is absent). `0` disables at every level. The
name is deliberately reused across scopes: it is the repo's existing vocabulary
(`bash { timeout_secs }`, `bg { wait, timeout_secs }`) and `.wcode/workflow.toml:144`
already anticipates it.

**A node timeout is `fail`, terminal** — it rides the *existing* escalation arm
(`scheduler.rs:179-186`) with no new failure path. A hung `judge` or `script`
edge stays `Unjudgeable` (block-and-escalate), never `Skip`: a hang is not a
verdict. The per-node deadline is carried on `RunSpec::{Session,Script}.timeout_secs`
so it is journaled and survives a resume (`should_instantiate_workflow` is false
on resume, so node ids come only from the journal — a side-map could not be
rebuilt).

**Cancel gets a real owner.** The token is minted once in `build_runtime`, a
clone is reachable from the run driver (via the `Orchestrator`), and the deadline
arms cancel it. Critically, **a cancel must not `fail` a node**: `run_command`
returns `(None, "cancelled")` on cancel (`tools/bash.rs:453-456`) and the
`Script` arm fails on any `None` — while `reconcile` only repairs `Doing`
(`tasks.rs:564-580`). A cancel-induced `Failed` would therefore be permanent and
**wedge the resume**. The detached script task leaves the node `Doing` when the
token is cancelled.

**Observable, always on.** The headless run prints, to **stderr**, each
node→root report (`MessageReceived`), the root's own message text, and every
`#id [state] title` transition (from the `TaskList` watch the driver already
holds). `--quiet` silences the log; the stdout summary and any `resume:` line
still print. Rationale for always-on: a silent multi-hour run is the bug being
fixed.

**Read-only status.** `wcode --status [group|latest]` prints the plan and needs
no model. It reads through a **replay-only** loader (`TaskList::read_at`: sink
`None`, **no** `reconcile`), because `load_at` writes. Latest = the descending
`list_groups` head.

**Resumable.** A `<groupdir>/run.json` sidecar (`{task, config, timeout_secs}`)
records the launch; any non-zero exit — including SIGINT, now handled — prints a
copy-pasteable `resume:` line. A sidecar rather than a widened `manifest.json`,
whose own doc scopes it to "team topology, written as a fast path only (D5)".

**Two guards.** `run_workflow` refuses an empty plan (exit 2) — otherwise a
group whose plan failed to load looks like instant success. `--serve` + `--task`
is refused rather than silently serving without running.

## Consequences

- Adding `RunSpec.timeout_secs` touches ~13 `RunSpec` construction sites (mostly
  tests) — accepted as the only journal-robust home for a per-node deadline.
- `Scheduler::dispatch_one` changes `&self` → `&mut self` to write the per-node
  `started: HashMap<u32, Instant>`; the map must be **pruned against the snapshot
  every loop iteration** (complete/reject/reset/reopen leave stale entries) and
  stays **in-memory only** (a journaled deadline would time out a node that never
  ran).
- An optional new config field is back-compatible for both an old config and an
  old `plan.ndjson` line (`Dep::Edged` serializes `EdgeCondition`); regression
  tests must prove both.
- `run_workflow_bounds_the_in_flight_turn` (an empty-plan fixture) is **updated**,
  not kept — it is the visible proof the guard landed.

## Known limitations (out of scope)

- **The transition log samples, it does not stream.** `watch::Receiver::changed()`
  coalesces: it reports that the version advanced, not how many times, and the
  driver then diffs the *current* snapshot against what it last printed. So a
  node whose `doing → done` both land between two wakes prints only `done` — the
  normal shape for a fast gate node that becomes ready only after its dependency
  completes. Observed live (`#2` printed `[todo]` then `[done]`, no `[doing]`).
  Nothing is lost: the next wake always reflects the newest state, and the
  stdout summary is authoritative for final states. Recovering the intermediate
  would need an op-level broadcast instead of the state watch — a different
  seam, not a fix. (Contrast B1, which *was* a blocking bug: the driver was not
  observing during the first turn at all, and a fresh `watch` receiver starts at
  the current version, so a state reached and held in that window was lost
  permanently.)
- **Per-worker tool streams** are not in the headless log:
  `Backend::from(root_handle).subscribe()` yields the root's events only. A
  per-worker view needs one subscription per worker.
- **F1** (the AND-fold stopping at the first `judge` edge, D029) is unaffected:
  a per-edge timeout bounds the edge that *is* evaluated and neither widens nor
  depends on the fold.
- No new protocol request and no socket surface: `--status` is file-based.

## Amendment (human sign-off, supersedes the per-node/per-edge defaults)

**The defaults flip: an unset node or edge bound means UNBOUNDED, not a built-in
const.** The original decision above gave `WorkflowNode.timeout_secs` a 300 s
default and `EdgeCondition.timeout_secs` a 30 s default. The human's ruling:

> "it needs to be no limit if not set, and there should be a limit for the whole
> task that can be set when start, condition timeout also should default to no
> limit."

So `workflow_bound`'s site-default form is **removed**; every bound site uses
`workflow_bound_opt` (`None` and `Some(0)` → no timer, `Some(n)` → n seconds):

| knob | unset | `= 0` | `= n` |
|---|---|---|---|
| `[[workflow.node]] timeout_secs` | **unbounded** | unbounded | n s |
| `condition.timeout_secs` (edge) | **unbounded** | unbounded | n s |
| `[workflow] timeout_secs` (run) | unbounded | unbounded | n s |
| `--timeout` flag | unbounded | unbounded | n s |

**The whole-run cap is the one bound that matters**, and it is set at launch —
`--timeout <secs>` (flag wins) or `[workflow] timeout_secs` (the default when the
flag is absent). It is the single knob an operator needs; per-node and per-edge
bounds are opt-in refinements for a step you know is slow.

**Rationale.** (a) The defaults were a back-compat *regression* in disguise: a
`Session` node was unbounded before W028, so "absent = 300 s" silently capped
every member node of a template that never asked for it. (b) One knob is easier
to reason about than four, and it is the one that survives a resume. (c) The
shipped `.wcode/workflow.toml` keeps explicit per-node values *and* sets a
whole-run cap, so the reference shows both the backstop and the refinement.

**Risk accepted, and mitigated.** An unset bound means a wedged worker or a hung
script edge is caught only by the whole-run cap — an edge runs synchronously on
the scheduler loop thread, so a hung edge freezes the frontier until that cap
fires. Mitigation: the cap is settable at start, and a capped run is *resumable*
(this record's resume contract), so the cost is a bounded stall, not lost work.
The template therefore sets `[workflow] timeout_secs` explicitly rather than
relying on the absent-is-unbounded default.

**The numbers, so the most likely exit path is documented.** The shipped
template's per-node caps sum to **19,200 s** against its **21,600 s** (6 h) run
cap: a clean pass fits with ~40 min of slack, but **one** `implement`+`verify`
rework adds **7,200 s** and hits the cap. That is accepted — the run exits
non-zero with a `resume:` line and the resume continues from the intact plan — but
if one full rework should fit inside a single launch, **28,800 s (8 h)** is the
next round number for `[workflow] timeout_secs`.

**Old journals resume unbounded.** A group journaled *before* this amendment
carries no per-node `timeout_secs` (the old 300 s default was a const, never
serialized), so resuming it now runs every such node **unbounded** — the run cap
is its only bound. Correct by the new rule, but newly true for pre-existing
groups: set `[workflow] timeout_secs` (or an explicit `--timeout`) before
resuming one you care about.
