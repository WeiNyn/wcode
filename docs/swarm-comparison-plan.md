# wcode — team vs jcode swarm (comparison & candidate plan)

Status: **active — C1 (typed report) + C2 (shared task list) landed** (`d99ab65`,
`7bd24bb`, `f1338c2`, `ca12f16`); second-layer APPROVED; C3 (the DAG) is the north
star. Revives the parked comparison in
[`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md) §10
("Multi-agent semantics worth taking"), §10.1 (orchestrator → group), roadmap
**S5** ("Task-DAG / deep swarm, only if wanted") and decision **#11**
("take the task-DAG reframe? no for v1"). That decision was taken *before* the
wcode team workstream (F1–F4) landed and *before* jcode's DAG-first reframe; this
doc re-opens it against **as-built** code on both sides and proposes the minimal
next step. It does **not** commit to building the DAG.

Companion to [`team-and-tui-plan.md`](team-and-tui-plan.md) (what wcode's team
is today), [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)
(the protocol) and [`next-steps.md`](next-steps.md) (tracker).

## 1. Why now

Two things changed since the comparison was parked:

- **wcode's team is real.** F1–F4 landed: customizable workers
  (`model`/`role`/`tools`), the `[team]` preset, a TUI sidebar + multi-surface,
  per-agent provider, remote `Request::Define`, and socket multiplexing with a
  live roster. The comparison is no longer "what might we take" but "what does
  jcode have that our *shipped* star lacks".
- **jcode reframed.** `SWARM_ARCHITECTURE.md` now marks itself superseded by
  `SWARM_TASK_GRAPH.md`, which makes the **task DAG the primary object** and
  demotes agents to fungible workers. The engine is largely built
  (`crates/jcode-plan/src/dag/*`, ~4.4k LOC incl. tests; `crates/jcode-swarm-core`,
  837 LOC). So the honest comparison target is now the DAG, not the coordinator.

## 2. Ground truth

**wcode (as-built).**

- Kernel seam: `Agent` → actor (`SessionActor`/`SessionHandle`), `Request` /
  `AgentEvent` — `crates/wcode-harness/src/{agent.rs,actor.rs,protocol.rs,event.rs}`.
- Delegation: `SessionFactory` is the orchestrator's spawn primitive; only the
  root holds it, so only the root spawns (bounded fan-out by construction) —
  `crates/wcode-cli/src/agents.rs`. `WorkerSpec { name, model, system, tools,
  base_url, api_key }`. Tools: `spawn`, `message`, `peers`
  (`crates/wcode-cli/src/tools/{spawn,message,peers}.rs`).
- Routing: `Registry` (`SessionId → Backend`, `set_owner`, `permitted(from,to)`,
  `resolve`, `deliver`, a `watch` roster) — `crates/wcode-protocol/src/registry.rs`.
  A `Phonebook` (name → address) sits above it (`agents.rs`).
- Topology: **star**. `message { to?, content, mode? }`, modes
  `notify|wake|interrupt|ask`; a worker's `to` defaults to its owner (report-up).
  `permitted()` bounds `to` to the owner/children edge. Completion reports
  auto-forward to the owner and wake it (`message.rs`, team plan §10.1).
- Preset/visibility: `[team]` config → startup spawn; TUI sidebar + `/surface`
  multi-surface; live roster push over a socket (team plan F3/F4, tier 2).
- Policy: `Hooks` (a `before_inbound` mirror is the documented way to express
  "who may message whom"); no ACL, no behavior config.

**jcode (as-designed; largely built).**

- Agent-first (now superseded): coordinator owns a shared `VersionedPlan`;
  worktree managers own integration; agents execute. DMs (preferred), subtree
  broadcast, topic channels, shared-context KV; 8 lifecycle states; soft-interrupt
  delivery at safe points; status/summary/full-context reads; conflicts via DM, no
  locks (`SWARM_ARCHITECTURE.md`).
- DAG-first (supersedes): the **task DAG is primary**; ownership is a *tree laid
  over* a dependency graph, mutated by append-style, ownership-partitioned ops
  (no coordinator bottleneck, no locks); nodes are **atomic** or **composite**
  (composite = decompose-then-synthesize, map/reduce); terminal kinds
  `explore|implement|verify|fix`; the **dependency edge is the data channel** — a
  typed `HandoffArtifact` (`findings, evidence, edge_cases_considered, validation,
  open_questions, confidence, what_i_did_not_check`) flows forward and *hydrates*
  each newly-runnable node's input; **critique/verify gates** are auto-inserted so
  coverage/failure becomes *structure*, not a prompt request; enforced graph API
  (`task_graph` / `expand_node` / `complete_node` / `run`); one total cap
  `MAX_SWARM_MEMBERS = 1000`; comm is being **reduced by subtraction** (dataflow
  primary; DM + subtree broadcast the exception; channels + shared-context
  deprecated) (`SWARM_TASK_GRAPH.md`).
- Built: `../jcode/crates/jcode-plan/src/dag/{mod,ops,schedule,sim}.rs`,
  `../jcode/crates/jcode-swarm-core/src/lib.rs`,
  `../jcode/crates/jcode-tui/src/tui/{swarm_plan_graph.rs,info_widget_*.rs}`.

## 3. Capability comparison

| capability | wcode (star, as-built) | jcode (DAG-first) |
|---|---|---|
| unit of work | a **session** (agent) | a **node** in a task DAG |
| who spawns | root only (by construction) | light: root only; deep: recursive, capped at 1000 |
| work graph | none | first-class DAG (`blocked_by` edges, scheduler) |
| dependency dataflow | report-back to owner (control only) | typed `HandoffArtifact` on the edge → hydrates dependents |
| decompose / synthesize | no (flat) | composite nodes: decompose → children → owner synthesizes |
| quality gate | prompt role (a "reviewer" preset) | **enforced** critique/verify gate before a node closes |
| coverage guarantee | prose / prompt-driven | structural (gaps become new nodes; `what_i_did_not_check`) |
| ownership | owner edge; `permitted(from,to)` | ownership tree over the graph; subtree-scoped ops |
| messaging | one `to` (owner or named peer); 4 modes | DM + subtree broadcast + channels (channels being cut) |
| broadcast | none | subtree-scoped (coordinator keeps whole-swarm) |
| shared state | the repo + session files | repo + typed artifacts (+ deprecated KV store) |
| lifecycle states | idle / running / done (sidebar) | 8 states (spawned…crashed) |
| completion report | auto-forward to owner, wakes it | auto-forward to coordinator; report schema enforced |
| worktrees / integration | none (single workspace) | optional worktrees + worktree managers |
| cap / runaway | none explicit | one cap: `MAX_SWARM_MEMBERS = 1000` |
| visibility | TUI surfaces + sidebar | swarm-graph + plan-graph widgets |
| policy knob | `Hooks` (`before_inbound`) | coordinator slot / (ACL-ish) — jcode is loosening this |

## 4. Gap analysis

What wcode's star genuinely lacks, and whether each is a *gap* or a *deliberate
omission*:

- **Typed handoff / report.** wcode forwards a worker's final prose to its owner.
  jcode forwards a *schema* and uses it as the next node's input. → small,
  self-contained gap; useful even without a DAG.
- **A work graph.** wcode has no tasks/dependencies/scheduler — work is
  prompt-sequenced by the orchestrator. → the big gap; it *is* S5.
- **Decompose-then-synthesize.** wcode workers are flat; only the root composes. →
  rides on the graph.
- **Enforced gates.** wcode's "reviewer" is a role/preset the model *chooses* to
  consult; jcode's gate is structurally unavoidable. → expressible in wcode as a
  `Hooks` policy, not a subsystem.
- **Subtree broadcast.** wcode messages one recipient. → only matters at deep
  fan-out; wcode's star is one-hop.
- **Deliberate omissions (keep).** worktree managers, topic channels,
  shared-context KV, a coordinator *entity*, an ACL/config policy layer. jcode is
  itself cutting channels + KV; wcode should not adopt them at all.

## 5. Candidate features

Ordered by the wcode minimalism / cost axis. Each is a *candidate*, not a
commitment — decide in §6/§7.

- **C1 — typed report.** Give the completion report a documentable shape (a
  template in the worker blurb + an optional schema on the auto-forward), so a
  report carries `findings / evidence / open_questions / what_i_did_not_check`.
  Cheapest; valuable on its own; no DAG. Fits the existing report-back path
  (`agents.rs`, `message.rs`). ~small.
- **C2 — shared task list.** A lightweight, orchestrator-owned list of tasks
  (id, owner, state) that the model assigns/completes via a tool and the TUI
  renders (`/tasks`). Bridges toward the DAG (a DAG is a task list *with edges
  and a scheduler*) without the scheduler. ~medium.
- **C3 — task DAG + scheduler (S5, the north star).** Nodes + edges, a ready
  walk, artifact dataflow hydrating dependents, composite decompose/synthesize.
  The full DAG-first model. Large; a project of its own.
- **C4 — gate as a `Hooks` policy.** Auto-run a verify step (a reviewer worker)
  after an `implement` node before the root accepts it. A `Hooks` impl, not
  config — stays true to the stance. Rides on C2/C3 to have a "node" to gate.
- **C5 — lifecycle footer.** Richer worker states in the sidebar; today it is
  idle/running/done. **Partial — `failed` shipped** (`tui:` `5911eab`): a
  run-failure `AgentEvent::Error` (the loop emits it before `AgentEnd`) marks a
  member `✗`, surviving `AgentEnd` and clearing on the next `AgentStart`; an idle
  command-reply `Error` does not. FP note: a fed-back, non-fatal error the run
  recovers from reads `failed` until the next run starts. **Deferred:** `blocked`
  (not a distinct state — a blocked `bash` is just an errored tool) and
  `waiting-on-detail` (a server-side fact needing a new event / `SessionInfo`
  field). Small; presentation-only.

## 6. Recommendation

Adopt the **minimal subset**: **C1 now** (it improves the shipped star with no new
subsystem), then **C2** as the natural next layer if the team needs explicit
task tracking. Treat **C3 (the DAG) as the documented north star** — build it only
if/when "leave no nook unexplored" comprehensiveness becomes an actual goal, and
then build it with C4's gates. Do **not** import the omitted machinery (channels,
KV, worktree managers, a coordinator entity, ACLs) — jcode is removing much of it.

Concretely: C1 is a commit-or-two; C2 is a small workstream; C3 gets its own plan
doc if chosen. This restates decision #11 with a sharper boundary: *take the
report shape and (maybe) the task list; leave the framework and the DAG.*

## 7. Decisions & open questions

- **D1 — revive scope.** This doc revives the *comparison*; it does not authorize
  C3. Building the DAG requires a separate plan + approval.
- **D2 — C1 shape.** Template-only (prompt text) vs. an enforced schema on the
  auto-forward. Lean template-only first; a schema is C3 territory.
- **Open — is C2 wanted?** A shared task list is a *second* source of truth beside
  the transcript; adopt only if the orchestrator's prose sequencing is actually the
  bottleneck.
- **Open — go/no-go for C3.** Depends entirely on whether deep, coverage-first
  exploration is a goal for wcode. If not, this doc closes at C1 (+maybe C2).

## 8. Phased tasks

- [x] **C1 — typed report**: the report shape is documented in the worker blurb;
      the auto-forward is tested. `cli:` `d99ab65`.
- [x] **C2 — shared task list**: a root-owned list + the `task` tool + TUI
      `/tasks`. `cli:` `7bd24bb`/`ca12f16`, `tui:` `f1338c2`.
- [ ] **C3 — task DAG + scheduler** (decision gate): own plan doc first.
- [ ] **C4 — verify gate as a `Hooks` policy** (rides on C2/C3).
- [ ] **C5 — lifecycle footer** (small, presentation): `failed` shipped
      (`5911eab`); `blocked`/`waiting-on-detail` deferred.

## 9. References

- wcode: `crates/wcode-cli/src/agents.rs`, `tools/{spawn,message,peers}.rs`,
  `crates/wcode-protocol/src/registry.rs`, `docs/team-and-tui-plan.md`,
  `docs/interface-protocol-brainstorm.md` §10/§10.1/S5/#11.
- jcode: `docs/SWARM_ARCHITECTURE.md`, `docs/SWARM_TASK_GRAPH.md`,
  `crates/jcode-plan/src/dag/*`, `crates/jcode-swarm-core/src/lib.rs`,
  `crates/jcode-tui/src/tui/swarm_plan_graph.rs`.
