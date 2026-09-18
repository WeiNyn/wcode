# wcode TUI — transcript browse mode (plan)

Status: **planned — phase 1 (skeleton) landing.** Companion to
[`tui-plan.md`](tui-plan.md) (the project), [`tui-design.md`](tui-design.md) (the
visual spec) and [`tui-polish-plan.md`](tui-polish-plan.md) (the pass this
extends). `wcode-tui` only; the kernel and the protocol are untouched.

## Problem

Every transcript shortcut is **position-blind**. `Ctrl-O` / `Ctrl-T` act on "the
last tool", `Ctrl-Y` (`/copy`) on "the last reply" — there is no way to point at
*a* block and act on it. A long transcript is a wall you can only scroll: you
cannot select the `bash` result three turns up to expand it, or the reply two
turns up to copy it. The gutter ([`tui-design.md`](tui-design.md) §4) already
reserves the marker column for exactly this — "the natural home for a `▌`
selection bar" — but nothing draws one.

## Design

### A mode, and a per-surface selection

- `enum Mode { Input, Browse }` on `App`, default `Input`. `Input` is the
  composer; `Browse` moves a selection over the **committed** transcript.
- `Surface` gains `selected: Option<usize>` — an index into the committed
  `transcript` only. The **live block is never a target**: it is transient and
  would shift to `len-1` the moment it commits.
- Enter with `Ctrl-G` (selecting the last committed block); leave with `Esc`,
  `q`, or `Ctrl-G`. Leaving preserves the composer draft and cursor — browse
  never writes to them.

### Mode gating

`Esc` in browse must leave browse and **never** reach `interrupt()`. A mode check
**precedes** the base key match — exactly like `on_overlay_key` /
`on_completion_key` — and browse owns *every* key while it is up, so a stray
keystroke cannot edit the composer behind the mode. Movement: `j` / `Down` next,
`k` / `Up` prev, `g` first, `G` last, `PgUp` / `PgDn` by a viewport (`Up` / `Down`
still recall history in input mode). The wheel keeps scrolling the view without
moving the selection.

### The bar: a second pass, not a threaded flag

The gutter is **not centralized** — `block_lines` bakes a prefix per arm, and
`markdown.rs` has its own `GUTTER`. Threading a `selected` flag through every
constructor across two modules would be large and fragile. Instead:

- In the loop that builds `lines`, record each committed block's `Range<usize>`
  (the start captured **after** the separator, so a range never spans the blank
  above it).
- Hand the ranges to the app via a new `App::set_block_ranges` → `Surface::ranges`.
  This is a **separate feedback method**, not a change to `sync_scroll`: that
  3-arg signature is called **directly by six `app.rs` tests**, so extending it
  would churn them for nothing.
- After the `start..end` drain, overwrite **column 0** of each visible row of the
  selected block with `▌` in `accent()`. Replace, never prepend — prepending
  shifts every glyph and breaks column alignment.

Ranges are re-measured every frame, so a resize (which re-wraps) never leaves a
stale bar.

### Scroll-to-reveal, and index hygiene

- When the selection moves, `scroll` is adjusted to show the block, biased to its
  top (a block taller than the viewport shows its first line). While browsing, a
  transcript that grows (a run appending blocks) re-anchors the view to the
  selection instead of yanking it to the tail.
- `selected` is clamped whenever the transcript changes length (a new run's
  blocks, `seed`'s front-inserted divider) — a stale index pointing at the wrong
  block is the failure mode here.
- The mode shows as a `▤ browse` token in the status band **only while browsing**,
  immediately before `state`, so an input-mode frame is unchanged and an in-flight
  `⠹ running` stays readable.

### Hard requirement

With `Mode::Input`, the frame is **byte-for-byte unchanged** and `total` /
wrapping are untouched: the bar injects no rows and the ranges add no lines.

## Phases

| phase | scope | status |
|-------|-------|--------|
| 1 | skeleton: mode, per-surface selection, second-pass bar, ranges feedback, scroll-to-reveal, mode indicator | ◐ landing |
| 2 | actions on the selection: `Enter` expand/collapse a tool, `y` copy a block | ☐ todo |
| 3 | navigation polish: `Home` / `End`, block-wise `{` / `}`, search | ☐ todo |
| 4 | unlocks: a `$PAGER` viewer, structured jumps (to the last error, …) | ☐ todo |

Phase 2+ is deliberately unimplemented here. A `$PAGER` needs a suspend/resume
path `terminal.rs` does not have (`TerminalGuard` only restores on drop) — its
own design, not this pass.

## Commits

| commit | subject |
|--------|---------|
| 1 | `docs: plan the transcript browse mode` |
| 2 | `tui: a browse mode with a selected transcript block` |

## Status

| # | commit | status |
|---|--------|--------|
| 1 | plan + tracker | ☑ done |
| 2 | browse-mode skeleton | ☐ todo |

Legend: ☑ done · ◐ in progress · ☐ todo.
