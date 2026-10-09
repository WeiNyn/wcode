# W005 — VS Code surface, direction A (editorial) — brief

- **Status:** in progress (the human signed off direction A)
- **Work item:** W005
- **Decisions:** [D006](../decisions/D006-editorial-supersedes-v3-decisions.md) — editorial supersedes four shipped v3 decisions
- **Author:** session `agent:brainstormer` (recorded by the orchestrator)
- **Date:** 2026-10-09

## 1. The ask

Implement **direction A · editorial** for the wcode VS Code webview surface. The human
reviewed six directions in `docs/design/vscode-modern-directions.md` and chose A; the
prototype `docs/design/vscode-modern-editorial-draft.html` **is the visual spec**. A's
one line: *"the panel is dated because it is dressed like a widget — a rail, a spine,
hairlines, a fill on your own words, a pill — wrapped around prose; it is modern when
the words ARE the interface: one quiet column, a byline, and air."* Dials
`VARIANCE 1 · MOTION 1 · DENSITY 3` — the calmest, lowest-density direction. The work
is a **redesign** (`design-taste` §7.1), so per §7.3 it **amends the shipped spec first**.

## 2. Scope / Non-scope

**In scope.**
- The masthead: replace the two-band header + trailing state strip with one quiet line —
  identity glyph + name, a right-aligned `stateWord · ctx N`, and a **dim team caption**
  below (`docs/design/vscode-modern-editorial-draft.html:7VRAj` `.masthead`, `:VsZrR`
  `.ident`, `:OIPxG` `.team`).
- Delete the **permanent rail** (`.side`, `media/chat.css:r0HaK`) and its collapsed
  avatar strip; the team survives only as the caption.
- The **reading column**: a centered `--ed-measure` (66ch) column, line-height 1.7
  (`:ZnugF` `.stream`, `:9ZQA6` `.col`, `:m6NwY` `.byline`).
- The **byline** replaces the `.who` head: small-caps mono, `.byline.you` for the user
  (`:m6NwY`, `:MsFVC`).
- Delete the **3px turn spine** (`.rail`), the **`you` fill**
  (`--vscode-textBlockQuote-background` on `.turn.you`), and the `.who::after` head hairline.
- Tool rows become **sentences**: a dim summary line, a fill-only recess on expand
  (`:yQw0I` `.tool`…).
- The code block becomes a **fill-only recess** (no card border) (`:VsKA5` `pre.quote`).
- Replace the **working pill** with **one live type-line** above the composer (`:lrBi1`
  `.liveline`).
- Restyle the **composer** as a field on a page: `❯` prefix, a hairline underline,
  text-link controls (`:xStnj` `.composer`, `:Y9XkG` `.inputline`, `:mbFOW` `.linkbtn`).
- Amend `docs/vscode-ui-v3-plan.md` (or a new sibling plan doc) to record the superseded
  decisions, and add a tracker row.

**Non-scope.**
- **The `/`-command menu, `@`-mention mechanics, the member-verbs menu** — A is a reading
  surface; its prototype does not render them. Keep as shipped; a flat restyle of
  `.cmdmenu` is a separate change.
- **The docked/editor host wiring** (`webviewView.ts`, `panel.ts`, `surface.ts`) — no host
  change; A is a webview-internal restyle.
- **The wire / kernel** — see §4.5: A needs **no new wire data**; `ToWebview`/`FromWebview`
  are untouched.
- **The in-panel change review** (`.review`/`.hunk`, `chat.ts:YVJx9`) — A's prototype shows
  no diff card; it stays a card (an interactive, foreign object), not restyled.
- **Native/QuickPick/palette work** — that is direction D/B, not A.
- **The Tasks (root plan) section** — A shows no Tasks; dropping it is a deliberate loss
  (flag §6).
- **A headless-browser test harness** — none exists in the repo (see §6 gates); the visual
  proof is the F5 host + opening the spec file.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| E0 | Amend the spec (v3 plan supersession notes + new plan doc + tracker + D006) | high | low | low | The redesign is unauthorized until the shipped spec says so (`design-taste:cgHk2`); docs-only, reversible. |
| E1 | Masthead + team caption (delete the rail) | high | medium | medium | Biggest structural DOM change (skeleton + `renderRails` removal); the team loses its persistent home. |
| E2 | The reading column (byline, drop spine/fill/hairline) | high | medium | low | Pure composition; touches `renderTurn` (`chat.ts:KoM4E`) + `view.ts:who` (`:Xptb4`); the `you` indent/fill deletion removes the V14 clip hazard. |
| E3 | Tool rows + code block (drop output card + Copy; fill-only code) | medium | medium | medium | Reverses V12's "output is a card"; drops the `Copy` affordance. |
| E4 | The live type-line (drop the pill) | medium | low | low | `workingGroup` (`view.ts:KeUWa`) already returns the count; the reveal-on-click affordance goes. |
| E5 | Composer restyle (inputline/`❯`/linkbtns) | medium | medium | low | Markup change in the skeleton; the mode control becomes a text toggle. |
| E6 | Dead-code sweep (rail/pill/swatch CSS + fns) | low | low | low | Keeps the sheet + `check-css.mjs` honest; a missed orphan fails nothing but leaves drift. |
| E7 | Visual proof (F5 + spec file) | high | low | medium | README says the webview was **never run**; A's whole value is a painted surface. |

## 4. Interface & structure

**Added.**
- `docs/vscode-ui-editorial-plan.md` (or a new § in the v3 plan) — the plan + the explicit
  "supersedes" list, citing the anchors below.
- New CSS classnames from the A spec: `.masthead`, `.ident`+`.name`/`.glyph`/`.status`/`.team`/`.m`,
  `.stream`/`.col`, `.byline`, `.liveline`/`.dots`, `.inputline`/`.pfx`, `.linkbtn`,
  `pre.quote` (`docs/design/vscode-modern-editorial-draft.html:7VRAj`,`:q9zFS`,`:OIPxG`,`:ZnugF`,`:9ZQA6`,`:m6NwY`,`:lrBi1`,`:Y9XkG`,`:mbFOW`,`:VsKA5`).
- (Likely) a `webview/view.ts` pure fn for the caption rows (`teamCaption`),
  replacing/repurposing `rosterRows` (`view.ts:shedw`).

**Changed (webview DOM — `src/webview/chat.ts`).**
- Skeleton `innerHTML` (`chat.ts:Bnfe4`): replace `#phead` (`:sphrf`) with `header.masthead`
  (identity + status + team caption); **delete the `#side` rail** (`:dNCZT`); replace the
  `#pill` button (`:CkW5y`) with a `.liveline` div; rebuild the composer block
  (`:NCAde`–`:ArrpJ`) as `.inputline` + `.foot`.
- `renderHeader` (`:5zavC`), `headerSignature` (`:r4NS1`), `headerCellNode` (`:4QHa7`),
  `identNode` (`:NEKPO`), `segNode` (`:u3Jn8`), `moreNode` (`:kE33A`), `disclosurePopover`
  (`:dOh30`) — the cell list loses `seg`/`more`/chip (settle §4.6).
- `renderTurn` (`:KoM4E`) — drop the `el("div","rail")` spine; `.who` → `.byline`; drop the
  `.av` avatar box.
- `renderToolFold` (`:nTiZP`) + `rawOutput` (`:aYpYa`) + `copyButton` (`:MiCOC`) — the output
  fold becomes a bare `<pre>` recess; the `.tool-output` card head + `Copy` go.
- `renderPill` (`:SeYtt`) + the pill listener (`:gKKQp`) + `revealLiveStep`/`scrollToLiveStep`
  (`:gkfax`/`:qKRK`) — replaced by a non-interactive `.liveline` (`role="status"`).
- `render` (`:jGu7k`) — call the liveline renderer instead of `renderPill`; drop the
  `renderRails` call (`:eWi03`).
- `renderRails` (`:EXy2k`), `sideToggle` (`:olvvf`), `applyCollapsed` (`:M6lew`), `stripChip`
  (`:RXTzh`), `teamSection` (`:iQ1Ve`), `tasksSection` (`:X9nZp`) — deleted (the caption
  replaces them).
- `renderComposer` (`:kvOm4`) — mode/attach/stop become `.linkbtn` text controls.

**Changed (pure halves).**
- `view.ts::panelHeader` (`s7HzW`) + the cell types `IdentCell` (`:tdvhd`), `SegCell`
  (`:LbH2g`), `MeterCell` (`:WWTfk`), `MoreCell` (`:VnBcR`) — trimmed to the masthead's real
  cells.
- `view.ts::who` (`:Xptb4`) + `Turn.who` (`:XXXmv`) — the byline shape (glyph + name;
  `.byline.you`).
- `view.ts::rosterRows` (`:shedw`) — reduced to the caption row (glyph + name + action; no
  `--sw` edge).
- `view.ts::workingGroup` (`:KeUWa`) — reused verbatim by the liveline (no avatars →
  `avatarStack` (`:7YNnr`) loses its caller).
- `view.ts::outputHead` (`:FIg9t`) — loses its caller if the output head is dropped.
- `view.ts::composerControls` (`:9Z8XG`) — kept; only the DOM restyle changes.
- `render.ts::renderContent` (`:YPLHf`) — the `.fold.thinking` markup likely unchanged;
  `.code` markup unchanged (only the border goes, §4.6).
- `reducer.ts::memberGlyph` (`RNWdV`), `sidebarRails` (`tAJNC`), `SessionMember` (`xSLIN`) —
  unchanged.

**Changed (stylesheet — `media/chat.css`).** Remove/rewrite: `.phead` (`8sYdY`), `.chip`
(`eGBrj`), `.seg` (`UA5Bb`), `.more` (`etpWM`), `.transcript` (`YyF7c`) → `.stream`, `.turn`
grid (`rrrmt`), `.turn.new` (`RjaB0`), `.rail` (`tR9Tl`), `.turn.you` (`EICFj`), `.turn
.who::after` (`nfinx`), `details.fold` (`tKorz`), `details.fold.tool` green rule (`EhEGR`),
`details.fold.tool[data-live]` wash (`CmEKK`), `.tname` (`mPJSD`), `.fold .tool-output`
(`u0pEP`), `.code` (`yEea1`), `.side`/`.roster`/`.mav`/`.avatars`
(`r0HaK`/`31mvL`/`rEw75`/`Wwrad`), `.pill`+`.pill .dots` (`VkDNe`), `textarea#input`
(`DxkQe`), composer `.seg`/`.ctoolbar` (`w864r`), and the motion gate (`.pill .dots` →
`.liveline .dots`, `9OH1B`).

### 4.5 Wire & kernel — UNCHANGED (confirmed)

Every fact A renders already reaches the webview: identity glyph ← `memberGlyph`
(`reducer.ts:RNWdV`); state word ← `stateLabel` (`view.ts:r9vQz`); `ctx N` ←
`status.contextUsed` (`view.ts:s7HzW`); the team caption ← `state.members` (`RenderedState`,
`render.ts:59Wvn`); the per-member `liveAction` ← `SessionMember.liveAction`
(`reducer.ts:OKFKP`); the liveline count ← `workingGroup` (`view.ts:KeUWa`); turns/blocks/tools
← `RenderedState.blocks`. `ToWebview`/`FromWebview` (`webview.ts:UQVMr`,`:63`) and `ViewMode`
(`:nrE1F`) are **not touched**. A is a **DOM + CSS** change only (no new wire field, no new
`FromWebview` variant). The one behaviour A drops (the pill's click-to-reveal) was
client-local (`data-live`, `chat.ts:mkxow`) and needs no protocol change to remove.

### 4.6 The v3 decisions A contradicts — settle explicitly

`docs/vscode-ui-v3-plan.md` is the shipped spec; its prototype
`docs/design/vscode-ui-v3-draft.html` "IS the spec" (`docs/vscode-ui-v3-plan.md:CRpGr`). Per
`design-taste:cgHk2` ("Never change a locked invariant silently… requires amending … first")
and §7.4 (`:gSW0v`), the amendment is an explicit act. **The A prototype is the new visual
spec; the v3 plan receives supersession notes** (it already carries an "Amends, explicitly"
block — `:gBoLf` — so this matches its own convention). Concretely:

| v3 item | anchor | A's verdict |
|---|---|---|
| **Decision 3** "the team has ONE home (the rail)" + two echoes | `docs/vscode-ui-v3-plan.md:8HCPY` (also §1.3 `:DZxmO`, §1.6 `:74by4`) | **Superseded** — the rail and the pill both go; the team is a dim caption, the aggregate a type-line. |
| **Decision 2** "the All/Focus control lives in the header" | `:M3ETf` (also `:xOWEi`) | **Kept-with-restyle** — the wire `mode` + retarget semantics remain; the control becomes a quiet text toggle, not a bordered `.seg`. **Settle here.** |
| **Decision 5** L1/L2a/L2b, "the code card and tool output are L2a" | `:Jkqsl` (also §1.5(c) `:htKh9`) | **Amended** — the code block and the tool output become fill-only recesses; only the change-review stays L2a. |
| **Decision 12 / V11b** the gauge `▰▰▰▱▱ …` | `:CSVRU` (also `:NyEQw` in view.ts) | **Amended** — A's masthead shows the plain `ctx 42k`; the gauge cell is dropped. **Settle here.** |
| **V12** tool output L2b → L2a (`⚙ tool · output` + `Copy`) | `:zTBVB`,`:SZZZE` (phase `:ZWSV3`) | **Superseded** — output reverts to a recess; the head/Copy go (identity moves to the summary line). |
| **V13r** "complete is GREEN" | `:cXMdP` (phase `:nF14`) | **Superseded** — complete mark is `--vscode-foreground` (neutral); running accent, error red (A `.tool .mark` `:6Mcg4`). |
| **V14** the turn head/spine/alignment/rhythm | `:Sc8DY` (§1.5f; phase `:1qJxz`) | **Superseded** — no spine, no head hairline, no `you` fill; the `.new` beat survives at `--wc-5` (A `.turn.new` `:d8s2s`), not `--wc-4`. |
| **The six `charts-*` swatches** | `:WJJlL`/`:v6gmP` | **Effectively dropped** — every home (rail edge, turn rail, `.who` `.av`, collapsed strip) is removed; the caption glyph is coloured by **state**, not identity. **Settle here.** |

**The recommendation:** add the supersession notes as a short "Amended by editorial (W005)"
line on each affected decision/section of `docs/vscode-ui-v3-plan.md`, **and** write the plan
itself as a new doc `docs/vscode-ui-editorial-plan.md` (nothing under `docs/` is moved —
D004/item 58). Open **D006** for "editorial supersedes v3 decisions 3 & 5, V12, V13r, V14"
(`traceability-log` §1: a locked choice re-litigated with a reason → an ADR). Add **one** row
to `docs/next-steps.md` (next free index = **61**; item 56 stays "shipped v3, superseded by
W005").

**Settled open questions (contested points the A prototype does not fully pin — a layer-1
reviewer must confirm):**
1. **All/Focus placement.** A's prototype shows the mode control only as *gallery chrome* and
   its own JS note says "you retarget from the masthead's team caption"
   (`docs/design/vscode-modern-editorial-draft.html:gwfD9`). Resolution: **keep the mode**
   (the wire concept and the merged-transcript semantics survive) as a **quiet text toggle in
   the masthead's status line**; the **team caption is the retarget affordance** (replacing
   the rail rows and the target chip).
2. **The `▾` disclosure** (session/model/effort, `view.ts:s7HzW`, `chat.ts:kE33A`). The A
   masthead does not render it. Resolution: **drop it from the surface**; record the loss
   (session id/model/effort no longer glanceable). Flagged, not asserted.
3. **The gauge** — dropped (plain `ctx 42k`).
4. **The swatches** — dropped (state colours only).
5. **The Tasks section** — dropped (deliberate loss; if wanted, a status-line count).

## 5. Plan

1. **Amend the spec.** Deliverable: `docs/vscode-ui-editorial-plan.md` + supersession notes
   in `docs/vscode-ui-v3-plan.md` + a `docs/next-steps.md` row (61) + D006. Gate: layer-1
   review APPROVE; `file:anchor` on each superseded item still names its fact.
2. **Masthead + team caption.** Deliverable: `header.masthead` in the skeleton; the rail gone;
   the caption rendered with glyph + name + action; retarget posts `focus-member`. Gate:
   `npm run typecheck` clean; F5 shows one masthead line + caption and a caption click
   retargets.
3. **Reading column.** Deliverable: `.stream`/`.col`/`.turn`/`.byline`; the spine, `you` fill
   and `.who::after` hairline deleted; `.new` beat at `--wc-5`. Gate: updated `view.test.ts`
   green; F5 shows one 66ch column, user turns set apart by the byline alone.
4. **Tool rows + code block.** Deliverable: the output fold is a bare recess; the `.code` card
   is fill-only; complete mark neutral, running accent, error red. Gate: updated `view.test.ts`;
   F5 shows a running tool with an accent mark and a completed tool neutral.
5. **The live type-line.** Deliverable: `.liveline` (role status) replaces `#pill`; the
   reveal-on-click gone; the motion gate names `.liveline .dots`. Gate: updated tests; F5 shows
   the line only while work is in flight; reduced-motion freezes the dots.
6. **Composer restyle.** Deliverable: `.inputline`+`❯`+`.foot` `.linkbtn`; mode as a text
   toggle. Gate: `composerControls` tests green; F5: focus underline, Send/Stop/Act/Plan
   reachable by keyboard.
7. **Dead-code sweep.** Deliverable: `.side`/`.roster`/`.avatars`/`.mav`/`.pill` rules and the
   orphan fns deleted. Gate: `node scripts/check-css.mjs` ok; `grep -rn` for each deleted
   class/fn returns nothing in `src/`+`media/`.
8. **Visual proof.** Deliverable: the F5 host screenshot + the spec file opened in a browser.
   Gate: §7.2.

## 6. Quality gates

Run **in `editors/vscode`** (cargo is **not** the gate here — the webview is not a Rust surface):

- `npm run typecheck` (`package.json:BGQPX`) → the host program **and** the webview program
  compile; **no output**.
- `npm test` (`package.json:0CAl6` = `node scripts/check-css.mjs && node --test test/*.test.ts`)
  → `check-css: ok (chat.css)` then `# pass <n>` / `# fail 0`.
- `npm run build` (`package.json:l2TGu`) → `out/extension.js` and `media/chat.js` written.
- Crawl the whole workspace is unaffected: `cargo test --workspace` /
  `cargo clippy --workspace --all-targets` **still** must be clean (no Rust touched — a guard,
  not a gate).
- **Regression tests kept green:** `parseFromWebview accepts the well-formed messages`,
  `memberGlyph maps each MemberState to the TUI glyph + class`, `a fenced code block renders as
  a titled .code card`, `emptySpec names copy for all six states`, all `workingGroup …` tests,
  `composerControls …`.
- **Browser render:** no headless-browser harness exists; the two honest proofs are (a) open
  `docs/design/vscode-modern-editorial-draft.html` in a browser (the spec renders standalone,
  `:SgyDu`) and (b) the F5 Extension Development Host click-path.

## 7. Testing

### 7.1 Automated

Tests that must **change** (each currently asserts a removed rule/element). In `test/view.test.ts`:

- `panelHeader: ONE row — identity, All/Focus seg, spacer, ctx meter, ▾ disclosure` (`:22v75`)
  — **update**: the cell set loses `seg`/`more` and the chip.
- `panelHeader: the seg cell carries the mode (the ONE All/Focus control)` (`:coOyF`) —
  **update** to the text-toggle cell (or keep if §4.6(1) keeps a `seg` map).
- `panelHeader: the ▾ disclosure carries session + model + effort …` (`:VPinN`) — **delete**
  (the disclosure is dropped).
- `panelHeader: Focus shows the TARGET member; All hides the chip …` (`:33NBs`) — **update**
  (chip replaced by the caption/glyph).
- `panelHeader: the identity glyph is the TARGET member's (a retarget changes it)` (`:PssoB`) —
  **update** (drop the `chip` assertion).
- `panelHeader: the meter becomes the GAUGE when the TARGET's window is known` (`:gpnWz`) /
  `gauge: …` (`:SxV9W`) — **update/delete** (the gauge is dropped).
- `rosterRows: the rail row is a --sw edge + the STATE glyph (no .mav box)` (`:LTjMI`) —
  **update** to the caption row (no swatch edge), or replace with `teamCaption`.
- `the three tool-status colours are GREEN / BLUE / RED in the sheet (V13r)` (`:Ii1fy`) —
  **update**: green → `--vscode-foreground`; keep the accent/error matches.
- `the V14 composition is in the sheet: spine, head, alignment, rhythm, the wash` (`:6XqAd`) —
  **update**: drop the spine/`grid-template-columns: 3px 1fr`, the `.turn .who::after` hairline,
  the `you` fill/`overflow`, the `[data-live]` wash; the `.new` beat is `--wc-5`.
- `a turn never shrinks: the transcript scrolls …` (`:PVWoP`) — **update/delete** (no
  `overflow:hidden` turn remains).
- All `turns: … who.className === "who …"` assertions
  (`:iezQa`,`:BbdYN`,`:45zQC`,`:eAc4E`,`:OXFEs`,`:mqbNp`) — **update** if `.who` → `.byline`.
- `outputHead: the output card's head is ⚙ {tool} · output` (`:uRgge`) — **delete** if the
  output head is dropped.
- `avatarStack: the first 3 chips …` (`:T64Ud`) and `workingGroup … swatch/initial` assertions
  (`:aoRK7`) — **trim** (the liveline keeps `count`; the avatar stack loses its caller).

Tests that stay **green** (behaviour unchanged): the `turns` grouping/`.new` logic (kept),
`composerControls`, `classNames`, `emptyKind`/`emptySpec`, `bodyKind`, `diffStat`,
`foldOpen`/`toggleFold`, `parseToWebview`, `memberGlyph`, and `test/purity.test.ts`
(`webview/view.ts` stays pure — `:OlvBf`) and `test/webview.test.ts` (the wire union is
untouched).

**New tests to add** (each asserts behaviour, not shape):
- `team caption` — one row per member, the glyph from `memberGlyph`, the dim `liveAction`, the
  root = `❯`.
- `the liveline count` — `role="status"` text is `{n} working` iff `workingGroup.count > 0`.
- `the masthead is one row` — identity + state word + `ctx N`, **no** chip/seg/more.
- `the code block has no border` — a CSS assertion
  (`/\.quote \{[^}]*background: var\(--vscode-textCodeBlock-background\)/` and no `border:` in
  `.code`/`.quote`).
- `reduced-motion freezes the liveline dots` — the gate names `.liveline .dots`.

### 7.2 How a human verifies it

```sh
cd editors/vscode
npm install
npm run typecheck && npm test && npm run build
# (visual) open the spec:
open ../../docs/design/vscode-modern-editorial-draft.html      # macOS
# (visual) the real surface: open the repo root in VS Code, press F5,
# then Cmd+Shift+P -> "wcode: Start Session"  (README click-path)
```

Look for: **one** masthead line (identity glyph + name, right-aligned `ctx N`) with a single
dim team caption under it; **no** left rail; the transcript a centered 66ch column; user turns
set apart only by a `.byline` and the gap (no spine, no fill); the `❯`+hairline composer; the
live type-line only while work runs. Compare each against the A prototype. If no endpoint is
configured, step "Start Session" ends `✗ crashed` — that is the failure state, not a silent
view.

## 8. Expected outcome

- `media/chat.css` has **no** `.side`, `.roster`, `.pill`, `.chip`, or `.rail` rule;
  `node scripts/check-css.mjs` prints `check-css: ok`.
- `src/webview/chat.ts` has **no** `renderPill`/`renderRails`/`teamSection`/`tasksSection`/`sideToggle`;
  `renderTurn` emits `.byline`, not `.rail`/`.who`.
- The webview renders one masthead line + caption, a centered reading column, a fill-only code
  block, a bare tool recess, and one live type-line.
- `docs/vscode-ui-v3-plan.md` carries a supersession note on decisions 2/3/5 and V12/V13r/V14;
  D006 exists; `docs/next-steps.md` has row 62 pointing at W005.
- `npm run typecheck` and `npm test` are clean; `cargo test --workspace` /
  `cargo clippy --workspace --all-targets` remain clean (untouched).

## 9. Log

### 2026-10-09 — E0 landed (the spec amendment)

- The human **signed off direction A · editorial**; the brief moves from `draft` to `in progress`.
- **Layer-1 review returned BLOCKING on three items; all three are folded:**
  1. the tracker row is **62**, not 61 (61 is W004);
  2. the plan doc lives at `docs/plans/vscode-ui-editorial-plan.md` (D004 categorised buckets), not the flat `docs/` path;
  3. the sketch's deletion block gains `targetSwatch` and the import pruning.
- The sketch's `<TBD>`s are settled: **A1** mode → `ModeCell`; **A2** the state word on `MeterCell`; **A3** `glyph` added to `TurnMember`; **A4** the caption glyph comes from `memberGlyph`; **A5** the caption action is `liveAction ?? row.state`; **A6** the liveline is count-only.
- **E0 landed** as `39206ff` (v3 supersession notes), `5014026` (the plan doc), `e2f8f8c` (D006), `c54a6c7` (tracker row 62 + item 56). The design directions + prototypes are committed in `7d72c45`. E1–E7 are not started.

### 2026-10-09 — direction A implemented (E1–E6); layer-2 APPROVED

- **Implemented** in three `vscode:` commits: `f441b1a` (E1/E4/E5 — the masthead, the team caption, the live type-line, the composer), `20d77e0` (E2/E3 — the reading column + tool sentences), `8233fda` (E6 — the dead chrome CSS + stale comments).
- **Gates** (in `editors/vscode`): `npm run typecheck` → exit 0; `npm test` → `check-css: ok`, 189 pass / 0 fail; `npm run build` → ok; **zero `SKETCH` markers**; the wire/kernel untouched.
- **Layer-2 review APPROVED** the diff (no code blockers). Its deviations were authorised: the tool row drops its L1 rule (the status lives on the `⚙` mark — **V13 narrowed**, D006), the composer `.inner` wrapper, the small `livelineLabel` helper, and `.code` keeping its `.chead` markup (border dropped).
- **Non-blocking notes folded (none deferred):** N1 the dead `data-live` stamp on a plain block is gone; N2 the caption row is keyboard-activatable (Enter/Space → `focus-member`); N3 two stale test names renamed; N4 this record synced.
- **E7 (the F5 visual proof) is the only open step** — a human action; no browser / VS Code host runs in this session.

### 2026-10-09 — E7 (F5) feedback folded: the composer + Stop

The human ran the F5 review of the editorial surface and reported two defects; both are fixed in `9f5eb3c`:

- **The composer forced two rows.** The placeholder (with its `, Esc to cancel` tail) wrapped and doubled the composer height — `autoGrow` sizes from the VALUE (`chat.ts`), so the placeholder was the only cause. It is now `Message wcode…`, with the hints moved to the textarea's `title` tooltip (`Enter to send · Shift+Enter for a newline · Esc to cancel`); the composer rests at ONE row.
- **There was no way to cancel agents.** `composerControls.stop` was `status.running`, which `renderState` makes PER-TARGET (`render.ts:82`) — with the root idle and workers running, no Stop appeared. It is now true whenever any member's `MemberState` is `running`. And Cancel is channel-aware: a new pure `cancelTargets` (`src/cancel.ts`) makes **Focus** cancel the focused member alone, while **All** (`target === null`) cancels the whole session — the root plus every running member (`extension.ts` `onCancel`; the wire is unchanged). `wcode.member.stop` is no longer inert (it now stops the focused member).
- **Tests:** `composerControls.stop` shows for a running non-root member while the root is idle (fails on the old per-target logic); `test/cancel.test.ts` asserts All-mode reaches the root AND each running member (fails if only the root is cancelled). Gates: `npm run typecheck` → 0; `npm test` → 195 pass / 0 fail; `npm run build` → ok.

### 2026-10-09 — the team list: Option 4 (glyph row + foot action) implemented

The human chose **Option 4** of `docs/design/vscode-team-options.md` (a glyph row + the
foot action). Implemented in `cddbde6`:

- **The roster** is now a wrapping strip of `glyph name` **pairs** (root first, each a keyed
  retarget control — `tabindex="0"` + Enter/Space → `focus-member`), the target pair
  **underlined**. The pairs are separated by the flex **`gap`**, not the middle-dot, to keep
  the locked "≤1 `·` per metadata line" rule.
- **The action moves to the transcript's foot.** The per-member `.act` leaves the roster; the
  liveline carries *what the active member is doing*: exactly one active member →
  `glyph name · action`; several → `⋯ {n} working`; none → hidden.
- **Pure halves:** `teamCaption`'s `CaptionRow` swaps `action` for `state` (the pair's
  `title`/`aria-label`); `workingGroup`'s `WorkingRow` gains `glyph`/`glyphClass` for the foot
  line. `view.ts`/`chat.ts`/`chat.css`; tests updated; wire/kernel untouched. The pair's `title` is
`<name> · <state>` and its `aria-label` is `<name>, <state>`.

(The design doc's status was "Not authorized" at the time; it now records the implementation.)

### 2026-10-09 — the composer: Send and Stop share ONE slot

The foot had two competing controls while a run was in flight (Send always visible, the
conditional Stop hidden behind the same styling) — so the interrupt read as absent. It now has
**ONE primary action in ONE slot**: `#send` and `#cancel` are adjacent (`#cattach · Act Plan
#send #cancel`) with **inverse visibility**, driven by a new pure
`composerActions(state) -> { send, stop }` (`sendBtn.hidden = actions.stop`;
`cancelBtn.hidden = !actions.stop`). Both carry `.composer .foot .send { margin-left: auto }`,
so whichever shows stays right-aligned (the swap neither shifts the layout nor shows two), and
the interrupt reads in the accent (`--wc-accent`, "active now"). **Cancel semantics unchanged:**
Stop posts `{ kind: "cancel" }` → the host's `cancelTargets`; Esc still cancels; the wire is
untouched. `7a946ad` (a first cut that merged the two) → `e019ef3` (the shipped shape, the
L2-folded presentation).

## 10. References

- `docs/design/vscode-modern-editorial-draft.html` — the A spec (the WHAT).
- `docs/design/vscode-modern-directions.md` §A (`:86wCa`) — the stance/dials.
- `docs/vscode-ui-v3-plan.md` — the shipped spec A amends (decisions `:8HCPY`,`:M3ETf`,`:Jkqsl`,`:CSVRU`;
  V12 `:zTBVB`; V13r `:cXMdP`; V14 `:Sc8DY`).
- `editors/vscode/media/chat.css`, `src/webview/chat.ts`, `src/webview/view.ts`, `src/render.ts`,
  `src/reducer.ts` — the code touched.
- `editors/vscode/test/view.test.ts` (+ `render.test.ts`, `webview.test.ts`, `purity.test.ts`) —
  the assertions that change.
- `editors/vscode/README.md` — the click-path and the "NOT verified" list.
- `.wcode/skills/design-taste/SKILL.md` §7.2/§7.3 (audit + amend-first),
  `.wcode/skills/two-layer-review/SKILL.md`, `.wcode/skills/traceability-log/SKILL.md`.

---

## Facts NOT verified

- **No browser, no VS Code, no `npm` was run** (read-only worker). The A spec was parse-checked
  by its authors, not painted (`docs/design/vscode-modern-directions.md:q4uT5`); the shipped
  webview was **never run** (`editors/vscode/README.md:IiyxP`). Every CSS/DOM claim here is read,
  not observed.
- **The contested settlements are proposals, not facts** — All/Focus placement, dropping the
  `▾` disclosure, the gauge, the six swatches, and the Tasks section are the author's reading of
  the A prototype; the human chose the *direction*, not these details (§4.6). They need a layer-1
  verdict.
- **Not read:** `src/webviewView.ts`, `src/panel.ts`, `src/surface.ts`, `.vscode/launch.json`,
  `tsconfig.webview.json` — asserted they need no change only from the fact that the skeleton
  IDs live solely in `chat.ts` (grep: `editors/vscode/src/webview/chat.ts:137`) and the wire
  types are untouched.
- **Not confirmed:** that removing `Copy`/the output head is intended — A's prototype omits them
  but its "components removed" prose names only the "code card border"; the tool output card is
  inferred from the prototype markup.
- **Not checked:** whether `webview/chat.ts`'s `.code` markup should keep its `.chead` (the
  language label) — recommended keeping the markup so `render.test.ts:rsewB`/`markdown.test.ts:zPAMY`
  stay green and only the border changes in CSS.
