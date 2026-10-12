# D023 — The TUI is prose only, fully bare (P5)

- **Status:** accepted (human sign-off — this session; W018)
- **Date:** this session
- **Relates to:** D009/D010 (inline tools; the pinned header + the tool tree),
  D012 (the assistant has no speaker head), D018 (live thinking as an annotation),
  D019/D020 (tool params; a collapsed tool is one row), D021 (no reference marks),
  D029/D030 (the grey ramp; the colour ladder), D032 (the affordance click
  geometry — **retired here**), D033 (thinking collapse)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) — the status line, §1, §2
  (the glyph table), §4 (Header · Speaker head · Tools · Thinking · Affordances ·
  Gutter · the composer foot), §5 (palette), §6
- **Tracker:** [`next-steps.md`](../next-steps.md) item 81 · **Work item:**
  [`work/W018-minimalist-tui.md`](../work/W018-minimalist-tui.md)

## Context

The TUI was dressed: three full-width `─` rules (header, composer, foot leader),
reverse-video `WCODE`/` YOU` labels, a `├`/`│`/`└` tool tree, a `▸`/`▾`/`▣`
disclosure-and-copy affix with mouse click geometry, a `···` thinking marker, and
a green→yellow→red `▰▱` context gauge. The human asked (verbatim) for "prose only,
fully bare": the transcript is the model's prose at default weight, *everything
else* `dim`, and **red the only colour**. Three phases, one logical change each:

- **A** `312a807` — *chrome*: the header/composer rules and the foot leader gone;
  `WCODE`/` YOU` plain; the gauge becomes text.
- **B** `0981bda` — *glyphs*: the tool tree, the affix + its click geometry, and
  the `···` marker retired.
- **C** `b62fd0e` — *palette*: `colored()`/`palette_b()`/`plain()` collapse to the
  grey ramp, `error` the only hue (`theme.rs:74` `error: Style::new().fg(Color::Red)`).

(A review nit folded in as `f7c13ec`.)

## Decision

Make the **bare look the fixed default** (palette B), not a preset:

1. **No band rules.** The header band is ONE row — `HEADER_H` is 1 (`ui.rs:62`)
   and `draw_header` draws only `session_head_row` (`ui.rs:419`); its `─` survives
   only as the *leader* between its runs (`ui.rs:712`). The composer has no rule
   row and its foot leader is spaces (`ui.rs:1801`
   `padded_row(&bl, &br, " ", dim(), w)`).
2. **Head labels are plain `dim`**, not reversed — `WCODE` (`ui.rs:677`) and
   ` YOU` (`ui.rs:624`). `Modifier::REVERSED` now survives **only** for
   drag-select (`selection_style`, `ui.rs:563`).
3. **Tools are treeless** — a flat row at col 3 (`TOOL_INDENT`, `ui.rs:33`;
   `collapsed_head_row`, `ui.rs:816`); the affix and its hit geometry are gone.
4. **Thinking** is a plain dim `thinking` row (`ui.rs:1232`); live it closes with
   the elapsed + `▌`.
5. **The gauge is text** — `used/limit`, flush-left (`ui.rs:1968`
   `format!("{}/{}", format_tokens(used), format_tokens(limit))`).
6. **One colour.** Every non-`error` role resolves to the grey ramp; the markdown
   fence uses a grey-only syntax theme (`markdown.rs:543` `grey_syntect_theme`).
   "One colour" means **no hue but red** — grey is the achromatic ramp, not a
   colour.

## Consequences

- **The mouse behaviour swaps.** A tool row is no longer a click-to-toggle/copy
  cell: a click now selects the block / enters browse. Keyboard parity is
  preserved — `Ctrl-T` toggles every tool's output, browse `Enter`/`Space`/`y` act
  per block — so the drop is a deliberate trade-off, not a gap.
- **The preset skeleton moves.** `colored()` is the bare skeleton every preset
  seeds from (`theme.rs:68`), so dropping the `accent`/`tool_name` bold also
  removes that weight from a preset, which now varies only `fg`; the seven presets
  stay coloured escape hatches.
- **The team state collapses.** `state_style` maps Running→`success()` and
  Idle→`dim()` (`ui.rs:2062`), now the SAME grey, so the `●`/`○` glyph
  (`lib.rs:231`) carries running-vs-idle instead of colour.
- **`NO_COLOR`.** `Theme::plain` keeps no `fg`, so red is unavailable and `error`
  degrades to **bold** (`theme.rs:104` `error: bold,`) — "no colour" still holds.
- **Cost / foreclosed:** a tool's reveal and copy are keyboard-only; the moving
  team state carries no colour cue beyond its glyph; the default loses per-token
  fence hue. No `bare` preset (the bare look *is* `default`); no colour but
  `error`. A release gate is a **pure-`─` line** — one composed *only* of `─` —
  never the head row's internal leader.
