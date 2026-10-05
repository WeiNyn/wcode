# D002 — a gate is structural, not verification

- **Status:** accepted
- **Date:** 2026-10-05
- **Supersedes / relates to:** `docs/swarm-comparison-plan.md` §5,
  `docs/task-dag-plan.md`

## Context

`verify_gate.rs` documents its own rule — R′ — as **shape, not enforcement**:
"Even so, R′ is **shape, not enforcement** — it constrains the plan's graph, it
does not prove the gate actually ran"
(`crates/wcode-cli/src/verify_gate.rs:9Vm1l`), and "R′ constrains the **shape**
of the plan, not the verdict" (`crates/wcode-cli/src/verify_gate.rs:0syUd`).

The task-list facts agree:

- A `Session` node completes when the root calls `task{op:"complete", id}`
  (`crates/wcode-cli/src/tasks.rs:5Cj4v`) regardless of whether the delivered
  artefact is correct; `complete` records state, it validates nothing
  (`crates/wcode-cli/src/tasks.rs:5Cj4v`).
- A `Script` gate's verdict is only an exit code — the `RunSpec::Script` arm
  (`crates/wcode-cli/src/tasks.rs:bstvm`, set via `configure`
  `crates/wcode-cli/src/tasks.rs:23nfo`) maps a non-zero exit to reject.
- `reject` re-opens the gate's deps and their downstream cone
  (`crates/wcode-cli/src/tasks.rs:5X2Rj`) — it moves state; it inspects no
  artefact.

## Decision

`[workflow]` gates are treated as **shape**: a gate node must exist above a work
node (R′), and a reject re-opens the cone. Gates are **never described as
verification** in docs, prompts, or role text. Correctness judgment lives in the
**reviewer role** — a prompt, not a graph property.

## Consequences

- Docs and prompts must stop implying a gate proves anything; the reviewer's
  verdict is the only correctness signal.
- The seam stays where it is: `Hooks` (`VerifyGateHooks`) enforces shape, the
  reviewer supplies judgment.
- No new gate type is added to make gates "real" — a per-artefact check would be
  a `Hooks` policy, not a config surface.
