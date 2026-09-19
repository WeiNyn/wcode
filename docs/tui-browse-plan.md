# wcode TUI — transcript browse mode (plan)

Status: **☑ phases 1–2 landed.** Companion to
[`tui-plan.md`](tui-plan.md) (the project), [`tui-design.md`](tui-design.md) (the
visual spec) and [`tui-polish-plan.md`](tui-polish-plan.md) (the pass this
extends). `wcode-tui` only; the kernel and the protocol are untouched.

## Problem

Every transcript shortcut is **position-blind**. `Ctrl-T` acts on *every* tool
and `Ctrl-Y` (`/copy`) on "the
last reply" — there is no way to point at
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
`on_completion_key` — and browse owns the text and navigation keys while it is
up, so a stray
keystroke cannot edit the composer behind the mode. **`F1`/`?` (help) and `Ctrl-C`
(cancel / quit) stay global**; every other key is ignored. Movement: `j` / `Down`
next,
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
| 1 | skeleton: mode, per-surface selection, second-pass bar, ranges feedback, scroll-to-reveal, mode indicator | ☑ done |
| 2 | actions on the selection: `Enter`/`Space` toggle a tool, `y` copy a block; `Ctrl-O` retired from input mode | ☑ done |
| 3 | navigation polish: `Home` / `End`, block-wise `{` / `}`, search | ☑ done |
| 4 | unlocks: a `$PAGER` viewer, structured jumps (to the last error, …) | ☐ todo |

Phase 2 landed: `Enter`/`Space` toggle the selected block and `y` copies it. The
input-mode `Ctrl-O` last-tool shortcut was **retired** — a shortcut whose target is
invisible is the defect — so per-block toggling lives only in browse mode, where
the bar draws the target; `Ctrl-T` (all tools) stays the input-mode escape hatch.
Phase 3 landed: `Home`/`End` anchor to the visible viewport, `{`/`}` step
block-wise, and `/` opens a search prompt (typing filters live; `Enter` jumps to
the first block at/after the selection whose copy-text contains the term,
case-insensitive, wrapping, then closes; `n`/`N` repeat it) — the composer
draft is never touched, and the search prompt is itself an overlay, so the
input-mode invariants below hold with it up too. The `$PAGER` unlock is still
unimplemented: it needs a suspend/resume path `terminal.rs` does not have
(`TerminalGuard` only restores on drop) — its own design.

Phase 1 landed as the skeleton, then `F1`/`?` and `Ctrl-C` kept
global in browse, plus a test-only commit pinning the invariants below.

### Browse keys

| key | action |
|-----|--------|
| `j` / `k` · `Down` / `Up` | next / previous block |
| `g` / `G` | first / last committed block |
| `Home` / `End` | topmost / bottommost block **intersecting the visible viewport** (falls back to first / last before the first frame) |
| `{` / `}` | previous / next block, clamped at the ends (an Alt-delivered chord — macOS Option-8/9, German AltGr — steps the same way) |
| `PgUp` / `PgDn` | a viewport of blocks |
| wheel | scroll the view without moving the selection |
| `Enter` / `Space` | expand / collapse the selected block's detail |
| `y` | copy the selected block |
| `/` | open the search prompt (see below) |
| `n` / `N` | repeat the last search forward / backward |
| `Esc` / `q` / `Ctrl-G` | leave browse |
| `F1` / `?` · `Ctrl-C` | global: help · cancel (quit when idle) |

While the **search prompt** is up (an overlay drawn above the input band, so the
base frame never reflows): typing filters the block list live, `Enter` jumps to
the first committed block at/after the selection whose copy-text contains the
term (case-insensitive, wrapping) and closes, `Backspace` edits the query, and
`Esc` closes without jumping. An empty term or no match is a no-op. `n`/`N`
repeat the last *executed* search strictly forward/backward from the current
selection, wrapping. The composer draft and cursor are never touched.

## Invariants future phases must preserve

- **`Mode::Input` is byte-identical.** The bar injects no rows, so the measured
  transcript `total` — and therefore `max_scroll` — is the same in both modes,
  and the ranges add no lines. Pinned by
  `browse_injects_no_rows_so_the_measured_total_is_identical` and
  `leaving_browse_restores_the_input_frame`.
- **`paint_bar` replaces, it never prepends.** It skips leading empty spans so
  the glyph at column 0 is the one replaced; a blank markdown row gains exactly
  the bar (`0 → 1`), and no text row's width changes. Pinned over every block kind
  by `paint_bar_never_shifts_a_text_row`. It replaces one glyph with one, so a
  future block kind whose rows did **not** start with a 1-cell gutter glyph would
  need a display-width-aware bar.
- **Browse owns the text and navigation keys; `F1`/`?` and `Ctrl-C` stay global;
  every other key is ignored.** A new browse key is an arm in `on_browse_key`,
  not a global binding.

## Commits

| commit | subject |
|--------|---------|
| 1 | `docs: plan the transcript browse mode` |
| 2 | `tui: a browse mode with a selected transcript block` |
| 3 | `tui: browse navigation & search (Home/End, {/}, /, n/N)` |

## Status

| # | commit | status |
|---|--------|--------|
| 1 | plan + tracker | ☑ done |
| 2 | browse-mode skeleton | ☑ done |
| 3 | browse actions (`Enter`/`Space`, `y`) + the last-tool shortcut retirement | ☑ done |
| 4 | navigation polish: `Home`/`End`, `{`/`}`, `/` search | ☑ done |

Legend: ☑ done · ◐ in progress · ☐ todo.
