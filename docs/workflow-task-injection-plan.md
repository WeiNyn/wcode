# wcode — workflow task injection

Status: **locked (pre-implementation).** This is the design the first-layer
sketch hangs on; it builds on [`task-dag-plan.md`](task-dag-plan.md) (the DAG +
scheduler it parameterizes) and the tracker [`next-steps.md`](next-steps.md).

> The design here is **user-approved and frozen**; the sketch must not redesign
> it. Where a bullet says "non-change" it is deliberate.

## 1. Problem

A `[workflow]` template (`config.rs` `Workflow` `JsABq` / `WorkflowNode` `rmQpz`)
seeds a role DAG at boot (`main.rs::instantiate_workflow` `g5NLM`). But a seeded
node is **contentless**: `instantiate_workflow` uses the node's `id` string as
its task title (`tasks.create(node.id.as_str(), …)`, `main.rs:g0pfv`), so the
graph carries the *roles* (`explore → sketch → implement → verify`) but never
says what the run is *about*. The operator's real job never reaches the plan.

Compounding it, there is **no way to drive a seeded plan headless**:
`one_shot` (`main.rs:6EEYH`) ends after a single root turn and the process exits
(`main.rs:PscEk`); `TaskList` has no completion predicate (explorer #1: "NO
completion predicate anywhere"). So a workflow can only be babysat in the TUI.

## 2. Philosophy

> The template stays generic; the **task is a run input** that parameterizes the
> node *titles*, and a run that seeds a plan can **drive it headless** to a
> terminal status.

Four commitments, each matching the code's grain:

1. **The task is input, not plan state.** `--task`/`WCODE_TASK` is scoped to an
   invocation like a flag, not stored or journaled in `TaskList` (contrast the
   `working_dir`/`max_attempts` precedent).
2. **Injection is one dumb substitution.** `{{task}}` in a node **title** is
   replaced; nothing else is templated.
3. **The model still proposes, the scheduler still disposes** (task-dag §2). The
   headless driver only *waits* and reads state; it adds no authority.
4. **`-p` is not overloaded.** A distinct carrier (`--task`) selects a distinct
   path (headless); `-p`, the REPL, and the TUI are untouched.

## 3. Model

### 3.1 Carrier

| source | field | meaning |
|---|---|---|
| `--task <text>` | `Args.task: Option<String>` (`main.rs:UWthZ`) | the run's task |
| `WCODE_TASK` env | — | fallback only when `--task` is absent |
| `--timeout <secs>` | `Args.timeout: Option<u64>` | cap the headless wait; `0`/absent = disabled |

`WCODE_TASK` is read **directly** (`std::env::var("WCODE_TASK").ok()`, mirroring
the `WCODE_CONFIG` read at `main.rs:Naj2q`) — **not** the `EnvVars` provider
struct, because it is an invocation input, not provider config. Resolution
happens in `main` before `build_runtime`; a whitespace-only env value counts as
unset.

### 3.2 Template

`WorkflowNode` (`config.rs:rmQpz`) gains **one** field:

```rust
/// Optional display title for the node. `{{task}}` is substituted with the
/// run's task. Absent → the node's `id` is used (back-compat).
#[serde(default)]
pub title: Option<String>,
```

- **Placeholder** = `{{task}}` (double braces), recognized **in titles only**.
- **Unknown brace tokens are left literal, NOT an error** (user decision). So
  `title = "merge {x} {{task}}"` with task `T` → `merge {x} T`.
- **Scripts are out of scope for v1** — a `{{task}}` in `script` is left
  literal, unsubstituted.

`impl Workflow` gains:

```rust
/// `true` iff any node title contains the `{{task}}` placeholder.
pub fn uses_task(&self) -> bool
```

and a pure substitution helper:

```rust
/// Replace every `{{task}}` with `task`. Pure; no other brace form is special.
pub fn substitute_task(text: &str, task: &str) -> String

/// The resolved title of a node: its `title` (with `{{task}}` substituted when a
/// task is present) else its `id`. Pure; `instantiate_workflow` calls it.
pub fn node_title(node: &WorkflowNode, task: Option<&str>) -> String
```

**`validate_workflow` (`config.rs:3ZuuU`) is explicitly UNCHANGED.** It never
inspects titles, and it must **not** learn to reject unknown braces — leaving
them literal is the locked behavior. `uses_task()`-without-a-task is a *boot*
error (a guard in `main`, §4.5), not a `ConfigError`.

### 3.3 Where the resolved title flows

`instantiate_workflow` creates each node with `node_title(node, task)` as the
`title` (instead of `node.id`). Everything downstream is **unchanged**:
`dispatch_content` (`scheduler.rs:fd3M2`) already renders `#<id> <title>`, so the
resolved title surfaces with no scheduler change.

## 4. Semantics

### 4.1 CLI parsing

`parse_args` (`main.rs:hT3hY`) gains two value arms (pattern: the `-p` arm
`bcdNP`):

```rust
"--task" => {
    a.task = Some(args.get(i).ok_or("--task requires text")?.clone());
    i += 1;
}
"--timeout" => {
    let v = args.get(i).ok_or("--timeout requires seconds")?;
    a.timeout = Some(v.parse::<u64>()
        .map_err(|_| format!("--timeout expects seconds, got `{v}`"))?);
    i += 1;
}
```

`USAGE` (`main.rs:Yy37H`) gains the two flags plus a `WCODE_TASK` env line.

### 4.2 Env resolution

In `main`, after `let args = parse_cli()` (`main.rs:Ds9gl`) and before
`build_runtime` — `args` becomes `let mut args`:

```rust
if args.task.is_none() {
    args.task = std::env::var("WCODE_TASK").ok().filter(|s| !s.trim().is_empty());
}
```

### 4.3 Instantiation

`instantiate_workflow` (`main.rs:g5NLM`) gains a `task: Option<&str>` param; the
`create` title (`main.rs:g0pfv`) becomes `node_title(node, task)`. The call site
(`main.rs:GbCBX`) passes `args.task.as_deref()`.

### 4.4 Dispatch branch

`dispatch` (`main.rs:irx6G`) today is `match &args.prompt { Some => one-shot;
None => TUI/REPL }`. Add a middle branch (refactor the two-arm `match` to
`if let Some(prompt) = &args.prompt { … } else if args.task.is_some() { … } else
{ … }` — the `Some` arm is unchanged, the final `else` is the current `None`
arm):

```text
Some(prompt)  → one_shot          (unchanged, main.rs:xjeiL)
--task        → run_workflow      (new headless branch)
otherwise     → choose_tui / REPL (unchanged)
```

`choose_tui` (`main.rs:ZaFjn`) also returns `false` when `args.task.is_some()`
(task ⇒ headless). The real TUI branch is not taken regardless, but this keeps
`choose_tui` honest as the single "is this interactive?" decision.

### 4.5 Guards (in the `build_runtime` guard block, near `main.rs:EWnWi`)

Follow the existing `eprintln!("error: …"); std::process::exit(2)` shape:

| condition | error |
|---|---|
| `--task` without `[workflow]` | `--task requires [workflow]` |
| `--task` without `--agents` | `--task requires --agents` |
| `--task` with `-p` | `--task cannot be combined with -p` |
| `--timeout` without `--task` | `--timeout requires --task` |
| `cfg.workflow.uses_task() && args.task.is_none()` | `[workflow] uses {{task}} but no --task/WCODE_TASK was given` |

### 4.6 Headless run-to-completion (the crux)

**The mismatch.** `one_shot`'s contract is one turn. A workflow needs many root
turns — one per worker `ReportBack` (`agents.rs:SeerE` → `Wake` → a root turn,
`agents.rs:xe9md`). The process must stay alive to accept/reject as those reports
land.

**`run_workflow`** (sketched near `one_shot` at `main.rs:6EEYH`):

```rust
/// The headless `--task` run. The plan is already seeded (P5 instantiation);
/// this spawns the root, seeds it, then waits for the plan to reach a terminal
/// state. Exit: 0 all-Done, 1 a Failed node (fail-fast) or timeout.
async fn run_workflow(
    backend: Backend,
    tasks: crate::tasks::TaskList,
    seed: String,
    timeout: Option<std::time::Duration>,
) -> i32
```

Body contract:

1. Spawn the drain (borrow `one_shot`'s drain, `main.rs:TTtdK`) so the
   subscription stays live across turns; **do not join** it.
2. `Submit` the seed text to the root (`backend.ask(Request::Submit { … })`,
   `main.rs:mAZx6`) so the root runs its first turn.
3. `let mut updates = tasks.subscribe();` (`tasks.rs:zNEtE`).
4. Loop, reading `tasks.snapshot()` (`tasks.rs:SCzgm`) via the pure predicate
   `terminal_code()` (§4.7): `Some(code)` → break; `None` → wait on
   `updates.changed()` or, when `timeout` is `Some`, a `tokio::select!` sleep.
5. Print a per-node summary (`#id [state] title`), then `flush_all`
   (`main.rs:3dbBW`) + `Background::shutdown_all` (`main.rs:QETJA`) +
   `process::exit(code)` (`main.rs:PscEk`).

**Seed text** (`fn workflow_seed(task: &str) -> String`): it must tell the root
that the plan is **already instantiated** and that its job is to `task complete`
/ `task reject` per report — **not** to author a duplicate plan. Draft:

```text
The task is:
<task>

A `[workflow]` plan is ALREADY instantiated (see `task list`). Do NOT create
nodes — the scheduler dispatches each ready node to its member. As each worker
reports, call `task complete <id>` with the report as `artifact`; for a gate,
call `task reject <id> <reason>` with the reason. Stop when no node is ready and
none is running.
```

### 4.7 The completion predicate (new, `tasks.rs`)

There is none today. Add one pure, testable method (read-only, like
`ready_ids` `tasks.rs:rjJsV`):

```rust
/// The headless driver's terminal verdict. `Some(0)` once every node is `Done`;
/// `Some(1)` as soon as any node is `Failed` (`tasks.rs:3IW7R`) — FAIL-FAST,
/// because a `Failed` node's blocked dependents can never progress, so waiting
/// for "all terminal" would hang. `None` = still running. Pure.
pub fn terminal_code(&self) -> Option<i32>
```

## 5. Where it lives

| piece | file | change |
|---|---|---|
| `WorkflowNode.title`, `Workflow::uses_task`, `substitute_task`, `node_title` | `crates/wcode-cli/src/config.rs` (`rmQpz`, within `JsABq`) | +1 field, +3 fns; `validate_workflow` (`3ZuuU`) **untouched** |
| `Args.task`/`Args.timeout`, parse arms, USAGE, env, guards, dispatch branch, `choose_tui`, `instantiate_workflow(task)`, `run_workflow`, `workflow_seed` | `crates/wcode-cli/src/main.rs` (`UWthZ`,`hT3hY`,`Yy37H`,`Naj2q`,`EWnWi`,`g5NLM`,`GbCBX`,`irx6G`,`ZaFjn`,`6EEYH`) | additive, except the `match`→`if/else if` and `let mut args` |
| `terminal_code` | `crates/wcode-cli/src/tasks.rs` | +1 pure method + tests |

**No `wcode-harness` change. No `wcode-protocol` change. No `scheduler.rs`
change.** `dispatch_content` needs nothing — the title already carries the task.

## 6. Phases

- **P0 — schema + substitution.** `WorkflowNode.title`, `uses_task`,
  `substitute_task`, `node_title`; tests (substitute; unknown braces literal;
  title fallback to `id`; a TOML round-trip with `title = "… {{task}}"`).
- **P1 — CLI + guards.** `Args.task`/`timeout`, parse arms, USAGE, `WCODE_TASK`
  resolution, the five guards; tests (`resolve`/parse, each guard).
- **P2 — instantiation.** `instantiate_workflow(task)` + call site; test: a
  `{{task}}` title resolves, an absent title falls back to `id`.
- **P3 — headless.** `terminal_code` (truth-table test) + `run_workflow` +
  `workflow_seed` + the dispatch branch + `choose_tui`; a **live check** against
  a keyless endpoint (`--base-url http://localhost:11434/v1`) driving a small
  `[workflow]` to `Done` and exiting 0, and a forced-`Failed` path exiting 1.

## 7. Locked decisions

1. Carrier = `--task <text>` + `WCODE_TASK` (direct `std::env::var`, `Naj2q`);
   `-p` is NOT the carrier and is not overloaded.
2. Placeholder = `{{task}}`, **titles only**; unknown braces left literal.
3. `WorkflowNode.title: Option<String>`; absent → `id` (back-compat `g0pfv`).
4. The root is also seeded with the task (judgment gates know the objective).
5. Headless (`--task` ⇒ no TUI); optional `--timeout <secs>` (0/absent = off).
6. Completion = all `Done` → 0; any `Failed` → 1 (fail-fast); timeout → 1.
7. `validate_workflow` unchanged; scripts and the scheduler untouched.
8. v1 is headless only — TUI-watch deferred.

## 8. Open questions

- **TUI-watch** (a live plan view during a `--task` run) — explicitly deferred.
- **Seed obedience** — nothing stops the root from calling `task create` and
  duplicating the seeded graph (see risk below); a `Hooks` policy could block it.
- **Rework under headless** — a judgment gate's `reject` re-opens the cone and
  the scheduler re-dispatches; the driver simply keeps waiting. The `--timeout`
  bounds a rework storm; an explicit cap surfaced to the driver is later.
- **Exit code nuance** — a `Failed` node while a sibling is still `Done`-able:
  fail-fast returns 1 immediately (locked), leaving the plan partially run.

> **Settled.** The open questions above — and the exit-ownership, root-error,
> and drain edges they touch — are resolved in **§11** below. §11 is binding on
> the developer; where it conflicts with §4.6, §11 wins.

## 9. Friction / where the design fights the code

- **`match &args.prompt` is two-arm** (`irx6G`/`xjeiL`); a third branch means
  refactoring it to `if/else if/else`. The final `else` must reproduce the
  current `None` arm byte-for-byte.
- **`args` is immutable** (`let args = parse_cli();`, `Ds9gl`); env resolution
  into `args.task` requires `let mut args`.
- **No completion predicate exists** (`TaskList`) — the headless loop is blocked
  on `terminal_code` (§4.7). This is the single new load-bearing method.
- **The root can disobey the seed** and author a duplicate plan (§8) — a
  behavioral risk with no current guard.
- **`choose_tui` already short-circuits on `args.prompt.is_some()`** (`zjJoA`);
  add `args.task.is_some()` so "is this interactive?" stays a single decision.

## 10. References

- wcode: `crates/wcode-cli/src/config.rs` (`Workflow` `JsABq`, `WorkflowNode`
  `rmQpz`, `validate_workflow` `3ZuuU`, `topo_order` `LfizH`),
  `crates/wcode-cli/src/main.rs` (`Args` `UWthZ`, `parse_args` `hT3hY`,
  `USAGE` `Yy37H`, `WCODE_CONFIG` read `Naj2q`, `let args` `Ds9gl`, guards
  `EWnWi`, `instantiate_workflow` `g5NLM`, `g0pfv`, call site `GbCBX`, `dispatch`
  `irx6G`, one-shot arm `xjeiL`, `one_shot` `6EEYH`/drain `TTtdK`/submit `mAZx6`,
  exit `PscEk`, `choose_tui` `ZaFjn`/`zjJoA`),
  `crates/wcode-cli/src/tasks.rs` (`ready_ids` `rjJsV`, `subscribe` `zNEtE`,
  `snapshot` `SCzgm`, `TaskState::Failed` `3IW7R`),
  `crates/wcode-cli/src/scheduler.rs` (`dispatch_content` `fd3M2`),
  `crates/wcode-cli/src/agents.rs` (`ReportBack` `ZSMjE`, `after_run` `SeerE`,
  `xe9md`).
- Plans: [`task-dag-plan.md`](task-dag-plan.md) (§2, §4, §9), and the tracker
  [`next-steps.md`](next-steps.md).

## 11. First-layer review — binding amendments

The design body (§1–§10) is frozen; this section **settles the §8 open
questions** and reconciles two internal inconsistencies. It is binding on the
developer. Anchors below were re-verified against the working tree.

**A1 — `run_workflow` exit ownership (reconciles §4.6 step 5 ↔ its `-> i32`).**
§4.6 step 5 puts `flush_all` + `Background::shutdown_all` + `process::exit`
*inside* `run_workflow`, yet the signature returns `i32`. Adopt the sketch's
split — it matches the `-p`/`one_shot` arm (`main.rs:6EEYH`): `run_workflow`
prints the per-node summary and **returns** `code`; the dispatch branch does
`flush_all` (`3dbBW`) + `shutdown_all` (`QETJA`) + `process::exit(code)`
(`PscEk`). §4.6 step 5's flush/exit sentence is superseded.

**A2 — the root's first turn must not hang the driver.**
§4.6 step 2 fires `backend.ask(Request::Submit{..})` and discards the reply. If
that turn ends `Stopped{StopReason::Error}` (`main.rs:NXDx7`) or `MaxTurns`
(`main.rs:1LCGn`) — or the session closed (`Err`, `main.rs:uOwHQ`) — no node
transitions, `terminal_code()` stays `None`, and with no `--timeout` the loop
waits forever, violating §2 ("drive it headless to a terminal status").
**Required:** match the first reply exactly as `one_shot` does and `return 1` on
any of those three; the locked exit-code policy already says "1 … or timeout".

**A3 — the drain is whole-run, not `one_shot`'s.**
§4.6 step 1 says "borrow `one_shot`'s drain (`TTtdK`) … the subscription stays
live across turns", but `TTtdK` breaks on the first `AgentEvent::AgentEnd`
(`main.rs:b6g02`), so it dies after turn 1. **Required:** a drain that consumes
for the whole run — `tokio::spawn(async move { while rx.recv().await.is_ok() {} })`.
(It is non-load-bearing anyway: `ask` replies via a `oneshot` (`actor.rs`
`question`) and a `broadcast` with no live reader never blocks the actor; the
loop is kept only to honour §4.6 step 1.)

**A4 — `backend` is not `mut`; `run_workflow` inputs.**
`Backend::ask` is `&self` (`crates/wcode-protocol/src/backend.rs:lLhEm`), so
`run_workflow(backend: Backend, tasks: TaskList, seed: String, timeout: Option<Duration>) -> i32`
needs no `mut backend`. The loop reads the verdict from `tasks.terminal_code()`
and waits on the receiver from `tasks.subscribe()` (`zNEtE`); `snapshot()`
(`SCzgm`) feeds the per-node summary only.

**A5 — `terminal_code` shape (settles §4.7 + reviewer question (c)).**
Stays a **method on `TaskList`**, `pub fn terminal_code(&self) -> Option<i32>` —
the same shape as `ready_ids` (`rjJsV`) — not a free `&[Task]` fn, so the loop
calls it without locking. Fail-fast is **correct**: a `Failed` dep is not `Done`,
and `ready_ids` gates each candidate on `deps.iter().all(|d| … == TaskState::Done)`
(`tasks.rs:XWHDT`/`gx5lb`), so a `Failed` node's dependents can never become
ready and "wait for all terminal" would hang. `Some(0)` = every node `Done`,
`Some(1)` = any `Failed` (`5JNbO`), else `None`. No `reset`/`restart` is ever
issued on the headless path, so nothing revives a `Failed` node.

**A6 — existing code the signature/field changes break (compile-forced).**
- `main.rs:rjoHP` — the `GbCBX` call site gains `args.task.as_deref()` (`args`
  is in scope inside `build_runtime`).
- `main.rs:VcrIm` — the existing test `SbUuc`
  (`instantiate_workflow_handles_a_later_authored_dependency`) calls
  `instantiate_workflow(&tasks, &w)`; it must pass `None` as the 3rd arg.
- `main.rs:4wrui` / `DhkYX` — the two `WorkflowNode { … }` literals must add
  `title: None`.
New tests go under distinct names.

**A7 — tracker.** Add an item to `docs/next-steps.md` pointing at this doc with
status (AGENTS.md convention).

**A8 — anchor corrections.** `TaskState::Failed` is `tasks.rs:5JNbO` (the doc
cites `3IW7R`, its doc-comment). USAGE is `main.rs:Yy37H` — the sketcher's
correction; the doc already uses `Yy37H`, not `mcTQ4`. `ReportBack` is
`agents.rs:447` (`ZSMjE` in §10 is a doc-comment line). Every anchor in §3–§5
otherwise resolves; `dispatch`'s `irx6G` is the `match &args.prompt` line (the
`fn` itself is `9vnyJ`), the intended target of the §4.4 refactor.

**A9 — `workflow_seed` (settles reviewer question (a)).** Final text — the root
is told the plan already exists and its one job is `task complete`/`task reject`
per report; it is *not* told how to plan (obedience stays a soft guarantee, §8):

```text
The task is:
<task>

A `[workflow]` plan is ALREADY instantiated (see `task list`). Do NOT create
nodes — the scheduler dispatches each ready node to its member. As each worker
reports, call `task complete <id>` with the report as `artifact`; for a gate,
call `task reject <id> <reason>` with the reason. Stop when no node is ready and
none is running.
```

**A10 — scheduler unchanged (settles reviewer question (e)).** Confirmed:
`dispatch_content` (`scheduler.rs:fd3M2`) renders `format!("#{} {}", task.id,
        task.title)` and the title is resolved at `create` time, so
`crates/wcode-cli/src/scheduler.rs` needs no change.

## Appendix — worked example

`.wcode/workflow.toml`:

```toml
[workflow]
[[workflow.node]]
id = "explore"
member = "explorer"
title = "Explore: {{task}}"
[[workflow.node]]
id = "implement"
member = "developer"
depends_on = ["explore"]
title = "Implement: {{task}}"
```

Run:

```sh
wcode --agents --config .wcode/workflow.toml --task "fix bug 123" --timeout 600
```

The `explore` node resolves to title `Explore: fix bug 123` (task id `1`), so
`dispatch_content` (`scheduler.rs:fd3M2`) emits a Wake whose first line is:

```text
#1 Explore: fix bug 123
```

The `implement` node (#2) dispatches after #1 completes, hydrated with #1's
`artifact`, and its title is `Implement: fix bug 123`. Exit `0` when both are
`Done`; exit `1` if any node fails or the 600 s cap elapses.
