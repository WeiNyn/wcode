# VS Code surface — direction A, refined: "paper / book reading"

- **Status:** draft, for a human to judge. **Not authorized** — nothing under
  `editors/vscode/` is touched and no locked spec is amended (W005's shipped
  editorial markup/CSS is the `WHAT` this refines; a genuine pick amends the
  spec first, per `design-taste` §7.3).
- **Prototype (the WHAT):** [`vscode-reading-draft.html`](vscode-reading-draft.html)
  — open it in a browser; it reads as the spec.
- **Refines:** `A · editorial` — [`vscode-modern-editorial-draft.html`](vscode-modern-editorial-draft.html),
  shipped in W005 (`docs/work/W005-editorial-vscode-surface.md`).
- **Date:** 2026-10-09

## §0 — surface read

> Reading this as: **a web screen (the VS Code webview)**, for **a developer
> reading a long multi-agent session inside VS Code**, in **VS Code's own
> theme-variable language** (only `--vscode-*` for colour, one accent), with the
> dominant constraint that **the whole panel reads as one printed page — a single
> measure governs the running head, the text block and the writing line.**

Medium **B** (web screen), the same surface and system as A — not the terminal
and not the wcode site.

## The stance (one line)

Direction A answered *"the widget was the problem"*: it removed the rail, the
spine, the fill, the pill, and left a quiet column, a byline and air. It is
still **screen-shaped** — three independently padded bands stacked in a
viewport, its turns a spaced list. It reads like a **page** when three habits of
printed matter are taken:

1. **One measure governs the whole sheet.** The running head, the text block and
   the writing line sit on the same column, and the two rules that bound them run
   to the column's edges, not the viewport's.
2. **The prose takes a reading face; the furniture does not.** What you read is
   set like a page; what you *operate* — bylines, tool sentences, code, the live
   line, the composer's controls — keeps the editor's mono/sans. It is still a
   machine surface, set in two languages.
3. **The paragraph rhythm is indent-led.** Within one speaker, paragraphs are set
   solid and separated by a first-line indent (a novel's rule); a *speaker
   change* gets the air. A is a spaced list of turns; this reads as running text.

## Dials

`VARIANCE 1 · MOTION 1 · DENSITY 3` — unchanged from A, and they must be:

- `VARIANCE 1` — a reading page has no layout variation left to spend; hierarchy
  is bought entirely in **type and space** (as in A).
- `MOTION 1` — the `▌` live cursor and the `⠋` spinner are the only motion, both
  under one `prefers-reduced-motion` gate (`vscode-reading-draft.html:nYVbs`,
  the `.spin, .body.live::after, .liveline .dots` rule). This refinement adds none.
- `DENSITY 3` — **a rhythm change, not a density change.** The indent *removes*
  the between-paragraph blank line while the shared measure *adds* margin: a wash.

The refinement spends the two lightest levers (`design-taste` §7.4 order):
**typography**, then **spacing**. No new colour, no new size, no new glyph.

## The three moves, with anchors

**1 · One measure (spacing / composition).** A centers only the transcript
(`docs/design/vscode-modern-editorial-draft.html:9ZQA6`); its masthead and
composer are full-bleed (`:7VRAj`, `:xStnj`), so the two rules run edge to edge
and the panel reads as three bands. The refinement puts all three on
`--ed-measure` and moves each rule onto the inner (measured) element:
`.masthead .inner` (`vscode-reading-draft.html:BGqVC`), `.col` (`:gWgZ7`,
`:ZnugF`), `.composer .inner` (`:fmDSK`). Under 560px the measure is wider than
the panel, so the rules fall back to full bleed (`:SsDEV`).

**2 · A reading face for prose (typography).** `.body` sets in a **local** serif
stack (`--ed-read`, `:MZHzL`) — `"Iowan Old Style","Charter","Palatino Linotype",
"Book Antiqua",Georgia,"Times New Roman",serif` — at `--wc-fs-body + .5px` and
`line-height 1.72` (`:5JdBP`). The stack *names locally installed families and
falls back to the generic `serif`*: this is **not** a webfont and issues **no
request** (the gallery toggles "Face: reading / terminal", `:lKQlg`, A/B it
against A's shipped sans). The furniture stays mono: bylines, `details.fold.tool`,
`pre.quote`, `.liveline`, the composer's `.foot` and `❯` (`.byline`, `:lrBi1`,
`:fmDSK`). The textarea shares the reading face so the sentence you write looks
like the sentence you will read (`:EWzAT`).

**3 · Indent-led paragraph rhythm (spacing / typography).** A spaces every
paragraph (`margin-bottom: --wc-2`) *and* beats at a speaker change. The
refinement keeps the beat and replaces the inter-paragraph space with a
first-line indent: `.body > p + p { text-indent: var(--ed-indent) }` (`:x6Guo`,
knob `:b5LpM`), the first paragraph of a turn un-indented (`:hTdT4`), and a
block (`pre`, `details`) opening a fresh, un-indented paragraph (`:WFkhQ`).
`text-wrap: pretty` + `orphans/widows: 2` (`:shUFh`) are the book habits that
keep a line from stranding — native, one declaration.

**Supporting moves.** The code block becomes a **plate**: a small-caps caption
line (reusing the shipped `.code`/`.chead` markup, `:xp5gr`) above A's fill-only
recess (`:VsKA5`) — a paper captions a listing. The masthead's right-aligned
status is read as the page's **folio** (`:vUCIp`, `.status`), kept from A.

## What changes, A → this refinement

| # | A (shipped W005) | refinement | anchor |
|---|---|---|---|
| 1 | transcript centered; head + writing line full-bleed | **one measure** for head, text, writing line; rules to the column | `:BGqVC`,`:gWgZ7`,`:fmDSK` |
| 2 | prose = the shipped sans; only mono for meta | **prose in a local serif reading face**; furniture stays mono | `:5JdBP`,`:MZHzL` |
| 3 | `p { margin-bottom: --wc-2 }` (spaced list) | **first-line indent** within a speaker; beat at a speaker change | `:x6Guo`,`:b5LpM` |
| 4 | no widow control | `text-wrap: pretty`, `orphans/widows: 2` | `:shUFh` |
| 5 | `pre.quote` = bare fill recess | **caption line** above it (reuse `.chead`), recess unchanged | `:xp5gr`,`:VsKA5` |
| 6 | `.col` only | head/liveline/writing line join the measure | `:BGqVC`,`:v82ol`,`:fmDSK` |

Everything else in A is carried verbatim: the masthead line + dim team caption,
the byline (small-caps mono), the tool sentence, the one live type-line, the
composer as a field with `❯` and text-link controls, the `.turn.new` beat at
`--wc-5`.

## Locked invariants carried (checked)

- **One accent only** (`--wc-accent` = `--vscode-charts-blue`, "active now");
  every colour is `--vscode-*`. No colour is added; the refinement is
  colour-silent.
- **Zero external requests** — the serif is a *named local stack* with a generic
  fallback (`:MZHzL`); no `<link>`, `@import`, `src=`, or remote family. Verified:
  no `http(s)://`, no `@import`, no `<link>` in the file.
- **No new tokens** — the three `--ed-*` are **local knobs** derived from shipped
  steps (a width, a family, an indent), exactly as A's own `--ed-measure` was
  (`docs/design/vscode-modern-editorial-draft.html:qRgMC`,
  `editors/vscode/media/chat.css:cFrwd`). The dial says `new tokens 0`, `local
  knobs 3` (`:4PhtB`).
- **Glyphs unchanged** — the locked set only (`❯ ⚙ ✓ ✗ ⋯ ▌ ⠋ ● ○ ▸ ▾ ·`); no
  ornament, no folio digit, no drop cap, no rule glyph.
- **Three bands only** — running head / transcript / writing line; the
  refinement realigns them, it does not add a fourth.
- **Motion gated** — one `prefers-reduced-motion` rule covers both animations.
- **No `—` in any rendered string** — the em-dashes in the file live only in CSS
  and JS comments (not rendered).
- **Legible at a sidebar width** — `@media (max-width:560px)` returns the rules
  to full bleed (`:SsDEV`).

## Deliberately refused (the anti-defaults)

- **A webfont serif** (the obvious "book" move) — an external request. Refused;
  replaced with the local stack.
- **A "paper white" fill or a forced light theme** — a container that is not
  content, and a mid-surface theme flip. Refused; the surface keeps tracking the
  host theme.
- **A section ornament** (a centered `· · ·`, a rule) — a new glyph/usage with no
  call site. Refused; the `--wc-5` beat stays.
- **A folio page number** (a decorative `p. 12`) — decorative chrome. Refused;
  `ctx 42k` stays *semantic* status.
- **A drop cap** — needs a new treatment and breaks the glyph rule. Refused.
- **Serif for code or tool sentences** — the whole mapping is *prose reads,
  furniture operates*. Refused.
- **Reverting any of W005's removals** (rail, spine, pill, `you` fill, cards) —
  out of scope; this refines A, it does not re-litigate it.

## Open questions to settle (a layer-1 verdict)

- **A1 · The reading face.** The one judgment call. A serif is the single biggest
  step toward "book" and the one most likely to read as foreign beside VS Code's
  own UI. The prototype's "Face" toggle exists so the human can A/B it; `terminal`
  reverts to A's sans with everything else held. **Recommend:** reading face for
  prose, mono kept for furniture.
- **A2 · The measure.** `61ch` for a serif vs A's `66ch` (`:b5LpM`). A taste call;
  either holds.
- **A3 · The code caption.** Adding `.chead` back as a *caption* reintroduces a
  small label A had dropped. **Recommend:** keep it (a paper captions a listing),
  but it is the second thing to drop if the surface reads busier than A.
- **A4 · The `.5px` size bump** on prose (`:d02Bh`) — a leading/size nudge, not a
  scale change. If the rule "reuse the scale" is read strictly, drop to
  `--wc-fs-body` exactly.
- **A5 · The textarea face** (`:EWzAT`) — the one place the reading face touches
  an *input*. **Recommend:** keep; it makes the writing line part of the page.

## How to view

Open the prototype in a browser (macOS):

```
open docs/design/vscode-reading-draft.html
```

Its toolbar drives it: **Face** (reading / terminal), **Rhythm** (indent /
spaced), View All / Focus, Dock editor / sidebar, a run simulation, Density, and
Theme dark / light. The **Face** and **Rhythm** toggles are the two A/Bs the
refinement is asking about; the rest are A's own gallery chrome, not the surface.

## References

- [`vscode-reading-draft.html`](vscode-reading-draft.html) — this refinement (the WHAT).
- [`vscode-modern-editorial-draft.html`](vscode-modern-editorial-draft.html) — direction A, shipped in W005.
- [`vscode-modern-directions.md`](vscode-modern-directions.md) §A — the stance, dials and locked vocabulary.
- `editors/vscode/media/chat.css` + `src/webview/chat.ts` — the shipped editorial implementation.
- `docs/work/W005-editorial-vscode-surface.md`, `docs/plans/vscode-ui-editorial-plan.md`,
  `docs/decisions/D006-editorial-supersedes-v3-decisions.md` — the authorized direction this refines.
- `.wcode/skills/design-taste/SKILL.md` §7.2/§7.3 (audit + amend-first), §5 (copy), §6 (`[web]` tells).

---

## Facts NOT verified

- **No browser and no VS Code host were run** by this draft (consistent with
  W005's own "the webview was never run", `editors/vscode/README.md`). Every CSS
  claim here is read and structurally checked (tag balance, brace balance, zero
  external refs), **not painted**. A human F5 is the proof step.
- **The serif stack resolves to the generic `serif`** on a host that has none of
  the named families (e.g. a bare Linux CI box). That is by design (no request),
  but the *painted* result differs by platform; it was not observed here.
- **Not confirmed:** that changing the measure (`61ch`) is intended; A's `66ch`
  may be kept.
