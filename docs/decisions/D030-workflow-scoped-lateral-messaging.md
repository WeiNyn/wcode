# D030 — Workflow-scoped lateral messaging (edge adjacency, not a mesh)

- **Status:** accepted (human sign-off on the W025 brief — sign-off #1/#2/#3)
- **Date:** this session
- **Relates to:** [`registry.rs`](../../crates/wcode-protocol/src/registry.rs)
  (`allow_pair`, widened `permitted`), [`main.rs`](../../crates/wcode-cli/src/main.rs)
  (`register_adjacency`, keyed by member address)
- **Amends:** [`task-dag-plan.md`](../task-dag-plan.md) §11 #1 —
  *"Topology: root-as-scheduler; star preserved; no `Registry` change."*
- **Work item:** [`work/W025-workflow-edges-and-lateral-messaging.md`](../work/W025-workflow-edges-and-lateral-messaging.md)

## Context

`docs/task-dag-plan.md` §11 #1 locked the topology to a **star**: the root is the
only hub, and `Registry::permitted` admitted a pair only when an **ownership** edge
joined them (`owners.get(from) == Some(to) || owners.get(to) == Some(from)`).
There were, by construction, no lateral (worker → worker) edges. That is exactly
right for a plain `[team]` — a bounded fan-out keeps the root in control.

But a `[workflow]` is a graph, and the human asked for **edge-joined nodes to talk
to each other** ("we will need to allow agent (step/node) can talk to each other").
Refusing every worker→worker message makes a node/edge graph a graph only in the
plan, not in the conversations it runs.

## Decision

**Adjacency follows the workflow edges, and nothing else.** A workflow spawn
registers, for each edge whose **both endpoints are `member` nodes**, an explicit
permitted pair via a new `Registry::allow_pair(from, to)` — which inserts **both**
directions. Adjacency is keyed by the **member's session address**
(`agent:<member>`, `message::address(m)`) — **not** the node id, because a node id
(`explore`) can differ from its member name (`explorer`), and only the address is
a registered session. A `script` endpoint has no address and contributes no pair.

**`permitted` widens to consult the allow-set**, alongside the ownership check:
`owned || allows.contains(&(from, to))`. The `owners` lock is taken and dropped
first, then `allows` is locked — the two leaf mutexes are never held at once.

## Consequences

- Edge-joined `[workflow]` nodes may `message` each other by address; a
  non-adjacent pair is still `RouteError::NotPermitted`.
- **A plain `[team]` is unchanged** — it registers no pair, so `permitted` stays
  today's exact `owner`-edge star. The `[team]` fan-out stays bounded; this is not
  a general mesh.
- This **amends `task-dag-plan.md` §11 #1** (the "no `Registry` change" lock): the
  `Registry` now carries an explicit, workflow-scoped allow-set. The star is still
  the default; adjacency is opt-in and edge-defined.
- A full / "group" mesh (shared context, any-to-any) remains **out of scope**
  (`interface-protocol-brainstorm.md` decision #16). `allow_pair` is the narrow
  seam that decision #16 reserved.
