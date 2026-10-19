# D032 — The All-mode merge orders each turn by its last activity

- **Status:** accepted
- **Date:** 2026-10-19
- **Relates to:** [D031](D031-tui-all-focus-view-mode.md) (refines its ordering rule)
- **Implements:** [W027](../work/W027-tui-all-turn-order.md)

## Context

D031 ordered the merged (All) view by each block's **arrival ordinal**, stamped when
the block is *appended* — `crates/wcode-tui/src/app.rs:4572`
`    pub(crate) fn merged_order(&self) -> Vec<(usize, usize)> {` reading `:4581`
`                let seq = surface.seqs.get(bi).copied().unwrap_or(0);`, stamped by
`stamp_new` (`:4614`). But a `Block::Turn` is appended at a run's **first** content
and then **grows in place** — `:2005` `                turn.content.extend(content);`
and `:2075` `            turn.tools.push(tool);` — which changes no transcript length,
so `stamp_new` no-ops (`:4619` `        if before >= len {`) and the turn keeps its
**open** ordinal.

So the orchestrator's turn — which carries its final reply — sorted **above** the
member turns it coordinated. The user's report: *"orchestrator is the one that
finishes the conversation, but the TUI renders messages from other members after
that."*

Two rules were defensible: order a turn by when it **opened** (its first word) or by
when it **last changed** (its last word). The last was chosen.

## Decision

Re-stamp a surface's open `Block::Turn` ordinal whenever it grows —
`App::touch_open_turn` (`app.rs:4660`), called from the `AppEvent::Agent` arm
(`:3901`) **before** `stamp_new` (`:3902`), so an in-apply growth precedes a block the
same event appended. Fire only on a **revision change** of the open turn; a **sealed**
turn (`open_turn` cleared) keeps its ordinal.

## Consequences

- A turn sits at its **last activity** → the last speaker is last. `Focus` is
  untouched (`merged_order` is All-only).
- A growing turn may leapfrog a mid-run notice on another surface — chronologically
  correct (a turn is no longer a single contiguous slot).
- `next_seq` stays monotone (a re-stamp only ever increases), so an ordinal never
  collides with a later block.
- An **open but idle** turn does not move when another surface commits.
- The orchestrator's turn still **bundles** its delegation rows and its reply — a
  message-granularity merge is the alternative, not taken.
