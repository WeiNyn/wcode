# D029 — Edges carry condition; a false/SKIP edge yields a terminal `Skipped`

- **Status:** accepted (human sign-off on the W025 brief — rev. 2, all six
  recommendations accepted)
- **Date:** this session
- **Relates to:** [`config.rs`](../../crates/wcode-cli/src/config.rs)
  (`WorkflowEdge`/`EdgeCondition`/`EdgeMode`), [`tasks.rs`](../../crates/wcode-cli/src/tasks.rs)
  (`Dep`, `Task.deps`, `TaskState::Skipped`, `reset_epoch`),
  [`scheduler.rs`](../../crates/wcode-cli/src/scheduler.rs)
  (`EdgeGate`/`edges_admit`/`on_verdict`/`prune_unjudgeable`),
  [`judge.rs`](../../crates/wcode-cli/src/judge.rs) (`Judge`/`ModelJudge`/`parse_verdict`)
- **Amends:** the `[workflow]` template shape in
  [`task-dag-plan.md`](../task-dag-plan.md) §8 and
  [`workflow-task-injection-plan.md`](../workflow-task-injection-plan.md) §12
  (a node's `depends_on` is replaced by `[[workflow.edge]]`)
- **Work item:** [`work/W025-workflow-edges-and-lateral-messaging.md`](../work/W025-workflow-edges-and-lateral-messaging.md)

## Context

A `[workflow]` was a **star the root re-drove in prose**, and its edges were a
node-level `depends_on: Vec<String>` with no way to say *"take this edge only
if …"*. A step that should not run had no structural expression — the model
silently dropped it (the exact bug the DAG exists to fix, `task-dag-plan.md` §1).

Two things were missing: (1) a first-class **edge**, so the graph is
`[[workflow.node]]` + `[[workflow.edge]]` rather than an adjacency list stranded
on each node; and (2) an optional **condition** on an edge, of exactly two kinds
— a synchronous shell test, or an asynchronous LLM judgment.

## Decision

**Edges are first-class.** `Workflow.edges: Vec<WorkflowEdge>` (`from`, `to`,
optional `condition`); `WorkflowNode.depends_on` is removed. One edge per
`(from, to)`; both endpoints must name a node; no self-edge; the graph is acyclic.
`topo_order` walks `edges`. A legacy `depends_on` key on a node is a hard parse
error (`deny_unknown_fields`).

**A condition is exactly one of two kinds** (`EdgeCondition`, flat inline table,
`deny_unknown_fields`, `Serialize` so it rides the journal):

- `condition = { script = "cmd" }` — a shell test run under the same
  `before_tool_call` policy as the `bash` tool. Exit `0` = the edge is taken.
- `condition = { judge = "judger", criterion = "…" }` — an async **one-shot,
  zero-tool** completion by the `judger` member (or, absent one, the root's model
  plus a built-in contract). The reply is parsed for `VERDICT: TAKE|SKIP`
  (`parse_verdict`, first matching line, case-insensitive); `TAKE` takes the
  edge, `SKIP` skips the target.

**A false/SKIP edge yields `TaskState::Skipped`** — a fifth, terminal state.
`Skipped` is **satisfied** for dependents' readiness and **success** for
`terminal_code` (a `Skipped` graph exits `0`); a `reopen` of any ancestor resets
the whole cone (including a `Skipped` node) back to `Todo`. The runtime edge model
is `Task.deps: Vec<Dep>` where `Dep::{Id(u32), Edged{ id, condition }}` is
**untagged**, so a legacy journal line `"deps":[1,2]` still decodes and an
unconditional edge still serializes as a bare id.

**Fail-safe, never a silent prune.** An unparseable judge reply, or a policy
refusal of a `script` edge, **blocks-and-escalates** — the node stays `Todo`, is
recorded `unjudgeable`, and the root gets one `Wake` naming the edge and pointing
at `task op: reset`. `Skipped` is used *only* for a genuine verdict (script exit
≠ 0, or judge `SKIP`). `Task.reset_epoch` is the reset escape hatch: it bumps
unconditionally on `reset`, so a reset of an otherwise-unchanged node is
observable and the scheduler re-arms a fresh judge run.

## Consequences

- `TaskState` has five variants; `ready_ids`, `complete`, and `terminal_code`
  treat `Done | Skipped` alike; the TUI shows `[skipped]` for free via `label()`.
- The judge is a CLI trait (`Judge`) owned by the scheduler, **not** a spawned
  session — a worker's `ReportBack` goes to its owner, not the scheduler, so a
  worker-judge's verdict would never come back. `ModelJudge` is one `stream_fn`
  call with `tools = &[]`; `FakeJudge` injects verdicts in tests (no live model).
- At most **one distinct `judge` name** across a workflow's edges (load
  validation): the scheduler holds one `Arc<dyn Judge>` and `decide` takes no
  member argument.
- A `Skipped` node's artifact is absent, so a dependent that hydrates from it
  runs with fewer inputs — documented, not fixed (a skip prunes a step).
- Pre-existing configs that used `depends_on` must migrate to `[[workflow.edge]]`
  (a loud parse error otherwise). `.wcode/workflow.toml`, `README.md`, and the
  plan docs were migrated in the same change.
