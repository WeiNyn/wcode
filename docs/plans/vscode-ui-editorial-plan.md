# VS Code surface — editorial (direction A) plan

- **Status:** authorised — the shipped v3 spec is amended (see below); E1–E7 not yet built
- **Work item:** [W005](../work/W005-editorial-vscode-surface.md)
- **Decision:** [D006](../decisions/D006-editorial-supersedes-v3-decisions.md)
- **Tracker:** [next-steps.md](../next-steps.md) item 62
- **Date:** 2026-10-09

**The spec:** `docs/design/vscode-modern-editorial-draft.html` IS the visual spec.
The direction is `A · editorial` (`docs/design/vscode-modern-directions.md:86wCa`) —
*"the panel is dated because it is dressed like a widget — a rail, a spine, hairlines,
a fill on your own words, a pill — wrapped around prose; it is modern when the words
**are** the interface: one quiet column, a byline, and air."* Dials
`VARIANCE 1 · MOTION 1 · DENSITY 3` — the calmest, lowest-density direction.

## 0. Why the spec is amended first (E0)

The redesign is a `design-taste` §7.1 change, and §7.3 (`:cgHk2`, "never change a locked
invariant silently") requires the shipped spec to say so **before** any implementation.
v3's plan is that spec and its prototype "IS the spec" (`docs/vscode-ui-v3-plan.md:CRpGr`).
So this change is **docs-only**: it records the supersessions and authorises the
redesign; E1–E7 then build it. Nothing under `docs/` moves (D004 item 58).

## 1. The plan (E0–E7, independently shippable)

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| **E0** | Amend the spec: these supersession notes + this plan + the tracker row + D006 | high | low | low | the redesign is unauthorised until the shipped spec says so (`design-taste:cgHk2`); docs-only, reversible |
| **E1** | Masthead + team caption (delete the rail) | high | medium | medium | the biggest structural DOM change; the team loses its persistent home and becomes a caption |
| **E2** | The reading column (byline; drop the spine, the `you` fill, the `who` hairline) | high | medium | low | pure composition; removes the V14 clip hazard |
| **E3** | Tool rows + code block (drop the output card + `Copy`; fill-only code) | medium | medium | medium | reverses V12's "the output is a card" |
| **E4** | The live type-line (drop the working pill) | medium | low | low | `workingGroup` (`view.ts:KeUWa`) already returns the count; the click-to-reveal goes |
| **E5** | Composer restyle (`❯` prefix, hairline underline, text-link controls) | medium | medium | low | the mode control becomes a text toggle |
| **E6** | Dead-code sweep (rail/pill/swatch CSS + orphan fns) | low | low | low | keeps the sheet and `check-css.mjs` honest |
| **E7** | Visual proof (F5 host + the spec file) | high | low | medium | the webview was never run; A's value is a painted surface |

Deliverables, integration points and per-step gates are in
[W005](../work/W005-editorial-vscode-surface.md) §5–§7. The wire/kernel is untouched:
every fact A renders already reaches the webview (`ToWebview`/`FromWebview` unchanged).
The one behaviour A drops — the pill's click-to-reveal — was client-local (`data-live`).

## 2. Supersession list (explicit, with anchors)

`docs/vscode-ui-v3-plan.md` keeps its own text; each affected item now carries an
"Amended by editorial (W005)" note (matching its existing "Amends, explicitly" block at
`:gBoLf`). The list:

| v3 item | anchor | editorial's verdict |
|---|---|---|
| **Decision 3** "the team has ONE home (the rail)" + two echoes | `:8HCPY` | **Superseded** — the rail and the working pill both go; the team is a dim caption, the aggregate a non-interactive live type-line |
| **Decision 2** "the All/Focus control lives in the header" | `:M3ETf` | **Kept-with-restyle** — the control stays in the masthead but becomes a quiet text toggle, not a bordered `.seg`; the wire `mode` and the retarget semantics are unchanged |
| **Decision 5** L1 / L2a / L2b | `:Jkqsl` | **Amended** — L2a narrows: the code block and the tool output become fill-only recesses; only the change-review card stays L2a |
| **Decision 12** "the V6 meter is the text `ctx 42k`" | `:CSVRU` | **Kept** (editorial agrees). What is **dropped** is the **V11b** gauge (`:Sw11b`), not this decision |
| **§1.4 / V9** the composer mode as a `.seg` | `:ywwQg` (phase `:XAlvl`) | **Superseded** — the composer mode control becomes a text toggle; the `Model:` removal, the `Stop` gate and the hint removal stand |
| **V12** tool output L2b → L2a (`⚙ {tool} · output` + `Copy`) | `:zTBVB` | **Superseded** — the output reverts to a fill-only recess; the head and `Copy` go; identity moves to the dim summary line |
| **V13** the tool row's L1 rule carries the status | `:nYqsK` | **Narrowed** — the rule is dropped (the tool is a dim sentence, `docs/design/vscode-modern-editorial-draft.html:yQw0I`); the status lives on the `⚙` mark (neutral complete / accent running / red error) |
| **V13r** "complete is GREEN" | `:cXMdP` | **Superseded** — complete is neutral (`--vscode-foreground`); running stays accent, error red |
| **V14** turn head / spine / alignment / rhythm | `:Sc8DY` | **Superseded** — no spine, no head hairline, no `you` fill, no live accent wash; the `.new` beat survives at `--wc-5`, not `--wc-4` |
| **the six `charts-*` swatches** | `:WJJlL` | **Dropped** — every home (rail edge, turn rail, `.who` avatar, collapsed strip) is removed; the caption glyph is coloured by state, not identity |

**Decisions 1, 4, 6–11 and V6–V11a are unaffected** and stay the shipped spec.

## 3. Non-goals

- The `/`-command menu, `@`-mention mechanics and the member-verbs menu — A is a reading
  surface; its prototype does not render them. A flat restyle is a separate change.
- The docked/editor host wiring (`webviewView.ts`, `panel.ts`, `surface.ts`) — no host change.
- The wire / kernel — A needs no new wire data.
- The in-panel change review (`.review`/`.hunk`) — it stays a card (an interactive, foreign
  object), not restyled.
- The Tasks (root plan) section — A shows no Tasks; dropping it is a deliberate loss
  (flagged in [W005](../work/W005-editorial-vscode-surface.md) §4.6).
- Zero remote assets, `--vscode-*` for colour only, one accent, no `—` in any rendered string.

## 4. Refined by paper (W008)

The shipped A surface was refined by the **paper** direction
(`docs/design/vscode-paper-draft.html`; stance `docs/design/vscode-paper-direction.md`),
recorded in [W008](../work/W008-paper-vscode-surface.md) and authorised by the
[refined by paper (W008)](../decisions/D006-editorial-supersedes-v3-decisions.md) addendum.
It is a *reading* refinement of A, not a new direction.

**Adopted (A1–A7).** **A1** keep the local serif for prose (the furniture stays mono);
**A2** tool output moves to **turn-foot footnotes** (a printed mark at the tool block's
boundary; the output behind a disclosure *inside* the note, no in-flow fold); **A3** the
**step count** is the folio (the page number); **A4** the running head is **static**;
**A5** keep the small-caps chapter open, add an **indent-led** paragraph rhythm; **A6** the
measure is **64ch**; **A7** **no** margin column (footnotes only, no sidenotes).

**Added vs A.** `FolioCell` + a pure `folio(blocks)` + a pure `turnNotes(blocks)`
(`src/webview/view.ts`); the `.masthead .inner` measure wrapper + a `#folio` placeholder
(`chat.ts`); the footnote builders (`fnmark`/`renderFootnotes`/`fnNote`, `chat.ts`);
`--ed-read`/`--ed-indent` and `.folio` / `.footnotes` / `.fnmark` / `.fn-head` /
`details.fn-out` plus the indent + chapter-open rules (`media/chat.css`).

**Changed vs A.** the composer's rule moves onto `.composer .inner` (both the head's and
the writing line's rules run to the column, not the viewport); `.masthead .inner` padding is
`--wc-4`; `.body` is the serif at `line-height: 1.75` with the indent rhythm; `.code .chead`
is a small-caps caption; `.liveline` is capped to the measure; `renderTurn` no longer paints
an in-flow tool fold.

**Removed vs A.** the in-flow `details.fold.tool` row (and its `renderToolFold` builder);
the inter-paragraph margin in `.body` (the first-line indent replaces it).

Nothing under `docs/` moves (D004 item 58); the wire/kernel is untouched (`src/webview.ts`).

## 5. References

- `docs/design/vscode-modern-editorial-draft.html` — the A spec (the WHAT).
- `docs/design/vscode-modern-directions.md:86wCa` — §A, the stance and the dials.
- `docs/vscode-ui-v3-plan.md` — the shipped spec this plan amends.
- [W005](../work/W005-editorial-vscode-surface.md) — the brief (scope, estimates, gates).
- [D006](../decisions/D006-editorial-supersedes-v3-decisions.md) — the decision record.
- `.wcode/skills/design-taste/SKILL.md` §7.1/§7.3 (audit + amend-first).
