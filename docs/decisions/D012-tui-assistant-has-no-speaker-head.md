# D012 — TUI: the assistant turn carries no speaker head

- **Status:** accepted (human sign-off 2026-10-10)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §4 (Speaker head)
- **Extends:** D009 (the `YOU`/`WCODE` speaker head) · D010 (the pinned header)

## Context

Every assistant turn opened with a `WCODE` head on **its own row** (the head run
carried no text), while the user block opened inline (` YOU  …`). That spent one
transcript row per turn on a bare marker and made the two speakers asymmetric.

## Decision

The **user block** keeps its `YOU` head (inline, reverse-video). The **assistant
turn has no head** — it opens directly on its body (the `··· thinking` row or the
prose). The turn already rides the gutter/content column, so the assistant reads
as the page itself; only the user is labelled.

## Consequences

- `Speaker` (the `You`/`Wcode` enum) is removed; `speaker_lines` renders the
  user block only. `ui::body()` is now unused and gone.
- `turn_lines` returns the body directly; `turn_inline`'s row walk starts at
  `off = 0` (was `1`, for the retired head), and the thinking affordance's hit
  row is `row0` (was `row0 + 1`).
- One transcript row saved per turn; the pinned header's `WCODE` is the only
  remaining `WCODE` on screen.
- No new glyph; the glyph table is unchanged.
