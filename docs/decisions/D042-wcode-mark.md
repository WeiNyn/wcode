# D042 — The wcode mark: `❯▌`, one ink, two tiers, root-surface-only

- **Status:** accepted — locked by the human ("follow your recommendation")
  and **implemented**: assets + spec (designer) and the TUI banner + the
  extension's marketplace `icon` (developer). Second-layer review **APPROVED**
  with nits only. Root-only seeding was verified by mutation: neutering the
  `is_root` branch makes `the_mark_is_seeded_only_on_the_root_surface` fail.
- **Date:** this session
- **Relates to:** [`wcode-mark.md`](../design/wcode-mark.md) (the geometry and the
  tiers), [`gen.py`](../design/identity/gen.py) (the reproducible source),
  [`media/wcode.svg`](../../editors/vscode/media/wcode.svg),
  [`media/icon.svg`](../../editors/vscode/media/icon.svg),
  [`media/icon.png`](../../editors/vscode/media/icon.png), [`favicon.svg`](../../favicon.svg),
  [`docs/book/favicon.svg`](../book/favicon.svg),
  [`app.rs`](../../crates/wcode-tui/src/app.rs) (`EMPTY_HINT`,
  `seed_hint_if_empty`, `clear_hint`, `is_root`),
  [`W037`](../work/W037-wcode-identity.md)
- **Amends:** [`tui-design.md`](../tui-design.md) §1.4 (one sentence) and §4
  Transcript (the seeded empty state); **§2 is byte-unchanged**
- **Distinct from:** [`D023`](D023-tui-prose-only-bare.md) — D023 made the screen
  bare, and this record is careful not to spend what it removed: the mark adds no
  glyph, no band and no hue

## Context

wcode had two disconnected pieces of identity and no mark spec: a drawn `>_`
chevron that existed only in the VS Code activity bar
(`editors/vscode/media/wcode.svg`), and a **font-rendered** `>_` favicon
(`favicon.svg`, `&gt;_</text>` in a monospace stack, byte-identical into the
book). The terminal — the product — showed **no mark at all**: the first screen was one
dim line, the `EMPTY_HINT` notice, reading `❯ type a message · /help for
commands · F1 for keys` — which also wrapped at 48 columns (52 cells with the
notice's 3-cell gutter).

The constraint that decides everything else is that the activity-bar icon is a
24×24 `fill="currentColor"` SVG the host tints, and the TUI's default theme is
bare (`tui-design.md` §1.3: "**`error` red is the ONLY hue**"). A mark that needs
two inks, or a hue, fails one of the two surfaces.

## Decision

1. **The mark is `❯▌`** — the prompt (`❯`) and the live cursor (`▌`), the two
   glyphs the TUI is made of. Geometry is fixed in
   [`wcode-mark.md`](../design/wcode-mark.md) §1: chevron centreline
   `(3.6,5) → (10.6,12) → (3.6,19)` at stroke 2.5 with a sharp mitre; cursor rect
   `13.5→21 × 16.5→19.5` (7.5 × 3) with square corners; axis-aligned edges on a
   1.5-unit grid so the cursor is exactly 5 × 2 whole pixels at 16 px.
   `editors/vscode/media/wcode.svg` **is** that file, byte-for-byte.
2. **One ink.** `fill="currentColor"` on the root, no hex in the file, filled
   paths rather than strokes (some hosts force `svg { fill: … }` from CSS). It
   reads in solid grey and in solid white-on-dark because it has no counter, no
   hole and no overlap.
3. **Two tiers, one drawing.**
   - *Px tier* — the 24×24 icon, and the tile: a 128-unit field shipping at
     256×256 (`icon.svg` → `icon.png`), `#3fb950` on `#0b0e13`.
   - *Terminal tier* — the root surface's empty state, one `Block::Notice`, two
     dim lines, in the locked copy:
     `❯▌ wcode` then `❯ type a message · /help · F1 keys`, joined by `\n`.
     **Zero new glyphs.**
4. **The colour pair is `#3fb950` on `#0b0e13` (7.61:1).** Both are borrowed:
   the green is the colour the mark already wore on the live favicon, and the
   ground is the live book's `--bg`. The favicons' stray `#0d1117` is retired, so
   the tile, the favicon and the book share one ground.
5. **Root surface only** (`app.rs:1480` `is_root: bool,`). A member surface's empty
   state is the hint line alone: the team surfaces are already the brand, and a
   logo repeated on every idle worker reads as chrome.
6. **Retirement goes through `clear_hint`** (`app.rs:1817`) — the function that
   already retires the hint, extended to match either shape
   (`:1820` `if t.as_str() == EMPTY_HINT || t.as_str() == ROOT_EMPTY_HINT`).
   There is no second retire path, and no new block kind: the banner is a
   `Notice`. The wiring's constants are `EMPTY_MARK` (`app.rs:495`
   `"❯▌ wcode"`) and `ROOT_EMPTY_HINT` (`:503`, the two lines joined by `\n`).
7. **§2 is untouched.** The amendment text in §4 says so explicitly, because a
   reviewer of a mark that quotes glyphs will otherwise assume the glyph table was
   bypassed.

## Alternatives rejected

- **B — the wordmark's `w`** (four strokes, acute vertices bevelled flat). The
  best drawing of the three at 64–128 px, and it *is* the name; rejected because
  the mark would duplicate the wordmark it always sits beside, and because a
  letter cannot be typed into the banner without becoming that duplicate.
- **C — the cursor cell** (a filled rounded square with the prompt knocked out).
  The best marketplace tile; rejected because at 16 px it is a blob among the
  activity bar's line icons — measured **48.4% ink, 3.3× the chosen mark** — and
  because its terminal form needs a reversed cell (see below).
- **Drawn block letters (`█ ▀ ▄`).** `docs/design/tui-turn-notes.md:694` ("**+3
  rows per message**") and `:696` ("**Glyphs:** `█ ▀ ▄` are **not** in
  `docs/tui-design.md` §2") already refused them on the every-message banner. At
  startup the row objection weakens — one screen, once, 20 free rows on an 80×24
  empty screen — but the glyph objection does not: three glyphs are not in §2,
  and §2 is closed. D023 removed exactly this kind of decoration.
- **Reverse video for the banner's letters.** It avoids the §2 amendment, but a
  reversed cell is *one* pixel of letterform against `█▀▄`'s two, so the same
  letterforms cost **6 rows** instead of 3 — and a filled run already means
  **selection** here (`crates/wcode-tui/src/ui.rs:895`
  `Style::default().add_modifier(Modifier::REVERSED)`), a meaning D023
  deliberately took away from the speaker head.
- **`❯_` as the terminal tier** (the brief's sketch). `_` is a baseline dash that
  at one row reads as an underline under nothing, it is not in the §2 table, and
  it would make the terminal tier a *different* drawing from the px tier. `▌` is
  in the table twice (live cursor, selection bar) and it **is** the SVG's cursor
  block, which is what keeps the two tiers one mark.
- **A hue in the TUI** (an accent-tinted mark). The bare default has one hue and
  it is `error` red; the terminal tier is dim ink, which measures 8.9:1 on
  `#0b0e13` — quiet, not invisible.
- **An ink-only tile, no green.** Coherent for a TUI-first product, and it would
  make the tile cost zero. Rejected because a hue-less favicon is a dark square
  among dark squares in a tab strip, and because green is the colour the mark
  already wore — this design *keeps* the one colour instead of inventing one.

## Consequences and accepted edges

- **The two banner rows share a leading `❯`** (the locked copy): line 1's is the
  mark, line 2's is the prompt it belongs to.
- **The banner is dim end to end** (`ui.rs:934`,
  `Block::Notice(text) => wrap(text, width, "   ", "   ", dim())`), so on a
  coloured preset the logo stays grey rather than glowing. A full-strength mark
  line would need a second block arm — the pattern exists (`Block::Transient` is
  "a RENDERING TWIN of `Block::Notice`") — and `clear_hint` would have to learn
  it, widening the one retirement path. Owned, not taken.
- **The favicon still renders `>_` through a monospace font**, so its shape
  varies by machine. Accepted: the `<text>` → drawn-path fix is a separate
  one-file change (W037 non-scope), and this record only unifies the *ground*.
- **`icon.png` is a committed artefact, not a build step.** `.vscodeignore` ships
  `media/`, so the VSIX carries it as-is; regenerating it is one `rsvg-convert`
  command (spec §7), which is how it was produced.
- **The tile's field is rounded, so `icon.png` is RGBA with transparent
  corners.** The marketplace composes it on its own background; a square field
  would be a different drawing from the favicon's.
- **At 24 px, DPR 1, the cursor's edges land on a half pixel** (the 1.5 grid is
  exact at 16/32/64/128 and at DPR 2). Soft, not wrong, and documented in the
  spec rather than "fixed" by moving off the grid.

## Verification

Run by the designer in this round; every gate is reproducible from the spec §7.

- `cmp docs/design/identity/candidate-a.svg editors/vscode/media/wcode.svg` → 0
  (the shipped icon **is** the drawn mark, so the two cannot drift silently).
- `diff favicon.svg docs/book/favicon.svg` → exit 0: the two favicons stay
  byte-identical after the `#0d1117` → `#0b0e13` change.
- `rsvg-convert` at 16 and 24 px with the ink supplied as a host `color` (a
  `color=` attribute injected into a copy, i.e. tinted from **outside** the
  file): the mark rasterises with the chevron and the cursor as **two separate
  ink islands** and 14.8% ink at 16 px — the gap survives the smallest size.
- `file editors/vscode/media/icon.png` → `PNG image data, 256 x 256, 8-bit/color
  RGBA, non-interlaced`.
- `python3 docs/design/identity/gen.py` → reproduces `candidate-a.svg`
  byte-for-byte (and is idempotent across runs).
- The `app.rs` citations in Decisions 5–6 are read from the developer's
  in-flight wiring tree at the time of writing (uncommitted, and not this
  record's to own); the quoted snippets, not the numbers, identify them.
- `docs/tui-design.md` §2 md5 identical before and after the amendment.

**Not verified.** No VS Code host on this machine, so the activity-bar tint is
simulated (`#c5c5c5` on dark, `#424242` on light) and the marketplace listing is
unseen. The banner was **not** rendered in a terminal: this round touches no
crate, so the terminal tier exists as a spec change plus a mockup built on a live
pty capture of the shipped empty screen. Legibility at 16 px on a real HiDPI
display is the human's call, not a measurement.
