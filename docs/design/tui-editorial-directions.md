# wcode TUI — editorial directions (5 proposals)

- **Status:** draft, for a human to react to. **Not authorized** — no code is
  changed, nothing in `crates/wcode-tui/` is touched, and `docs/tui-design.md`
  is **not** amended. A genuine pick amends that spec first (`design-taste`
  §7.3).
- **Refines / extends:** [`docs/work/tui-render/mockups.md`](../work/tui-render/mockups.md)
  (v1, selection mockups) and
  [`docs/work/tui-render/redesign-v2.md`](../work/tui-render/redesign-v2.md)
  (v2, D29–D36 — shipped). Neither is repeated; every frame here is *new*.
- **Stance precedent:** [`docs/design/vscode-paper-direction.md`](vscode-paper-direction.md)
  — the "paper" refinement for the VS Code **webview** (Medium B). These are
  **terminal translations of that stance** (Medium A), not copies of the web CSS:
  no serif, no small caps, no proportional type exists here.
- **Date:** 2026-02-14

---

## 0. Surface read

> Reading this as: **a terminal surface (Medium A)**, for **a developer reading a
> long-multi-agent session**, in **the locked wcode spec language**
> (`docs/tui-design.md` + `crates/wcode-tui/src/theme.rs`), with the dominant
> constraint that **density at 80×24 is fixed by the spec while editorial
> reading wants air** — the one tension every direction below has to pay for.

Medium **A**, not B. The site's token rules and the VS Code webview's serif are
irrelevant here; the law is §1–§2 of `docs/tui-design.md` and the 17 roles in
`theme.rs`. *(No clarifying question: the brief names the surface, the shape
("book / arXiv"), and the constraint.)*

---

## 1. The three dials — and the density fight (the central problem)

The `design-taste` skill **pins** the TUI dials (`SKILL.md:41-52`):

| dial | value | why |
|---|---|---|
| `DESIGN_VARIANCE` | **1, fixed** | "the spec is the design"; a reading page has no layout freedom left to spend. Hierarchy is bought in *space*, never in a new arrangement. |
| `MOTION_INTENSITY` | **1–2** | only the `▌` live cursor and the `⠋⠙⠹⠸` spinner exist; both are frozen under `NO_COLOR`/reduced-motion. These directions add **no** motion. |
| `VISUAL_DENSITY` | **the fight** | the skill pins TUI density at **7–8** (`SKILL.md:43`); editorial reading wants **≈5–6**. |

**This is the whole design problem, stated plainly.** "Editorial" in a terminal
is *leading* (blank rows), and every blank row is one fewer transcript row on a
21-row grid. The skill pins tight because an 80×24 grid is scarce. A reading
surface wants air. **Every direction below must *buy* its air and say what it
spent.** The three ways to pay:

1. **Move** apparatus out of the running text instead of deleting it — a footnote
   at the turn's foot or a margin column costs no *reading* rows, only rows the
   apparatus already occupied (Preprint, Marginalia).
2. **Trade** one air move for another — the first-line **indent** *replaces* the
   between-paragraph blank rather than adding to it (Chapter; the same wash
   stated in `vscode-paper-direction.md:43` "a rhythm change, not a density
   change").
3. **Spend, then buy back** — pay for leading by collapsing *all* apparatus into
   one summary row (Reader).

A direction that cannot name its payment is a direction that just fails at
80×24, and is marked so below.

**What is fixed for all five** (the spec invariants; cited in §13):

- Exactly **three bands** — transcript (flex) → [team rider, 0–3 rows] → input
  box (`crates/wcode-tui/src/ui.rs:124-129`, `docs/tui-design.md:14-21`). No
  fourth band, no title bar (`docs/tui-design.md:322`).
- The **gutter**: a 1-col margin, a marker column, content at a fixed column
  (`docs/tui-design.md:22-24`).
- Glyphs come **only** from the §2 table (`docs/tui-design.md:50-79`); a new
  glyph is a spec amendment with a real call site.
- Palette = the **17 named roles** (`crates/wcode-tui/src/theme.rs:19-58`); **≤1
  accent** (`accent`, cyan); hex only under `Rgb`.
- A blank line **between roles** (`crates/wcode-tui/src/app.rs:1235-1237`,
  `docs/tui-design.md:42`).
- Every interactive thing has a **key** and a `KEYS` entry
  (`docs/tui-design.md:275-296`).

**Where a "running head" and a "folio" can live without a title bar.** A *fixed*
top row is a title bar and is banned (`docs/tui-design.md:322`). So:

- the **running head** reuses the input box's **top-left corner**, which the
  binary already composes as `project · ⎇ branch · session a1b2c3d4` — **two
  `·`** — via `join_title` (`crates/wcode-tui/src/ui.rs:1441`
  `format!("{base} · {extra}")`, called at `:1379-1382`); and
- the **folio** (a page-number-like position marker) has one **new home per
  direction** — the preferred one is a **new right-hand cell** in
  `corner_titles`' bottom row, beside the existing `↑ N` scroll cell
  (`crates/wcode-tui/src/ui.rs:1405-1425`, the `br` closure), joined by a
  **space, not a second `·`** (so the ≤1 `·` rule, `SKILL.md:143`, still holds);
  Chapter instead prints it at the **chapter's foot**, like a printed page
  number. Either way it is a real call site, so the folio is a *spec amendment*,
  not an invention.

---

## 2. How to read the frames

Style is invisible in plain text, so each frame carries a **legend** naming the
`theme.rs` roles. Column 1 is the 1-col margin; content sits at the fixed gutter
column (`docs/tui-design.md:22-24`); a tool panel's body sits at **col 6**
(`docs/tui-design.md:24-27`, `crates/wcode-tui/src/ui.rs:34` `PANEL_INDENT`). The
input box (bottom 3 rows) is **unchanged** in every frame except where noted —
these directions re-set the *transcript*, not the box.

Role vocabulary used across the frames
(`crates/wcode-tui/src/theme.rs:19-58`): `accent` (cyan, bold), `user`, `body`
(default), `dim` (the workhorse grey), `muted`, `border`, `heading`,
`heading_sub`, `code` (yellow), `tool_name`, `thinking`, `link`, `error`,
`success`, `warn`, `diff_add`, `diff_del`.

---

## 3. Direction 1 — **Preprint**

### (a) Stance + dials

> **A session read as an arXiv preprint:** each turn is a numbered **section**,
> one centered **measure**, each tool call leaves a printed **reference** in the
> sentence, and the apparatus collects as **footnotes** at the turn's foot under
> a short rule; a code block is a captioned **plate**.

`VARIANCE 1 · MOTION 1 · DENSITY 6`. **Payment (a *Move*, §1):** the tool
apparatus *relocates* — the D31 panel becomes a turn-foot footnote. It is **net
≈ 0 rows**: the footnote **drops** the panel's top and bottom frame rows (−2) and
**adds** the short rule and one blank row (+2), and it spends **no reading row**
the prose was using. The only air it *buys* is the measure (free, horizontal).

### (b) Rendered frame — 80×24 (idle, one completed turn)

````
 ❯ how does edit resolve an anchor?

 §1  the anchor is the text

   An anchor is the text you quote — the old 5-char hash is gone (D007), so
   a line's address is its own content, and a reformatter moves it. The read
   tool prints `file:line` and edit matches that text literally.¹

   The drift-proofing is the point: every edit targets one snapshot, so an
   edit above a target never shifts it — there is no number left to shift.

   ── notes ────────────────────────────────────────────────────────────
   1 ⚙ read  crates/wcode-cli/src/tools/edit.rs
     ✓ read · 128 lines · 12ms

   ▸ plate 1 — resolve: the literal-match path
╭ wcode · ⎇ main · session a1b2c3d4 ──────────────── gpt-5-codex · high ╮
│ ❯ ▌                                                                  │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────── ⏻ plan · ⏸ idle   1/1 ╯
````

*The folio (`1/1`, `dim`) is a new bottom-right corner cell, space-joined so
the bottom-right title keeps its one `·` (see §1, §3(e)).*

### (c) Rendered frame — 120×40 (the measure needs the width)

The measure is 68 cols, **centered** with a 26-col margin each side; the running
head and the folio sit on the box, so the wide screen is all margin — the printed
page.

````
                                                    ❯ how does edit resolve an anchor?

                                                    §1  the anchor is the text

                                                      An anchor is the text you quote — the old 5-char
                                                      hash is gone (D007), so a line's address is its
                                                      own content, and a reformatter moves it. The read
                                                      tool prints `file:line` and edit matches that text
                                                      literally.¹

                                                      The drift-proofing is the point: every edit targets
                                                      one snapshot, so an edit above a target never
                                                      shifts it.

                                                      ── notes ────────────────────────────────────
                                                      1 ⚙ read  crates/wcode-cli/src/tools/edit.rs
                                                        ✓ read · 128 lines · 12ms

                                                      ▸ plate 1 — resolve: the literal-match path
╭ wcode · ⎇ main · session a1b2c3d4 ────────────────────────── gpt-5-codex · high ╮
│ ❯ ▌                                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────────────────── ⏻ plan · ⏸ idle   1/1 ╯
````

### (d) Legend

- `§1  the anchor is the text` — `§1` `heading`, the title `heading`
  (`crates/wcode-tui/src/theme.rs:76`). One `·`-free head line per turn.
- prose — `body` (default); the **footnote reference** `¹` — `link` (a printed
  reference, like the paper's `.fnmark`, never `accent`; `accent` stays the
  prompt/run state).
- `── notes ──` rule + `1` marker — `dim`; the note's `⚙`/`✓` and its body —
  `dim`/`tool_name` (the shipped tool styling, relocated).
- `▸ plate 1 — …` — `▸` `dim` (collapsed affordance, §2 table); the caption
  `dim`; fenced code under it keeps the shipped `` ` `` gutter `dim` +
  `code` body (`crates/wcode-tui/src/markdown.rs:583-604`).
- Running head — top-left corner, `muted` + `dim` (unchanged chrome,
  `crates/wcode-tui/src/ui.rs:1367-1375`); the **folio** (`1/1`) — bottom-right
  corner, `dim`, space-joined beside `⏻ plan · ⏸ idle`.

### (e) Spec amendments + new glyph

Amends **`docs/tui-design.md`**:

- **§2 Vocabulary (`:50-79`)** — **two NEW glyphs**, each with a call site:
  - **`§`** — the section counter, emitted as each assistant turn's head line.
    *Call site:* a new `Block::TurnHead { n, title: Option<String> }` rendered
    before an assistant block (same seam as
    `crates/wcode-tui/src/ui.rs:577-591` `block_lines`).
    *(ASCII-clean fallback if refused: `1.`.)*
  - **`¹²³`** (superscript digits) — the footnote reference, emitted at each
    `ContentBlock::ToolCall` position. *Call site:* the assistant
    `content_lines` walk (`crates/wcode-tui/src/ui.rs:607-632`), which currently
    **drops** tool calls (`ContentBlock::ToolCall { .. } => {}` at `:630`) — it
    would instead emit a reference and *collect* the tool for the foot.
    *(ASCII-clean fallback: `[1]`.)*
- **§4 Transcript (Tool, `:192-199`)** — the D31 tool **panel** is replaced (per
  turn) by a **footnote list** under a short rule. The panel's `▸`/`▾`/`⧉`
  affordances move onto the note.
- **§4 Layout (`:240-256`)** — the folio cell is added to the input box's
  bottom-right corner (`crates/wcode-tui/src/ui.rs:1405-1425`); the box carries
  it, so **no fourth band**.
- **§1.4 Block separation (`:42`)** — a turn's head adds a blank *above* it (a
  section break), folded into the existing between-roles blank.

### (f) Cost + what it breaks

- **Cost:** **net ≈ 0 rows** — the footnote drops the panel's 2 frame rows and
  adds a rule + a blank (see (a)); the measure spends 0 rows (horizontal only)
  but at 80 cols leaves only ~64 usable cols of prose.
- **Breaks / risks:** the shipped D31 panel is *the* look of a tool today; a
  turn-foot footnote re-composes `content_lines` and the D32 affordance hit-map
  (`crates/wcode-tui/src/ui.rs:472-502`), which assumes a **panel's first row**
  is the toggle target. That is a real recomposition, not a restyle.

### (g) The section head — pinned

The head is `§N` + an optional title. This settles (f)'s open question:

| # | question | pinned |
|---|---|---|
| 1 | the **counter** | **per assistant turn** (`§1`, `§2`, … one per reply) — *not* per markdown heading; free, no model cooperation |
| 2 | the **title source** | the reply's **first top-level `# h1`**, **consumed** into the head (not rendered inline); **no `# h1` → a bare `§N`** |
| 3 | where the head **sits** | its own line directly before the assistant block — `Block::TurnHead { n, title: Option<String> }` (call site `crates/wcode-tui/src/ui.rs:577-591` `block_lines`) |
| 4 | the number's **scope** | **per session, monotonic** (survives a resume); the **folio** (`N/M`) is the same series — a first turn reads `§1` and `1/1` |

The §3(b)/(c) frames are the **titled** case: the reply's markdown opened
`# the anchor is the text`, which the head **consumes**, so the title is
grounded, not an aspiration. A reply with no `# h1` renders a **bare `§1`**.

---

## 4. Direction 2 — **Chapter**

### (a) Stance + dials

> **A session read as a book chapter:** a chapter open per major turn,
> **indent-led** prose (the first line of each paragraph is indented, so no blank
> row separates paragraphs), a running head, and the gutter read as a **margin**.

`VARIANCE 1 · MOTION 1 · DENSITY 6`. **Payment — the cheapest of the five:** the
between-paragraph **blank becomes an indent** (the `vscode-paper-direction.md:43-44`
move, translated). Rows are *freed*, not spent; the chapter open spends them
back. Net ≈ 0 — this is how the TUI buys leading without losing density.

### (b) Rendered frame — 80×24

````
 ❯ draft the release notes

 Chapter 4 · the release

     Four crates ship in dependency order, and the tag releases end to
 end; the Homebrew formula commits for every release.
     The digest cache no longer needs a re-read after a mutation — a
 mutator appends the post-write digest as a trailer, so read → edit →
 edit is one pass.

 ⚙    shipped v0.3.5 · 4 crates, 2 commits

                                            4
╭ wcode · ⎇ main · session a1b2c3d4 ────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

### (c) Legend

- `Chapter 4 · the release` — `heading`; the `·` is the locked status separator
  (`crates/wcode-tui/src/ui.rs:1484` `sep()`), ≤1 per line.
- Indent-led paragraphs — first line indented 4 cols, continuation at the gutter;
  `body`.
- `⚙    shipped …` — the gutter **is** the margin: the `⚙` marker (`dim`) sits in
  the marker column and the note text follows in the content column; the tool
  name is `tool_name`.
- `4` (folio) — `dim`, right-aligned at the chapter foot (a printed page number;
  Chapter's folio lives here, not on the box corner — see §1).

### (d) Spec amendments + new glyph

- **§1.2 Gutter (`:22-27`)** — the gutter is named as a **margin**: it holds not
  only markers but short marginal notes. (The 3-col `PANEL_INDENT` /
  markdown `GUTTER` are already the margin; `crates/wcode-tui/src/ui.rs:34`,
  `crates/wcode-tui/src/markdown.rs:20`.)
- **§4 Transcript / markdown (`:243-245`)** — **first-line paragraph indent**
  within one speaker (markdown `p + p`). *Call site:* `markdown::render`'s leaf
  flush (`crates/wcode-tui/src/markdown.rs:404-427` `flush_leaf`) currently
  prefixes **every** line with the same `GUTTER` (`:359-368` `prefixes`); it would
  prepend an indent to the **first** physical line of a non-first paragraph.
- **§1.4 Block separation (`:42`)** — *within a speaker*, paragraphs are
  separated by the **indent**, not a blank (the blank survives only between
  *roles*). This is the amendment that frees the rows.
- **No new glyph.** Everything in the frame is in the §2 table (`⚙`, `·`, `─`
  unused here, `⎇`, `▰▱`).

### (e) Cost + what it breaks

- **Cost:** the chapter open (~2 rows) per major turn; the indent is free
  (replaces the blank).
- **Breaks / risks:** the indent changes the **markdown contract** — today
  `render` never emits a differing first-line prefix for paragraphs, and the
  transcript block cache keys on `(rev, width)` (`crates/wcode-tui/src/app.rs:1220-1231`),
  which is unaffected, but `markdown.rs`'s tests pin exact line prefixes
  (`crates/wcode-tui/src/markdown.rs:886` `assert_eq!(text_of(&lines), ["   h1", …])`).
  A `p + p` indent is a real markdown change with test churn.
- **One open question for the human:** is a *chapter* a turn (every reply) or a
  *macro* (a run, a session phase)? Every-turn chapters may number so fast the
  number is noise.

---

## 5. Direction 3 — **Marginalia**

### (a) Stance + dials

> **Tufte sidenotes:** at **≥120 cols** a content measure sits left and a **right
> margin column** holds tools / thinking / diffs as sidenotes; below 120 cols the
> margin folds to the **foot** (endnotes). The truest "book" reading.

`VARIANCE 1 · MOTION 1 · DENSITY 6` (wide) / **7** (narrow). **Payment:** at
≥120 the apparatus leaves the reading column entirely — the notes live in margin
rows the prose is *not* using, so the measure is uninterrupted and **no reading
row is spent** (the margin is a new *region*, like the sidebar, not fourth-band
chrome). At 80 the margin folds to the foot (Direction 1's footnote, in the
simplest form).

### (b) Rendered frame — 120×40 (the shape requires the width)

````
 ❯ why does the anchor move when I reformat?

 An anchor is the text you quote — the old 5-char     notes · turn 3
 hash is gone (D007), so a line's address is its
 own content. Reformatting reindents the line, so    ··· I should confirm how the
 its address changes; re-read after a formatter.         hash covers the raw line…

 The drift-proofing is the point: every edit           ⚙ read  edit.rs
 targets one snapshot, so an edit above a target       ✓ read · 128 lines · 12ms
 never shifts it.                                       ▸ diff  crates/…/edit.rs
                                                        +3 −0

                                                     ── (more notes scroll) ──
╭ wcode · ⎇ main · session a1b2c3d4 ────────────────────────── gpt-5-codex · high ╮
│ ❯ ▌                                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────────────────── ⏻ plan · ⏸ idle   3/3 ╯
````

### (c) Rendered frame — 80×24 (the margin folds to the foot)

````
 ❯ why does the anchor move when I reformat?

 An anchor is the text you quote — the old 5-char hash is gone (D007), so a
 line's address is its own content. Reformatting reindents the line, so its
 address changes; re-read after a formatter.

 The drift-proofing is the point: every edit targets one snapshot.

 ── notes ──────────────────────────────────────────────────────────────
 ··· thinking · 218 chars
 ⚙ read  crates/wcode-cli/src/tools/edit.rs
 ✓ read · 128 lines · 12ms
 ⚙ edit  crates/wcode-cli/src/tools/edit.rs
 ✓ edit · +3 −0 · 9ms
╭ wcode · ⎇ main · session a1b2c3d4 ────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

### (d) Legend

- Left column (the measure, ~50 cols at 120; ~72 at 80) — `body`; the `❯` prompt
  — `accent`.
- Right margin head `notes · turn 3` — `muted`; the `·` — `dim`.
- `··· thinking` — `thinking` (`crates/wcode-tui/src/theme.rs:80`); `⚙`/`✓` —
  `tool_name`/`dim`/`success`; `▸ diff` — `▸` `dim`, `+3` `diff_add`, `−0`
  `diff_del` (`crates/wcode-tui/src/theme.rs:81-82`).
- A vertical `│` rule may separate measure from margin — `border`
  (`crates/wcode-tui/src/theme.rs:69`, `border: …DarkGray`) — **`│` is already in
  the §2 table.**

### (e) Spec amendments + new glyph

- **§1.1 / §4 Layout (`:14-21`, `:240-256`)** — a **right margin region** docks
  only when the terminal is **≥120 cols** (mirroring the sidebar's ≥80 dock
  rule, `docs/tui-design.md:258`, `crates/wcode-tui/src/ui.rs:53`
  `SIDEBAR_MIN_WIDTH: u16 = 80`). Below 120 the base layout is **byte-identical**
  and the notes fold to a turn-foot list (Direction 1's block). This is a new
  *region*, not a fourth band; it reflows nothing (`docs/tui-design.md:20-21`).
- **§4 Transcript (Tool/Thinking, `:192-201`)** — tool, thinking, and diff blocks
  gain a **margin placement** above 120 cols.
- **§2 (`:50-79`)** — **no new glyph**; `│` (margin rule) is already the table's
  table/list frame glyph. If the margin is ever toggled by key it needs a `KEYS`
  entry (`docs/tui-design.md:275`); by default it is **width-driven and
  deterministic**, so it needs none.
- **§4 Keys (`:275-296`)** — *only if* the human wants a manual toggle; otherwise
  unchanged.

### (f) Cost + what it breaks

- **Cost:** **none in reading rows** at ≥120 (the margin is new horizontal
  space). At <120 it folds to Direction 1's foot (**net ≈ 0 rows**). At 120 it
  spends ~50 cols of margin — a *horizontal* cost, the inverse of the sidebar's.
- **Breaks / risks:** this is the **largest** amendment (a second column). It
  risks exactly what `vscode-paper-direction.md:181` refused: "**A margin
  (sidenote) column** — a second column breaks the single-measure stance and
  dies at sidebar width." Here the sidebar **is** the collision: at 120 cols
  with `Ctrl-B` open, margin + sidebar + measure no longer fit — the margin must
  yield (fold to foot) when the sidebar docks. That interaction is the hard part.
- **One open question for the human:** at 120 cols with the **sidebar docked**
  (`Ctrl-B`, 30 cols), does the margin fold to the foot (recommended), or does
  the sidebar take priority so the margin never appears alongside it?

---

## 6. Direction 4 — **Broadsheet**

### (a) Stance + dials

> **A printed article:** a **masthead** (a rule above and below), a **dateline**
> (session + model), a bold **standfirst/deck**, one prose column, and the
> apparatus as a legible **subordinate block**.

`VARIANCE 1 · MOTION 1 · DENSITY 7`. **Payment:** the masthead is the most
expensive head of the five — **3 rows** (rule + name, the dateline row, the
closing rule) — bought back by keeping apparatus as a single compact block
(two `⚙ … ✓` lines, no frames) rather than D31 panels.

### (b) Rendered frame — 80×24

````
 ❯ release the four crates

 wcode ───────────────────────────────────────────────────────────────
 THE RELEASE DESK                    session a1b2c3d4 · gpt-5-codex (high)
 ────────────────────────────────────────────────────────────────────
   Four crates ship in dependency order; the tag releases end to end. A
   mutator appends the post-write digest as a trailer, so read → edit
   → edit needs no re-read.

   ⚙ bash   cargo publish -p wcode-harness          ✓ 4.2s
   ⚙ bash   cargo publish -p wcode-tui              ✓ 3.9s

   Homebrew's formula commits for every release; the tap mirror stays
   opt-in behind a token.
╭ wcode · ⎇ main · session a1b2c3d4 ────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

### (c) Legend

- `wcode ───` masthead name + top rule — `heading` / `border`.
- `THE RELEASE DESK` — the section label, `heading_sub`.
- dateline `session a1b2c3d4 · gpt-5-codex (high)` — `muted`; **one `·`** (the
  copy rule caps `·` at 1 per metadata line, `SKILL.md:143` — the model's effort
  rides in parentheses rather than a second `·`; see §9).
- standfirst/deck (the bold first paragraph) — `body` + `BOLD`.
- apparatus `⚙ bash … ✓` — `tool_name` + `dim` + `success`; **no frame**
  (`border` unused per tool).
- bottom rule — `border`.

### (d) Spec amendments + new glyph

- **§1.1 (`:14-21`)** — the masthead is a **transcript-head block** (the
  session's first block, which *scrolls*), **not** fixed chrome, so it does not
  become a title bar (`docs/tui-design.md:322`). *Call site:* a new
  `Block::Masthead` at the head of the seeded transcript
  (`docs/tui-design.md:187-190` seeded-history region).
- **§4 Transcript (Tool, `:192-199`)** — a **compact tool block** (one
  `⚙ name  cmd  ✓ note` line, no panel) as the *default* when a turn is
  apparatus-heavy; the D31 panel becomes the *expanded* form. (This is the
  "compact mode" that `redesign-v2.md:483-486` flagged as a follow-up render —
  Broadsheet is that render.)
- **§1.4 (`:42`)** — the masthead rule is a *role* separator (its own blank line
  above and below).
- **§2 (`:50-79`)** — **no new glyph.** `─` (rule), `⚙ ✓`, `·` are all in the
  table.
- **§4 Layout (`:240-256`)** — unchanged (no folio cell needed; the dateline
  carries position).

### (e) Cost + what it breaks

- **Cost:** **3 rows** of head (the name sits on the first rule, then the
  dateline row, then the closing rule) up to the deck — the **most expensive**
  of the five; the deck is free (it is the first paragraph).
- **Breaks / risks:** a masthead is one keystroke away from the **banned title
  bar** (`docs/tui-design.md:322`). The only honest reading is "it scrolls away
  with the session's first content" — but then a mid-session view has **no**
  masthead, and the direction only shows its face on turn 1. That is a real
  weakness, not a footnote.
- **One open question for the human:** should the masthead be a **once-per-
  session** head (a real front page, scrolls once) or **re-drawn at each turn**
  (a running masthead — which risks reading as a title bar)?

---

## 7. Direction 5 — **Reader**

### (a) Stance + dials

> **The clean reading surface (iA Writer / Kindle):** chrome nearly gone, one
> measure, generous leading, and **all** apparatus banished to a collapsed
> **Notes** section at the foot. The quietest of the five.

`VARIANCE 1 · MOTION 1 · DENSITY 5` (the airiest — deliberately below the pinned
band, see the open question). **Payment:** it *spends* on leading (a blank
between **every** paragraph) and buys the rows back by collapsing **all**
apparatus — tools, thinking, diffs — into a **single** `▸ N notes` row per turn.

### (b) Rendered frame — 80×24

````
 ❯ what did we change today?

   Four crates shipped in dependency order. The digest cache no longer
   needs a re-read after a mutation.

   The tag releases end to end, and the formula commits for every
   release.

   We removed hash-based addressing; anchors are literal text now.

   ▸ 6 notes                                              turn 3 of 3
╭ wcode · ⎇ main · session a1b2c3d4 ────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

### (c) Legend

- Prose — `body`; `❯` prompt — `accent`; the leading blank is unstyled
  (terminal background — **the TUI has no `bg` role, the terminal owns it**,
  `SKILL.md:149`).
- `▸ 6 notes` — `▸` `dim` (collapsed affordance, §2 table); `6 notes` `dim`;
  `turn 3 of 3` right-aligned `muted`.
- Expanded (`▸`→`▾`, on key) reveals the shipped tool/thinking blocks inline,
  `dim`/`tool_name`/`thinking`, exactly as `redesign-v2.md:193-278` renders them.

### (d) Spec amendments + new glyph

- **§4 Transcript (`:192-201`)** — a **`Notes` summary block**: a turn's tool +
  thinking + diff blocks collapse to one `▸ N notes` row (expanded on `Ctrl-T`
  or browse `Enter` — the existing all-tools key, `docs/tui-design.md:282`
  "`Ctrl-T` expands/collapses every tool's output"). *Call site:* the assistant
  block's affordance path already collapses thinking
  (`crates/wcode-tui/src/ui.rs:472-484`); the `Notes` block extends the same
  mechanism to tools/diffs.
- **§1.4 Block separation (`:42`)** — **within a speaker**, a blank between
  **every** paragraph (the airiest reading); between *roles*, unchanged.
- **§2 (`:50-79`)** — **no new glyph** (`▸` is in the table).
- **§4 Keys (`:275-296`)** — **no new key**: `Ctrl-T` (all tools) and browse
  `Enter` already toggle apparatus.

### (e) Cost + what it breaks

- **Cost:** a blank between every paragraph — the airiest; **all** bought back by
  the single `▸ N notes` row per turn. Net ≈ the shipped density when apparatus
  is heavy; **more** air than shipped when a turn is prose-only.
- **Breaks / risks:** it sits **below** the skill's pinned density (5 vs 7–8),
  which is a **deliberate** deviation to state, not hide — the skill itself says
  air is the editorial point, and this is the one direction that leans all the
  way into it. It also hides reasoning by default (already D33's stance,
  `docs/tui-design.md:320`), which some users read live.
- **One open question for the human:** is **DENSITY 5** acceptable for this
  direction alone (an opt-in "reading mode"), or must every direction stay inside
  the pinned 7–8 band?

---

## 8. Comparison table (side by side)

| # | direction | one-line stance | new rows (air) | buys it by | new glyph? | amends (spec) | at 80×24? |
|---|---|---|---|---|---|---|---|
| 1 | **Preprint** | arXiv paper: numbered sections, one measure, turn-foot footnotes, code plates | ≈0 | apparatus *moves* to the foot | **`§`, `¹²³`** | §2, §4 Tool, §4 Layout, §1.4 | yes |
| 2 | **Chapter** | a book chapter: indent-led prose, margin gutter | ≈0 | indent **replaces** the blank | none | §1.2, §4 markdown, §1.4 | yes |
| 3 | **Marginalia** | Tufte sidenotes: measure + right margin, folds to foot | ≈0 | margin is new horizontal space | none | §1.1, §4 Layout, §4 Tool/Thinking | yes (folded) |
| 4 | **Broadsheet** | a printed article: masthead, dateline, deck | +3 | compact tool block, no frames | none | §1.1, §4 Tool, §1.4 | yes |
| 5 | **Reader** | the clean reading surface: chrome gone, all apparatus → Notes | leading-heavy | collapse **all** apparatus | none | §4 Transcript, §1.4 | yes |

*(Tables are orthogonal to the five: all of them share the `booktabs` re-set —
§10 — which is a glyph **removal** (`│`, `┼`), not a new glyph, so the
"new glyph?" column above is unchanged. The tool **frame** is likewise shared —
§11 — a **re-scope**, not a removal: the frame glyphs stay declared (now for the
box/overlays). The **composer** is likewise shared — §12: its 4-corner box
becomes a rule-set writing line.)*

**Recommendation: Direction 2 (Chapter), staged toward Direction 1 (Preprint).**
Reasoning, in the skill's own terms:

- **Chapter is the only direction whose air is free.** It *trades* the
  between-paragraph blank for an indent (`vscode-paper-direction.md`'s exact
  "rhythm change, not a density change"), so it respects the pinned DENSITY 7–8
  while reading as prose — the central problem (§1) is *solved*, not paid for.
- **Preprint is the ambition**, but it needs the two new glyphs and a
  recomposition of the tool block, which is the highest-risk change here. Land
  Chapter first; then add Preprint's **section head** (`§N`) and **footnote**
  as the next step — Chapter's indent-led rhythm and Preprint's footnotes
  compose cleanly.
- **Marginalia** is the truest "book" at ≥120 and the best *wide-terminal*
  payoff, but the sidebar interaction (§5f) is unresolved; hold it until that is
  decided.
- **Broadsheet** and **Reader** are the endpoints of the air axis (most / least
  chrome). Both are worth a later opt-in "mode", not the default.

---

## 9. What is common (the moves all five share)

Whatever is picked, eight moves are shared and can be factored once:

1. **One measure.** Every direction centers (or left-margins) a content column
   narrower than the terminal, and puts the **rules** on that measure, not
   full-bleed. Below 84 cols the measure is the full band width; at ≥84 cols it
   is a 68-col column, centered. This is the single biggest "editorial" lever and
   it costs **no rows**. *(The
   input box stays a full-width band — it cannot be centered without a layout
   change — so the measure governs the **transcript** only; the box is the one
   piece of furniture that stays edge-to-edge, and its corners carry the running
   head.)*
2. **A running head + a folio.** The head reuses the input box top-left corner
   (`project · ⎇ branch · session`, `crates/wcode-tui/src/ui.rs:1441`); the
   folio is one new bottom-right corner cell (`:1405-1425`) — or, in Chapter, a
   number at the chapter's foot. No new band.
3. **A note apparatus.** Tools, thinking, and diffs leave the running text —
   foot, margin, or collapsed `Notes` — and **never** sit mid-sentence as a
   frame. The D31 panel either moves (Preprint/Marginalia/Reader) or simplifies
   to one line (Broadsheet).
4. **A short rule.** A `─` rule (already in the §2 table; the same glyph
   `markdown.rs`'s thematic break uses, `crates/wcode-tui/src/markdown.rs:462-467`)
   sets off the apparatus from the prose.
5. **Leading discipline.** Blank rows are spent on *reading* (paragraphs) and
   *roles* — never on chrome. The between-roles blank
   (`crates/wcode-tui/src/app.rs:1235-1237`) is preserved in all five.
6. **Tables are `booktabs` plates.** The markdown table loses its vertical
   grid (`│`, `┼`) for three `─` rules and a whitespace gutter — a `body`-level
   move every direction inherits (**§10**).
7. **The frame is a rule, not a box.** The tool panel stops using the
   `╭ ╮ ╰ ╯`/`│` frame (the glyphs stay declared for the box/overlays) for a head
   line + two `─` rules — a `body`-level move every direction inherits (**§11**).
8. **The composer is a writing line, not a box.** The 4-corner input box becomes
   a head rule + the input + a gauge/state line — a `body`-level move every
   direction inherits (**§12**).

**Copy rules that bite all five** (`SKILL.md:127-144`): the `·` separator is
**≤1 per metadata line** for *new* chrome, so a dateline / note head that wants
two `·` must drop one (use commas or parentheses). **Two shipped rows are
explicitly exempt** — they already carry more than one `·` and these directions
keep them byte-identical: the **input-box head row** — the *whole* shipped top
border, where `project · ⎇ branch · session` meets `model · effort`, so it carries
**three `·`** once the two top titles share one row (the §12 head line is this same
row; `crates/wcode-tui/src/ui.rs:1350-1436` `corner_titles` top row) — and the
**tool-summary line**
(`✓ name · note · note`, e.g. `✓ read · 128 lines · 12ms`,
`docs/tui-design.md:100`). Every other frame line is ≤1 `·`.

The em-dash `—` is **allowed in the TUI** — it is the assistant-prose glyph
(`| assistant prose | — | default fg |`, `docs/tui-design.md:55`) and the
empty-section marker (`docs/tui-design.md:270`, "An empty section keeps its
header with a dim —"). In the frames it appears in **prose** and in the **plate
caption** (`▸ plate 1 — resolve: …`, which is apparatus/caption matter, not a UI
string label) — never in a status or heading label. *(Note: `SKILL.md:136` cites
this row as `docs/tui-design.md:37`; line 37 is now the D30 colour-tier prose,
so the row has drifted to `:55` — cite the row, not the skill's number.)*

---

## 10. Tables — a booktabs plate

A **shared** move — a `body`-level change every direction inherits, below the
direction layer. The human asked whether we also re-set the **table** to match
arXiv. Today the answer is **no**: the shipped renderer draws a full vertical
grid — a `│` between every column and a `┼` in the rule — which reads as a
*spreadsheet*, not a *paper table*. A paper uses **`booktabs`**: no vertical
rules and no `┼`, three horizontal rules only (top, under the header, bottom),
columns separated by **whitespace**, header bold, and — where a caption exists —
a `Table N: …` line above the top rule.

### Before — the shipped grid

````
   name      │ qty │ ms
   ──────────┼─────┼────
   harness   │  12 │  4
   tui       │   8 │  3
````

*legend:* header `heading` (bold); the rule and its `┼` `dim`; cells `body`. The
per-column `│` and the crossing `┼` are the two non-paper marks.

### After — the booktabs plate (proposed)

````
   Table 1: build times
   ─────────────────────
   name       qty   ms
   ─────────────────────
   harness     12    4
   tui          8    3
   ─────────────────────
````

*legend:* the caption `Table 1: …` `dim`; the top rule `dim`; the header
`heading` (bold); the under-header rule `dim`; body rows `body`; the bottom rule
`dim`. No `│`, no `┼` — column separation is carried by the whitespace gutter
alone. (Column widths here are illustrative.)

### The move, with anchors

- **The join** — `crates/wcode-tui/src/markdown.rs:709-710`
  `format!("{GUTTER}{}", parts.join(" │ "))` → a **whitespace gutter** (`"  "`,
  two columns) instead of `" │ "`. This is what removes every vertical `│`.
- **The rule** — `crates/wcode-tui/src/markdown.rs:673-677`
  `widths.iter().map(|w| "─".repeat(*w)).collect::<Vec<_>>().join("─┼─")` → a
  **solid `─` run spanning the full table width** (the sum of the columns plus
  the gutters). The `┼` disappears with the join.
- **Where the rules go** — `booktabs` wants three: **above the header**, **under
  the header** (the shipped renderer already emits this one, minus the `┼`), and
  **below the last row** (new). So `table_lines` (`:636-686`) gains two — one
  above the header, one at the foot.
- **The width budget** — `crates/wcode-tui/src/markdown.rs:651-654` reserves
  `let overhead = disp(GUTTER) + 3 * cols.saturating_sub(1);` — **3 columns per
  `" │ "` join**. With a 2-space gutter that becomes `2 * (cols - 1)`. The shrink
  loop (`:656-665`) still works, but the reserved overhead — and so every wrap
  point — moves. **This is a real behaviour change, not cosmetic.**
- **The caption** — `Table N: …` above the top rule (`dim`). GFM has no table
  caption, so it is a *proposed* block (a `Block`-level addition, §4), not a
  restyle.

### Spec amendment — §2, a *removal* (no new glyph)

`docs/tui-design.md:68` — the §2 glyph-table row

````
| table | `│ ─ ┼` | header bold; columns aligned, cells wrap |
````

becomes the **`booktabs` set: `─` only**, already in the table. **`│` and `┼`
are removed from the *table* row** — a *removal*, so **no new glyph is
introduced** (`│`/`─` remain in the table as the quote/tree/rule/frame glyphs;
only their *use for tables* goes). Named here, **not amended** here — a genuine
pick amends `docs/tui-design.md` first (`SKILL.md` §7.3). This is the only §2
change any direction needs, and it is shared.

### Cost / risk

- **Cost:** the vertical rules carry the column separation; removing them leaves
  **whitespace only**, which needs the wider 2-space gutter to stay legible — a
  *horizontal* cost, **no rows**.
- **Tests pin the grid** — `crates/wcode-tui/src/markdown.rs:1211-1214`:
  `assert_eq!(text[0], "   name │ qty");`,
  `assert!(text[1].starts_with("   ─") && text[1].contains('┼'));`,
  `assert_eq!(text[2], "   a    │   1");`, `assert_eq!(text[3], "   bb   │  22");`.
  Every one must change to the `booktabs` form. **Real churn**, not a one-line
  edit — plus the three-rule emission is behaviour `table_lines` does not have
  today.
- **One open question for the human:** does the `Table N:` caption number
  **per-turn** or **per-transcript**? A paper numbers within a section; a
  terminal turn has no section, so per-turn is the natural reading — confirm.

---

## 11. The frame is a rule, not a box

A **shared** move (beside §10 tables): the tool panel stops being a **box** and
becomes a **rule-set plate**. A box is a *container* — and the paper precedent
already refused the container idiom:

> **A "paper white" fill or a forced light theme** — a container that is not
> content, and a mid-surface theme flip. Refused; the surface tracks the host.
> — `docs/design/vscode-paper-direction.md:171-172`

A printed paper has **no boxes**: `listings`/`verbatim` are set off by **rules**,
not frames. We already made exactly this call for **tables** (§10, `booktabs`);
the tool panel is the same *apparatus* and must follow the same logic.

### The inconsistency to name

Of the five directions, **four already relocate the tool out of a frame** —
Preprint → a turn-foot footnote (§3), Marginalia → the margin column (§5),
Broadsheet → "no frame" (§6d), Reader → the collapsed `Notes` block (§7). **But
Chapter — the recommended direction — does not**: its amendment list (§4d, this
doc `:292-304`) changes the gutter and the prose rhythm and **never mentions the
tool block**, so *as specified* Chapter inherits the **shipped framed D31
panel**. The recommended pick is thus the one that keeps a box. The fix must be a
**shared** move, not a per-direction one.

### Before — the shipped framed panel (D31)

````
   ╭─ ⚙ bash ─────────────────────────────── ▸ ⧉ ─╮
   │  cmd  cargo test -p wcode-cli                  │
   │  ✓ 41 passed · 12 lines · 9ms                  │
   ╰────────────────────────────────────────────────╯
````

error:

````
   ╭─ ⚙ edit ─────────────────────────────── ▸ ⧉ ─╮
   │  path  crates/wcode-cli/src/tools/edit.rs      │
   │  ✗ E_NO_MATCH · no literal match · 3ms         │
   ╰────────────────────────────────────────────────╯
````

### After — the rule-set plate

````
   ⚙ bash                                   ▸ ⧉
   ────────────────────────────────────────────────────
   cmd  cargo test -p wcode-cli
   ────────────────────────────────────────────────────
   ✓ 41 passed · 12 lines · 9ms
````

error:

````
   ⚙ edit                                   ▸ ⧉
   ────────────────────────────────────────────────────
   path  crates/wcode-cli/src/tools/edit.rs
   ────────────────────────────────────────────────────
   ✗ E_NO_MATCH · no literal match · 3ms
````

*legend:* the head line (`⚙ bash`) — name `tool_name`, mark/adds `dim`; the two
`─` rules `dim`; the params keys `dim`, values `body`; the `✓`/`✗` summary
`success`/`error`. **No side `│`, no corner.**

### The move, with anchors

- **Drop the corners** — `crates/wcode-tui/src/ui.rs:942`
  `Span::styled("╭─ ".to_string(), border())` and `:949`
  `Span::styled("─╮".to_string(), border())` (in `panel_top_spans`, `:927-951`)
  go; the head line carries the name + affordances instead.
- **Drop the side walls** — `crates/wcode-tui/src/ui.rs:955-967`
  (`panel_content_row`) emit `│` on both sides (`:961`/`:965`
  `Span::styled("│".to_string(), border())`); the plate keeps only the
  `PANEL_INDENT` gutter and the measure.
- **The rules become plain `─`** — `crates/wcode-tui/src/ui.rs:918`
  `panel_rule(indent, '├', '┤', panel_w)` and `:923`
  `panel_rule(indent, '╰', '╯', panel_w)` lose their corner chars
  (`panel_rule` already builds `"─".repeat(inner)`, `:970-976`).
- **The frame fn** — `crates/wcode-tui/src/ui.rs:898` `fn panel_frame` is
  re-composed into a head line + two rules + content rows.

### Spec amendment (§2 + §4 — a *re-scope*, no glyph retired or added)

`docs/tui-design.md:69` — the row
``| tool panel frame | `╭ ╮ ╰ ╯ ─ │` | border |``
is **re-scoped** to the **box / overlay frame** —
``| box / overlay frame | `╭ ╮ ╰ ╯ ─ │` | border |`` — it **keeps `╭ ╮ ╰ ╯ ─ │`
declared**, because the input box (`crates/wcode-tui/src/ui.rs:1330`
`.border_type(BorderType::Rounded)`) and the overlays (`:243`/`:281`) still draw
them, and the tool panel is the *only* surface that stops. A **new** §2 row
declares the tool plate:
``| tool plate rule | `─` | dim |``. **No glyph is retired from the table** —
the frame glyphs are **re-scoped** (tool panel → box/overlays), and `─` is
already declared. §4 **D31** (`docs/tui-design.md:192-199`, "a framed **panel**")
becomes a **rule-set plate**. Named here, **not amended** here.

### What keeps its frame — settled for the box in §12

- **The input box** — **settled in §12**: it becomes a **rule-set writing
  line** (the recommended option keeps the box's 3 rows). The composer is
  interactive **furniture** — the paper doc kept a composer field — and it now
  reads as the page's writing line, not a container (`crates/wcode-tui/src/ui.rs:1326-1339`).
- **The overlays** (`Ctrl-B` picker, `F1` help, the completion/search popups)
  are **transient chrome**, not content (`crates/wcode-tui/src/ui.rs:243`, `:281`
  `.border_type(BorderType::Rounded)`) — they **keep** their rounded frames.
- **Resolved:** the box → a writing line (**§12**); the overlays → keep.

### Cost / risk

- **Cost:** the framed panel is **4 rows** (`╭` top, params, summary, `╰`
  bottom); the literal plate is **5** (the head line is separate from the rule).
  A **name-on-the-rule** variant — `⚙ bash ────────── ▸ ⧉` (drop only the
  corners, keep the name riding the rule) — is **0 extra rows** and is the
  cheaper reading, offered as an alternative.
- **Risk:** `panel_frame` is re-composed, so the D32 affordance **hit-map**
  (`docs/tui-design.md:210-212`) moves from the panel's top rule to the head
  line — a real change, not a restyle; and the panel tests pin the frame.
- **One open question for the human:** the **name-on-the-rule** variant (0 extra
  rows) or the full plate (head line + rule, +1 row)?

---

## 12. The composer — a writing line, not a box

§11 left one open question — *"do the box and the overlays keep rounded frames?"*
(`:875-877`). This section **settles the box**: it answers the way §11 answered
for the tool panel, because a composer box is the **same container idiom** the
paper precedent refused:

> **A "paper white" fill or a forced light theme** — a container that is not
> content, and a mid-surface theme flip. Refused; the surface tracks the host.
> — `docs/design/vscode-paper-direction.md:171-172`

A printed page's **writing line is a rule, not a box**. The shipped composer is a
4-sided rounded box whose **four corners carry all chrome** (`docs/tui-design.md:248-252`
"a rounded border whose **four corners carry all chrome**";
`crates/wcode-tui/src/ui.rs:1326-1339` `fn draw_input_box`, `:1329-1330`
`WidgetBlock::default().borders(Borders::ALL).border_type(BorderType::Rounded)`).
Three ways to make it a page.

**No title bar** (`docs/tui-design.md:322`): every option keeps the chrome on the
**composer** (bottom), never a fixed top row.

**The team strip** (`crates/wcode-tui/src/ui.rs:43` `const TEAM_MIN_WIDTH: u16 = 50;`,
`:1792` `fn draw_working_team`) is **unchanged** — it stays a 0–3-row rider above
the composer's first line, whatever that line is.

### Before — the shipped box (80×24)

idle:

````
╭ wcode · ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

running:

````
╭ wcode · ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▰▰▱▱▱ 15.8k / 272k ──────────── ⏻ plan · ⠹ running 3.1s ╯
````

*legend:* frame `border`; `wcode` `muted`, the rest of the head `dim`; the gauge
`success`; `⏻ plan` `accent`; `⏸ idle` `dim` / `⠹ running` `accent`; `❯` `accent`.

---

### A · The writing line (recommended)

**Stance:** one rule above the input; the **running head rides the rule**, the
gauge + state a line below — the input *is* the page's writing line.

idle:

````
 wcode · ⎇ main · session a1b2c3d4 ──────────────── gpt-5-codex · high
 ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ───────────────────────────── ⏻ plan · ⏸ idle
````

running:

````
 wcode · ⎇ main · session a1b2c3d4 ──────────────── gpt-5-codex · high
 ❯ ▌
 ▰▰▰▰▰▱▱▱ 15.8k / 272k ─────────────────────────── ⏻ plan · ⠹ running 3.1s
````

**120×40** (the rule earns the width — the head is never truncated):

````
 wcode · ⎇ main · session a1b2c3d4 ─────────────────────────────────────────────────────── gpt-5-codex · high
 ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────────────────────────────────────────────────────────── ⏻ plan · ⏸ idle
````

**Legend:** the rule + its chrome `dim` (the project name `wcode` `muted`); `❯`
`accent`; the gauge `success`; `⏻ plan` `accent`; state `dim`/`accent`.

**§2 / §4 amendment.** §4 **Layout** (`docs/tui-design.md:248-252`) — the
"rounded border whose **four corners carry all chrome**" becomes a **rule-set
writing line** (the head on the rule, the gauge + state on the line below). The
`:69` frame row — already re-scoped by §11 to `box / overlay frame` — then
**narrows to `overlay frame`**: the composer no longer needs `╭ ╮ ╰ ╯`, so only
the **overlays** keep them. **No new glyph** (`─` is in the table).

**Cost / risk.** **0 rows** — the same 3 rows as the box. `draw_input_box`
(`crates/wcode-tui/src/ui.rs:1326-1339`) uses `block.inner(area)`, so the
composer wraps at `area.width - 2`; frameless it wraps at the **full width** —
the wrap points move. `corner_titles` (`:1350-1436`) / `pick_titles` (`:1449`) is
re-composed from **4 corners to 2 rule-lines**.

**Open question:** the head **on the rule** (shown) or on a line **above** it (the
C shape)?

---

### B · Two hairlines

**Stance:** a rule **above and below** the input — a set-off writing line, like a
quoted/inset block; chrome on the rules.

idle:

````
 wcode · ⎇ main · session a1b2c3d4 ──────────────── gpt-5-codex · high
 ❯ ▌
──────────────────────────────────────────────────────────────────────
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ───────────────────────────── ⏻ plan · ⏸ idle
````

running:

````
 wcode · ⎇ main · session a1b2c3d4 ──────────────── gpt-5-codex · high
 ❯ ▌
──────────────────────────────────────────────────────────────────────
 ▰▰▰▰▰▱▱▱ 15.8k / 272k ─────────────────────────── ⏻ plan · ⠹ running 3.1s
````

**Legend:** as A; the **second** `─` rule `dim`.

**§2 / §4 amendment.** As A, plus a **second** full `─` rule below the input (the
inset reading).

**Cost / risk.** **+1 row** (4 vs the box). The bottom rule may crowd the team
rider — check the 80×24 stack. Same `corner_titles` recomposition as A.

**Open question:** does the bottom rule double with the team strip's own boundary?

---

### C · A rule-set head + open line

**Stance:** a **running-head block** (the head line + a rule, exactly a paper's
running head) above the input; the input and the folio/gauge below as plain
lines; **no box at all**.

idle:

````
 wcode · ⎇ main · session a1b2c3d4            gpt-5-codex · high
──────────────────────────────────────────────────────────────────────
 ❯ ▌
 ▰▰▰▱▱▱▱▱ 14.2k / 272k ───────────────────────────── ⏻ plan · ⏸ idle
````

running:

````
 wcode · ⎇ main · session a1b2c3d4            gpt-5-codex · high
──────────────────────────────────────────────────────────────────────
 ❯ ▌
 ▰▰▰▰▰▱▱▱ 15.8k / 272k ─────────────────────────── ⏻ plan · ⠹ running 3.1s
````

**Legend:** the head line `dim` (project `muted`); the rule `dim`; `❯` `accent`;
the gauge `success`; state `dim`/`accent`.

**§2 / §4 amendment.** As A — the composer becomes a **running-head block** (head
line + rule) + two plain lines; `:69` narrows to `overlay frame`.

**Cost / risk.** **+1 row** (4). The head line is the **§11 literal-plate shape**
(the head line separate from the rule), so the two sections agree.

**Open question:** is the head a fixed header above the composer band, or part of
it? (It is part of the **composer band** — not a title bar.)

---

### Comparison + recommendation

| option | shape | rows | rules | `:69` frame row | risk |
|---|---|---|---|---|---|
| Before | 4-sided box | 3 | — | `box / overlay` | the container idiom |
| **A · writing line** | head on 1 rule + foot | **3** | 1 | `overlay` only | low, **0 rows** |
| B · two hairlines | head rule + input + rule + foot | 4 | 2 | `overlay` only | +1 row |
| C · head + open line | head line + rule + input + foot | 4 | 1 | `overlay` only | +1 row |

**Recommendation: A (the writing line).** In the skill's terms:

- **It fixes the container idiom for 0 rows** — the box's 3 rows become a
  head-rule + input + foot, so density is untouched (the pinned DENSITY 7–8
  holds). **B** and **C** each spend +1 row.
- **No new glyph** — it reuses the shipped `·`-separated head strings and the `─`
  glyph (already in the table), drops one `Block`, and `:69` **narrows** to
  `overlay frame` (a *narrowing* — no glyph retired, none added).
- It is the **paper's writing line**: the running head rides the rule (the shipped
  chrome, re-homed), the gauge + state below — exactly §11's "a rule, not a box".
- **B** and **C** are the more set-off readings if the human wants the air; **C**
  is the truest "running head" and matches §11's literal plate.

### Cost / risk (shared)

- `draw_input_box` (`crates/wcode-tui/src/ui.rs:1326-1339`) and `corner_titles`
  (`:1350-1436`) are re-composed; the corner **drop ladder** (`pick_titles`,
  `:1449`) is re-derived for 2 rule-lines, and the composer's wrap width changes
  (`area.width - 2` → `area.width`).
- **`NO_COLOR`:** the rules are structural (`─`), so the reading survives with no
  colour.
- **`≤1 accent`:** unchanged (`accent` = the prompt + running state +
  `⏻ plan`/`▤ browse`).
- **`·` ≤1 per metadata line:** the head keeps the shipped
  `project · ⎇ branch · session` string (two `·`) **unchanged**, under §9's
  shipped-corner exemption.

---

## 13. Spec-amendment list (per direction)

Every line names a **`docs/tui-design.md`** section (the only spec). Numbered
sections in bold are the ones the direction *must* amend before implementing
(`design-taste` §7.3); the rest are consequences.

### Shared (all directions) — §10, §11, §12
- **§2 Vocabulary (`docs/tui-design.md:68`)** — the table row `│ ─ ┼` → the
  `booktabs` set (`─` only); a *removal* (`│`, `┼`), **no new glyph**.
- **§4 Transcript — markdown table** — three `─` rules + a whitespace gutter +
  an optional `Table N:` caption block.
- **§2 Vocabulary (`docs/tui-design.md:69`) + §4 D31** — the `:69` row is
  **re-scoped** to `box / overlay frame` (the frame glyphs stay declared) and a
  **new row** `| tool plate rule | `─` | dim |` declares the tool plate; **no
  glyph retired, no glyph added** (**§11**).
- **§4 Layout (`docs/tui-design.md:240-256`) + §2 `:69`** — the composer's
  4-corner box → a **rule-set writing line**; the `:69` row then **narrows to
  `overlay frame`** (only the overlays keep `╭ ╮ ╰ ╯`); **no new glyph**.
  **Options in §12 — not yet settled** (**§12**).

### Preprint (§3)
- **§2 Vocabulary** — new glyphs `§`, `¹²³` (+ call sites).
- **§4 Transcript — Tool** — panel → turn-foot footnote list.
- **§4 Layout** — add the folio corner cell.
- §1.4 — a blank above the section head.

### Chapter (§4)
- **§1.2 Gutter** — name the gutter a *margin*.
- **§4 Transcript / markdown** — first-line `p + p` indent.
- **§1.4 Block separation** — within-speaker paragraphs separated by the indent.
- (No glyph, no layout change.)

### Marginalia (§5)
- **§1.1 / §4 Layout** — a right margin region docked ≥120 cols only.
- **§4 Transcript — Tool / Thinking** — a margin placement ≥120.
- §4 Keys — a toggle key **only if** the human wants manual control (recommended:
  none; width-driven).

### Broadsheet (§6)
- **§1.1** — a scroll-away `Block::Masthead` at the transcript head.
- **§4 Transcript — Tool** — compact one-line tool as the default; panel on expand.
- §1.4 — the masthead rule as a role separator.

### Reader (§7)
- **§4 Transcript** — a `Notes` summary block collapsing all apparatus.
- **§1.4 Block separation** — blank between *every* within-speaker paragraph.

---

## 14. Keys delta (all directions)

The skill's rule is absolute: **every interactive thing has a key + a `KEYS`
entry** (`SKILL.md:98`, `docs/tui-design.md:275-296`). The directions aim to add
**zero** keys:

| interactive thing (which direction) | key | reuse? |
|---|---|---|
| expand a turn's apparatus | `Ctrl-T` (all tools) / browse `Enter` | **reused** — shipped |
| expand one footnote (Preprint) | browse `Enter` / `Space` | **reused** — shipped (`docs/tui-design.md:210`) |
| collapse to / from `Notes` (Reader) | `Ctrl-T` | **reused** |
| the margin column (Marginalia) | *none* — width-driven, deterministic | n/a (like the sidebar's ≥80 dock) |

If the human asks for a *manual* margin toggle (Marginalia), that is one new key
and one `KEYS` entry (`crates/wcode-tui/src/app.rs`, the `KEYS` table) — flagged,
not assumed.

---

## 15. Pre-flight (`design-taste` §8, checked for this proposal)

**Both** — §0 surface line emitted (§0) ✓ · dials stated and reasoned (§1) ✓ ·
truth source named (the locked spec, §1/§2 of `docs/tui-design.md`) ✓ · §3–§4
rules met (below) ✓.

**Terminal (A), per the skill's checklist (`SKILL.md:184-188`):**

| box | status |
|---|---|
| exactly three bands, no title bar | ✓ — every direction keeps transcript → rider → box; the "running head" rides the **box corner** (no title bar); Broadsheet's masthead is a **scroll-away block** (§6d) |
| blank line between roles | ✓ — preserved in all five (`crates/wcode-tui/src/app.rs:1235-1237`) |
| every glyph from the locked table, no new glyph | ✓ **except** Preprint's `§` + `¹²³`, declared as §2 amendments with call sites (§3e); the table glyph row `│ ─ ┼` is a *removal* (§10) and the tool frame is a *re-scope* (§11: the glyphs stay declared for the box/overlays) — and §12 proposes narrowing the composer's use too, no new glyph; `→` in frames is **model prose**, not chrome |
| palette roles unchanged, ≤1 accent, new role has a call site | ✓ — **no new role**; `accent` stays the prompt + running state |
| hex only under `Rgb`; `NO_COLOR` still `Theme::plain` | ✓ — these directions add **no** colour (`crates/wcode-tui/src/theme.rs:87-109`) |
| legible at 80 cols | ✓ — every frame is drawn at 80 unless it *needs* 120 (Marginalia, Preprint) |
| every interactive thing has a key + a keymap entry | ✓ — §14; the directions add **zero** keys |

**Medium B rules are N/A** (this is Medium A; `SKILL.md:100`) — the site's tokens
and the em-dash/vs-code rules do not reach the terminal.

---

## 16. What I did NOT verify (honest)

- **No terminal run and no captured frames.** Every frame above is
  **hand-authored text**, not a `TestBackend` snapshot or a real crossterm draw.
  Column counts and ruler offsets are approximate and were **not** measured
  against a running binary.
- **The new glyphs were not rendered.** `§` and `¹²³` (Preprint) were not checked
  for width — `¹²³` are treated as 1 cell by `markdown.rs`'s `disp`
  (`crates/wcode-tui/src/markdown.rs:779-781`, "wide glyphs are treated as one
  cell for now"), which may be wrong on some terminals; **this is a real risk,
  not verified.**
- **No code was compiled or tested.** No `cargo build`/`test`/`clippy` applies —
  this doc changes no Rust. The 17-role list, the band layout, the gutter
  constants, and the corner-title code were **read**, not exercised.
- **The composer move was not rendered.** The §12 frames are hand-authored; the
  row counts (3/4) and the wrap-width change (`area.width - 2` → `area.width`)
  were **read** from `draw_input_box` (`crates/wcode-tui/src/ui.rs:1326-1339`),
  not measured.
- **The frame-to-rule move was not rendered.** The before/after plates are
  hand-authored; `panel_frame` (`crates/wcode-tui/src/ui.rs:898-925`) was
  **read**, not changed.
- **The table move was not rendered.** The `booktabs` before/after frames are
  hand-authored; `table_lines` was **read** (`crates/wcode-tui/src/markdown.rs:636-686`),
  not run, and its grid tests (`:1211-1214`) were **not** executed against a
  `booktabs` change.
- **The density numbers are judgments**, not measurements: the **≈0** air costs
  (Preprint, Chapter, Marginalia) and **+3** (Broadsheet) are hand-counted rows
  in the frames above, and a real draw (with the team rider present, or the box
  grown to 8 rows) changes them.
- **The sidebar × margin interaction (Marginalia ≥120 + `Ctrl-B`)** was **not**
  resolved — it is stated as an open question, not solved.
- **`docs/tui-design.md` was not amended**, and **nothing outside `docs/`** was
  touched. This is a draft for a human; a pick amends the spec first.

---

## 17. References

- [`docs/tui-design.md`](../tui-design.md) — the **locked** spec (§1 principles,
  §2 the single glyph table, §4 components, §6 open questions). The authority.
- [`crates/wcode-tui/src/theme.rs`](../../crates/wcode-tui/src/theme.rs) — the 17
  named roles + the color-mode ladder.
- [`crates/wcode-tui/src/ui.rs`](../../crates/wcode-tui/src/ui.rs) — the renderer
  (`PANEL_INDENT`, the band `Layout`, the input-box corners).
- [`crates/wcode-tui/src/markdown.rs`](../../crates/wcode-tui/src/markdown.rs) —
  what the transcript can render (the ceiling on "editorial typography").
- [`docs/work/tui-render/mockups.md`](../work/tui-render/mockups.md) (v1) and
  [`redesign-v2.md`](../work/tui-render/redesign-v2.md) (v2, D29–D36) — the prior
  proposals these extend.
- [`docs/design/tui-tool-glyphs.md`](tui-tool-glyphs.md) — the companion proposal
  for the tool panel's **marks** (`⚙`/`✓`/`✗`, and the at-risk `⧉` copy glyph):
  three systems, a recommendation, and a `disp`-width correctness fix.
- [`docs/design/vscode-paper-direction.md`](vscode-paper-direction.md) — the
  "paper" stance (Medium B) these translate to the terminal.
- [`.wcode/skills/design-taste/SKILL.md`](../../.wcode/skills/design-taste/SKILL.md)
  — §0 surface read, §1 dials, §3 Medium A rules, §5 copy, §7 amend-first, §8
  pre-flight.
