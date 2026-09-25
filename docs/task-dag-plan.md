# wcode — task DAG (Option A: root-as-scheduler)

Status: **planned — locked, not yet built.** This is C3 of
[`swarm-comparison-plan.md`](swarm-comparison-plan.md) (§5/§6), scoped to the
**minimal on-doctrine variant**: the graph lives under the existing star, the
root stays the only dispatcher, and rework is a runtime control action rather
than a graph edge. It does **not** widen the message topology (that is Option B,
left undesigned). Companion to [`team-and-tui-plan.md`](team-and-tui-plan.md)
(what the team is today), [`swarm-comparison-plan.md`](swarm-comparison-plan.md)
(C1/C2/C3), and [`next-steps.md`](next-steps.md) (tracker).

## 1. Problem

Today the root orchestrator sequences work in **prose**: it spawns a worker,
reads the report-back, decides the next step, spawns or re-wakes another. There
is no first-class work *graph*, so:

- **Dependencies are implicit.** Nothing records "B must follow A" — B *is* the
  next thing the model happens to say.
- **No readiness.** The model must remember what has and hasn't completed.
- **Rework is ad-hoc.** A review that rejects ("go fix it and resubmit") is a
  free-form prompt; the model must notice the earlier work is now stale and
  re-drive every downstream step by hand. It routinely misses one, shipping a
  result derived from a superseded artifact.
- **Nothing survives a crash.** The plan (`TaskList`) is in-memory only, so a
  restart loses where the team was.

The DAG fixes exactly this: **the plan becomes a graph the root owns and the
scheduler executes.**

## 2. Philosophy

Four commitments, each matching the code's existing grain:

1. **The graph is data, not a subsystem.** It is the *existing* root-owned
   `TaskList` (`crates/wcode-cli/src/tasks.rs`, `TaskList` at anchor `uDRme`)
   with two additions: **edges** and a **run state**. No new authority, no
   daemon, no separate process. The "scheduler" is a pure function of list state.
2. **The static graph is acyclic — always.** A back-edge `verify → implement`
   would make an SCC and destroy topological scheduling. So "go back" is a
   **runtime control action** (`reopen`), never an edge. Structure only ever
   grows forward; rework happens *inside a node*.
3. **The model proposes; the scheduler disposes.** The root model writes the
   graph and makes *judgment* calls (accept / reject). The scheduler owns the
   *mechanical* facts: readiness, dispatch, hydration, invalidation, the attempt
   cap. The model never has to remember "now wake w2".
4. **The root mediates everything.** Artifacts flow forward and verdicts flow
   back along the *same* edge, both as root-mediated `Wake`s. The topology stays
   the star (`registry.rs` `permitted`, anchor `gztuc`, untouched); bounded
   fan-out stays by construction. **Nothing in `wcode-harness` or
   `wcode-protocol` changes.**

One-line thesis: *a DAG is the root's plan with edges; rework is a state
transition plus a downstream invalidation, bounded by an attempt cap.*

## 3. The model

**Node** = the existing `Task` (anchor `uNP2G`), extended:

```rust
pub struct Task {
    pub id: u32,
    pub title: String,
    pub owner: Option<SessionId>,
    pub state: TaskState,
    // NEW — edges + run memory
    pub deps: Vec<u32>,           // blocked_by: run after these are Done
    pub attempts: u32,            // rework rounds (reopen bumps it)
    pub feedback: Option<String>, // latest reject reason → the next run's input
    pub artifact: Option<String>, // this node's output (hydrates dependents)
    pub gate: bool,               // its reject re-opens `deps`, not itself
    pub run: RunSpec,             // how the node runs
}
```

**States** — four values (anchor `2cvah` plus `Failed`):

```
Todo  ── Doing ──▶ Done ── rework ──▶ Todo   (attempts+1, feedback set)
  ▲                  │
  └── reject/invalidate ┘        Doing ─▶ Failed  (attempt cap hit — terminal)
```

**Derived, never stored** (the key simplification):

- `blocked(n)` = any dep of `n` is not `Done`
- `ready(n)`   = `state(n) == Todo` ∧ `¬blocked(n)`
- invalidation = **`Done → Todo`** — there is no separate `Stale`/`Dirty`
  state. Resetting a descendant to `Todo` makes `blocked`/`ready` recompute
  correctly *for free*.

**How a node runs** — two orthogonal axes (`gate` × `RunSpec`):

```rust
pub enum RunSpec {
    /// A model/team node: a spawned worker session produces the outcome.
    Session { member: Option<String>, /* + inline model/role/tools/… */ },
    /// A physical node: a definable action; the exit code IS the verdict.
    Script  { command: String },
}
```

A node has **exactly one** of `member` / `script` (validated at load; an inline
`model`/`role`/`tools`/`read_only`/`effort`/provider reuses the `WorkerSpec`
fields, anchor `ZV2bh`).

## 4. Execution semantics

**The scheduler** is a root-side task (new `crates/wcode-cli/src/scheduler.rs`)
owned by `Orchestrator` (`agents.rs`, anchor `IueNb`). It:

1. subscribes to `TaskList::subscribe()` (anchor `zNEtE` — the `watch` already
   exists, and the TUI feed at `main.rs` anchor `9Y3Yw` already consumes it);
2. on every change computes `ready_ids()`;
3. for each ready node with an owner: **dispatch** →
   `Registry::deliver(root, owner, Wake { content })` (anchor `TobHl`), then
   flip it `Doing` — single-threaded, so no double-dispatch;
4. the dispatch *content* is **hydrated**: `title` + the `artifact` of each dep
   + `feedback` when `attempts > 0`.

**Who completes a node.** The one place the model stays in the loop: a worker
auto-reports to the root (`ReportBack`, anchor `J8fXd`); the root model reads
the report and calls `task op: complete id` (accept, carrying the artifact) or
`op: reject id reason` (judgment). Acceptance is the model's; *everything
mechanical is the scheduler's*. A `Script` node is the exception — its verdict
is structural and needs no model (§5).

**Termination.** The static graph is a DAG, so a topological order exists; the
run ends when every node is `Done` (or terminal `Failed`). The only unbounded
risk is the rework loop — closed by the cap (§6).

## 5. Gate kinds — judgment vs. physical

The verdict a gate emits is produced **either** by a model judgment **or** by a
script's exit code; the scheduler consumes a `Verdict` and does not care which.
The four combinations:

| `gate` | `RunSpec` | meaning |
|---|---|---|
| `false` | `Session` | a work/team node — produces an artifact; the model accepts |
| `false` | `Script`  | a deterministic step (a build) — exit code sets `Done`/`Failed` |
| `true`  | `Session` | a **judgment gate** — a reviewer worker; the model calls `reject(id, reason)` |
| `true`  | `Script`  | a **physical gate** — a test/lint/schema check; non-zero exit → `reject` automatically |

A **physical gate** ("run a specific script/definable action to verify") reuses
the `bash` executor (`crates/wcode-cli/src/tools/bash.rs` — same spill/limit
machinery) and therefore runs under `BashRiskHooks`, so a definable action stays
inside the doctrine (no approval prompts; policy in code, not config). On a
non-zero exit the scheduler calls `reject(gated, tail(output))` with **no model
in the loop**, and the cone invalidates.

## 6. The rework loop

The case that motivated this doc — a node submits for review, is rejected, and
must go back:

```
[1 explore] ──▶ [2 implement] ──▶ [3 verify  (gate)]
```

| # | actor | action |
|---|---|---|
| 1 | scheduler | 1 ready → wake explorer → `Done(A₁)` |
| 2 | scheduler | 2 ready (dep 1 done) → wake builder with `A₁` → `Done(A₂)` |
| 3 | scheduler | 3 ready → wake verifier with `A₂` |
| 4 | verifier | reports **reject("missing edge case X")** |
| 5 | **model** | `task reject 3 "missing edge case X"` (or, for a physical gate, the script's exit code) |
| 6 | **scheduler** | `reopen(2, reason)`: node 2 → `Todo`, `attempts = 1`, `feedback = reason`; **invalidate the cone `{2,3}` → `Todo`**, drop `A₂` |
| 7 | scheduler | 2 ready again (dep 1 still `Done`) → wake builder with `A₁` **+ the reject reason** → `Done(A₂′)` |
| 8 | scheduler | 3 re-ready → re-verify → **approve** → `Done`; graph complete |

Two invariants make it correct:

- **A gate does not "finish" when it rejects.** A reject is a control signal,
  not a `Done`; the gate is in the cone, so it re-runs.
- **The whole downstream cone is invalidated, not just the predecessor.**
  Anything derived from `A₂` is stale; shipping it is a silent wrong answer.
  `descendants(2)` is reset.

**The cap.** `attempts` counts reworks, so `if attempts(n) > max_attempts`
(= 3) → do **not** reopen: set `n = Failed` and escalate (a `Wake` to the
`n = Failed` and escalate (a `Wake` to the root/human: *"node 2 failed after 3
reworks"*). This is the escape hatch to judgment and the guarantee of
termination. (Option B's forward `fix` node may later merge here as the
*escalation materialization* — out of scope.)

## 7. Persistence & crash recovery

The plan must survive a crash. `TaskList` is in-memory today; the team already
persists via session groups (`session_groups.rs`: a group dir with `root.jsonl`
+ `members/<name>.jsonl` + a manifest). A DAG implies a team, so the graph's
home is that same group dir.

**Design: an append-only NDJSON journal, event-sourced** — the shape of wcode's
session JSONL and the protocol's frames.

`<groupdir>/plan.ndjson`, **one op per line**, recorded on every public mutation:

```
{"op":"create","id":2,"title":"implement","deps":[1],"gate":true}
{"op":"assign","id":2,"owner":"agent:builder"}
{"op":"start","id":2}
{"op":"complete","id":2,"artifact":"…A₂…"}
{"op":"reject","gate":3,"reason":"missing edge case X"}
{"op":"reopen","id":2,"reason":"missing edge case X"}
```

Why a journal, not a snapshot: crash-safe by append+flush (no
read-modify-write race), idiomatic (mirrors `Session`), and it yields the
**rework history for free** — every `reject`/`reopen` is on the tape. A periodic
`{"op":"snapshot","tasks":…}` line bounds replay cost; past a line threshold the
file is rewritten as one `snapshot` (atomic temp+rename).

**One op per public mutation.** Each mutation records exactly one line — a whole
`reject` is a single `{"op":"reject",…}` that **recomputes the cone on replay**,
*not* a fine-grained `reject`/`reopen`/`invalidate` batch (a partial batch on a
crash would replay a *prefix* → a half-invalidated graph; a single line is
atomic). `invalidate` is derived, not an op. Recording happens **before** the
mutation is applied (durable-or-nothing): the journal never lags the in-memory
state.

Replay runs with recording **suppressed** (a `replaying` flag on the sink), so
applying ops through the public methods never re-journals. A `create`'s id is
authoritative from the op — replay inserts it and advances `next_id`, so a
`snapshot` … `create` sequence cannot collide. A truncated trailing line is
dropped on reload (line-oriented, tolerant); a bad line *earlier* in the file is
an error.

The in-memory API and the TUI `watch` are unchanged; the sink is `None` when
there is no group.

**Resume** (`--resume <groupdir>/root.jsonl`, the existing path):

1. Load the manifest → rebuild the team (`rebuild_team`, existing).
2. Replay `plan.ndjson` (last snapshot + tail) → rebuild the `TaskList` with
   states, attempts, feedback, and artifacts.
3. **Reconcile interrupted runs:** any node that was `Doing` at crash → `Todo`
   (its run was partial), journal a `restart` op — **without** bumping
   `attempts` (a crash is not the node's fault); `Done` stays done; artifacts
   rehydrate dependents.
4. The scheduler resumes: compute the ready frontier and re-dispatch. The cap
   bounds a node that keeps crashing.

**Artifacts are stored inline** on `complete` (bounded — a report), so the graph
is self-contained. Per-worker context still lives in `members/<name>.jsonl` and
is restored by the existing group resume — which is *why* a reworked node
**reuses its session**: the worker keeps its context, and the graph keeps its
node state, and both survive the crash independently.

**Orchestrator-only.** A `[workflow]` requires an orchestrator (`--agents`); its
plan lives on that root's `TaskList` and journals to the session group's
`<groupdir>/plan.ndjson` (a fresh root always has a group). A `member` node
additionally requires its `[team]` entry. This keeps the journal's home
unambiguous (there is nothing to journal for a socket client or a served
`--owner` worker — neither has an orchestrator).

## 8. Creating & customizing a graph

Three paths, primary first.

**(a) Dynamic — model-authored at runtime (primary).** The root model builds the
DAG with the `task` tool, exactly as it builds the flat list today
(`tools/task.rs`). New ops: `depends`, `reject` (+reason); `create` gains
`deps`; `list` shows deps/attempts. This is the
doctrine-native path — the plan is authored in the loop, not configured.

**(b) Declarative — a reusable workflow template (the `[team]` precedent).**
`[team]` members are already *data* (`config.rs`, `TeamMember` at anchor
`DDmnB`); a graph template is the same kind of thing — a plan skeleton:

```toml
[workflow]
max_attempts = 3            # rework cap (default 3)

[[workflow.node]]
id = "explore"
member = "explorer"         # references a [[team]] member
[[workflow.node]]
id = "implement"
member = "builder"
depends_on = ["explore"]
[[workflow.node]]
id = "verify"
depends_on = ["implement"]
gate = true
script = "cargo test --workspace"   # physical gate (or `member = "reviewer"`)
```

Instantiated at boot (the `[team]` startup loop is the model — `main.rs`, anchor
`KQsCE`), after the team and before the scheduler. **Validated at load** — ids
unique, every `depends_on` names a sibling, **acyclic** (a topological sort; a
back-edge or self-edge is a loud `ConfigError`), exactly one of `member`/`script`
per node, `member` names a `[team]` member, and a `gate` has at least one dep.
Instantiation creates nodes in topological order, so author order is free.

**(c) Programmatic — a code seam.** For compiled variants (the doctrine answer:
*build the variant instead of configuring one*), a workflow can be a Rust
builder / `Hooks`. The config is sugar over this.

**Customization axes:**

| axis | where | default |
|---|---|---|
| per-node worker (model / role / tools / read_only / effort / provider) | reuse `TeamMember` fields verbatim | inherit |
| edges | `deps` / `depends_on` | none |
| gate vs. work | `gate: bool` | work |
| judgment vs. physical | `member` xor `script` | `member` |
| rework cap | `max_attempts` | 3 |
| reject scope | cone (only cone, for now) | cone |
<!-- on_cap dropped: only `escalate` exists; add when a second policy lands. -->
| reject scope | cone (only cone, for now) | cone |

**Doctrine note.** A workflow *template* is plan **data**, like `[team]` —
defensible. But the *meaning* of a reject (the criterion, who judges) is
**policy** and belongs in a `Hooks` impl, not config. Keep the config thin: it
names the shape, never the behavior.

## 9. Where it lives

| piece | file | change |
|---|---|---|
| edges, `attempts`/`feedback`/`artifact`/`gate`/`run`, `reject`/`reopen`/`ready`/`descendants`, journal sink | `crates/wcode-cli/src/tasks.rs` | extend `Task`/`TaskState`/`TaskList` (`uNP2G`,`2cvah`,`uDRme`) — pure + tests |
| the scheduler loop | **new** `crates/wcode-cli/src/scheduler.rs`; owned by `Orchestrator` (`IueNb`) | subscribe → ready → dispatch + hydrate; reject → invalidate |
| `task` tool ops | `crates/wcode-cli/src/tools/task.rs` | `+depends`, `+reject`, `+instantiate`; `create` takes `deps` |
| workflow schema + validate | `crates/wcode-cli/src/config.rs` | `Workflow`/`WorkflowNode` over `TeamMember` (`DDmnB`); cycle check |
| boot instantiation + TUI feed | `crates/wcode-cli/src/main.rs` | after `[team]` (`KQsCE`); feed (`9Y3Yw`) maps `TaskItem` |
| view fields | `crates/wcode-tui/src/app.rs` | `TaskItem` (`Jqd4E`) gains `deps`/`attempts`/`feedback`; `/tasks` render |
| journal + resume | `crates/wcode-cli/src/session_groups.rs` | `<groupdir>/plan.ndjson`; replay on `--resume` |
| tracker | `docs/next-steps.md` + this doc | |

**No `wcode-harness` change. No `wcode-protocol` change.** The star is intact.

## 10. Phases

- **P0 — data model.** `deps`/`attempts`/`feedback`/`artifact`/`gate`/`run`,
  `reject`/`reopen`/`ready`/`descendants`; `task` ops; tests including a
  **cone-invalidation** test and a **cycle-rejection** test.
- **P1 — scheduler.** subscribe → dispatch ready → hydrate; wired into
  `Orchestrator`; live end-to-end check.
- **P2 — rework loop.** reject → reopen → invalidate cone → re-dispatch; cap=3
  + escalation; tests + live check.
- **P3 — physical gates.** `RunSpec::Script` via the `bash` executor +
  `BashRiskHooks`; automatic verdict.
- **P4 — persistence.** `<groupdir>/plan.ndjson` + snapshot; resume/reconcile.
- **P5 — config & polish.** `[[workflow]]` schema + acyclic validation + boot
  instantiation; `TaskItem` fields + `/tasks`; optional TUI graph view;
  auto-gate criteria beyond exit code.

## 11. Locked decisions

1. **Topology:** root-as-scheduler; star preserved; no `Registry` change.
2. **Static graph acyclic**; rework = `reopen` + transitive-cone invalidation
   (no back-edges).
3. **Node model:** `gate: bool` × `RunSpec::{Session | Script}` — judgment nodes
   (`complete`/`reject` with reason) and physical nodes (exit code = verdict, run
   via `bash` + `BashRiskHooks`).
4. **Model proposes, scheduler disposes**; acceptance is the model's, dispatch /
   invalidation are the scheduler's.
5. **Reuse** the worker session on rework.
6. **Cap:** at most `max_attempts = 3` reworks, then `Failed` + escalate
   (`attempts > max_attempts` — the retry idiom, like `WCODE_RETRY_MAX`).
7. **Persistence:** append-only `<groupdir>/plan.ndjson`, artifacts inline,
   **one op per public mutation** (a `reject` is a single line that recomputes the
   cone on replay), recorded before apply; resume rebuilds the list, `Doing → Todo`
   without bumping attempts.
8. **Orchestrator-only**; a `[workflow]` requires `--agents` (a `member` node, its
   `[team]` entry).
9. **Config:** `[workflow]` is plan *data*; gate *policy* stays in `Hooks`.

## 12. Open questions

- **Typed artifact** (C1 → a real schema): keep the report as prose for now, or
  make the edge payload a struct? (Template-only first; a schema is later.)
- **Auto-gate criteria** beyond exit code (parse a report? a JSON protocol?) —
  deferred to P5.
- **Cap escalation target** — root vs. human-facing notice; both are a `Wake`.
- **Snapshot cadence** for journal compaction — every N ops vs. on group write.
- **`on_cap = fix-node`** — the Option B merge point, if it is ever wanted.

## 13. References

- wcode: `crates/wcode-cli/src/tasks.rs` (`Task`/`TaskList`),
  `crates/wcode-cli/src/tools/task.rs`, `crates/wcode-cli/src/agents.rs`
  (`Orchestrator`, `ReportBack`, `WorkerSpec`),
  `crates/wcode-protocol/src/registry.rs` (`permitted`, `deliver`),
  `crates/wcode-cli/src/session_groups.rs` (the group dir),
  `crates/wcode-cli/src/main.rs` (team startup, TUI task feed),
  `crates/wcode-tui/src/app.rs` (`TaskItem`).
- Plans: [`swarm-comparison-plan.md`](swarm-comparison-plan.md) (C1/C2/C3/C4/C5,
  §5–§8), [`team-and-tui-plan.md`](team-and-tui-plan.md),
  [`session-groups.md`](session-groups.md),
  [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md) §10.1
  (the star), [`gap-analysis-jcode.md`](gap-analysis-jcode.md).
- jcode (reference only): `docs/SWARM_TASK_GRAPH.md` (terminal kinds
  `explore|implement|verify|fix`, the enforced gate, the artifact dataflow).
