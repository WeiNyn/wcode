# W008 — VS Code surface, "paper" refinement of direction A (editorial) — brief

- **Status:** draft, for a human/orchestrator to record
- **Work item:** W008
- **Refines / supersedes nothing wholesale:** `A · editorial` (W005) — this is a *refinement* of the shipped spec, not a new direction
- **Decision:** [D006](../decisions/D006-editorial-supersedes-v3-decisions.md) (to be given a "refined by paper (W008)" addendum)
- **Plan being refined:** [docs/plans/vscode-ui-editorial-plan.md](../plans/vscode-ui-editorial-plan.md)
- **Tracker:** row **65** (`docs/next-steps.md`)
- **Author:** session `agent:brainstormer` (recorded by the orchestrator)
- **Date:** 2026-10-09

---

## 1. The ask

The human reviewed the designer's **paper** draft (`docs/design/vscode-paper-draft.html`, stance in `docs/design/vscode-paper-direction.md`) and **approved it for implementation**, adopting seven settled decisions (A1–A7, below). Implement the paper refinement of the shipped editorial VS Code webview: the panel should read as **one printed page** — a single measure governing the running head, the text block and the writing line; the prose set in a **local serif reading face** while the furniture stays mono; **indent-led** paragraph rhythm with a small-caps chapter open per speaker turn; a **running head + a folio** (the step count) as a page number; and the tool apparatus moved from an in-flow fold **into turn-foot footnotes** (a printed mark in the sentence; the output behind a disclosure inside the note). The refinement is a `design-taste` §7.1 change, so per §7.3 (`:cgHk2`) it **amends the shipped spec first** (W005's plan + D006 + a tracker row) before any code. Dials `VARIANCE 1 · MOTION 1 · DENSITY 3` — unchanged from A.

**Adopted, not to be re-opened:**

| # | decision | settlement |
|---|----------|------------|
| A1 | reading face | keep the **local serif for prose**; furniture stays mono |
| A2 | footnotes | tool output moves to **turn-foot footnotes** (mark in the sentence; output behind a disclosure *inside* the note) |
| A3 | folio | the **step count** (the `tool`-block count) is the page number |
| A4 | running head | **static** (the session identity), not scroll-tracking |
| A5 | chapter open | keep the **small-caps first line per speaker turn** |
| A6 | measure | **64ch** |
| A7 | margin column | **no** — footnotes only, no Tufte sidenotes |

---

## 2. Scope / Non-scope

### 2.1 In scope

- **One measure governs the sheet** (`vscode-paper-draft.html` §1 `:xSpPp`, `:pIzFv`, `:gWgZ7`, `:fmDSK`). The head, the text and the writing line share `--ed-measure`; each rule sits on the *inner* element. In the shipped code the text already shares it (`media/chat.css:Xhadd` `.col`, `:CMe7u` `.composer .inner`); the **head does not** (`media/chat.css:KXIip` `.masthead` is full-bleed with the border) and the **liveline does not** (`:ZLzzY`).
- **The measure is 64ch** (`media/chat.css:cFrwd` currently `66ch`).
- **The prose is set, the furniture is not** (A1). Add a local serif knob `--ed-read` and set `.body` in it; the running head, bylines, footnotes, live line and composer controls keep mono (`vscode-paper-draft.html:5JdBP` `.body`, `:MZHzL` `--ed-read`).
- **Indent-led paragraph rhythm** (A5 + §4). `p + p` takes a first-line indent (`:x6Guo`, knob `:fSqUT`); the first paragraph of a turn is un-indented (`:hTdT4`); a non-`p` sibling resets it (`:WFkhQ`, `:1zmMv`); the first line of a speaker turn is small-caps (`:Dg6We`).
- **A running head and a folio** (A3, A4). The head stays static; a **folio** cell (`.folio`, `:MzQWx`) at the row's outer edge shows the step count, tabular (`font-variant-numeric:tabular-nums`).
- **Footnotes, not a fold** (A2). A tool call leaves a printed reference (`<button class="fnmark">`, `:RDs6C`) in the sentence; the apparatus is a `.footnotes` `<ol>` at the turn's foot (`:tH0Eu`) under a **short** rule (44% width), one line per step, with the output behind a native `details.fn-out` (`:GODr0`).
- **A code block is a plate** (§6). A small-caps caption line (`:xp5gr`, reusing the shipped `.code`/`.chead` markup) above the fill-only recess.
- **The record (E0, mandatory-first):** a refinement note in `docs/plans/vscode-ui-editorial-plan.md` + a D006 addendum + tracker row 65.

### 2.2 Non-scope (the boundary)

- **The type-scale / serif question.** A font *family* is **not** a type-scale change. The locked scale is the four SIZE steps (`--wc-fs-meta/ui/body/code`) and the `--wc-1…5` grid; paper **adds no size** and the prose keeps `--wc-fs-body` (`docs/design/vscode-paper-direction.md` §"type-scale question"). `--ed-read` is a *derived local knob*, exactly like A's own `--ed-measure` (`media/chat.css:cFrwd`). **No `--wc-fs-*` step is added.** The prototype's **Face toggle is gallery chrome, not shipped** (see §4.3) — the A/B is a *review* instrument, not a surface feature.
- **The footnote trade.** Only **turn-scoped** footnotes are in scope. **Session-scoped** footnotes are out (the direction doc lists this as "not confirmed"; the shipped `renderTurn` (`chat.ts:KoM4E`) already loops a *turn's* blocks, so turn-scope is the natural and adopted reading). A **margin/sidenote column** is **refused outright** (A7: it breaks the single-measure stance and dies at sidebar width).
- **The in-panel change review** (`.review`/`.hunk`, `chat.ts:YVJx9` `reviewBlock`) **stays a card** — unchanged. A diff-bearing tool's *footnote body* renders the same `.review` card inside its note; the card itself is not restyled (W005 §2 kept it, `docs/plans/vscode-ui-editorial-plan.md:J1uHo`).
- **The `/`-command menu, `@`-mention mechanics, member-verbs menu** — a reading surface; keep as shipped (`docs/plans/vscode-ui-editorial-plan.md:YBhZK`).
- **The docked/editor host wiring** (`webviewView.ts`, `panel.ts`, `surface.ts`) — no host change.
- **The wire / kernel** — every fact the paper renders already reaches the webview; `ToWebview`/`FromWebview` (`src/webview.ts`) are **untouched**. This is a **DOM + CSS** change only.
- **A webfont serif, a "paper white" fill, a forced light theme, a section rule per speaker, a drop cap, a decorative folio, serif code/footnotes** — all **deliberately refused** (`docs/design/vscode-paper-direction.md` §"Deliberately refused"). The serif stack names locally-installed families with a generic fallback — **zero requests**.
- **The trailing-orphan sweep of unrelated dead code** — only the classes/functions *this* change orphans.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why (one line) |
|---|------------|-------|------------|------|----------------|
| **E0** | Record the adoption: refinement note in the plan + D006 addendum + tracker 65 | high | low | low | The refinement is unauthorised until the shipped spec says so (`design-taste:cgHk2`); docs-only, reversible. |
| **P1** | One measure + indent rhythm + chapter open (pure CSS, plus a head `.inner` wrapper) | high | low | low | This *is* the stance's spine (the sheet, not three bands); CSS only, no wire, no new data. |
| **P2** | The reading face (serif) + the code caption (pure CSS) | medium | low | low-medium | The one judgment call — a family, not a size; A/B-able and reversible; the caption is cosmetic. |
| **P3** | The folio: a pure count + a header cell + `renderMasthead` painting | medium | low | low | A real reading position; one pure fn and one new cell in `panelHeader`. |
| **P4** | Footnote matter: the `renderTurn` recomposition + the `.fnmark` button | high | medium | medium | The tool leaves the flow — the biggest DOM change; the shipped fold override must move to the note's `details` or the note collapses on every stream tick. |
| **P5** | The sweep: stale comments/rules + orphaned functions | low | low | low | Keeps the sheet and `check-css.mjs` honest; a missed orphan fails nothing but leaves drift. |
| **P6** | Visual proof (F5 host + the spec file) | high | low | medium | The webview was never run (`editors/vscode/README.md`); the paper's whole value is a painted page. |

---

## 4. Interface & structure

### 4.1 The folio — the seam the direction doc names

**Added (pure).** A `FolioCell` in `editors/vscode/src/webview/view.ts` beside the existing cell union `HeaderCell` (`view.ts:nvspD`, `export type HeaderCell = IdentCell | SpacerCell | MeterCell | ModeCell`) and a pure count:

```
export interface FolioCell { kind: "folio"; steps: number; }
```

The count is `RenderedState.blocks`' number of `tool` blocks — `RenderedState.blocks` is `render.ts:59Wvn` (`blocks: RenderedBlock[]`), and `RenderedBlock.kind` is `render.ts:qFotY`. `state.blocks` is always the *routed target's* transcript (`render.ts:MxEmW` `renderState` → `transcriptOf(state, target)`), so the folio is the current transcript's step count in both All and Focus.

**Changed (pure).** `view.ts:panelHeader` (`s7HzW`) appends the folio cell last, after the mode cell (`view.ts:2sD0V` `{ kind: "mode", mode }`):

```
{ kind: "meter", word: …, text },
{ kind: "mode", mode },
{ kind: "folio", steps },
```

**Changed (DOM).** `chat.ts:renderMasthead` (`T33oI`) gains a `cell.kind === "folio"` branch appending a `.folio` node after `.status`. The header-signature guard `chat.ts:mastheadSignature` (`L284G`) stringifies the cells, so the folio automatically re-renders when the count changes — no extra key. The skeleton `chat.ts:Bnfe4` (`app.innerHTML = [`) needs a `#folio` element (or is painted into `.row1` directly). CSS `.folio` (`vscode-paper-draft.html:MzQWx`) sits beside `media/chat.css:KXIip`.

### 4.2 Footnote matter — the `renderTurn` seam

**Changed (DOM).** `chat.ts:renderTurn` (`KoM4E`) — the loop `for (const block of turn.blocks)` (currently `wrap.appendChild(block.kind === "tool" ? renderToolFold(block) : blockShell(block))`, `chat.ts:2UCYq`) is recomposed:
- a `tool` block **emits a `.fnmark` `<button>`** (a printed reference) instead of an in-flow `renderToolFold`;
- the `tool` blocks are **collected**, not painted in flow;
- after the turn's prose, a **`.footnotes` `<ol>`** is appended (`vscode-paper-draft.html:tH0Eu`), one `<li id="fn-N">` per collected tool.

**Reused by the note.** The note head reuses `toolMeta` (`chat.ts:alUZD`) and `toolStatus` (`view.ts:Ltjt1`); the note body reuses `rawOutput` (`chat.ts:aYpYa`, the fill-only recess) **or** `reviewBlock` (`chat.ts:YVJx9`, the change-review card for a diff-bearing tool — non-scope, unchanged).

**The `.fnmark` key (the required interface detail).** It is a `<button class="fnmark" type="button">` (`vscode-paper-draft.html:RDs6C`), so **Enter/Space are free** — no new keymap entry. It carries `aria-controls` → the note `<li>`'s `details.fn-out` and `aria-expanded` mirroring that `details`' `open` (the prototype's `bindNote` / `sync` at `vscode-paper-draft.html:xKiP6`). Its colour is the **link** token, never `--wc-accent` (it is not "active now", `docs/design/vscode-paper-direction.md` §invariants).

**The re-render hazard — settle it in P4.** `render` (`chat.ts:jGu7k`) does `transcriptEl.textContent = ""` and rebuilds on every snapshot, so a **native** `details.fn-out` open state is lost on every stream tick. The shipped code already solved this for the in-flow fold with the override map `foldOverrides` + `foldOpen` (`view.ts:Kh2gM`) + `toggleFold` (`view.ts:jyvqY`). **Recommendation: retarget the existing override map to the note's `details.fn-out`** (keyed by `tool.callId`, set `open` from `foldOpen(...)` in `renderTurn`, flip via `toggleFold` on the mark's click and re-render) — i.e. "the existing fold override/`details` behaviour moves with it" (`docs/design/vscode-paper-direction.md` §seam). `view.ts:classNames` (`1j0Q9`, the `tool` case `fold tool…`) loses its only tool caller and the in-flow `details.fold.tool` rules (`media/chat.css:tKorz`) retire.

**Testability seam.** `renderTurn` is impure (DOM). To keep the footnote *content* testable the way the codebase tests the pure half, factor the **numbering + collection** into a pure helper in `view.ts` (e.g. `turnNotes(blocks)` → `Array<{ n; name; target; outputText; done; isError; diff?; hasDiff; durationMs? }>`), and let `renderTurn` assemble the DOM from it. (A landing decision for the layer-1 review; flagged in §9.)

### 4.3 Measure / face / rhythm — pure CSS (no DOM beyond one wrapper)

**Changed (`media/chat.css`).**
- `:root` (`FGqrn`) — the two derived knobs `--ed-read` and `--ed-indent` (`vscode-paper-draft.html:MZHzL`, `:fSqUT`); `--ed-measure` (`cFrwd`) 66ch → **64ch**.
- `.masthead` (`KXIip`) — a new `.inner` wrapper carries `max-width:var(--ed-measure)` and the `border-bottom` (the page's top edge runs to the column, not the viewport, `vscode-paper-draft.html:IhZBU`/`:pIzFv`); the outer `.masthead` keeps only padding. (`chat.ts:Bnfe4` gains the `.inner` wrapper.) The `.liveline` (`ZLzzY`) is capped to the measure too.
- `.body` (`3ncbW`) — `font-family: var(--ed-read)`, `line-height: 1.75`, `text-wrap: pretty`, `orphans/widows: 2` (`vscode-paper-draft.html:5JdBP`, `:shUFh`); the indent rhythm rules (`:x6Guo`, `:WFkhQ`, `:1zmMv`, `:hTdT4`) and the small-caps chapter open `.turn.new .body > p:first-child::first-line` (`:Dg6We`).
- `.code .chead` (`L2mCc`) — a small-caps caption (`vscode-paper-draft.html:xp5gr`); `.code` (`yEea1`) keeps its fill-only recess.
- New: `.folio`, `.footnotes`, `.fnmark`, `.fn-head`, `details.fn-out` (`vscode-paper-draft.html:MzQWx`, `:tH0Eu`, `:RDs6C`, `:J9uBw`, `:GODr0`). The single motion gate (`media/chat.css:9OH1B`) already names the two animations; the note adds none.

**Key call to settle (flag):** keep the shipped class name `.masthead` (the paper's `.runninghead` is the same node) to avoid gratuitous rename churn across CSS + tests — the structural change (the `.inner` measure wrapper) is what matters, not the class string. Flagged in §9.

### 4.4 Added / changed — at a glance

**Added:** `FolioCell` + the pure count + a pure `turnNotes(...)` (`view.ts`); a `renderFootnotes`/`fnNote`/`fnmark` builder (`chat.ts`); `.folio`/`.footnotes`/`.fnmark`/`.fn-head`/`details.fn-out` + `--ed-read`/`--ed-indent` + the indent/chapter rules (CSS); the E0 docs (plan note, D006 addendum, tracker 65).

**Changed:** `chat.ts:renderTurn` (`KoM4E`), `chat.ts:renderMasthead` (`T33oI`), `chat.ts:mastheadSignature` (`L284G`), the skeleton (`Bnfe4`); `view.ts:panelHeader` (`s7HzW`) + `HeaderCell` (`nvspD`); CSS `:root` (`FGqrn`), `.masthead` (`KXIip`), `.body` (`3ncbW`), `.liveline` (`ZLzzY`), `.code .chead` (`L2mCc`), `details.fold` (`tKorz`).

**Unchanged:** `RenderedTool` (`render.ts:pTODe`), `RenderedState` (`render.ts:59Wvn`), `renderContent` (`render.ts:YPLHf`), `classNames` (`view.ts:1j0Q9`), `turns` (`view.ts:SDBOT`), `who` (`view.ts:nsUTe`), `teamCaption` (`view.ts:uWZi8`), `composerControls` (`view.ts:9Z8XG`), the wire union, the host, the kernel.

---

## 5. The plan

1. **E0 — record the adoption.** Deliverable: a "Refined by paper (W008)" note in `docs/plans/vscode-ui-editorial-plan.md` (the added/removed components vs A + the A1–A7 settlements), a "Refined by paper (W008)" addendum to `docs/decisions/D006-editorial-supersedes-v3-decisions.md`, and tracker row **65** in `docs/next-steps.md`. Gate: layer-1 review APPROVE; every added/removed item is anchored to the paper spec; **no code touched** (`git show --stat` touches only `docs/`).

2. **P1 — one measure + the indent rhythm.** Deliverable: `--ed-measure: 64ch`; a `.masthead .inner` wrapper with the measure + the head's border; `.liveline` capped to the measure; the `p + p` indent / first-para reset / non-`p` reset rules; the small-caps chapter open. Gate: `npm run typecheck && npm test` green (`check-css: ok`); a new CSS assertion (the measure is shared by `.col`, `.composer .inner`, `.masthead .inner`); F5 shows the two rules running to the column.

3. **P2 — the reading face + the code caption.** Deliverable: `--ed-read` + `--ed-indent` knobs; `.body` serif at 1.75 with `text-wrap: pretty` + `orphans/widows`; `.code .chead` as a small-caps caption. Gate: CSS assertion tests (serif stack has **no** `@import`/`<link>`/`http`; measure/face present); F5 A/B by editing the knob.

4. **P3 — the folio.** Deliverable: `FolioCell` + the pure count in `view.ts`; the cell pushed by `panelHeader`; the branch in `renderMasthead`; the `.folio` CSS. Gate: a `view.test.ts` case asserting `panelHeader(...).cells` contains a `folio` cell whose `steps` equals the `tool`-block count (fails on the raw block count); F5 shows the tabular step count at the edge.

5. **P4 — footnote matter.** Deliverable: a pure `turnNotes(blocks)` descriptor; `renderTurn` emits `.fnmark` buttons + a `.footnotes` `<ol>`; the note head reuses `toolMeta`, the body `rawOutput`/`reviewBlock`; the note's `details.fn-out` `open` driven by the **retargeted** override map (`foldOpen`/`toggleFold`) so it survives a stream re-render; no new keymap. Gate: a pure test on `turnNotes` (one note per `tool` block, numbered in order, target/summary carried); `view.test.ts` `foldOpen`/`toggleFold` still green (same semantics, new host); F5 — the running tool's mark sits in the sentence and its note expands, and the open state survives a live tick.

6. **P5 — the sweep.** Deliverable: the retired in-flow tool-fold rules and any now-orphan helper comments removed; if a class was renamed, every reference updated. Gate: `node scripts/check-css.mjs` prints `check-css: ok`; `grep -rn` for each removed class/function returns nothing under `src/` + `media/`.

7. **P6 — visual proof.** Deliverable: the F5 screenshot + the paper spec opened in a browser. Gate: §7.2.

---

## 6. Quality gates

Run **in `editors/vscode`** (cargo is *not* this change's gate — the webview is not a Rust surface):

```sh
cd editors/vscode
npm run typecheck && npm test && npm run build
```
- `npm run typecheck` (`package.json:BGQPX` = `tsc --noEmit && tsc --noEmit -p tsconfig.webview.json`) → **no output, exit 0**.
- `npm test` (`package.json:0CAl6` = `node scripts/check-css.mjs && node --test test/*.test.ts`) → `check-css: ok (chat.css)` then `# pass <n>` / `# fail 0`.
- `npm run build` (`package.json:l2TGu` = `node esbuild.mjs`) → `out/extension.js` and `media/chat.js` written.

**Untouched guard (no Rust is touched):**
```sh
cargo test --workspace
cargo clippy --workspace --all-targets
```
Both must stay clean — a guard, not a gate.

**Regression tests kept green (by name):** `parseFromWebview accepts the well-formed messages`; `memberGlyph maps each MemberState to the TUI glyph + class`; `a fenced code block renders as a titled .code card` (`test/render.test.ts:zPAMY` area); `emptySpec names copy for all six states`; the `workingGroup …` cases; the `composerControls …` cases; `foldOpen`/`toggleFold` (reused, same semantics); `turns: …` grouping; `webview/view.ts imports no host and no I/O` (`test/purity.test.ts:OlvBf`); `parseFromWebview rejects junk`; `renderMerged …`.

---

## 7. Testing

### 7.1 Automated

- **New — the folio (pure, `view.test.ts`).** `panelHeader` returns a `folio` cell whose `steps` equals the number of `tool` blocks; asserts **behaviour** (a 3-tool transcript → `steps === 3`), and fails if it counts all blocks instead.
- **New — the footnote descriptors (pure, `view.test.ts`).** `turnNotes(blocks)` returns one descriptor per `tool` block, **numbered in block order**, carrying `name`/`target`/`outputText`/`done`/`isError`; asserts the collection removes `tool` blocks from the flow and prints a mark for each.
- **New — CSS assertions (regex over `media/chat.css`, the pattern W005 uses).** (a) `--ed-measure` is `64ch` and shared by `.col`, `.composer .inner`, `.masthead .inner`; (b) `.body` names `var(--ed-read)` and `line-height: 1.75`; (c) `.body > p + p` has a `text-indent`, `.body > p:first-child` resets it, `.turn.new .body > p:first-child::first-line` sets small caps; (d) `.footnotes` exists with `width: 44%` and a `border-top`; (e) the serif stack contains **no** `@import`/`http`/`url(` (zero requests); (f) `.fnmark` is styled with `--vscode-textLink-foreground`, **not** `--wc-accent`.
- **Changed — `foldOpen`/`toggleFold`.** Re-used as-is (the note's `details` is now their host); keep them green, add a case that the override **expires** at the running→done transition (already `test/view.test.ts:326`).
- **Kept green:** the W005 §7.1 "stay green" set above; `purity.test.ts` (the change adds no host/I-O import to `view.ts`).

### 7.2 How a human verifies it

```sh
cd editors/vscode
npm install
npm run typecheck && npm test && npm run build
# (spec) open the paper draft standalone:
open ../../docs/design/vscode-paper-draft.html          # macOS
# (real surface) open the repo root in VS Code, press F5,
# then Cmd+Shift+P -> "wcode: Start Session"
```

Look for: **one** measure governing the head, the text and the writing line, with the head's and composer's rules running to the column, not the viewport; the prose in a **serif** while the bylines, the head, the notes and the composer stay **mono**; within one speaker, paragraphs set solid with a **first-line indent**, and the first line of each speaker turn in **small caps**; a **folio** (step count, tabular) at the head's outer edge; a tool call leaving a small **superscript mark** in the sentence, with its note (a `⚙ name · target · status` line) at the turn's foot under a **short** 44%-width rule, whose output expands behind a disclosure **without** pushing the prose around; the measure at `64ch`. If no endpoint is configured, "Start Session" ends `✗ crashed` — the failure state, not a silent view. Compare each against the paper draft; the draft's **Face/Notes toggles are gallery chrome**, not shipped.

---

## 8. Expected outcome

- `media/chat.css` declares `--ed-read` + `--ed-indent`, sets `--ed-measure: 64ch`, sets `.body` in the serif at 1.75, and carries `.folio` / `.footnotes` / `.fnmark` / `.fn-head` / `details.fn-out`; `node scripts/check-css.mjs` prints `check-css: ok`.
- `src/webview/view.ts` exports a `FolioCell` (in the `HeaderCell` union), a pure step count, and a pure `turnNotes(...)`; `purity.test.ts` still passes.
- `src/webview/chat.ts:renderTurn` no longer paints an in-flow `details.fold.tool`; it emits `.fnmark` buttons and a `.footnotes` `<ol>`, with the note's open state surviving a stream re-render.
- `renderMasthead` paints a `.folio` cell; the head's rule sits on the measured inner element.
- `docs/plans/vscode-ui-editorial-plan.md` carries a "Refined by paper (W008)" note; `D006` carries its addendum; `docs/next-steps.md` has row **65** pointing at W008.
- `npm run typecheck`, `npm test`, `npm run build` are clean; `cargo test --workspace` / `cargo clippy --workspace --all-targets` remain clean (untouched).

---

## 9. Log

### 2026-10-09 — the paper refinement landed (E0–P5); layer-2 APPROVED

- **Implemented** in six commits: E0 `90211dd` (the record); P1 `348600f` (one measure + the indent rhythm); P2 `47621b1` (the reading face + the code caption); P3 `ca68d99` (the folio); P4 `e8b7dc6` (footnote matter); P5 `5f6343c` (the sweep).
- **Gates:** `npm run typecheck` → 0; `npm test` → **206 pass / 0 fail** (run serially — the full parallel glob flakes on `test/live.test.ts`); `npm run build` → ok; **zero `SKETCH` markers**; the wire/kernel untouched.
- **Layer-2 review APPROVED** (no blockers). Two notes folded:
  1. **The writing line takes the reading face** — `textarea { font-family: var(--ed-read); font-size: var(--wc-fs-body); }`, so the sentence you write looks like the sentence you will read (the `.linkbtn` controls stay mono). `10d6468`.
  2. **The dead `.tsum` is swept** — its only producer (`renderToolFold`) is gone, so the `.tsum` and `details.fold.thinking .tsum` rules go and the two `toolTarget` test names stop naming it. No `.tsum` reference remains under `src/`, `media/` or `test/`. `5808bfa`.
- **P6 (the F5 visual proof) is the only open step** — a human action; no browser / VS Code host runs here.

### 2026-10-09 — F5 fix: the footnote notes take the full measure

At F5 the footnote apparatus read too tall: the 44% width was on the whole `.footnotes`
**list**, so a note's output recess wrapped inside 44% and grew tall. The short rule is now
a `.footnotes::before` separator (44%); the `<ol>` is full width, so the notes fit. The
view test asserts the RULE is short AND the list is not (it fails if the list regresses to
44%). `26bc741`.

### 2026-10-09 — flagged, NOT taken: a wide-figure output bleed

The footnote notes now take the full 64ch measure (`26bc741`), but a wide `bash`/`read`
output is still monospace `white-space: pre-wrap`, so it wraps a lot inside the column. A
paper-true alternative — let the output **bleed past the measure to the panel width, like a
wide figure in a book** — was raised and **flagged, not implemented**; it awaits the human.

CSS shape (the standard full-bleed technique; the `.footnotes` sits inside the measure-capped
`.col`, so it must escape it):

    /* OPTION (not taken): a wide tool output bleeds past the measure, like a wide figure. */
    details.fn-out .inner,
    details.fn-out .inner pre.quote {
      width: 100vw;
      margin-inline: calc(50% - 50vw);   /* escape the 64ch .col, span the panel */
    }

Caveat for whoever implements it: `100vw` counts the scrollbar, so `.stream` would want
`overflow-x: hidden` (or the width derived from the scroller) or the bleed adds a horizontal
scrollbar. A lesser variant stays inside the column but un-pads to the measure’s own edge:
`width: calc(100% + 2 * var(--wc-4)); margin-left: calc(-1 * var(--wc-4))`.

Trade-off: it stops a wide output wrapping tall, at the cost of breaking the ONE-MEASURE
stance for one element. Not decided here (`design-taste`: the measured column is the stance).

### 2026-10-09 — the team list: Option 4 implemented (glyph row + foot action)

See the **W005 record** — [`docs/work/W005-editorial-vscode-surface.md`](W005-editorial-vscode-surface.md)
§9 — for the fuller (primary) entry. In brief: the roster is a wrapping strip of `glyph name`
pairs (the per-member `.act` gone) and the live action moves to the transcript's foot.
Implementation `cddbde6`; the pure `livelineSpec` seam + tests `e7939d9`.

### 2026-10-09 — the tool apparatus follows event order (per-message footnotes)

The human reported a multi-round run reading out of order — "all thinking and message at
top, all tools at the bottom" — while running AND at turn end. Root cause: `renderTurn`
pooled the WHOLE turn's tool blocks into ONE `.footnotes` at the turn's foot. Fixed by a new
pure `turnSegments` (`src/webview/view.ts`) that splits a turn at each message boundary;
`renderTurn` now sets one `.footnotes` group PER message, so each message's detail follows
it, in event order, identically while running and settled (the `.fnmark` numbering stays
block-ordered across segments). Implemented by the orchestrator (the developer stalled) —
commit `2b96fcc`; second-layer APPROVED (226/226). The commit subject's `W010` citation is a
misnomer: this is a W008 defect fix, not a new work item.

## 10. References

- `docs/design/vscode-paper-draft.html` — the refinement's visual spec (the WHAT).
- `docs/design/vscode-paper-direction.md` — the stance, dials, moves-with-anchors, the Implementation seam, the type-scale note, the refusals.
- `docs/design/vscode-modern-editorial-draft.html` — direction A, shipped in W005.
- `docs/plans/vscode-ui-editorial-plan.md` — the plan this refines (E0–E7).
- `docs/decisions/D006-editorial-supersedes-v3-decisions.md` — the authorized direction.
- `docs/work/W005-editorial-vscode-surface.md` — the A brief (scope, estimates, gates).
- `editors/vscode/media/chat.css`, `src/webview/chat.ts`, `src/webview/view.ts`, `src/render.ts` — the shipped code and the seams.
- `editors/vscode/test/view.test.ts`, `test/render.test.ts`, `test/purity.test.ts`, `scripts/check-css.mjs`, `package.json` — the assertions and the gates.
- `.wcode/skills/design-taste/SKILL.md` §5 (copy), §6 (`[web]` tells), §7.2/§7.3 (audit + amend-first, `:cgHk2`) — the pre-flight and the amend-first rule.

---

## Facts NOT verified

- **No `npm`, node, browser, or VS Code host was run** (read-only worker). The paper draft was authored parse-checked, not painted (`docs/design/vscode-paper-direction.md` §"Facts NOT verified"); the shipped webview's paint is asserted from code reading only. The serif resolves to the generic `serif` where none of the named families is installed — by design, but the painted result differs by platform and was **not observed**.
- **The `.fnmark` placement is a proposal, not a fact.** The paper draft puts the mark *inside a sentence*, but the shipped model renders each prose block as one pre-rendered HTML node (`render.ts:YPLHf` `renderContent`, then `chat.ts:pB3y0` `blockShell` sets `innerHTML`), so a mark cannot be injected mid-sentence without string surgery. The recommended seam appends the mark as a trailing inline node at the tool block's position; that reading needs a layer-1 verdict.
- **The multi-`body` turn question.** A turn can hold several `assistant` blocks (each a `.body` sibling). The paper draws one `.body` per turn containing all prose; whether the refinement merges prose blocks, or appends `.footnotes` after the last prose sibling, is **not settled** by the direction doc — flagged.
- **The class-rename decision** (`.masthead` → `.runninghead`, `.team` → `.masthead-team`) is the author's call to **keep the shipped names**; the paper's names are the same nodes. Not a fact, a recommendation.
- **The `--ed-read` stack and the exact `--ed-indent`/`--ed-measure` values** are read off the prototype, not audited in a browser.
- **The fold-override retarget** (reusing `foldOpen`/`toggleFold` for the note's `details`) is the recommended mechanism; the direction doc says the behaviour "has to move with it" but does not name the function. Not confirmed against a running re-render.
- **Not read:** `editors/vscode/src/webviewView.ts`, `panel.ts`, `surface.ts`, `.vscode/launch.json`, `tsconfig.webview.json` — asserted to need no change only because the skeleton IDs live in `chat.ts` and the wire types (`src/webview.ts`) are untouched.
- **The tracker row number (65)** and the plan/D006 amendment shape are taken from the orchestrator's instruction, not verified against a counter.

The brief ends with the brief.
