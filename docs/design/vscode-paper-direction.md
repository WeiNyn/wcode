# VS Code surface — direction A, refined: "paper"

- **Status:** draft, for a human to react to. **Not authorized** — nothing under
  `editors/vscode/` is touched and no locked spec is amended. A genuine pick
  amends the spec first (`design-taste` §7.3).
- **Prototype (the WHAT):** [`vscode-paper-draft.html`](vscode-paper-draft.html)
  — self-contained, open it in a browser; it reads as the spec.
- **Refines:** `A · editorial` — [`vscode-modern-editorial-draft.html`](vscode-modern-editorial-draft.html),
  shipped in W005 (`docs/work/W005-editorial-vscode-surface.md`).
- **Date:** 2026-10-09

## §0 — surface read

> Reading this as: **a web screen (the VS Code webview)**, for **a developer
> reading a long multi-agent session inside VS Code**, in **VS Code's own
> theme-variable language** (only `--vscode-*` for colour, one accent), with the
> dominant constraint that **the whole panel reads as one printed page: a single
> measure governs the running head, the text block and the writing line.**

Medium **B** (web screen), the same surface and system as A — not the terminal,
not the wcode site.

## The stance (one line)

A answered "the widget was the problem": it removed the rail, the spine, the
fill, the pill, and left a quiet column, a byline and air — but the column is
still **screen-shaped**, three independently padded bands in a viewport, its
turns a spaced list. It reads like a **page** when five habits of printed matter
are taken: one measure governs the sheet; the prose is *set* while the furniture
stays *mono*; a running head carries the work and a folio carries the position;
paragraphs are indent-led; and the tool apparatus is **footnote matter**, not a
fold in the running text.

## Dials

`VARIANCE 1 · MOTION 1 · DENSITY 3` — unchanged from A, and they must be:

- `VARIANCE 1` — a reading page has no layout variation left to spend; hierarchy
  is bought in **type and space** alone (as in A).
- `MOTION 1` — the `▌` live cursor and the `⠋` spinner are the only motion; both
  are under the single `prefers-reduced-motion` gate
  (`vscode-paper-draft.html:nYVbs`). This refinement adds none.
- `DENSITY 3` — **a rhythm change, not a density change.** The indent *removes*
  the between-paragraph blank; relocating the tool output into footnotes *moves*
  it rather than adding it; the shared measure *adds* margin. A wash.

## Levers spent (§7.4 order)

1. **Typography** — a serif reading face for the prose (`:MZHzL`), a `1.75`
   leading (`:5JdBP`), `text-wrap: pretty` + `orphans/widows` (`:shUFh`), and a
   small-caps chapter open (`:Dg6We`).
2. **Spacing** — one measure for head, text and writing line; the indent-led
   paragraph rhythm; the short footnote separator rule.
3. **One section's recomposition** — the tool fold becomes footnote matter
   (`renderTurn` collects `tool` blocks out of the flow; see the seam below).

`--wc-1…5`, `--wc-fs-*`, `--wc-radius` are reused; **no scale token is added**,
no colour beyond `--vscode-*`, no glyph beyond the locked set.

## The moves, with anchors

1. **One measure governs the sheet.** A centers only the transcript
   (`docs/design/vscode-modern-editorial-draft.html:9ZQA6`); its head and
   composer are full-bleed (`:7VRAj`, `:xStnj`), so the rules run edge to edge.
   Here the head (`.runninghead .inner` `:pIzFv`), the text (`.col` `:gWgZ7`) and
   the writing line (`.composer .inner` `:fmDSK`) share `--ed-measure` (`:btoH5`)
   and each rule sits on the measured (inner) element — the page's top and bottom
   edges. Under 560px the rules fall back to full bleed (`:SsDEV`).

2. **The prose is set; the furniture is not.** `.body` takes a **local** serif
   stack (`--ed-read` `:MZHzL`) at `line-height: 1.75` (`:5JdBP`), while the
   running head, bylines, footnotes, live line and composer controls keep the
   editor's mono. It is one surface in two voices: what you *read* is set, what
   you *operate* is code-adjacent.

3. **A running head and a folio.** The head (`:xSpPp`) is the work — the state
   glyph, the identity, the quiet All/Focus toggle (`:tGhxQ`), the `ctx N` — and
   the **folio** (`:MzQWx`) is the **step count** at the outer edge, tabular, the
   way a page number locates you. (A's masthead was already a running head; the
   folio is what it lacked.)

4. **Indent-led paragraph rhythm.** Within one speaker, `p + p` takes a
   first-line indent (`:x6Guo`, knob `:fSqUT`); the first paragraph of a turn is
   un-indented (`:hTdT4`); a block opens a fresh paragraph (`:WFkhQ`); a speaker
   change keeps the air (`:C7Ozm`). The chapter open sets the first line in
   small caps (`:Dg6We`).

5. **Tool detail is footnote matter, not a fold.** A tool call leaves a printed
   reference (`<button class="fnmark">`, `:RDs6C`) in the sentence; the apparatus
   is a `.footnotes` list at the turn's foot (`:tH0Eu`) under a **short** rule
   (44% width), one line per step (`⚙ bash · cargo test --workspace · ✓`), with
   the output behind a native disclosure *inside the note* (`details.fn-out`
   `:GODr0`). The reading text is never interrupted by an expanding block.

6. **A code block is a plate.** A small-caps caption line (`:xp5gr`, reusing the
   shipped `.code`/`.chead` markup) above A's fill-only recess (`:hDztO`) — the
   way a paper captions a listing.

## Components added / removed vs A

| | A (shipped W005) | paper |
|---|---|---|
| **Added** | — | a **folio** (the step count); `.fnmark` references + a `.footnotes` list; a code **caption**; a small-caps chapter open; a reading face |
| **Removed** | a working pill (→ liveline) | the inline **tool fold** in the flow (→ footnote matter); the masthead's separate `working` word is folded into the state glyph |
| **Kept** | the masthead line + team caption, the byline, the liveline, the composer field with `❯` and text-link controls, the `.turn.new` beat, the change-review card | all of them |

## Implementation seam (it is implementable)

Every fact the paper surface renders already reaches the webview — **no wire
change**. The seams, in the real code:

- **Folio** — the step count is `RenderedState.blocks`' count of `tool` blocks
  (or a new pure `folio(blocks)` beside `panelHeader`, `view.ts:s7HzW`). It rides
  a new header cell in `view.ts:panelHeader` (`:s7HzW`, the `MeterCell` `:WWTfk`
  neighbourhood) and paints in `chat.ts:renderMasthead` (`:T33oI`).
- **Running head / measure** — the skeleton already emits `#masthead`,
  `#transcript`, `#composer` (`chat.ts:Bnfe4`); the measure is **pure CSS** in
  `media/chat.css`. No DOM change.
- **Footnotes** — `chat.ts:renderTurn` (`:KoM4E`) already loops a turn's blocks;
  it gains "emit a `.fnmark` at each `tool` block's position, collect the `tool`
  blocks, and append the `.footnotes` `<ol>` after `.body`". The note's head
  reuses `toolMeta` (`chat.ts:alUZD`) and its body reuses `rawOutput` (`:aYpYa`)
  or `reviewBlock` (`:YVJx9`). The change-review card **stays a card** (non-scope).
- **Reading face / measure / rhythm** — pure CSS. No pure-half change.
- **Keys** — the `.fnmark` is a `<button>` (Enter/Space), `aria-expanded` mirrors
  the note; the note's disclosure is a native `details` (its key is already
  paid). No new keymap entry beyond activation.

## The type-scale question (explicit, as asked)

**A font stack is not a type-scale change.** The locked scale is the four SIZE
steps — `--wc-fs-meta/ui/body/code` — and the `--wc-1…5` grid; paper **adds no
size**, reuses every step, and the prose keeps `--wc-fs-body`. What it adds is a
*family* (`--ed-read`), a derived local knob exactly like A's own `--ed-measure`
(`docs/design/vscode-modern-editorial-draft.html:qRgMC`,
`editors/vscode/media/chat.css:cFrwd`). The stack **names locally installed
families and falls back to the generic `serif`** — no webfont, no `<link>`, no
`@import`, **zero requests**. It is the one judgment call, and the prototype's
**Face** toggle A/Bs it (page ↔ terminal = A's shipped sans).

## Locked invariants carried (checked)

- One accent (`--wc-accent`) only; every colour is `--vscode-*`. The footnote
  **reference is a link** (`--vscode-textLink-foreground`), never the accent,
  because it is not "active now".
- Zero external requests (audited: no `http(s)://`, no `<link>`, no `@import`,
  no `src=`). The serif is a named local stack with a generic fallback.
- No new tokens: the three `--ed-*` are derived knobs; the eleven shipped
  `--wc-*` are the only `--wc-*` declared (audited).
- Glyphs unchanged — the locked set only (rendered: `❯ ⚙ ✓ ○ ⠋ ⋯ −`).
- Three bands only (running head / text / writing line); the refinement realigns
  them, it does not add a fourth.
- Motion rare and gated by one `prefers-reduced-motion` rule (`:nYVbs`).
- ≤1 `·` per metadata line (the footnote head uses none; the note's stat uses at
  most one).
- No `—` in any rendered string (audited: the only em-dashes are CSS/JS comments).

## Risk / effort

**Low / low-medium.** The risky half is *taste*, not structure: (a) a serif
beside VS Code's own UI will read as foreign to one reviewer and as the whole
point to another — hence the Face toggle; (b) footnote matter is a real
*recomposition* (the tool moves out of flow), so `renderTurn` changes more than
any other move, and the existing fold override/`details` behaviour has to move
with it; (c) the running head + folio risks reading *busier* than A's bare line.
No structural data change, no wire change, no kernel change.

## Deliberately refused (anti-defaults)

- **A webfont serif** (the obvious "book" move) — an external request. Refused.
- **A "paper white" fill or a forced light theme** — a container that is not
  content, and a mid-surface theme flip. Refused; the surface tracks the host.
- **A section rule above each speaker** (a chapter divider) — that is the
  `.who::after` hairline W005 deliberately removed (D006). Refused; the chapter
  feel comes from leading, the heading and the first line's small caps.
- **A drop cap** — a new treatment and a glyph-rule risk. Refused.
- **A decorative folio page number** — the folio here is the **real** step count,
  not chrome; a number would be decorative only if it were invented.
- **Serif for code or footnotes** — the mapping is *prose is set, apparatus
  operates*. Refused.
- **A margin (sidenote) column** — a second column breaks the single-measure
  stance and dies at sidebar width. Offered only as an open question.

## Open questions for the human

- **A1 · The reading face.** Keep the local serif for prose (mono for furniture),
  or revert to A's sans? **Recommend:** keep; A/B with the Face toggle.
- **A2 · The footnote move.** Is relocating the tool output into turn-foot
  footnotes the right trade (a quieter reading text vs a longer scroll past
  apparatus)? **Recommend:** footnotes; the reference stays in the sentence.
- **A3 · The folio.** Is the step count the right "page number", or the *turn*
  count, or `ctx N`? **Recommend:** the step count (a real reading position).
- **A4 · The running head.** Static (the session identity, as here) or
  scroll-tracking (the speaker you are reading)? A scroll-tracking head is more
  book-true but adds scroll-spy JS and motion.
- **A5 · The chapter open.** Small caps on the first line of every speaker turn,
  or only on a larger section break? **Recommend:** keep; can be dropped.
- **A6 · The measure.** `64ch` on the serif vs A's `66ch`. **Recommend:** `64ch`.
- **A7 · A margin column.** Offer true Tufte sidenotes at editor width, falling
  back to footnotes below 560px? More book-true, more risk.

## Facts NOT verified

- **No browser and no VS Code host were run.** The prototype was **parse-checked
  and token-audited, not painted**: `node --check` on the inline script (OK),
  tag balance (0 unclosed), CSS brace balance (125/125), and audits for stray
  hex, new tokens, external requests and emitted em-dashes. A human F5 against
  the real webview is the proof step.
- **The serif stack resolves to the generic `serif`** where none of the named
  families is installed (e.g. a bare Linux box) — by design, but the *painted*
  result differs by platform and was not observed here.
- **The step count is illustrative.** The folio in the prototype is a static
  number bumped by the run simulation, not read from a session; the *real* value
  is `RenderedState.blocks` (see the seam) and was not wired here.
- **Not confirmed:** that the tool move should be turn-scoped (not session-scoped)
  footnotes, and that D006's removals should be *added to* rather than held.
- **Not touched:** `editors/vscode/**` and every spec doc — this is a draft only.

## References

- [`vscode-paper-draft.html`](vscode-paper-draft.html) — this refinement (the WHAT).
- [`vscode-modern-editorial-draft.html`](vscode-modern-editorial-draft.html) — direction A, shipped in W005.
- [`vscode-modern-directions.md`](vscode-modern-directions.md) §A — the stance, dials and locked vocabulary.
- `editors/vscode/media/chat.css`, `src/webview/chat.ts`, `src/webview/view.ts` — the shipped editorial implementation and the seams.
- `docs/plans/vscode-ui-editorial-plan.md`, `docs/decisions/D006-editorial-supersedes-v3-decisions.md` — the authorized direction this refines.
- `.wcode/skills/design-taste/SKILL.md` §7.2/§7.3 (audit + amend-first), §5 (copy), §6 (`[web]` tells).
