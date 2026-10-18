# W025 — Workflow graph as nodes + conditional edges, and node-to-node (lateral) talk — brief

- **Status:** **IMPLEMENTED — second-layer review pending.** Design was APPROVED
  (rev. 2; all six §0 recommendations signed off). Landed in commits `1a80f1d`
  (config edges) · `fa128d1` (runtime `Dep`/`Skipped`/`reset_epoch`) · `1fd5c03`
  (`Registry::allow_pair` + widened `permitted`) · `b9abb48` (the judge) ·
  `6ed6296` (scheduler edge conditions) · `dd405ed` (adjacency + wiring) + the
  migration/docs commit. `cargo test --workspace` + `cargo clippy --workspace
  --all-targets` clean; script-condition skip live-verified. The D029/D030
  decision records and the tracker row (item 83) record the outcome.
- **Work item:** W025 (`docs/work/W025-workflow-edges-and-lateral-messaging.md`)
- **Decision records:** **D029** (conditional edges + `TaskState::Skipped`) and
  **D030** (workflow-scoped lateral messaging; amends
  [`task-dag-plan.md`](../task-dag-plan.md) §11 #1 *"star preserved; no `Registry`
  change"*).
- **Author:** brainstormer (read-only brief; recorded by the orchestrator)
- **Revision read:** `1bcbcb8` (tree carries an unrelated ` M .wcode/team.toml`)
- **Supersedes:** the rev-1 draft's open questions 4–5 (now settled: first-class
  edges, two condition kinds).

## §0 Human sign-off amendments

### Settled

| # | decision | source |
|---|----------|--------|
| 1 | Communication scope = **edge adjacency**, not an open mesh | human |
| 2 | Permission = **widen `Registry::permitted`** with an explicit allow-set | human |
| 3 | Delivery = **direct `deliver`** (registry seam; no root relay) | human |
| 4 | Graph = **`[[workflow.node]]` + `[[workflow.edge]]`**; **drop `depends_on`** (migrate `.wcode/workflow.toml`, tests, docs) | human |
| 5 | Conditions = **two kinds**: `script` (sync, shell, exit 0) **+** `judge` (async, a dedicated **no-tool `judger`** member; verdict **parsed** from the reply) | human |
| 6 | **One work item** (W025) | human |
| 7 | **Two decision records** (D029, D030) | human (delegated) |

### Rev-2 — approved by the human (all six recommendations accepted)

1. **Judge fallback: skip vs block-and-escalate.** *Rec:* **block-and-escalate** —
   a silent SKIP is invisible pruning; blocking never runs the guarded step, is
   visible, and `--timeout` bounds a headless stall. — **APPROVED**
2. **TOML shape: flat inline table vs internally-tagged `kind`.** *Rec:* **flat**
   `condition = { script = … }` / `{ judge = …, criterion = … }` with
   exactly-one validation. — **APPROVED**
3. **`judger` origin.** *Rec:* a normal `.wcode/agents/judger.md` (or `[[team]]`),
   executed as a one-shot completion; absent → the root's model + a built-in
   contract. — **APPROVED**
4. **Runtime dep model: `Task.deps: Vec<Dep>` vs a parallel conditions list.**
   *Rec:* **`Vec<Dep>` (untagged)** — one source of truth; the legacy journal line
   still decodes. — **APPROVED**
5. **Edge conditions AND-ed vs OR-ed.** *Rec:* **AND** (matches the DAG's
   blocked-by model; OR is a later model). — **APPROVED**
6. **Adjacency scope: edge-joined only vs full mesh.** *Rec:* **edge-joined only**
   (per sign-off #1). — **APPROVED**

## §1 The ask

"In wcode, workflow mode still based on star model, we will need to improve it to
allow agent (step/node) can talk to each other, and the workflow definition is
node and edge where edge can have condition."

Two intertwined changes over the shipped star + DAG:

- **(A)** today nodes can only talk to the root; let edge-joined nodes talk to
  each other.
- **(B)** an edge is today an unconditional `depends_on: Vec<String>`; make the
  workflow a node/edge graph where **an edge carries a condition** (a shell test
  or an LLM judge).

Problem it solves: a workflow is a role sequence the root re-drives in prose, and
a step that should not run has no structural way to be skipped — the model
silently drops it.

## §2 Scope / Non-scope

**In scope.**

- **B1 — edges become first-class config.** `[[workflow.edge]]` with `from`,
  `to`, optional `condition`; `WorkflowNode.depends_on` **removed**; one edge per
  `(from,to)`; edges need no `id`.
- **B2 — condition kinds.** `condition = { script = "cmd" }` (sync; exit 0 =
  taken) and `condition = { judge = "judger", criterion = "…" }` (async; verdict
  parsed).
- **B3 — `TaskState::Skipped`** (terminal; satisfied for readiness; success for
  `terminal_code`; cone re-`Todo`).
- **B4 — runtime edge model:** `Task.deps: Vec<Dep>` where
  `Dep { id, condition }`; the journal accepts both the legacy bare-number and the
  new table form.
- **A1 — `Registry::allow_pair` + a widened `permitted`**; adjacency populated by
  the workflow spawn path.
- **A2 — a worker may address a peer as `agent:<id>`** once permitted
  (`address()` already supports it).
- **Migration** of `.wcode/workflow.toml`, `README.md`, the plan docs, and every
  test that uses `depends_on`.

**Non-scope.**

- **A full mesh / "group" mode** (shared context, any-member-to-any-member) —
  reserved by `interface-protocol-brainstorm.md:520-524` (decision #16); A1 is
  *edge-defined* adjacency only.
- **Widening the star for a plain `[team]`** — non-workflow sessions keep today's
  exact `owner`-edge-only `permitted`; the DAG's bounded fan-out stays true when
  no workflow is seeded.
- **A predicate DSL / JSON-protocol condition / model-judgment *at the node*** —
  conditions are `script` or a guarded `judge` only.
- **`judge` as a spawned session** — the judge is a one-shot, no-tool completion,
  never a plan node, never a session (§4.3).
- **Conditional edges authored at runtime by the root model** (`task create`/
  `depends` stay unconditional in v1); **TUI rendering of conditions** beyond a
  state label.
- **`OR` semantics** — multiple incoming edges are **AND**-ed (all must be taken),
  matching the DAG's blocked-by model.

## §3 Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| B1 | Config: `[[workflow.edge]]` + `EdgeCondition` + validation + `topo_order` | high | medium | low | The graph shape the human named; contained in `config.rs`; load-time validation is the existing idiom. |
| B4 | `Task.deps: Vec<Dep>` + journal back-compat | high | high | medium | One source of truth, but ripples through `ready_ids`/`complete`/`dispatch_content`/TUI/journal. |
| B3 | `Skipped` state | medium | medium | medium | Fans out through readiness, `terminal_code`, `complete`'s dep check, the journal, the TUI. |
| B2s | `script` condition (sync, `BashRiskHooks`) | high | low | medium | Reuses the `Script` node path (`scheduler.rs:226`); a wrong predicate silently prunes a node. |
| B2j | `judge` seam + verdict contract + fallback | high | high | high | New async seam, a parse contract, a fail-safe decision; the crux this brief settles (§4.3). |
| A1 | `allow_pair` + widened `permitted` | high | medium | high | Amends a locked decision; rewrites `lateral_message_between_workers_is_refused`; touches `wcode-protocol`. |
| M | Migration of config/docs/tests | medium | medium | low | Mechanical but wide; a missed test is a compile error, not a silent break. |

## §4 Interface & structure

### 4.1 Config (B1, B2)

**Added** `crates/wcode-cli/src/config.rs`:

```rust
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub condition: Option<EdgeCondition>,
}

/// Exactly one mode: `script`, or `judge` + `criterion`. Validated at load.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
pub struct EdgeCondition {
    #[serde(default)] pub script: Option<String>,
    #[serde(default)] pub judge: Option<String>,
    #[serde(default)] pub criterion: Option<String>,
}
```

**TOML shape (recommended — flat inline table, tag by key):**

```toml
[[workflow.edge]]
from = "sketch"
to = "implement"

[[workflow.edge]]
from = "implement"
to = "verify"
condition = { script = "test -f docs/sketch.md" }

[[workflow.edge]]
from = "verify"
to = "publish"
condition = { judge = "judger", criterion = "Did the diff pass review and the tests?" }
```

**Changed** `config.rs:246` `WorkflowNode` — **drop** `pub depends_on: Vec<String>,`
(`config.rs:260`); `gate` stays (`config.rs:263`). `Workflow` (`config.rs:235`)
gains `#[serde(rename = "edge", default)] pub edges: Vec<WorkflowEdge>`.

**Changed** `config.rs:853` `validate_workflow_structure`:

- keep unique ids, member-xor-script (`config.rs:867`), gate-needs-≥1-incoming-edge;
- every `edge.from`/`edge.to` names a node id; no self-edge; **reject a duplicate
  `(from,to)` pair**;
- a `script` mode needs a non-empty `script`; a `judge` mode needs non-empty
  `judge` **and** `criterion`; exactly one mode; unknown keys are an error (loud,
  D14);
- `topo_order` (`config.rs:933`) walks `edges` (indegree per `to`).
- `validate_workflow_members` (`config.rs:913`) is unchanged.

> Overlays **replace arrays wholesale** (`config.rs:2032
> overlay_replaces_arrays_wholesale`), so a `[workflow]` overlay replaces the whole
> `nodes` + `edges` set — no concatenation to add.

### 4.2 Runtime task model (B3, B4)

**Added** `crates/wcode-cli/src/tasks.rs`:

```rust
/// One blocked-by edge: the dep id plus its optional edge condition.
/// Untagged so a legacy journal line `deps:[1,2]` still decodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dep {
    Id(u32),
    Edged { id: u32, #[serde(default)] condition: Option<EdgeCondition> },
}
impl Dep { pub fn id(&self) -> u32 { match self { Dep::Id(i) => *i, Dep::Edged { id, .. } => *id } } }
```

**Changed** `tasks.rs:105` `Task` — `pub deps: Vec<u32>` → `pub deps: Vec<Dep>`.
Add `TaskState::Skipped` at `tasks.rs:44`:

```rust
pub enum TaskState { Todo, Doing, Done, Failed, Skipped }
```

**Changed** the predicates:

- `tasks.rs:882 ready_ids`: `x.state == TaskState::Done` →
  `matches!(x.state, TaskState::Done | TaskState::Skipped)`.
- `tasks.rs:909 terminal_code`: `all(|t| t.state == Done)` →
  `all(|t| matches!(t.state, Done | Skipped))`.
- `tasks.rs:697 complete`: the dep check `is_some_and(|t| t.state ==
  TaskState::Done)` → `Done | Skipped`.
- `tasks.rs:736 reopen_inner`: already resets every cone node to `Todo`
  unconditionally — no change; a **`skip` op** is added:
  `pub fn skip(&self, id: u32) -> Result<(), String>` + `PlanOp::Skip { id }`,
  mirroring `fail` (`tasks.rs:811`).
- `PlanOp::Create.deps: Vec<Dep>` (`tasks.rs:160`) — a legacy `[1,2]` still decodes
  via the untagged `Dep`; unstructured edges still serialize as `[1,2]` bytes.
- `main.rs:2543 task_item`: `deps: task.deps.iter().map(|d| d.id()).collect()`.

**Semantics (settled):** a node whose incoming condition is false is itself
`Skipped`; a `Skipped` node counts as *satisfied* for its dependents (they still
become ready). Hydration already skips absent artifacts (`scheduler.rs:338
dispatch_content` filters `dep.artifact`), so a dependent runs with fewer inputs —
documented consequence.

### 4.3 The judge seam — **recommendation: (i) a scheduler-side one-shot judge (`JudgeFn`)**

| | non-blocking | testable w/o a live model | judge is no-tool | on-doctrine |
|---|---|---|---|---|
| **(i) `JudgeFn` seam** | yes (spawn + park) | **yes** (inject a fake) | **yes** (zero tools by construction) | **yes** (a CLI trait next to the state, like `verify_gate.rs`/`ReportBack`) |
| (ii) judged pseudo-node | yes | no — needs a live actor + a parse seam | no — a session keeps `message` (D3) | partly — pollutes the plan/TUI with a synthetic node |
| (iii) re-pointed worker owner | no — the report lands at an owner, not the scheduler | hardest — needs re-pointed ownership | no | no — mutates the star's ownership contract |

**(i) wins.** A worker's `ReportBack` (`agents.rs:493`) forwards its reply to its
**owner** (`deliver(&self.me, &self.owner, Request::Wake …)`), so a worker-judge's
verdict would never reach the scheduler — (iii) is exactly that hole. (ii)
compiles the judge into a plan node that must then be parsed *and* gated, doubling
the seam. (i) is a single trait the scheduler owns, and its default is one
completion.

**Added** `crates/wcode-cli/src/judge.rs`:

```rust
/// A judge decides whether an edge is taken. `None` = unparseable (fallback).
pub trait Judge: Send + Sync {
    fn decide<'a>(
        &'a self,
        criterion: &'a str,
        upstream: Option<&'a str>,           // the source dep's artifact
    ) -> Pin<Box<dyn Future<Output = Option<bool>> + Send + 'a>>;
}

pub struct ModelJudge { llm: LlmOpts, stream_fn: StreamFn, system: String }
```

- `ModelJudge::decide` builds `sys` = the judge contract + the member's role,
  `ctx = [one user message: criterion + upstream artifact]`, `tools = &[]`,
  `opts = llm`, calls `stream_fn` (`streamfn.rs:285 pub type StreamFn = …`),
  collects `LlmStreamEvent::TextDelta` (`event.rs:6`), and returns
  `parse_verdict(&text)`.
- **Verdict contract** (crisp, parseable):

  ```
  You are a workflow-edge judge. You have no tools. Answer only with:
  VERDICT: TAKE
  REASON: <one short line>
  or VERDICT: SKIP / REASON: …
  ```

  `parse_verdict` = first line matching `(?i)^\s*VERDICT:\s*(TAKE|SKIP)\b`;
  `Some(true)`/`Some(false)`; no match → `None`. Pure, unit-tested
  (markdown-wrapped, multi-line, both-tokens, empty).
- **Unparseable-reply fallback = block-and-escalate** (recommended). The node stays
  `Todo`, the edge is recorded in a scheduler-local `unjudgeable` set (no tight
  re-judge loop), and `escalate` (`scheduler.rs:177`) sends one `Wake` naming the
  edge + a reply tail. *Why:* a silent SKIP prunes work invisibly — the exact bug
  the DAG fixes (`task-dag-plan.md` §1); blocking never runs the guarded step, so
  it is at least as safe as skipping, and it is visible (the root can `task
  reset`, `--timeout` bounds headless). Skipping is *only* the verdict when the
  judge says so.
- **The `judger` member:** a normal `.wcode/agents/judger.md` (or `[[team]]`)
  resolved from the pool; `ModelJudge` reads its `model`/`base_url`/`api_key`/
  `effort`/`role`. It is **not spawned**, so its `tools` field is ignored (zero
  tools in the completion) — resolving the D3 "only `message`" ambiguity. No
  `judger` member → fall back to the root's template (`WorkerTemplate.llm`/
  `stream_fn`) + the built-in contract.

**Changed** `scheduler.rs`:

- `Scheduler` gains `judge: Arc<dyn Judge>` and a `judge_tx/judge_rx` `mpsc`
  channel; `judging: HashSet<u32>`, `unjudgeable: HashSet<u32>` (loop-local). All
  state mutation stays on the loop thread.
- `frontier` (`scheduler.rs:159`) filters out ids in `judging`.
- `dispatch` (`scheduler.rs:201`): for a ready node, evaluate its incoming edges in
  order — a `script` condition runs synchronously under `before_tool_call`
  (mirroring `scheduler.rs:226`); a `judge` condition spawns `judge.decide` (mark
  `judging`) and sends `(id, verdict)` on the channel; `Some(false)` →
  `tasks.skip(id)`; `None` → `unjudgeable` + `escalate`.
- `run` (`scheduler.rs:122`) selects on `updates.changed()` **and**
  `judge_rx.recv()`; on `(id, Some(true))` it dispatches (the normal `Wake` +
  `start`); `unjudgeable` is cleared by a `plan reset`.

**Changed** `agents.rs:764 spawn_scheduler` — build the judge and pass it in; add
`SessionFactory::judge(&self, member: Option<&TeamMember>) -> Arc<dyn Judge>` (the
factory owns `template: WorkerTemplate`, `agents.rs:169`, private).

### 4.4 Lateral adjacency (A)

**Added** `crates/wcode-protocol/src/registry.rs`:

```rust
/// Allow `from` ↔ `to` (workflow edge adjacency). Explicit, not a mesh.
pub fn allow_pair(&self, from: &SessionId, to: &SessionId) { /* insert both directions */ }
```

**Changed** `registry.rs:306`:

```rust
// owners.get(from) == Some(to) || owners.get(to) == Some(from)
owners.get(from) == Some(to)
    || owners.get(to) == Some(from)
    || self.inner.allows.lock().unwrap().contains(&(from.clone(), to.clone()))
```

and amend the doc at `registry.rs:305` (`There are no lateral (worker → worker)
edges.`) to name the workflow-scoped exception. The `Inner` struct
(`registry.rs:62`) gains `allows: Mutex<HashSet<(SessionId, SessionId)>>`.

**Changed** `agents.rs:407` — on a `[workflow]` spawn, register each node's
edge-neighbours (`allow_pair`). The worker keeps `Phonebook::default()`;
`message.rs:96` already resolves a bare `to` via `address()` (`message.rs:57`: a
bare name → `agent:<name>`).

**Integration points (verify before coding).**

- `registry.rs:306` — `owners.get(from) == Some(to) || owners.get(to) ==
  Some(from)` — the permit A1 widens.
- `registry.rs:231` — `set_owner` — `"This is the whole ownership tree — a star in
  v1."` — the adjacency API sits beside it.
- `registry.rs:314` `resolve` — the enforcement point both tests and A1 ride.
- `config.rs:246` `WorkflowNode` / `:260` `pub depends_on: Vec<String>,` — B1
  removes the field.
- `config.rs:853` `validate_workflow_structure` / `:933` `topo_order` — B1
  rewrites the graph walk.
- `tasks.rs:44` `TaskState` / `:105` `Task` / `:882` `ready_ids` / `:909`
  `terminal_code` / `:697` `complete` / `:736` `reopen_inner` — B3/B4.
- `scheduler.rs:159` `frontier` / `:201` `dispatch` / `:226` `RunSpec::Script {
  command } =>` / `:177` `escalate` — B2s, B2j.
- `agents.rs:493` `struct ReportBack` — why a worker-judge cannot report to the
  scheduler (§4.3).
- `agents.rs:764` `pub fn spawn_scheduler` — where the judge is built.
- `agents.rs:407` `tools.push(erased(Message::new(` — where adjacency is
  registered (A).
- `main.rs:2486` `instantiate_workflow` / `:2507`
  `tasks.create(crate::config::node_title(node, task), deps, node.gate)` —
  edge-driven instantiation.
- `main.rs:2543` `fn task_item` — `Dep::id()` mapping to the TUI.
- `streamfn.rs:285` `pub type StreamFn =` / `event.rs:6` `pub enum LlmStreamEvent`
  — the judge's one-shot completion.

## §5 Plan

1. **Edges in config.** Deliverable: `WorkflowEdge`/`EdgeCondition`,
   `Workflow.edges`, `WorkflowNode.depends_on` removed,
   `validate_workflow_structure` + `topo_order` over edges (dup/self/dangling/
   cycle, gate-needs-an-incoming-edge, exactly-one condition mode). Gate:
   `cargo test -p wcode-cli config` — rewritten `workflow_rejects_a_bad_graph`,
   `structure_still_rejects_cycle_dup_and_dangling_dep`,
   `toml_workflow_parses_nodes`.
2. **Runtime `Dep`.** Deliverable: `Dep { id, condition }` (untagged),
   `Task.deps: Vec<Dep>`, `PlanOp::Create.deps: Vec<Dep>`; the journal still
   decodes a legacy `depends_on` line. Gate: a test decodes a legacy
   `{"op":"create","deps":[1,2]}` line into `Dep::Id`s;
   `create_rejects_an_unknown_dep` (`tasks.rs:960`) green.
3. **`Skipped` state.** Deliverable: the variant, `skip()` + `PlanOp::Skip`,
   readiness/`terminal_code`/`complete` treat it as satisfied/success, the TUI maps
   it, `reopen`'s cone re-`Todo`s it. Gate:
   `ready_ids_treats_a_skipped_dep_as_satisfied`,
   `terminal_code_is_zero_when_all_done_or_skipped`,
   `reopen_re_evaluates_a_skipped_descendant`; `ready_ids_tracks_the_frontier`
   (`tasks.rs:1069`) and `reopen_resets_the_transitive_cone` (`tasks.rs:1085`)
   green.
4. **`script` conditions.** Deliverable: the scheduler evaluates a ready node's
   `script` edges synchronously under `before_tool_call`, exit 0 = taken, else
   `skip(id)`. Gate: `a_false_script_condition_skips_the_target`;
   `frontier_includes_a_script_node_but_not_an_ownerless_session`
   (`scheduler.rs:931`) green.
5. **`judge` seam.** Deliverable: `judge.rs` (`Judge`, `ModelJudge`,
   `parse_verdict`, `FakeJudge`), the scheduler's `judge_rx` parking,
   `unjudgeable` + escalate; `SessionFactory::judge` + `spawn_scheduler` wiring.
   Gate: `parse_verdict_accepts_ta_take_skip_and_rejects_prose`,
   `a_judge_take_dispatches_and_skip_marks_skipped`,
   `an_unparseable_verdict_escalates_once` — all with `FakeJudge`, no live model.
6. **Migration.** Deliverable: `.wcode/workflow.toml` rewritten as
   `[[workflow.edge]]` (7 nodes, 8 edges: explore→sketch→review-sketch→implement→
   verify→publish, implement→ci, publish→ci); `README.md:157-199`,
   `docs/task-dag-plan.md:272-291`, `docs/workflow-task-injection-plan.md:415-491`
   updated; `main.rs:3259 instantiate_workflow_handles_a_later_authored_dependency`
   rewritten. Gate: `cargo test --workspace`; `wcode --config .wcode/workflow.toml`
   loads under `--dump-config` (with `--task`).
7. **Lateral adjacency.** Deliverable: `Registry::allow_pair`, widened `permitted`,
   workflow spawn registers neighbours. Gate: rewritten
   `lateral_message_between_workers_is_refused` → adjacent allowed / non-adjacent
   refused; `orchestrator_and_worker_reach_each_other` (`registry.rs:403`) and
   `an_unowned_pair_is_not_permitted` (`registry.rs:639`) green.
8. **Docs + decisions.** Deliverable: `D029`, `D030` (amending `task-dag-plan.md`
   §11 #1), the tracker row pointing at W025, `README.md`'s `[workflow]` section
   describing both condition kinds. Gate: `cargo test --workspace` + clippy clean;
   docs links resolve.

## §6 Quality gates

- `cargo test --workspace` → all pass (expect +10–14 over the current suite).
- `cargo clippy --workspace --all-targets` → clean.
- Live script-condition: `wcode --config <tmp>.toml --task "x" --base-url
  http://localhost:11434/v1` → the summary (`main.rs:2438 print_workflow_summary`)
  prints `#[id] [skipped] …` and exits 0.
- Live judge-condition: same, with `condition = { judge = "judger", criterion =
  "…" }` and a `judger` agent file → the summary shows the target
  `[done]`/`[skipped]` per the verdict.
- Regression kept green / deliberately rewritten then re-green:
  `workflow_rejects_a_bad_graph` (`config.rs:1750`),
  `structure_still_rejects_cycle_dup_and_dangling_dep` (`config.rs:1770`),
  `a_valid_workflow_is_carried_through` (`config.rs:1889`),
  `instantiate_workflow_handles_a_later_authored_dependency` (`main.rs:3259`),
  `ready_ids_tracks_the_frontier` (`tasks.rs:1069`),
  `reopen_resets_the_transitive_cone` (`tasks.rs:1085`),
  `lateral_message_between_workers_is_refused` (`registry.rs:451`),
  `orchestrator_and_worker_reach_each_other` (`registry.rs:403`),
  `an_unowned_pair_is_not_permitted` (`registry.rs:639`),
  `spawn_registers_and_owns_the_worker` (`agents.rs:883`),
  `frontier_includes_a_script_node_but_not_an_ownerless_session`
  (`scheduler.rs:931`).

## §7 Testing

### 7.1 Automated

- `edge_parses_an_unconditional_and_a_script_condition` — both `[[workflow.edge]]`
  forms deserialize; `depends_on` on a node is a hard error.
- `a_duplicate_edge_pair_is_rejected` — two `(from,to)` edges are a load error.
- `an_edge_needs_exactly_one_condition_mode` — `script` xor `judge`+`criterion`;
  both/neither is an error.
- `reference_journal_line_still_decodes` — `{"op":"create","deps":[1,2]}` →
  `Dep::Id`s (back-compat).
- `ready_ids_treats_a_skipped_dep_as_satisfied` — the dependent becomes ready.
- `terminal_code_is_zero_when_all_done_or_skipped` — a skipped graph is success.
- `a_false_script_condition_skips_the_target` — exit ≠ 0 → `Skipped`, never
  dispatched.
- `a_judge_take_dispatches_and_skip_marks_skipped` — `FakeJudge` returns
  `Some(true)`/`Some(false)`.
- `parse_verdict_*` — `TAKE`/`SKIP`/markdown/first-line/both-tokens/empty.
- `an_unparseable_verdict_escalates_once` — `None` → escalate, node `Todo`, no
  tight loop.
- `an_adjacent_pair_may_message` — `allow_pair(w1,w2)` → `deliver` succeeds.
- `a_non_adjacent_pair_is_still_refused` — the star bound survives outside the
  workflow.

### 7.2 How a human verifies it

```sh
# Script condition: `b` runs only if `a`'s test passes.
cat > /tmp/wf.toml <<'EOF'
[workflow]
[[workflow.node]]
id = "a"
script = "true"
[[workflow.node]]
id = "b"
script = "echo b"
[[workflow.edge]]
from = "a"
to = "b"
condition = { script = "false" }
EOF
cargo run -p wcode-cli -- --no-instructions --base-url http://localhost:11434/v1 \
  --task "x" --config /tmp/wf.toml 2>&1 | tail -5
```

Look for: `#1 [done]` and `#2 [skipped]` in the summary — `b` never ran. Flip
`script` to `"true"` → `#2 [done]`.

For the judge path without a live model, run the unit test:

```sh
cargo test -p wcode-cli a_judge_take_dispatches_and_skip_marks_skipped -- --nocapture
```

For a live judge, add `.wcode/agents/judger.md` and a `condition = { judge =
"judger", criterion = "…" }` edge, then run the same headless command with
`--base-url http://localhost:11434/v1`.

Error path: `WCODE_RETRY_MAX=2` against `http://127.0.0.1:9/v1` still prints the
plan summary (condition logic is model-independent for scripts).

## §8 Expected outcome

- A `[workflow]` declares `[[workflow.node]]` + `[[workflow.edge]]`; `depends_on`
  no longer parses (dropped, migrated).
- An edge may carry a `script` or `judge` condition; a false/SKIP verdict makes the
  target `Skipped`, shown as `[skipped]` in the summary and `/tasks`.
- `TaskState` has five variants; `ready_ids` and `terminal_code` treat `Skipped` as
  satisfied/success; `complete` accepts a skipped dep; `reopen` re-`Todo`s it.
- A `judge` is a one-shot no-tool completion; its verdict is parsed from `VERDICT:
  TAKE|SKIP`; an unparseable reply escalates instead of silently pruning.
- Edge-joined workflow nodes may `message` each other by `agent:<id>`; a
  non-adjacent pair is still `NotPermitted`; a plain `[team]` keeps today's exact
  star.
- `task-dag-plan.md` §11 #1 is amended by D030; D029 records the
  condition/`Skipped` semantics.

## §9 Open questions (with a recommendation)

1. **Judge fallback: skip vs block-and-escalate.** *Recommend block-and-escalate.*
   A silent SKIP is invisible pruning; blocking never runs the guarded step and is
   visible. Cost: a stalled headless run until `--timeout`.
2. **TOML shape: flat inline table vs internally-tagged `kind`.** *Recommend the
   flat `{ script = … }` / `{ judge = …, criterion = … }`* with exactly-one
   validation (matches the human's example and the repo's `member` xor `script`
   idiom).
3. **`judger` origin.** *Recommend a normal `.wcode/agents/judger.md` (or
   `[[team]]`), executed as a one-shot completion; no member → the root's model +
   a built-in contract.*
4. **Runtime `Task.deps: Vec<Dep>` vs a parallel `conditions` list.** *Recommend
   `Vec<Dep>` (untagged)* — one source of truth; the journal stays byte-identical
   for unconditional edges.
5. **Edge conditions AND-ed vs OR-ed.** *Recommend AND.* Matches the DAG's
   blocked-by model; OR is a later model.
6. **Adjacency scope: edge-joined only vs a full mesh.** *Recommend edge-joined
   only* (per sign-off); a mesh stays reserved (decision #16).

## §10 Facts NOT verified

- No binary and no test was run — no live end-to-end check; all "expected output"
  is reasoned from code.
- `cargo test`/`cargo clippy` were not run; the suite size is inferred, not
  measured.
- It is **not** confirmed that a script-only `[workflow]` drives the scheduler to
  completion headless without a functioning model; the live recipe may need a live
  model for the root turn.
- serde's untagged `Dep` decoding against a real legacy `plan.ndjson` is not
  verified (only the shape is reasoned).
- `stream_fn`'s behavior for a zero-tool, single-message one-shot (the judge path)
  on a real endpoint is not confirmed.
- The TUI `TaskItem` rendering for a `skipped` label beyond the `main.rs:2543`
  mapping was not inspected.
- Whether `[[workflow.edge]]` interacts with the `[[workflow.node]]`
  overlay-replace rule beyond `overlay_replaces_arrays_wholesale` was not checked.
- Line numbers are from HEAD `1bcbcb8`; the tree has an unrelated ` M
  .wcode/team.toml`. Every quoted snippet must be re-grepped before coding.

## §11 References

- `docs/task-dag-plan.md:47-51, 360-361, 387` — the "star preserved; no `Registry`
  change" lock this brief amends, and the `on_cap = fix-node` Option-B merge point.
- `docs/interface-protocol-brainstorm.md:389-437, 520-524` — §10.1 (orchestrator v1
  → group later) and decision #16; the reserved group mode.
- `docs/swarm-comparison-plan.md:63-75, 117-121` — jcode's "edge is the data
  channel"; "wcode's star is one-hop".
- `docs/workflow-task-injection-plan.md` — the `[workflow]` template + `{{task}}`
  precedent this extends.
- `.wcode/skills/brainstorm-brief/SKILL.md` — the brief form.
