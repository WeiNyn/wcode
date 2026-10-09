# VS Code paper page — the team list: placement & improvement options

- **Status:** options, for a human to pick. **Not authorized** — nothing under
  `editors/vscode/` is touched and no spec is amended. The paper draft's default
  is unchanged (the caption).
- **Prototype:** [`vscode-paper-draft.html`](vscode-paper-draft.html) — the
  **Team** toolbar button cycles all five placements live (gallery chrome only).
- **Context:** refines direction A on the paper page
  ([`vscode-paper-direction.md`](vscode-paper-direction.md)); the shipped
  editorial surface is W005 ([`docs/work/W005-editorial-vscode-surface.md`](../work/W005-editorial-vscode-surface.md)).
- **Date:** 2026-10-09

## The object

The team list today is A's **caption**: a dim, wrapping `role="list"` of
`glyph name action` rows directly under the running head
(`vscode-paper-draft.html:vlYeQ`; markup `:FVael`). Each row is the retarget
affordance — `tabindex="0"`, Enter/Space (`view.ts:DQHV6`) → `focus-member`
(`chat.ts:KWHuo`, `view.ts:uWZi8`). Under D006 it is the team's **one home**
(the rail was removed), so any move has to keep a home and keep the key.

## Why it is the last widget on a page

Everything else on the paper surface became a **page artifact** — the running
head (`:xSpPp`), the folio (`:MzQWx`), the chapter headings (`:m6NwY`), the
footnote apparatus (`:tH0Eu`), the one live line (`:lrBi1`). The roster is the
one strip still reading as a *status widget* bolted under the head. Three
frictions:

1. **It duplicates the liveline.** The live line already reports who is active
   (`view.ts:KeUWa`/`psxWY`); the caption repeats that plus the idle rows.
2. **It is always a block of vertical space above the page**, and the idle rows
   carry noise (`waiting`, `turn done`).
3. **It competes with the folio** for the reader's first glance, on the head's
   single line.

And by the A stance — *the words are the interface* — the cast is **already
legible in the text**: every turn names its speaker (the heading, `view.ts:nsUTe`).

## The shared improvement levers (apply with *any* placement)

- **Active-first order** — sort by the liveline's own predicate
  (`running || liveAction !== undefined`, `view.ts:KeUWa`), so the glance lines
  up with the live line.
- **Elide the idle tail** into a count (`+2`) — one line; quiet when nothing runs.
- **Drop the per-member `action`** where the liveline already carries it — the
  roster keeps names + state glyph only.
- **A real location** (the *index*, below): each name gets its turn/step numbers
  — the same numbering the folio prints.
- **Keep the activation key** — whatever carries retarget stays a focusable
  element with Enter/Space (`isActivationKey`, `view.ts:DQHV6`); the role's rule
  is *every interactive thing has a key*.

## The five placements

### Option 1 · Caption (status quo, refined)

- **Where:** under the running head (`:vlYeQ`, `:FVael`).
- **What it carries:** `glyph name`, the whole roster, one line after the levers.
- **Reading:** A's, tightened — the lowest-risk placement; the improvement is
  purely the levers.
- **Cost:** still the last widget on the page; still competes with the folio.
- **Key:** the rows (unchanged).

### Option 2 · Colophon (move to the foot)

- **Where:** the page's **imprint** at the very foot, below the writing line
  (`:mYBuS`; shown only in this mode, `:iTezZ`).
- **What it carries:** `glyph name` per member, no per-member action.
- **Reading:** the page *opens* with only the work and the folio (a book's
  imprint sits at the front or the back, never in the reading line).
- **Cost / risk:** a persistent foot band **is a status bar by another name** —
  the exact widget the A stance opposed — and it competes with the composer's
  foot. Mitigation: show it **only when nothing is running**; the liveline owns
  the active case, so two team displays are never on screen at once.
- **Key:** the rows (unchanged).

### Option 3 · Index (name + location)

- **Where:** in the head (a running cast-index) **or** at the foot (a true
  back-of-book index); the prototype styles it in the head (`:qvAD2`, `:V5uFC`;
  it hides the action, `:VLLDH`).
- **What it carries:** `name` + its **turn/step numbers** — real locations, the
  same numbering the folio prints.
- **Reading:** this is the strongest *improvement* per the brief: it turns a
  status strip into a **reference**, which is what a paper/book index is. It
  adds information rather than moving a strip.
- **Cost / risk:** more marks on the line; the numbers must be real (derived,
  not decorative); the per-member action needs a home — dropped, or the liveline
  carries it.
- **Key:** the rows (unchanged), and each number is a jump target (an optional
  extra affordance).

### Option 4 · Dissolve (no band)

- **Where:** nowhere. The cast is (a) the **turn headings** — every speaker
  names itself — and (b) the **liveline** — who is active *now*.
- **Reading:** the purest "the words are the interface": the last widget goes,
  and the page opens with only the work and the folio.
- **Cost / risk:** the deepest loss — no whole-roster glance; an idle member is
  invisible until it speaks; it deepens D006's accepted loss. Retarget must move
  onto the heading (the byline becomes `tabindex="0"` + Enter/Space).
- **Key:** the **heading** becomes the retarget target (a new keymap entry).

### Option 5 · Margin (marginalia)

- **Where:** the **outer margin**, aligned to the turn it belongs to
  (`:68Pr9`; positioned at `:sDn6t`).
- **What it carries:** the member's **live action** (what the state band carried),
  *not* the name — the name is already the heading.
- **Reading:** the most book-true (a Tufte sidenote): the team's state sits
  *where the text is*, not in a band.
- **Cost / risk:** a margin column is a second column → tension with the
  single-measure stance; it needs width, and **falls back to the caption** at a
  sidebar dock (`:VSvfh`, `:YdcB2`) and below 560px (`:oOoeI`).
- **Key:** the margin note is the retarget target (focusable).

## Comparison

| # | placement | carries | glance | retarget home | vertical cost | width-safe | book-ness | risk |
|---|---|---|---|---|---|---|---|---|
| 1 | head caption | glyph + name (+action) | best | rows | a line under the head | yes | low | **low** |
| 2 | foot colophon | glyph + name | fair (look down) | rows | a line above the foot | yes | high | low |
| 3 | head/foot index | name + **location** | good | rows + numbers | a line | yes | **highest value** | medium |
| 4 | none | nothing | none | the heading | **zero** | yes | highest (pure) | medium |
| 5 | margin | the live action | contextual | the note | zero (margin) | **no** (falls back) | highest (form) | medium-high |

## Recommendation

A **sequence**, not a single pick:

1. **Adopt the levers now**, on top of Option 1 (active-first, elide the idle
   tail, drop the per-member action, one line). That alone makes the caption
   quiet and costs nothing structural.
2. **Target Option 3 placed as the colophon (2+3 fused):** the foot imprint is
   an **index of the cast with its locations**, folded to a single dim line and
   shown only when nothing is running. It removes the strip from the head (the
   page opens clean), gives the roster a book-true home (the imprint), and *adds*
   the one thing a status strip never had: real locations.
3. Keep **Option 4** as the "if you want the last widget gone" endgame, and
   **Option 5** as an opt-in at editor width only.

## Open questions

- **Q1 · Placement.** 1, 2, 3, 4 or 5 — or the recommended 2+3 fusion?
- **Q2 · The action.** Does the per-member `action` belong on the roster at all,
  or only on the liveline (and, in Option 5, in the margin)?
- **Q3 · The location.** Turn ordinal, or the **folio/step number** the paper
  draft already prints? (The latter keeps one numbering on the page.)
- **Q4 · The idle case.** Elide to `+2`, or hide the roster entirely when nothing
  runs (leaving only the liveline)?
- **Q5 · Retarget.** Keep it on the roster rows, or move it to the heading
  (Option 4) / the margin note (Option 5)?
- **Q6 · Scope.** Is the roster the whole **cast** (session-scoped), or only the
  active members the liveline already reports?

## Facts NOT verified

- **No browser and no VS Code host were run.** The prototype was parse-checked
  (`node --check` on the inline script) and token-audited (tag/brace balance,
  glyph set, zero external refs, no rendered em-dash) — but **not painted**.
- **The margin fallback is approximate.** The gallery's "sidebar dock" is an
  *app* width, not a viewport width; the guard is app-width (`:VSvfh`,`:YdcB2`)
  plus a real `@media (max-width:560px)` (`:oOoeI`). In the true webview the panel
  *is* the viewport, so the media query alone would govern.
- **The index numbers are illustrative** — baked into the prototype, not derived
  from a session.
- **Not confirmed:** that the roster should be session-scoped; that the action
  should leave the roster; that D006's "one home" should be *moved* rather than
  *deepened* (Option 4). All are proposals.
- **Not touched:** `editors/vscode/**` and every spec doc — this is options only.

## References

- [`vscode-paper-draft.html`](vscode-paper-draft.html) — the paper prototype (the **Team** toggle drives these).
- [`vscode-paper-direction.md`](vscode-paper-direction.md) — the paper stance, dials and refusals.
- `editors/vscode/src/webview/chat.ts` (`renderCaption:KWHuo`, `renderMasthead:T33oI`), `src/webview/view.ts` (`teamCaption:uWZi8`, `CaptionRow:LXxM3`, `isActivationKey:DQHV6`, `workingGroup:KeUWa`) — the shipped roster.
- `docs/decisions/D006-editorial-supersedes-v3-decisions.md` — the rail's removal (the team's "one home").
- `.wcode/skills/design-taste/SKILL.md` §7.2/§7.3 (audit + amend-first).
