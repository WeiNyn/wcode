# VS Code surface — modern design directions

**Status:** design draft, for a human to pick. Six distinct prototypes; no
implementation. Nothing under `editors/vscode/` is touched, and no locked spec is
changed. Each prototype is self-contained: open it in a browser and it reads as
the spec.

**What is being modernised.** The shipped surface is v3
([`vscode-ui-v3-plan.md`](../vscode-ui-v3-plan.md), the prototype
[`vscode-ui-v3-draft.html`](vscode-ui-v3-draft.html), the stylesheet
`editors/vscode/media/chat.css`): one header row, a team rail, a transcript of
L1-lines and L2-cards, a working pill, a composer. It is *coherent* — the
question is not whether it is broken, it is what a "more modern" read of a
multi-agent work surface looks like. Each direction answers that from a
genuinely different stance, not from a different shade.

## The locked constraints every direction carries

- **Colour** is ONLY `--vscode-*` tokens plus the ONE accent
  `--wc-accent = --vscode-charts-blue`, whose meaning is fixed: *"active now."*
  Zero remote assets. No invented glyph.
- **The shipped system owns spacing / type / radius**: `--wc-1…5` (a 4px grid),
  `--wc-fs-meta/ui/body/code`, `--wc-radius` 6px. Every direction **reuses those
  tokens and adds none** — the air, the density and the rhythm come from
  re-spending the existing steps. No direction changes the scale itself.
- **Motion** is rare and always `prefers-reduced-motion`-gated. Every prototype
  ships its gate.
- **The glyph vocabulary** is the locked TUI set — `❯ ⚙ ✓ ✗ ⋯ ▌ ⠋ ● ○` (plus the
  already-shipped `☑ ▸ ▾ ▰ ▱` and the `·` separator). No direction invents one;
  the "native" direction draws VS Code's own TreeView chevrons because *the host
  draws them*, not wcode.
- **Interactive things carry a key.** The "command" direction makes that literal
  (a keymap legend); the others inherit the shipped keys and add none.
- **One accent, ≤1 `·` per metadata line, no em-dash in a rendered string.**

Each prototype states its `--vscode-*` fallbacks in `:root` so it renders
standalone (the shipped drafts do the same); the gallery frame around the panel,
and the toolbar that drives it, are prototype chrome, not the surface.

---

## A · editorial — the words are the interface

- **Prototype:** [`vscode-modern-editorial-draft.html`](vscode-modern-editorial-draft.html)
- **Stance (one line):** the panel is dated because it is dressed like a widget —
  a rail, a spine, hairlines, a fill on your own words, a pill — wrapped around
  prose; it is modern when the words *are* the interface: one quiet column, a
  byline, and air.
- **§0 surface line:** *Reading this as: a web screen (the VS Code webview), for
  a developer driving a multi-agent session inside VS Code, in VS Code's own
  theme-variable language, with the dominant constraint that nothing may be a
  container that is not content.*
- **Dials:** `VARIANCE 1` · `MOTION 1` · `DENSITY 3`.
  - `VARIANCE 1` — the whole point is a single, uniform rhythm; there is no
    layout variation left to spend. Hierarchy is bought entirely in **type**
    (weight, size, colour) and space.
  - `MOTION 1` — the live cursor (`▌`) is the only animation; the spinner is
    dropped for the two members that are running (they read by their state
    word). This is the least motion of any direction.
  - `DENSITY 3` — down hard from v3's 6. A reading column is airy by definition.
- **Levers spent (§7.4 order):** **typography** then **spacing** — the first two,
  the lightest possible intervention. No new token; the air is `--wc-5` between
  speakers, a 66ch measure, and a 1.7 line-height.
- **Components added / removed:**
  - *Added:* a centered reading column; a `.byline` (name as small-caps mono);
    a single live type-line in place of the pill.
  - *Removed:* the header bar (a quiet masthead line instead), the permanent
    rail (the team is a dim caption), the 3px turn spine, the `who` hairline,
    the `you` fill, the working pill, the code card (a fill-only recess).
- **Risk / effort:** **low / low-medium.** No structural change to the data;
  it is CSS over the existing DOM plus dropping a few elements. The risk is
  taste: a colleague who reads the surface as a *dashboard* may find it too
  quiet, and the team loses its persistent home (it is a caption).
- **Refinement (2026-10-09): "paper":** [`vscode-paper-direction.md`](vscode-paper-direction.md)
  and its prototype [`vscode-paper-draft.html`](vscode-paper-draft.html) — A pushed
  toward a *printed page*: one measure governs the running head, the text and the writing
  line; a serif reading face for the prose (the furniture stays mono); a running head and
  a **folio** (the step count); an indent-led paragraph rhythm; and the tool apparatus as
  **footnote matter** rather than a fold. **A is the shipped spec; this refinement is a
  draft for a human to judge**, not authorized (W005 is the shipped direction).

---

## B · command — the keyboard is the interface

- **Prototype:** [`vscode-modern-command-draft.html`](vscode-modern-command-draft.html)
- **Stance (one line):** the panel is dated because it is mouse-first — every
  action is a button you hunt for; it is modern when the composer doubles as a
  command bar, every control has a key, and the hard menus go to VS Code's own
  QuickPick.
- **§0 surface line:** *Reading this as: a web screen (the VS Code webview), for
  a developer driving a multi-agent session inside VS Code, in VS Code's own
  theme-variable language, with the dominant constraint that the surface is
  driven from the keyboard and hands its hard menus to VS Code's QuickPick.*
- **Dials:** `VARIANCE 3` · `MOTION 1` · `DENSITY 7`.
  - `VARIANCE 3` — a command bar, a dense transcript and a palette layer are
    three different shapes, so variation is real.
  - `MOTION 1` — a keyboard flow gains nothing from motion; only the spinner
    and the live cursor remain.
  - `DENSITY 7` — a command surface packs rows; the shipped `[data-density]`
    switch is set to `dense`, not re-authored.
- **Levers spent:** **recomposition** (one section: the composer becomes a
  command bar, and the `/` menu is promoted into a real palette overlay), with
  typography/mono key chips as support. Colour is untouched; motion is reduced.
- **Components added / removed:**
  - *Added:* a command bar (the composer, with a `❯` prompt); a command
    **palette** overlay with per-row keys; a numbered member strip (retarget
    with `1..4`); a keymap legend.
  - *Removed:* the header's `▾` disclosure and the composer's mode `.seg` (both
    become palette commands with keys), the `@` button (the palette owns it),
    the always-on `Stop` (a key).
- **Risk / effort:** **low / medium.** It is one block recomposed plus one
  overlay. The honest caveat: the palette and QuickPick integration imply
  `vscode.commands`/keybinding contributions, i.e. a slice of host work the
  other directions do not need — but none of it is kernel work.

---

## C · conversation — the transcript is the hero

- **Prototype:** [`vscode-modern-conversation-draft.html`](vscode-modern-conversation-draft.html)
- **Stance (one line):** the panel is dated because three stacked chrome bands
  and a permanent rail compete with the thing you actually read; it is modern
  when the conversation gets the whole panel and every other control is
  transient.
- **§0 surface line:** *Reading this as: a web screen (the VS Code webview), for
  a developer reading a multi-agent session inside VS Code, in VS Code's own
  theme-variable language, with the dominant constraint that the transcript gets
  the whole panel and every other control is transient.*
- **Dials:** `VARIANCE 2` · `MOTION 2` · `DENSITY 5`.
  - `VARIANCE 2` — one column; the variation left is in the turn composition
    (which v3 got right and this keeps).
  - `MOTION 2` — the live cursor, the spinner, and two gated opacity fades (the
    top line dims until you are near it; a turn's actions appear on hover).
  - `DENSITY 5` — the transcript is airy; the chrome is dense only when it
    appears.
- **Levers spent:** **spacing** (the rail's fixed 216px column goes) then
  **recomposition** (a hover-revealed per-turn action row; the team as a popover).
- **Components added / removed:**
  - *Added:* a per-turn hover toolbar (Copy / Show diff / Steer); a team popover
    off a single header button; a working segment in the top line.
  - *Removed:* the permanent left rail, the working pill (folded into the thin
    top line), the composer's chips row (context chips move inline).
- **Risk / effort:** **medium / medium.** It re-opens the shipped v3 *decision 3*
  ("the team has ONE home: the rail") — the re-open is the point, but it is the
  direction's real cost: the team stops being glanceable when the popover is
  closed. The top line's dim-until-hover behaviour must not hide state from a
  keyboard user (the prototype reveals on `:focus-within` too).

---

## D · native — stop imitating the host

- **Prototype:** [`vscode-modern-native-draft.html`](vscode-modern-native-draft.html)
- **Stance (one line):** the panel is dated because the webview re-draws VS
  Code's own widgets — a rail that wants to be a TreeView, a header that wants to
  be a status bar; it is modern when it *stops imitating the host* and uses the
  real parts.
- **§0 surface line:** *Reading this as: a web screen (the VS Code workbench
  around a minimal webview), for a developer driving a multi-agent session inside
  VS Code, in VS Code's own native parts (TreeView, status bar, QuickPick,
  settings), with the dominant constraint that the surface uses the host's real
  components instead of re-drawing them.*
- **Dials:** `VARIANCE 3` · `MOTION 1` · `DENSITY 7`.
  - `VARIANCE 3` — the surface is now several host parts, not one panel.
  - `MOTION 1` — the host owns all motion; wcode adds none.
  - `DENSITY 7` — the host's own density conventions (22px list rows, uppercase
    section headers) are the floor, not a dial.
- **Levers spent:** **full block replacement** (§7.4's last lever) — the
  heaviest, and deliberately so. The team block, the header, and the pickers are
  replaced by host primitives, not restyled.
- **Components added / removed:**
  - *Added:* a TreeView (the team's real home), a `StatusBarItem`
    (`⠋ wcode`), QuickPick for `/model` and `/resume`, a `settings.json` panel.
  - *Removed (from the webview):* the rail, the header bar, the popovers, the
    in-panel model/policy display. The webview keeps the transcript and the
    composer and nothing else.
- **Risk / effort:** **high / large.** It re-opens the shipped v3 non-goal "not
  the workbench chrome," and it is the only direction that is an *extension*
  change (`TreeDataProvider`, `StatusBarItem`, contributed commands/settings),
  not a stylesheet change. It does not touch the kernel, and the frame protocol
  is unchanged — but the manual click-path in
  [`editors/vscode/README.md`](../../editors/vscode/README.md) changes materially.
  This is the honest endgame if the extension is to feel native.

---

## E · ambient — the team's state is always on screen

- **Prototype:** [`vscode-modern-ambient-draft.html`](vscode-modern-ambient-draft.html)
- **Stance (one line):** the panel is dated because you have to open a tab and
  read a transcript to learn what your agents are doing; it is modern when that
  state is *ambient* — the first thing on screen, glanceable in zero clicks, one
  row per active member with its current action, collapsing to a single quiet
  line when nothing runs.
- **§0 surface line:** *Reading this as: a web screen (the VS Code webview), for
  a developer supervising several agents inside VS Code, in VS Code's own
  theme-variable language, with the dominant constraint that the team's live
  state is the first thing on screen and costs no click to read.*
- **Dials:** `VARIANCE 3` · `MOTION 2` · `DENSITY 6`.
  - `VARIANCE 3` — a board plus a transcript are two shapes; within the board,
    a running row and an idle row differ by glyph and colour.
  - `MOTION 2` — the running spinner per active row, plus the live cursor; both
    gated. The board's activity is what the motion is *for*.
  - `DENSITY 6` — v3's reading, kept: the board is a glance, the transcript is
    the record.
- **Levers spent:** **one section's recomposition** — the top band (v3's header
  plus the working pill) becomes the activity board — with the accent re-policed
  to mean *running now* and nothing else.
- **Components added / removed:**
  - *Added:* an activity board (glyph · name · current action · elapsed per
    active member), a collapsed one-line summary, a manual collapse toggle.
  - *Removed:* the working pill (its aggregate is now the board), the header's
    `ctx` cell (moved to the summary/board foot), the `▾` disclosure (the board
    is the details).
- **Risk / effort:** **medium / medium.** All data (`memberGlyph`,
  `liveAction`, the roster) already reaches the webview, so there is **no wire
  change**; the work is the board's layout and the collapse state. The risk is
  vertical space at a docked sidebar width — mitigated by the collapse and
  proven in the prototype's narrow dock.

---

## F · flat — the surface is reduced

- **Prototype:** [`vscode-modern-flat-draft.html`](vscode-modern-flat-draft.html)
- **Stance (one line):** the panel is dated because it is decorated — hairlines,
  fills, a spine, a pill, a border on every second thing; it is modern when it is
  **reduced**: flat and borderless, generous uniform white space, a neutral
  surface, restrained type, and the one accent reserved.
- **§0 surface line:** *Reading this as: a web screen (the VS Code webview), for
  a developer driving a multi-agent session inside VS Code, in VS Code's own
  theme-variable language (only --vscode-* tokens for colour, plus the one accent),
  with the dominant constraint that the surface is reduced to flat, borderless
  grouping and white space, and that decoration carries no information.*
- **Grounding — the lineage and two named systems:**
  - **The lineage:** flat design ← Swiss style ← International Typographic Style
    ← Bauhaus. ("It is believed that it was the Swiss style that formed the basis
    of the modern computer design system, in particular Flat Design.") Modern-flat's
    DNA is a **modular grid**, **left / flush-left alignment**, sans-serif,
    asymmetry, and **white space**.
  - **Flat design:** minimalist simple elements + typography + **flat colour**,
    defined by contrast with skeuomorphism — no shadow, no gradient, no faux
    depth. Its documented failure (Nielsen) is lost affordance (a control reads
    like an indicator; flat UIs measured ~22% slower), so this direction keeps
    affordance wherever a control lives.
  - **Minimalism:** "reduce to its necessary elements"; emptiness as a *positive*
    (the Japanese *Ma* — empty / open space); neutral colours; **eliminate
    decoration**; "less is more" (Mies) / "less, but better" (Rams).
- **Dials:** `VARIANCE 2` · `MOTION 1` · `DENSITY 4`.
  - `VARIANCE 2` — uniform; there is no asymmetry for its own sake. It sits just
    above A (`VARIANCE 1`) because it keeps the operational column structure
    (team + transcript + composer) where A drops it.
  - `MOTION 1` — only the live cursor (`▌`) and the running spinner, both
    reduced-motion gated. Nothing else moves.
  - `DENSITY 4` — air is the medium, but the tool rows must stay legible at a
    docked sidebar width, so `4`, not `3`.
- **Not direction A.** A drops the *structure* (the rail, the pill) and is
  prose-first — "the words are the interface". **F keeps the operational
  structure** (status line, team, tools, composer) and **flattens its surface**:
  the same parts, no decoration, with flat recess fills doing the grouping that
  borders used to do.
- **No bold typography.** Hierarchy comes from **space, order and modest weight**
  (400 / 500 / 600 at most) — never heavy or display weight. Only the four locked
  sizes are used; no fifth size, no weight 800. Where hierarchy has to speak it
  wins by **isolation and air**.
- **Levers spent (§7.4 order):** **spacing** (a uniform `--wc-3` / `--wc-4` /
  `--wc-5` rhythm replacing every ad-hoc margin) → **one section's recomposition**
  (the bordered cards become flat recesses). Colour is untouched (the neutral
  surface); motion is reduced. No new token, and `--wc-radius` is reused unchanged
  at `6px`.
- **Flat / non-border, and the trade.** Every `--vscode-panel-border` hairline in
  the surface is replaced: the tool-output card, the `.code` card and the
  `.statecard` → a **flat recess fill** (`--vscode-textCodeBlock-background`) + a
  quiet label; the header's bottom rule and the composer's top rule → **pure white
  space**; `.menu` / `.cmdmenu` → a **flat recess**
  (`--vscode-textBlockQuote-background`) whose selected row keeps a **fill**; the
  turn spine and the `you` fill → deleted (the `.who` label and the gap do the
  work). Affordance is kept where a control lives: the composer field (a fill +
  the `❯` mark), `Send` (a flat fill), the selected rail / menu row (a fill).
  **The trade, one line:** *a border says "this is a region" and a fill says "this
  is a thing you can act on"; moving the boundary from the region to the control
  makes the surface quieter, but an affordance must then be bought back on every
  control or flat design's documented failure returns.*
- **Components added / removed:**
  - *Added:* a uniform white-space rhythm; flat recesses for grouping; a quiet
    recess label.
  - *Removed:* every `--vscode-panel-border` hairline in the surface; the turn
    spine; the `you` fill; the member swatches (identity is the name, state is the
    glyph); every shadow.
- **Risk / effort:** **low-medium / low-medium.** CSS-only over the existing DOM —
  no wire change, no extension change. The risk is named and bounded: flat
  design's affordance failure, mitigated per-control (fills, position, the `❯`
  mark). The quieter risk is that a neutral, undecorated surface can read as
  "unfinished" to a colleague who expects chrome.

---

## Comparison at a glance

| | stance | `VARIANCE` | `MOTION` | `DENSITY` | lever(s) spent | biggest change | risk / effort |
|---|---|---|---|---|---|---|---|
| **A** editorial | words are the interface | 1 | 1 | 3 | type → space | drops all furniture | low / low-med |
| **B** command | keyboard is the interface | 3 | 1 | 7 | recomposition | composer → command bar + palette | low / medium |
| **C** conversation | transcript is the hero | 2 | 2 | 5 | space → recomposition | the rail is gone | medium / medium |
| **D** native | use the host's parts | 3 | 1 | 7 | full replacement | webview shrinks to transcript | **high / large** |
| **E** ambient | team state is ambient | 3 | 2 | 6 | one section | a live activity board | medium / medium |
| **F** flat | flat, borderless, minimal | 2 | 1 | 4 | spacing → recesses | every border deleted; recesses group | low-med / low-med |

## Ranked recommendation

1. **E · ambient** — the best fit for what wcode uniquely *is*. The exploration
   doc's own finding is that "a first-class team" is the hero no competitor has;
   ambient is the only direction that makes that hero glanceable in zero clicks.
   It needs no wire change, it keeps v3's density read, and it is the largest
   "modern" delta per unit of risk. **Recommend this as the spine.**
2. **F · flat** — the strongest answer to the human's own ask (minimal, flat,
   non-border) and the safest: CSS-only, and it keeps the whole operational
   structure while removing the decoration. It is ranked just behind E because it
   is the *calmest* large change, not the boldest idea; laid onto E's spine it is
   a likely final look. If "modern" means *quiet and undecorated* to you, promote
   F to first.
3. **D · native** — the deepest modernisation and the most honest long-term
   shape. Every imitation of the host is a bug waiting to drift; the TreeView and
   the status bar are the right homes. It sits below F because it is the one
   direction that is an *extension* change, not a stylesheet change — adopt it if
   the team is willing to rewire `extension.ts`, otherwise keep it as the endgame.
4. **B · command** — cheap, modern, narrowly scoped. It improves the composer
   and the menus without disturbing the transcript. Best applied as a
   *component* on top of the spine (E) rather than as a whole surface.
5. **C · conversation** — genuinely good, but it re-opens the hardest shipped
   decision and trades the team's persistent home for the transcript's width. Do
   it only if the user's mental model is "one conversation at a time."
6. **A · editorial** — the calmest and the cheapest, but the smallest delta, and
   it runs against v3's own direction (which spent effort *adding* composition
   depth). It is a strong counter-proposal, not a likely winner.

**A combined recommendation, if the human wants one:** take **E** as the surface
spine (the ambient board above the transcript), lay **F** over it for the look
(the flat recesses, the white space, the deleted hairlines), fold **B** into the composer (the
command bar and the palette), and treat **D** as the follow-up that moves the
team to a TreeView and the status to a status-bar item once the shell is settled.

## Open questions for the human

1. **Which stance is "modern" to you** — quieter type (A), keyboard-first (B),
   conversation-first (C), host-native parts (D), ambient status (E), or
   flat / minimal (F)? They are not mutually exclusive, but which one should lead?
2. **Is the self-contained webview a constraint or a default?** The native
   direction (D) lifts it; the other four keep it. This is the single decision
   with the largest effort delta.
3. **How always-on should the team be** — a permanent rail (v3 today), a board
   (E), a hover toolbar plus a popover (C), or a TreeView (D)? v3 already spent
   this argument once (decision 3); re-opening it should be deliberate.
4. **Motion budget:** every direction here is at `MOTION 1` or `2`. If any
   direction should be allowed a third animation, which, and justified in one
   sentence (design-taste §7.4)?

## How these were checked

- Every prototype parses (Python `html.parser`, no unclosed or stray tags) and
  every inline script passes `node --check`.
- Rendered strings contain **zero** em-dashes; markup glyphs are drawn only from
  the locked vocabulary.
- CSS colour outside the token blocks is confined to the **gallery chrome** (the
  `.draft` block, the driving toolbar, the frame border, the page background) —
  exactly the shipped drafts' convention. The surface itself uses only
  `--vscode-*` and `--wc-*`.
- Every file carries its `prefers-reduced-motion` gate.
- The **flat** prototype passes the same audit plus two extras: **no `box-shadow`
  and no gradient anywhere in the surface** (flat design draws no faux depth), and
  it reuses `--wc-radius` at the shipped `6px` (no scale amendment).
- *Not checked:* no browser was run here, so the panels were not painted — the
  proof is the parse, the token audit and the `node --check` above, not a
  screenshot.
