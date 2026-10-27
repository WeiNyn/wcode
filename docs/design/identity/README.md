# wcode identity — candidates, round 1

**Status: candidates only. Nothing here is wired.** The live marks
(`editors/vscode/media/wcode.svg`, `favicon.svg`, `docs/book/favicon.svg`), the
TUI, and `package.json` are untouched. This directory is the decision surface:
`candidates.html` is the sheet to open, `gen.py` is the source of all eight
SVGs, and this file is the argument.

```
docs/design/identity/
├── candidates.html          the contact sheet (open this)
├── candidate-a.svg          ❯▌   prompt + cursor      one ink, 24×24
├── candidate-b.svg          w    the wordmark's w     one ink, 24×24
├── candidate-c.svg          ▣    the cursor cell      one ink, 24×24
├── candidate-{a,b,c}-tile.svg     128×128 tiles (the only place colour lives)
└── gen.py                   regenerates all of the above
```

## 0. The constraints that are not up for negotiation

Verified, not assumed:

- **One ink.** The activity-bar icon is a 24×24 `fill="currentColor"` SVG the
  host tints, so colour is the host's. The TUI's default theme is bare:
  `docs/tui-design.md:40` "**`error` red is the ONLY hue**, and every other role
  is the grey ramp". So every mark here is a single solid ink and is checked at
  16×16 in solid grey and in solid white-on-dark.
- **The glyph table is closed** (`docs/tui-design.md:67`: "The **single** glyph
  table"). Sanctioned: `❯ ▌ ─ │ ╭ ╮ ╰ ╯ · ⠋ ⎇ ⏻ ● ○ ✓ ✗ ☑ ☐ › ⇥ ⏹ ❰ ❱`.
  `█ ▀ ▄` are **not**. Every terminal rendition below is built from the table.
- **The mark has never been drawn in a terminal.** The first screen is one dim
  line, `crates/wcode-tui/src/app.rs:480`
  `const EMPTY_HINT: &str = "❯ type a message · /help for commands · F1 for keys";`
  That gap — not the VS Code icon — is what round 1 is really about.

## 1. The three marks

Each is one 24×24 file with `fill="currentColor"` on the root and no hex
anywhere. Construction grid: **axis-aligned edges on a 1.5-unit grid**, so they
land on a whole pixel at 16, 32, 64 and 128 px (24/16 = 1.5, so a coordinate `c`
lands at `2c/3` px and a multiple of 1.5 is an integer). At 24 px, DPR 1, those
edges sit on a half pixel: visibly soft, never wrong, and whole again at DPR 2.
Diagonals cannot snap to any raster and are on a 0.1-unit grid.

### A — Prompt + cursor (`candidate-a.svg`)

**Concept.** The two glyphs the TUI is made of: the `❯` prompt and the `▌` live
cursor. Today's `>_` evolved — the underscore becomes a cursor block.

**Construction.** Chevron centreline `(3.6,5) → (10.6,12) → (3.6,19)`, stroke
**2.5** (today's chevron, kept), sharp mitre at the apex. Cursor: an
axis-aligned rect `x 13.5→21, y 16.5→19.5`, i.e. **7.5 × 3 units**, square
corners (a terminal cursor cell has no radius), bottom at the chevron's baseline.
Placed on the 1.5 grid, so at 16 px the block is exactly **5 × 2 whole pixels**.
Artwork box 18.28 × 15.77, margins 2.72 / 3.00 / 4.12 / 4.12 — optically centred,
a hair left, which is right for a right-heavy composition.

**At 16 px.** Two ink islands, measured: the chevron and the cursor never
touch. The block is 2 px of solid ink where the shipped bar is 1.47 px of
anti-aliased grey — this is the defect the candidate exists to fix.

**One ink.** The chevron is a filled band, the block is a filled rect; there is
no counter, no hole and no overlap, so inversion (white on dark) and grey-on-grey
both read identically.

### B — The wordmark's `w` (`candidate-b.svg`)

**Concept.** The initial, drawn in the same chevron language as `❯`, with every
acute vertex cut flat where a mitre would draw a needle.

**Construction.** Centreline `(3.7,6.1) (7.85,18.1) (12,6.1) (16.15,18.1)
(20.3,6.1)`, x-height band 12 units, stroke **2.2** (the shipped chevron's
weight), butt caps, and a **mitre limit of 1.6**: the three interior joints sit
at a 38° interior angle (mitre ratio 3.06), so each is bevel-cut flat, 2.2 wide.
That bevel is not a fudge — it is what a pixel font does to a `w` vertex, and it
is why the letter survives 16 px instead of ending in three 4-unit needles.

**At 16 px.** One island, 12.45 × 8.5 px of ink at 1.47 px stroke — the weight
of a bold terminal `w` in the very font the product renders in.

**One ink.** A monoline outline with no counter: nothing to fill, nothing to
show through.

**Where it fails.** It is a letter. It says "wcode" and nothing about what wcode
does, it makes the wordmark and the mark compete, and — the real problem for
this product — **it cannot be typed**: see §3.

### C — The cursor cell (`candidate-c.svg`)

**Concept.** The live cursor made solid, with the prompt knocked out of it: one
mass, one ink.

**Construction.** A rounded square `2.5 → 21.5` with radius **4.5**, which is
18.75% of 24 — exactly the ratio `docs/book/favicon.svg` already uses (`rx=6` on
32). Inside it, a chevron void: centreline `(9.1,7.4) → (13.7,12) → (9.1,16.6)`,
stroke **3.2**, mitre at the apex, drawn as a second subpath with
`fill-rule="evenodd"`.

**At 16 px.** One island, 48.4% ink — **3.3× candidate A** — and the void is
2.1 px wide inside a 12.7 px field. It reads, but it reads as a filled chip, not
as a line icon.

**One ink.** Yes, and it is the most robust of the three at any size, because
the shape is one mass with one hole: no thin strokes to lose.

**Where it fails.** In the activity bar it is a blob among line icons, and — see
§3 — it has no honest terminal rendition.

## 2. What was measured (not asserted)

`rsvg-convert` at each real size, the file's own `currentColor` resolved from a
host-supplied `color`, ink = pixels above the half-coverage level, islands =
8-connected components of that ink.

| mark | 16 px ink | islands | 24 px ink | 32 px ink |
|---|---|---|---|---|
| shipped `>_` | 8.6% | 2 | 10.6% | 9.6% |
| **A** `❯▌` | 14.8% | **2** | 14.9% | 12.9% |
| **B** `w` | 17.2% | 1 | 17.7% | 17.8% |
| **C** cell | 48.4% | 1 | 54.5% | 54.2% |

Read it as: A keeps the shipped silhouette and adds 6 points of ink, all of it
in the cursor (the shipped bar is 1.47 px; A's block is 2 px of solid ink); B is
the same weight as A in one piece; C is three times the ink of either, which is
fine on a 128 px tile and heavy in a 24 px bar.

## 3. The startup banner

### Today, from a live binary (not a mockup)

`target/debug/wcode --tui --base-url http://127.0.0.1:9/v1`, captured from a
real pty at 80x24 and 48x24. Rows are elided, not reflowed:

```
WCODE · session d25e154a ───────────────── wcode · ⎇ main*   deepseek-v4.1-flash
   ❯ type a message · /help for commands · F1 for keys
[… 20 blank rows …]
 ❯ ▌
                                                                          ⏸ idle
```

```
WCODE ──── wcode · ⎇ main*   deepseek-v4.1-flash
   ❯ type a message · /help for commands · F1
   for keys
[… 19 blank rows …]
 ❯ ▌
                                          ⏸ idle
```

Two facts fall out. An 80x24 empty screen has **20 free rows** between the hint
and the composer — a startup-only banner is spending nobody else's rows. And at
48 cols the shipped hint **already wraps** (its third row is `for keys`), so a
banner that ships a shorter hint string *fixes* a live defect rather than adding
to it.

### The renditions

The banner replaces the seeded `EMPTY_HINT` block: one `Block::Notice` whose
string has two lines, **both `dim`** — a notice is dim from end to end
(`crates/wcode-tui/src/ui.rs:934`, `wrap(text, width, "   ", "   ", dim())`),
which is why the hint already sits at that indent and why its wrapped
continuation lines up under it. It retires on the first submitted turn exactly
as the hint does today — `crates/wcode-tui/src/app.rs:1771`
(`seed_hint_if_empty`) pushes the constant, `:1784` (`clear_hint`) matches
`Block::Notice(t) if t.as_str() == EMPTY_HINT`, so a two-line constant keeps
both the seeding and the retirement working with no new block kind. A `\n` in a
notice already renders as lines (`render_todos` returns exactly that shape).
Root surface only (`app.rs:1457` `is_root`): the hint belongs on every empty
surface, a brand banner repeated on each idle worker does not.

**Both lines dim is the recommendation, not a compromise.** It is the only
variant that costs nothing: one constant, no new block kind, `clear_hint`
untouched, and `dim` is the spec's workhorse — the mark reads as quiet chrome on
an otherwise empty screen, which is what a bare TUI-first product should look
like — `dim` is `#a9b1ba`, **8.9:1** on a `#0b0e13` terminal, so a quiet banner
is still legible ink, not a whisper. Making the mark line full strength is the *costed* alternative: a Notice
carries one style, so it needs a second block arm (the pattern exists —
`Block::Transient` is "a RENDERING TWIN of `Block::Notice`") and `clear_hint`
would have to learn it, i.e. the one function that retires the banner would no
longer be one function. That trade — a louder logo for a wider retirement path
plus a §4 line — is the human's call, not the design's.

#### A — prompt + cursor

80 cols:

```
WCODE · session d25e154a ───────────────── wcode · ⎇ main*   deepseek-v4.1-flash
   ❯▌ wcode
   ❯ type a message · /help · F1 keys
[… 19 blank rows …]
 ❯ ▌
                                                                          ⏸ idle
```

48 cols:

```
WCODE ──── wcode · ⎇ main*   deepseek-v4.1-flash
   ❯▌ wcode
   ❯ type a message · /help · F1 keys
[… 19 blank rows …]
 ❯ ▌
                                          ⏸ idle
```

Cost: **+1 row** at 80 (row 3 was blank), **+0 rows** at 48 (it replaces the
wrapped second line), **+0 glyphs** (`❯` and `▌` are both §2 rows). The wordmark
is lowercase because the binary is `wcode`; the header keeps its uppercase
running head, so the two spellings stay distinguishable.

#### B — the wordmark's `w`

80 cols:

```
WCODE · session d25e154a ───────────────── wcode · ⎇ main*   deepseek-v4.1-flash
   wcode
   ❯ type a message · /help · F1 keys
[… 19 blank rows …]
 ❯ ▌
                                                                          ⏸ idle
```

48 cols:

```
WCODE ──── wcode · ⎇ main*   deepseek-v4.1-flash
   wcode
   ❯ type a message · /help · F1 keys
[… 19 blank rows …]
 ❯ ▌
                                          ⏸ idle
```

That is the failing: B's mark *is* the wordmark, so it cannot be drawn beside the
name it duplicates. B's only distinct terminal form is the drawn block-letter
wordmark, priced in §3.1.

#### C — the cursor cell

80 cols:

```
WCODE · session d25e154a ───────────────── wcode · ⎇ main*   deepseek-v4.1-flash
    ❯ wcode
   ❯ type a message · /help · F1 keys
[… 19 blank rows …]
 ❯ ▌
                                                                          ⏸ idle
```

48 cols:

```
WCODE ──── wcode · ⎇ main*   deepseek-v4.1-flash
    ❯ wcode
   ❯ type a message · /help · F1 keys
[… 19 blank rows …]
 ❯ ▌
                                          ⏸ idle
```

The mark needs a filled cell, and `█` is not sanctioned, so C's rendition is
reverse video (SGR 7) over the three cells `" ❯ "` — a *style*, and
`Modifier::REVERSED` is already reified at `crates/wcode-tui/src/ui.rs:895`
(`selection_style()`). The bytes are `"    ❯ wcode"`: three gutter spaces, then a
filled cell containing the prompt, then the wordmark. Because the bytes are
spaces this needs no font coverage at all — but a terminal that ignores SGR 7,
or a `NO_COLOR` path that strips attributes, shows a bare `❯`: candidate A minus
the cursor. **C has no fallback that is still C.**

Worse, reverse video is not a free style in this TUI: it already means
**selected text** (`crates/wcode-tui/src/ui.rs:895`, `selection_style()`, used by
the drag highlight and `recolor_range`), and D023 retired the reverse-video
speaker head on exactly that ground. A static filled cell would be a second
meaning for the one highlight the product has.

### 3.1 The block letters, priced again

`docs/design/tui-turn-notes.md:676` drew a 3-row `█▀▄` `WCODE` and rejected it
at `:693`: "**+3 rows per message** … **Glyphs:** `█ ▀ ▄` are **not** in
`docs/tui-design.md` §2". That verdict was about a banner on *every message*, and
a **startup-only** banner escapes the row half of it: one screen, once, with 20
free rows (§3). The glyph half survives, and it is the half that matters —
`W` has no spelling in `❯ ▌ ─ │ ╭ ╮ ╰ ╯ ·`.

What the earlier record did not cost out: `█▀▄` is a **half-block font** — two
vertical sub-pixels per cell, so a 6-pixel-high letterform in 3 text rows.
Reverse video over spaces gives **one** pixel per cell. So the choice is:

| route | letterform | rows | new glyphs | meaning clash |
|---|---|---|---|---|
| `█ ▀ ▄` | 6 px high | **3** | 3 → §2 amendment | none (new glyphs) |
| reverse video, 3 rows | 3 px high (crude) | 3 | none | **`REVERSED` = selection** |
| reverse video, 6 rows | 6 px high (same as `█▀▄`) | **6** | none | **`REVERSED` = selection** |
| **the mark, 2 rows** | n/a | **2** | **none** | none |

**Verdict: do not draw the letters.** The `█▀▄` route buys the best-looking
banner and costs a §2 amendment for a decoration the spec spent D023 removing;
the reverse-video routes avoid the amendment but spend the product's only
highlight (a filled run already means *selection*, `ui.rs:895`) and either
double the rows for the same letterforms or halve the letterforms for the same
rows. A two-row mark banner
costs no glyph, no amendment, and reads in one ink under `NO_COLOR`. If the human
wants the drawn letters anyway, the order is: reverse video at 6 rows (no
amendment) > `█▀▄` at 3 rows (amendment) — and B is the candidate that makes the
best case for it, because its mark is a letter.

## 4. Colour: one brand hex, and it is not the TUI's

The TUI gets **no** colour from this: the mark is ink, and the bare default has
exactly one hue (`error` red). Colour appears in exactly two places, both web:
the 128 px tile and the favicon.

Four palettes disagree; the tie-break is *which one already attaches a colour to
the mark*:

| source | hex | role |
|---|---|---|
| `docs/book/favicon.svg` (live, served) | `#3fb950` on `#0d1117` | **the mark's ink** |
| `docs/book/style.css:6` (live site) | `#0b0e13` | the site's surface |
| `docs/book/style.css:13` | `#6ea8fe` | the site's structural accent (links) |
| `style.css:12` (root site, **retired**, unserved) | `#58a6ff` / `#3fb950` | superseded |

**Recommendation: ink `#3fb950`, surface `#0b0e13`.** Green is the only colour
ever attached to the *mark* (`docs/book/favicon.svg` is the live favicon), and
`#0b0e13` is the live site's own background — adopting it retires the favicon's
stray near-duplicate `#0d1117` and puts the tile on the same surface as the page
it appears on. The book's `#6ea8fe` is *structural* (a link colour), not a mark
colour; promoting it would repaint the identity to match a hyperlink.

**The defensible alternative, stated so the human can take it:** ink-only. A
white mark on `#0b0e13`, no hue at all, is coherent for a bare TUI-first product
and makes the tile cost zero. What it loses is the tab strip: a hue-less favicon
is a dark square among dark squares. If the human picks ink-only, every tile file
here becomes `fill="#f4f4f0"` on the same surface and the shape argument is
unchanged.

## 5. What a wiring round would have to amend

Per candidate, the *spec* work is the same shape; only the mark changes.

1. **`docs/tui-design.md` §4** — "With **no** history the transcript opens on a
   dim seeded hint" becomes a two-line dim seeded **banner**: the mark and
   wordmark over the hint, retired by the first block, unchanged otherwise. Plus
   one line in §1 naming the mark as the product's only logo. (Full-strength
   mark line, if the human wants it, is the extra block arm priced in §3.)
2. **`EMPTY_HINT`** (`app.rs:480`) — the string, and nothing else: `❯▌ wcode` +
   the shortened hint. Both `seed_hint_if_empty` and `clear_hint` read the same
   constant, so no code moves.
3. **§2 stays untouched for A and B.** C needs a §4 line saying the banner may
   carry a reversed cell, and an explicit `NO_COLOR` fallback (the current §1.3
   sentence lists "only bold/italic/dim" for `Theme::plain`, so reverse video
   under `NO_COLOR` is currently undefined).
4. **The extension icon** — a file swap for `editors/vscode/media/wcode.svg` plus
   the tile at `editors/vscode/media/` for the marketplace, and the same 24×24
   file for the two favicons. No spec governs these; the contract is the
   `fill="currentColor"` root, which all three candidates satisfy.

Nothing in this directory needs deleting to take a decision: `candidates.html`
is the only file a later round reads, and `gen.py` regenerates the rest.

## 6. Relation to the W037 brief

`docs/work/W037-wcode-identity.md` §10 asked four questions. How this round
answers them, including where it disagrees:

- **Q1 (what is the mark)** — the orchestrator asked for three genuinely
  different ideas, so the slots went to breadth rather than to the brief's
  "2-3 proportions of one concept + at most one control". **A is the brief's
  recommendation** ("evolve `>_`") and is drawn to finished proportions: stroke
  2.5, block 7.5 x 3, a 1.5-unit grid. Re-tuning A to three proportion variants
  is one number away (`A_CHEVRON`, `A_BLOCK` in `gen.py`) and needs no redraw.
  B and C are the two controls the brief said would be one.
- **Q2 (what the banner costs)** — the brief's (iii) sketch is `❯_` on one row,
  a blank line, then the dim hint. This round differs twice. First, **`▌` not
  `_`**: `_` is a baseline dash that reads as an underline under nothing, and it
  is not in the §2 table; `▌` is the table's own cursor and selection glyph and
  it is *the same idea as the SVG's cursor block*, so the px tier and the
  terminal tier are one drawing rather than two marks that resemble each other.
  Second, **no blank line** inside the banner: the mark row and the hint are one
  `Notice` block, and the blank-line rule separates blocks, not lines.
- **Q2 (i), reverse video** — the brief objected that a static filled run
  dilutes `REVERSED` = selection. That objection is adopted here and it is why
  **C ranks last**: its terminal rendition needs the same highlight (§3).
- **Q3 (colour)** — agrees on `#3fb950`, with two amendments. The live
  justification is `docs/book/favicon.svg` (served), not the root site's
  `--prompt: #3fb950`, whose page is retired and unserved
  (`docs/next-steps.md` row 93). And the tile should adopt the live book's
  **surface** `#0b0e13`, which retires the favicon's stray `#0d1117`.
- **Q4 (tile now)** — the tiles here are SVG, not PNG, because a candidate round
  does not commit binaries. `rsvg-convert` rasterises them at any size on this
  machine, so the PNG is a one-line step at wiring time.
- **Q5 / Q6** — adopted as written: root surface only (`app.rs:1457` `is_root`),
  retirement through `clear_hint`.

## 7. Verified / not verified

**Verified.** The eight SVGs parse and rasterise (`rsvg-convert`); each resolve
`currentColor` from a host-supplied `color` with no hex in the file; the ink and
island numbers above are measured from those rasters; `candidates.html` was
rendered headless (Chromium) and checked to have no external request, all six
`<use>` references resolving, and ink in every cell; the two 24-row terminal
frames are a real pty capture of the shipped TUI binary.

**Not verified.** Legibility in a human's eyes at 16 px on a real HiDPI display —
the rasters prove ink and separation, not taste. No VS Code host was available,
so the activity-bar tint is simulated (`#c5c5c5` on dark, `#424242` on light).
The banner was **not** rendered in a terminal: it exists as a spec change and a
mockup, and the mockup's numbers come from the captured frame rather than from a
patched binary. Terminal font coverage matters only for C's reversed cell; A and
B use `❯` and `▌`, which the shipped hint and composer already draw on every
screen.
