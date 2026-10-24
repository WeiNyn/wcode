# D040 — The readiness walk is one pass, and the frontier takes one lock

- **Status:** accepted (first-layer APPROVE with amendments A1–A8; implemented)
- **Date:** this session
- **Relates to:** [`tasks.rs`](../../crates/wcode-cli/src/tasks.rs) (`ready_ids_in`,
  `ready_ids`, `ready_ids_where`), [`scheduler.rs`](../../crates/wcode-cli/src/scheduler.rs)
  (`frontier`, the wake loop's `run`), [`12-the-task-dag.html`](../book/12-the-task-dag.html)
  §*Readiness is derived, never stored*
- **Complement to:** [`D039`](D039-tui-tool-run-rail.md) — D039 rails a batch of
  tool rows, D040 is the same instinct one layer down: one pass over a batch,
  never a rescan per member
- **Distinct from:** [`D033`](D033-workflow-hardening.md) and the W028 deadline
  work — those bound a run in TIME; this bounds a wake in WORK

## Context

`ready_ids` (the plan's derived readiness set, §3 of the DAG design) answered
"which nodes may run now?" with a per-dep rescan:

```rust
t.state == TaskState::Todo
    && t.deps.iter().all(|d| {
        tasks.iter().find(|x| x.id == d.id()) …   // a full scan PER DEP
    })
```

That is O(len × Σ deps) per wake. On the plan sizes wcode actually runs (tens of
nodes) it was never hot, and no user complained — which is exactly why the change
is a *constant*, not a feature: the same answer, computed once over the graph
instead of once per edge per node.

The scheduler's `frontier` then paid twice more: it took the `tasks` lock twice
(`snapshot()` at `scheduler.rs:310`, `ready_ids()` at `:311`) and cloned the whole
`Vec<Task>` just to look at two fields (`owner`, `run`) of the rows it kept.

## Decision

1. **One pass, no cache.** A free function `ready_ids_in(&[Task], keep) ->
   Vec<u32>` builds an id→state index in one pass, then walks the tasks in one
   pass: **O(len + Σ deps)**. Every dep is a map hit, never a rescan. Nothing is
   stored, invalidated or kept warm.

2. **`ready_ids` keeps its signature and contract.** It stays `pub`, takes one
   lock, and now just delegates (`ready_ids_in(&…, |_| true)`). Its order is
   stated as what it always really was — **storage order** (creation order in
   practice, since ids are monotonic, but `PlanOp::Snapshot` reassigns the vec
   wholesale, so any order is legal).

3. **A caller filter runs INSIDE that lock.** `pub(crate) fn
   ready_ids_where(&self, keep: impl FnMut(&Task) -> bool) -> Vec<u32>` is
   `ready_ids` plus a predicate applied to each *ready* task, in storage order,
   under the same lock. Its doc carries the new hazard as a contract: `keep` runs
   with the `tasks` mutex HELD (`std::sync::Mutex`, not reentrant), so it may read
   the `&Task` and captured immutable state and nothing else — a call back into
   `TaskList` deadlocks. A caller that needs more must filter outside, as before.

4. **`frontier` is one lock, zero clones.** It becomes
   `self.tasks.ready_ids_where(|t| …)` with exactly three clauses: not `judging`,
   not `unjudgeable`, and `owner.is_some() || matches!(t.run, RunSpec::Script { .. })`.
   Storage order is preserved, so **dispatch order is unchanged** — the
   determinism the scheduler relies on.

5. **The two frontier call sites stay two calls.** `run` asks at the loop top
   (`:239`) and again for the stall check (`:251`). The second is load-bearing:
   `dispatch` flips nodes to `Doing` (`tasks.start`), so "nothing ready now" is a
   different, correct answer. Hoisting it would report a stall that is not one.

## Three behaviour deltas (stated, not implied)

All three follow from *when* and *where* the answer is computed. Each is a
removed race or a stricter local rule, never a change on a quiescent plan.

1. **A ready node created mid-wake is no longer dropped.** Today a `create`
   landing between `snapshot()` and `ready_ids()` produces an id the earlier
   snapshot has no row for, so `snapshot.iter().any(..)` silently drops a node
   that is ready. One lock makes the window impossible. No test can pin a race;
   the equality tests assert the quiescent result.

2. **The owner/`Script` decision moves from the snapshot instant to the walk
   instant.** Today the owner read comes from a `Vec<Task>` cloned before the
   readiness lock was taken; now it is read from the same row the walk just
   classified. An `assign` racing a wake can therefore be seen one iteration
   earlier — never later, and never inconsistently with the state that made the
   node ready.

3. **The predicate becomes row-local.** `scheduler.rs:318`'s
   `snapshot.iter().any(|t| t.id == *id && (t.owner.is_some() || …))` kept an id
   if ANY row carrying that id had an owner. Row-local ships as the honest
   semantic: a duplicate id is only reachable through a hand-written `Snapshot`,
   and readiness has always been row-first (`find` took the first row). One row,
   one verdict, for state and for owner alike.

## Rejections (explicit)

- **F3 — a cached / incrementally maintained ready set: REJECTED.** It needs
  invalidation at every mutation path (`create`/`depends`/`assign`/`configure`/
  `start`/`complete`/`reopen`/`reject`/`reset`/`fail`/`skip`/`restart`/`Snapshot`
  replay/`reconcile`), and a stale cache is a CORRECTNESS bug where a linear walk
  is merely a constant. The plan is tens of nodes; one pass per wake is already
  below measurement. Deriving readiness is the property that lets the TUI, the
  scheduler and `task list` never disagree — a cache is what would break it.
- **F4 — narrowing `ready_ids` to `pub(crate)` (or deleting it): REJECTED.**
  After this change the binary's only caller is `ready_ids_where`, so `pub` is
  wider than needed. The decision is behaviour-identical + minimal: the public
  read surface of the plan stays as it was, and the tests read `ready_ids`
  directly. It carries `#[allow(dead_code)]` with a comment saying exactly that,
  rather than a narrowed signature that would churn every caller's contract.

## Mechanism

- `ready_ids_in` (`tasks.rs`) sits beside `cone` and for the same reason: a free
  function over `&[Task]`, so exactly one caller may hold the `tasks` lock while
  it walks. Pass 1 fills `HashMap<u32, TaskState>` with `entry(id).or_insert(state)`
  — **first wins**, matching the `find` it replaces; `collect::<HashMap<_,_>>()`
  would keep the LAST and is a bug on a duplicate-id snapshot.
- Two edge cases need no special case, and the tests pin both: a **dangling dep**
  is a missing map entry, which `is_some_and` reads as "not satisfied", and a
  **self-dep** is unready by the `Todo` clause alone.
- `frontier` (`scheduler.rs`) passes the three-clause predicate; the closure
  borrows `&self` for `judging`/`unjudgeable` (no lock of their own) and never
  touches `self.tasks`.

## Consequences

- The win is **asymptotic, not measured**: one pass, O(len + Σ deps), against
  O(len × Σ deps). No speedup figure is claimed — at this plan size it is not
  observable.
- **Deliberately out of scope**, each a candidate follow-up if ever hot:
  `publish` clones the whole vec on every mutation; `cone`/`descendants` stay
  O(n²) per call (`reopen`-only); `terminal_code` is untouched; and,
  scheduler-side, `prune_unjudgeable`'s snapshot, `started.retain`'s snapshot,
  `failed_ids`/`failed_tasks` and every `snapshot()` inside
  `dispatch_one`/`edges_admit`. Not the graph walk.
- **Cost shape is not machine-verified.** No test can observe the walk's shape
  without instrumenting production types (a `#[cfg(test)]` counter on `Inner` was
  considered and rejected as production churn for a test). The guards are: review
  of the finished body (no `iter().find(` inside `ready_ids_in`); a 2000-node
  chain test that readies exactly one id per level (a human-noticeable tripwire,
  **not** a timing assertion); and a behaviour-equality test against the deleted
  algorithm, kept test-only as `ready_ids_naive`.
- **Accepted drift (prose, deliberately not touched).** Two comments still name
  `ready_ids` where the wake loop now calls `ready_ids_where`:
  `scheduler.rs`'s module header ("[`TaskList::ready_ids`] terminates, so the loop
  is a pure function of list state") and `main.rs:1481` ("a `Doing` node never
  re-enters `ready_ids()`"). Both statements are still TRUE of the contract —
  `ready_ids_where` is `ready_ids` plus a filter — and renaming them would churn
  two unrelated files for a doc-only edit. Likewise the page-12 excerpt
  *paraphrases* the source comments (the `// pass 1` / `// pass 2` lines are
  condensed, not verbatim); the code shape it shows is exact and the anchor is
  re-based, and the book's own convention already says snippets are trimmed to
  the point being made.
- Gate: `cargo test --workspace` green · `cargo clippy --workspace --all-targets`
  clean · a live headless run of a three-node `Script` workflow (two ready roots
  dispatched together, the join only after both) — see
  [`work/W034`](../work/W034-ready-walk-one-pass.md).
