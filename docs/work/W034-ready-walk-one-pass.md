# W034 — The readiness walk is one pass (`ready_ids_in` + `ready_ids_where`)

**Status:** **shipped** — second-layer **APPROVED at `f28b053`**; first-layer
APPROVE with 8 mandatory amendments (A1–A8), all landed; implemented in code
commit `f28b053` plus this bookkeeping commit.
Gates at hand-off: `cargo test --workspace` **1489 passed / 4 ignored**, `cargo
clippy --workspace --all-targets` **clean**, plus a live headless workflow run (§4).
**Decision record:** [`decisions/D040-ready-walk-one-pass.md`](../decisions/D040-ready-walk-one-pass.md).
**Tracker:** row 102 in [`next-steps.md`](../next-steps.md).

---

## 0. The ask

"Walk through the graph, speedly." Referent = the plan's task DAG; the "walk" =
the readiness traversal the scheduler runs on every wake; "speedly" = that walk is
ONE pass instead of a per-dep `tasks.iter().find(..)` rescan. Scope was locked to
two files (`tasks.rs`, `scheduler.rs:frontier`) — no new tool op, no TUI graph
view, no `--status` redesign, no new crate, and `cone`/`descendants` untouched.

## 1. What changed

| where | before | after |
|---|---|---|
| `tasks.rs` readability | `ready_ids` rescanned all rows **per dep** — O(len × Σ deps) | `ready_ids_in(&[Task], keep)` — one id→state index pass, one walk: **O(len + Σ deps)** |
| `tasks.rs` surface | `ready_ids` only | `ready_ids` (unchanged signature, delegates) + `pub(crate) ready_ids_where(keep)`, both under ONE lock |
| `scheduler.rs:frontier` | `snapshot()` (clone) + `ready_ids()` = two locks, a full `Vec<Task>` clone, and two filters outside any lock | one lock, zero clones: `ready_ids_where` with the three clauses (`!judging`, `!unjudgeable`, `owner || Script`) |

Order and membership are unchanged on a quiescent plan: storage order in, storage
order out, so **dispatch order is unchanged**. The three real deltas (a
mid-wake `create` no longer dropped; the owner decision moves to the walk instant;
the predicate becomes row-local) are stated in D040 §*Three behaviour deltas*.

The book excerpt that quoted the old body verbatim
(`docs/book/12-the-task-dag.html`, §*Readiness is derived, never stored*) was
re-anchored to `tasks.rs:445` `fn ready_ids_in` and re-quoted in the same commit
(A6) — no deliberate staleness, so no follow-up row.

## 2. Amendments A1–A8, and where each landed

- **A1 — tests belong in `mod tests`, not `mod terminal_code_tests`.** Landed: the
  first insertion went into the wrong module (the sketch's own placement was at
  the tail of `mod terminal_code_tests`) and was caught by the compiler
  (`cannot find function 'task'`); the whole block was moved into `mod tests`
  (`ready_ids_naive`, `row`, `BranchingPlan`, `w034_branching_plan`, seven tests),
  reusing the existing `fn task(id, title)` helper and the `PlanOp::Snapshot`
  replay precedent.
- **A2 — no 7-element positional tuple.** Landed: `BranchingPlan` is a test-local
  struct with named fields (`chain`, `diamond_root`, `skipped_dep`, `failed_dep`,
  `doing_dep`, `dangling`, `ownerless`, `script`) — plus `self_dep`, because the
  sketch's `skipped_satisfies_but_failed_and_self_deps_do_not` needs a self-dep
  row and a named field beats a bare id.
- **A3 — the commit message states all three deltas.** Landed in the `cli:`
  commit body (mid-wake `create` dropped; owner/`Script` at the walk instant;
  row-local predicate vs the snapshot's any-row rule).
- **A4 — the scheduler reference test owns a ~6-node plan, unique ids, ONE
  `snapshot()` + production `ready_ids()` filtered outside the lock, compared
  `Vec` to `Vec`, never sorted.** Landed as
  `frontier_equals_the_two_lock_reference`; `ready_ids_naive` was NOT widened to
  `pub(crate)` (it stays a `tasks::tests` helper).
- **A5 — the 2000-node chain is seeded with ONE `PlanOp::Snapshot`, not 2000
  `create` calls.** Landed in `a_long_chain_stays_correct_at_scale`: one snapshot,
  then `n - 1` completes each asserting exactly one newly-ready id, and an empty
  frontier at the end. No timing assert.
- **A6 — re-anchor the book excerpt in the same commit.** Landed (§1).
- **A7 — bookkeeping.** This file + D040 + tracker row 102.
- **A8 — commit only W034 scope.** The dirty `docs/book/13-workflow-mode.html` is
  left untouched and out of both commits.

## 3. Tests

`tasks.rs::tests` — 8, the behaviour gate plus the edge cases the old body had
implicitly:

1. `ready_ids_keeps_storage_order_for_ids_out_of_ascending_order` — a hand-written
   `Snapshot` in order 7, 3, 5 stays 7, 3, 5 (a stray sort fails).
2. `a_duplicate_id_reads_first_wins_like_find_did` — `(Todo, Done)` leaves the
   dependent blocked, `(Done, Todo)` unblocks it: `or_insert`, not `collect`.
3. `ready_ids_matches_the_naive_reference_on_a_branching_plan` — new vs
   `ready_ids_naive` (the deleted algorithm, test-only) on a 17-row plan, with the
   reference's own answer pinned so it cannot drift into agreeing with a bug.
4. `ready_ids_reads_a_dep_that_sits_later_in_storage_order` — the dependent's
   row comes FIRST and its `Done` dep later, so a positional (streaming) index
   fails even though a full index passes.
5. `a_dangling_dep_keeps_its_dependent_blocked` — reachable only via `Snapshot`;
   `create` refuses the dep.
6. `skipped_satisfies_but_failed_and_self_deps_do_not`.
7. `ready_ids_where_equals_ready_ids_filtered_by_hand` — same ids, same order, and
   `keep` ran ONLY for ready tasks (the seen-id list is asserted).
8. `a_long_chain_stays_correct_at_scale` — 2000 nodes, one id per level.

`docs/book/12-the-task-dag.html` — the excerpt's own anchor (`ready_ids_in`) was
re-based in the same commit, and so were the page's five other `tasks.rs`
citations the diff moved (`TaskState`, `Task`, `PlanOp`, `complete`,
`reopen_inner`) — each re-read against the quoted item, not just renumbered.

`scheduler.rs::tests` — 3 new + the pre-existing keep-test unchanged:
`frontier_equals_the_two_lock_reference`, `frontier_excludes_a_judging_node`,
`frontier_excludes_an_unjudgeable_node`; the existing
`frontier_includes_a_script_node_but_not_an_ownerless_session` passes verbatim,
which is itself evidence the surface did not move.

## 4. Gates

- `cargo test --workspace` → 1489 passed, 4 ignored (1478 before: +11 tests).
- `cargo clippy --workspace --all-targets` → clean.
- **Live end-to-end** (the check `cargo build` cannot give): a three-node
  `Script`-only `[workflow]` — `left`, `right`, and `join` depending on both —
  driven headless against a keyless local endpoint
  (`--base-url http://localhost:11434/v1`, `--model gemma4:e4b`,
  `--config /tmp/w034-wf.toml --task … --timeout 180`). Log showed
  `#1 [done] left` and `#2 [done] right` dispatched together, then
  `#3 [doing] join` → `#3 [done] join`; exit **0** in 39 s. That is
  `frontier` → `ready_ids_where` → `dispatch` over a real plan in a real binary,
  including the multi-id frontier (two ready roots at once) and the join that only
  becomes ready after both.

## 5. What is NOT verified

- **The cost shape is reasoned, not measured.** No timing assertion exists (a
  timing test would be flaky on a loaded machine, and instrumenting
  `TaskList::Inner` with a counter was rejected as production churn for a test).
  The guards are the review of `ready_ids_in` (no `iter().find(` inside it), the
  2000-node chain (a human-noticeable tripwire), and the naive-equality test
  (semantic drift only). **No speedup figure is claimed** — the win is
  asymptotic.
- **The two race claims are reasoned, not reproduced.** A test cannot pin either
  window deterministically; both are argued from the lock shapes in D040.
- The scheduler's OTHER per-iteration snapshots (`prune_unjudgeable`,
  `started.retain`, `failed_ids`/`failed_tasks`, `dispatch_one`/`edges_admit`) were
  read and deliberately left alone — they are not the graph walk.
