# wcode TUI — mouse support (plan)

Status: **☑ shipped (P1 + P2).** Companion to [`tui-plan.md`](tui-plan.md) (the project),
[`tui-design.md`](tui-design.md) (the visual spec) and
[`tui-browse-plan.md`](tui-browse-plan.md) (the mode this extends). `wcode-tui`
only; the kernel and the protocol are untouched.

## Problem

The TUI captures the mouse (`terminal.rs` — `EnableMouseCapture`) but throws the
*position* away: `event.rs::translate_mouse` maps only `ScrollUp`/`ScrollDown` to
a `Key` and returns `None` for every other kind — column and row never cross into
the app. So the only mouse gesture is the wheel; there is no way to **point at**
anything on screen. Three concrete gaps:

1. **No text selection.** With mouse capture on, the terminal's native selection
   is disabled; there is no in-app selection either, so a transcript line cannot
   be copied with the mouse (only `/copy` / `Ctrl-Y` / browse `y`).
2. **A block cannot be clicked.** Browse mode ([`tui-browse-plan.md`](tui-browse-plan.md))
   moves a `▌` bar with `j`/`k`/`{`/`}`, but a *mouse* cannot put it on a block.
3. **A team member cannot be clicked.** The docked sidebar draws the roster
   (`ui::draw_sidebar`), but switching surfaces is keyboard-only
   (`Ctrl-N` / `BackTab` / `Alt-1..9`).

## Design

### Positioned mouse events

One new event carries the position; the wheel keeps its current path.

- `event.rs::translate_mouse` stops discarding position: for `Down` / `Drag` /
  `Up` it emits a new `AppEvent::Mouse(MouseEvent { kind, col, row })`, where
  `MouseKind` is our own `Down | Drag | Up` (crossterm's `MouseEventKind` stays
  inside `event.rs`, the only crossterm-aware module). `Moved` (no button) is
  still ignored, as today.
- **The wheel is unchanged**: `ScrollUp` / `ScrollDown` still translate to
  `Key::ScrollUp` / `Key::ScrollDown`, so `on_key`'s scroll arms and their tests
  are untouched.
- `App::handle` routes `AppEvent::Mouse` to a new `App::on_mouse`, **not** gated
  by `Mode`: a mouse is not a key, so it works in both `Input` and `Browse`.
  (Browse still owns every *key*.)

### A hit map, published by the draw pass

There is no hit-testing infrastructure today. The cleanest seam is the one
`set_block_ranges` already uses: the draw pass *publishes* screen geometry, the
app consumes it. A small struct on `App`:

```rust
struct HitMap {
    /// The transcript viewport: its rect, the global line index of its top
    /// row, and the plain text of each visible row (index 0 = the top row).
    transcript: Option<TranscriptHit>,
    /// The open sidebar: its rect and each member row's (surface index, y).
    sidebar: Option<SidebarHit>,
}
```

- `ui::draw` **clears** it at the top of the frame, so a closed sidebar leaves no
  stale sidebar hit and a resize never leaves stale geometry.
- `draw_transcript` publishes `TranscriptHit { rect: body, top_line, rows }`,
  where `top_line = total - scroll - height` (the same `start` the bar pass uses,
  `ui.rs`) and `rows` is the plain text of the visible window it just built.
- `draw_sidebar` publishes `SidebarHit { rect, members: Vec<(usize, u16)> }` —
  each non-root surface's index and its screen `y`. This needs an index-carrying
  sibling of `member_rows` (`member_rows` today sorts and **drops** the surface
  index); `member_rows` stays for the strip and `/team`.

The app needs `ratatui::layout::Rect` for this — already an accepted coupling
(`app.rs` imports `ratatui::text::Line` for the render cache).

### Click = a Down/Up with no movement; drag = movement between them

`on_mouse` holds a tiny transient:

```rust
mouse_down: Option<(u16, u16)>, // the cell the button went down on
mouse_focus: (u16, u16),        // the latest cell
```

- `Down` → record the cell (and, if it is over a sidebar member row, focus that
  surface and stop — the sidebar has no drag semantics).
- `Drag` → update the focus cell.
- `Up` → if the focus cell differs from the down cell it was a **drag** (feature
  3); otherwise a **click** (feature 2).

### 2 — click a transcript block selects it (browse)

`Down`/`Up` on the same cell inside the transcript rect: map the row to a global
line (`top_line + (row - rect.y)`) and find the block whose recorded range
contains it (`focused().ranges`). If found: **enter `Browse`** (if not already)
and set `selected` to that block, then `reveal_selected`. A click on the live
(streaming) block or a blank separator is a no-op — the live block is never a
selection target ([`tui-browse-plan.md`](tui-browse-plan.md) invariant).

### 3 — drag selects transcript text, then copies it

- The selection lives in **global line/char space**: `(line, col)..(line, col)`,
  normalized. `line` indexes the rendered transcript rows (the same space as
  `ranges`); `col` is a char index in that row's plain text.
- A second pass in `draw_transcript` (mirroring the bar pass) restyles the
  selected chars with `Modifier::REVERSED`. It **only restyles** — it splits
  spans and changes no content, so no row's width changes and no row is injected
  (preserves `paint_bar_never_shifts_a_text_row` and
  `browse_injects_no_rows_so_the_measured_total_is_identical`).
- On release, the app joins the selected rows (char-sliced, `trim_end` per line)
  and pushes `Action::Copy` → the existing OSC-52 path. The selection highlight
  persists until the next `Down` in the transcript.
- **WYSIWYG**: the copied text is exactly the highlighted cells — the leading
  gutter is *included* (gutters are variable width — 3 cols for most blocks, 5
  for `btw`). Browse `y` remains the clean-text path for a whole block. Chars map
  to cells 1:1 (wide glyphs count as one), consistent with `markdown::disp`.

## Invariants future phases must preserve

- **No mouse state ⇒ the frame is byte-identical.** With no selection and no
  click, the hit map is invisible to the renderer; the input-mode frame and the
  measured `total` are unchanged. Pinned by a test.
- **A highlight restyles, never reflows.** `paint_bar_never_shifts_a_text_row`'s
  rule extends to the selection pass: a restyled row's width is unchanged.
- **Mouse never edits the composer.** A click/drag touches only the transcript
  selection and the focused surface; the input buffer and cursor are untouched.

## Locked decisions

1. One `AppEvent::Mouse` for `Down`/`Drag`/`Up`; the wheel stays `Key`-based.
2. The draw pass publishes a hit map; the app holds no layout constants.
3. Mouse is mode-independent (works in `Input` and `Browse`).
4. Click = no movement; drag = movement. A drag never selects a block; a click
   never copies text.
5. A transcript click **enters** `Browse` (the bar shows the target).
6. A sidebar click focuses the surface (`set_focus`); ignored when the sidebar is
   closed or the terminal is narrower than `SIDEBAR_MIN_WIDTH`.
7. Text selection is global line/char space; copy is WYSIWYG (gutter included).
8. No edge autoscroll while dragging (parked, P3).

## Open questions

- Should `Esc` clear a live text selection before it interrupts? **Settled: no**
  — `Esc` keeps its cancel/quit meaning; a new `Down` clears the selection.
- Should the highlight survive a wheel scroll? **Settled: yes** (it is global),
  but the *copy* is taken at release, so scrolling afterward cannot corrupt it.
- Double-click to expand a tool? Parked (P3).

## Phases

| phase | scope | status |
|-------|-------|--------|
| 1 | positioned mouse events + hit map + click-to-select block + click-to-focus surface | ☑ done |
| 2 | drag-to-select transcript text + highlight + copy | ☑ done |
| 3 | unlocks: edge autoscroll, double-click to expand, wide-char-exact mapping | ☐ todo |

## Commits

| # | subject |
|---|---------|
| 1 | `docs: plan TUI mouse support` |
| 2 | `tui: positioned mouse events + a hit map (click a block, focus a surface)` |
| 3 | `tui: drag to select transcript text and copy it` |

## Status

| # | commit | status |
|---|--------|--------|
| 1 | plan + tracker | ☑ done |
| 2 | clicks (block + sidebar) | ☑ done |
| 3 | drag text selection | ☑ done |

Legend: ☑ done · ◐ in progress · ☐ todo.
