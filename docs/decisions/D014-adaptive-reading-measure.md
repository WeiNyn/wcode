# D014 — VS Code surface: the reading measure is adaptive, not a fixed 64ch

- **Status:** accepted (human sign-off 2026-10-10, this session)
- **Date:** 2026-10-10
- **Amends:** W008 (the "paper" surface) — decision **A6**, the 64ch measure
  (`editors/vscode/media/chat.css`, the `--ed-measure` token)

## Context

The "paper" surface (W008, `docs/work/W008-paper-vscode-surface.md`) gave the
transcript **one measure** — `--ed-measure: 64ch` — shared by the reading column
(`.col`), the masthead (`.masthead .inner`) and the composer (`.composer .inner`),
all centered with `margin: 0 auto`. The 64ch cap is asserted by tests as a locked
invariant (`test/view.test.ts`, "the measure: --ed-measure is 64ch…"; also the
composition test's "the column is capped").

In the editor area — which is far wider than the ~64 characters a 64ch column
occupies (≈ 450 px) — this left most of the width unused: the transcript sat as a
narrow ribbon in the middle of a wide panel, and code blocks in particular were
cramped. The request: **make the width adaptive** so the surface uses the editor.

## Decision

**`--ed-measure` becomes `100%`** — the reading column FILLS the available width
(its side gutters come from `.col`'s own `padding: 0 var(--wc-4)`), and the
masthead and composer follow, since all three share the token. There is **no
fixed `ch` cap**; the measure now tracks the container.

The "one centered column" concept is kept — `margin: 0 auto` stays (a no-op at
100%, but it is what makes the column re-center if a cap is ever re-added) — and
the three surfaces still share the ONE `--ed-measure` token, so they stay aligned.

## Consequences

- The transcript, masthead and composer span the editor/panel width; a narrow
  docked sidebar is naturally narrow (100% of a small box), a wide editor is wide.
- The two tests that pinned `64ch` are updated to assert the measure is `100%`
  and that no `ch` cap remains (`test/view.test.ts`).
- **Trade-off, stated:** very wide editors now produce long prose lines. A `ch`
  or `px` cap can be re-added in one line (`--ed-measure: min(100%, 100ch)`) if
  readability on ultra-wide displays matters more than width; that is a follow-up,
  not part of this change.
