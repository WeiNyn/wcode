# D004 — the docs taxonomy

- **Status:** accepted
- **Date:** 2026-10-05
- **Supersedes / relates to:** `docs/next-steps.md`

## Context

`docs/` is flat: 36 top-level `.md` files (54 files counting subdirectories and
html sketches). The directory is referenced throughout the repo — `git grep`
finds 112 `docs/*.md` references across 45 tracked files (`README.md`,
`AGENTS.md`, and Rust doc comments among them). Renaming or moving those files
would churn all 112 links at once.

## Decision

New documents go to categorised buckets:

- `docs/decisions/` — DNNN ADRs.
- `docs/work/` — WNNN work items (brief → plan → gates → outcome).
- `docs/plans/`, `docs/designs/`, `docs/analysis/` — created empty, `gitkeep`ed.

`docs/next-steps.md` **stays at the root** as the tracker/index. The existing
`docs/sketches/` and `docs/superpowers/` subdirectories keep their current shape.
Moving the already-existing flat files is a **separate, later change** (Phase 2)
carrying its own link check.

## Consequences

- Zero link churn now; new artefacts are categorised from the start.
- A two-layout tree persists until Phase 2: root docs plus the new buckets.
