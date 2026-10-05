# VS Code surface — v3 plan (curation + a real visual system)

**Status:** design proposed. The prototype [`design/vscode-ui-v3-draft.html`](design/vscode-ui-v3-draft.html)
IS the spec. V6–V13 shipped (each second-layer APPROVED); **V13r and V14 proposed**.
**Companion docs:** the rationale is [`design/vscode-ui-exploration.md`](design/vscode-ui-exploration.md);
v1's locked UI is [`vscode-ui-rework-plan.md`](vscode-ui-rework-plan.md) (P1–P5 shipped);
v2's one-dockable-surface plan is [`vscode-ui-v2-plan.md`](vscode-ui-v2-plan.md) (V1–V5 shipped, V4b open).

**Amends, explicitly:** this plan supersedes three decisions of those docs — the
**two-row header** (`vscode-ui-rework-plan.md` §2/§4, `vscode-ui-v2-plan.md` §0),
the **separate All/Focus band** (`vscode-ui-v2-plan.md` §0 item 3, shipped as
`#modes`), and **"the team shown once in the sidebar"** (`vscode-ui-rework-plan.md`
§5 decision 3) which the shipped code contradicts by rendering the team in three
places. Nothing else in the locked plans is touched; the kernel and the wire
contract are untouched.

---

## 0. Surface read (design-taste §0)

> Reading this as: a **web screen** (the VS Code webview), for a developer driving a
> multi-agent session inside VS Code, in **VS Code's own theme-variable language**
> (`--vscode-*` for every colour, plus the workbench's density conventions: 22px
> controls, list rows, uppercase section labels), with the dominant constraint that
> the surface uses **only `--vscode-*` tokens for colour** and **zero remote assets**.

**Truth source (design-taste §2.C):** the installed system is **VS Code's own design
language** — the token set plus its workbench density. v3 does not invent a second
one; it obeys the host's. `--vscode-*` owns every colour; the only new tokens are
**lengths and type sizes** (`--wc-1…5`, `--wc-fs-*`), which carry no colour at all
and are what makes the system enforceable. One accent, unchanged:
`--wc-accent: var(--vscode-charts-blue)` (`media/chat.css:RAbMj`).

**Dials — `VARIANCE 3 · MOTION 2 · DENSITY 6`** (v1 and v2 both read `3 · 2 · 8`; two
of the three move, and the moves are the point):

- **`VARIANCE 3`, up from 2 (V14).** v3 first read 2: the chrome's flatness was a
  *hierarchy* problem, and v1 answered it with more bordered boxes, which produced the wall
  of cards. That holds for the CHROME, but the transcript itself stayed a uniform column —
  every turn the same shape, the same head, the same rhythm. **V14 raises the dial to 3**
  to buy composition differences *between turns* (a turn head, a spine, conversation
  alignment, a role-aware rhythm) with no new level and no new box. Hierarchy is still
  bought with **weight + colour + space** (design-taste §4), not with more containers.
  symmetry problem — it is a *hierarchy* problem, and v1 answered it with more
  bordered boxes, which is what produced the current wall of cards. Hierarchy is
  bought with **weight + colour + space** (design-taste §4), not with more containers.
- **`MOTION 2`, held.** The dial stays at 2: motion remains rare and each piece has
  one job. The surface ships **three** animations, up from two — the running glyph
  (`⠋`, `media/chat.css:HGFMq`), the live cursor (`▌`, `:68UvA`), and the working pill's
  `⋯` pulse (`§1.3`, new). The third is justified in one sentence: *"live" is the
  pill's entire message, and a pulse is what separates a typing indicator from a static
  label.* It is one `@keyframes` on one glyph already in the locked vocabulary, and all
  three are reduced-motion gated (`:9OH1B`).
- **`DENSITY 6`, down from 8.** 8 was a claim about a work surface; the user's verdict
  is that the work surface was operator-hostile. Tool-grade legibility at a docked
  sidebar width is the **floor** (a constraint, not a dial); the dial buys air —
  fewer cells, a 4px grid, one-line rows, disclosure. Dense mode still exists as a
  single token switch (`[data-density="dense"]` in the prototype), and it is
  deliberately *still calm*.

**The identity bridge is unchanged:** the locked TUI glyph vocabulary
(`docs/tui-design.md:34-50`) is the one glyph set — `❯ ⚙ ✓ ✗ ⋯ ▌ ⠋ ● ○` plus the
todo boxes `☑ ▸ ☐` and the fold chevrons `▸ ▾`. v3 adds no glyph.

---

## 1. What changes, and why

Two headline problems. (1) **Information curation** — 16 header cells, three stacked
chrome bands, the team in three regions, a 7-control composer. (2) **No system** —
a dead spacing token, ten type sizes, everything a bordered card, an accent doing six
jobs. Every change below is anchored to the shipped code.

### 1.1 The header: 16 cells → 5, one row

**Now.** `panelHeader` (`view.ts:uzMW3`) builds `r1` (up to 11 cells: dot, state, sep,
session, sep, target, plan, sep, running, sep, error — `view.ts:2xZKw`…`:DBXdl`) and
`r2` (model, sep, `ctx N`, spacer, `"{n} members"` — `view.ts:5an6F`…`:vpYZ3`), all at
`12px`/`11px` (`media/chat.css:UA5Bb`, `:nZSoo`) separated by `·` at `opacity:.5`
(`:Il2nW`). `renderHeader` (`chat.ts:hcKrh`) paints both rows; `headerSignature`
(`chat.ts:6QLvB`) gates on all of it. Mixed semantics on one line: a *state*, an
*identity*, a *mode*, a *resource*, and an *error*.

**v3.** One row, five cells:

| # | cell | source |
|---|---|---|
| 1 | identity: the **state glyph** (+ the **target chip** in Focus; see the All-mode rule) | `memberGlyph` (`reducer.ts:RNWdV`) + `state.target` (`render.ts:bvzrt`) |
| 2 | the **All / Focus** `.seg` control | `mode` (`webview.ts:nrE1F`) |
| 3 | spacer | layout |
| 4 | the **context meter**: the text `ctx 42k` | `status.contextUsed` (`reducer.ts:2BU6k`) |
| 5 | the **`▾` disclosure** | new (session id, model, effort) |

- The **dot** (`media/chat.css:jQK5r`) and the standalone **state word** are removed:
  the state is the *identity glyph's* shape and colour, and `title`/`aria-label` spell
  it out. The word returns **only** for `starting` / `stopped` / `crashed` — a failure
  must be unmistakable; a healthy idle session needs no label. On `crashed` the glyph
  is `✗` in `--vscode-errorForeground`, never a recoloured spinner. This also closes
  the audit's
  finding that `memberGlyph` returns `⠋ ✓ ✗ ○` (`reducer.ts:RNWdV`) that the UI
  **never renders**: the header is its new call site. The glyph is `⠋` running, `✓`
  done, `✗` failed, `○` idle (`reducer.ts:RNWdV`), and the word is the matching
  `stateLabel` (`view.ts:r9vQz`):

  | session state | glyph | word in the header |
  |---|---|---|
  | `ready`, idle | `○` (`g-idle`) | none |
  | `ready`, running | `⠋` (`g-run`, spinning) | none |
  | `starting` | `⠋` (`g-run`) | `starting` (a `.tag`) |
  | `crashed` | `✗` (`g-err`) | `crashed` (a `.tag`) |
  | `stopped` | `○` (`g-idle`) | `stopped` (a `.tag`) |

  So a crash reads `✗ crashed`: **the glyph switches to the error glyph, it does not
  merely recolour a frozen spinner**, and the word is added. A healthy session adds
  no word at all.
- **All mode has no target, so cell 1 loses the chip.** `chat.ts` already hides the
  target chip's whole wrap in All mode (`chat.ts:YoBUD`, because the merged transcript
  is nobody's particular transcript). v3 keeps that rule and states it: in **All**
  mode, cell 1 is **the ROOT member's state glyph alone** (no chip, no name); in
  **Focus** mode, cell 1 is the root's-or-target's glyph **plus** the chip. The mode
  `.seg` (cell 2) says which mode you are in, and the transcript's per-turn `.who`
  lines (`chat.ts:KoM4E`) carry the per-member identity in All mode. The chip is
  `hidden`, not removed, so its menu state survives a mode flip.
- **`pchip plan`** (`view.ts:bsBjE`) is removed: `status.planMode` is a *mode*, and the
  composer already owns the mode control (`composerControls`, `view.ts:0oNQG`). One
  fact, one control.
- **`running…`** (`view.ts:pQS7R`) is removed from the header: the running glyph spins,
  and the fold head carries `running…` (`chat.ts:alUZD`).
- **`lastError`** (`view.ts:DBXdl`) is **removed from the header** — and nothing new is
  added. The run failure **already renders in the transcript** as the reducer's `error`
  block (`reducer.ts:306`, alongside `lastError` at `:310`), which `render.ts:114`
  paints as `.body.error` (already styled, `media/chat.css:348`). v3 only drops the
  redundant header cell (V6); the header returns to a steady state instead of holding an
  error banner forever.
- **model** (`view.ts:8Ft2X`) and **`{n} members`** (`view.ts:vpYZ3`) move off the bar:
  the model is a *setting* (rarely needed, set by `/model`), and the member count
  duplicates the rail's own head. Both leave the glance line.
- **session id** (`view.ts:qAmFf`) moves behind the `▾` disclosure (`▾` is the existing
  fold chevron, `media/chat.css:ndUEI`).

### 1.2 Three chrome bands → one

**Now.** `.phead` (2 rows, `media/chat.css:8sYdY`) + `#modes` (`chat.ts:XilT1`, `.modes`
`media/chat.css:Y6lrk`) + `#ribbon.focusbar` (`chat.ts:et41X`). The ribbon's only
button is "Show all" (`chat.ts:renderRibbon`, `:2Ujpw`) — a **duplicate** of the All/Focus
switch six lines above it; the rest of the ribbon is a sentence explaining a control
that already says it.

**v3.** The All/Focus control moves **into** the header as a `.seg` (cell 2). `#modes`
and `#ribbon` are deleted, along with `renderRibbon`. Three stacked bands become the
header's own row. Nothing is lost: the mode is still one click, the header still names
the target in Focus.

### 1.3 The team: three regions → one home + one pill

**Now.** The team renders in three places: the header's `{n} members`
(`view.ts:vpYZ3`) + the target chip (`view.ts:U2map`); the `.roster` rail
(`chat.ts:iQ1Ve` / `reducer.ts:tAJNC`); and the transcript-tail `.group` box
(`chat.ts:aee9V` / `view.ts:KeUWa`).

**v3.** **One home: the rail** (`.side`). It is durable, it already carries state +
`liveAction` + the todo plan (`reducer.ts:tAJNC`), and it does not retarget.
Changes to it:

- the rail row drops the 18px `.mav` swatch box (`media/chat.css:rEw75`) for a **`--sw`
  coloured left edge** (2px, `3px` on the target) + the **state glyph** + name + dim
  action. Identity is the edge colour; state is the glyph's colour. That is **the same
  language the transcript turn rail already speaks** (`.turn.wcode .rail`,
  `media/chat.css:4XTIL`), so the two regions read as one product. The swatch box
  survives only in the collapsed strip, where a 22px chip needs a fill.
- **the `✓ ✗ ○` glyphs get their call site** (audit finding). `⠋` while running, `✓`
  done, `✗` failed, `○` idle (`reducer.ts:RNWdV`) — the same four states the TUI uses.
- **two echoes, each with its own fact.** (a) **Identity:** the header's target chip
  (cell 1), which names the member being read and toggles the menu. (b) **Liveness,
  aggregate:** the **working pill** (below). The header count is gone; the `.group` box
  is gone. Neither echo repeats the other, and neither repeats the rail:

  | surface | the fact it owns |
  |---|---|
  | the rail (home) | **per member**: its state glyph, its `liveAction` |
  | the header chip (echo 1) | **which member** the transcript is showing |
  | the working pill (echo 2) | **the aggregate**: how many OTHERS are working, and a way to jump to them |

**The `.group` box** (`chat.ts:aee9V`, `media/chat.css:uaEAR`) is removed because the
rail is the home, and its replacement is the **working pill**: the durable "typing"
indicator a chat app puts above its input.

#### The working pill (the durable typing affordance)

- **Anchor: an IN-FLOW band, its own row between the transcript scroller and the
  composer.** It is a **sibling of `.transcript`** (not a child of the scroller), so it
  never scrolls away, but it is **in flow**: it occupies real layout height and **never
  overlaps rendered transcript content** at any scroll position or docked width. It is
  the thing you look at when you are about to type.
  - *Rejected: a floating / `position:absolute` pill.* Any overlay **covers rendered
    content** — a correctness problem, not a taste one.
  - *Rejected: a sticky row inside the transcript scroller.* A tail row lives **inside**
    the scroller: it gets pushed by new content and competes with the transcript's own
    auto-stick (`chat.ts:aNXcu` `nearBottom`).
- **Avatars: an overlapping stack, up to 3, then `+N`.** Each chip is a 20px `.mav`
  (`background: color-mix(--sw 20%)`, `color: --sw`, ring `0 0 0 2px` in the pill's own
  background) carrying `memberInitial` (`view.ts:TJpz6`: the root's `❯`, else the
  initial). The 2nd and 3rd chip carry `margin-left: -7px` (a ~35% overlap). The 4th
  and beyond collapse into a single neutral `+N` chip in
  `--vscode-descriptionForeground` (no swatch: it is a count, not a member). **3** is
  the cap because the pill is a glance, not a roster, and the rail is where the full
  list lives.
- **"working" + a motion.** The label is `{n} working` (`--wc-fs-ui`, the count at
  weight 600) followed by a `⋯` (a **locked-table glyph**, `docs/tui-design.md:34-50`)
  that pulses. This is the surface's **third** animation and it earns its place in one
  sentence: *"live" is the pill's entire message, and a pulse is what separates a
  typing indicator from a static label.* It is one `@keyframes` on one existing glyph —
  no new glyph, no new element — and it is `prefers-reduced-motion` gated (static, still
  visible, when reduced).
- **A real button.** The pill is a `<button>` with a visible `:focus-visible` ring, a
  hover state and an `aria-label` (`"Show the newest working step ({n} members
  working)"`). **Click reveals the live step**, and the mechanism is client-local (no
  new `FromWebview` variant): in **All** mode it scrolls the transcript to the newest
  `data-live` element; in **Focus** mode, where the other members' steps are not in the
  transcript, it first switches to **All** and then scrolls. One affordance ("see what
  is happening"), two mechanisms. **`data-live` is a client-local DOM attribute V7 ADDED**
  (it is view state, not wire state — no protocol change): the live assistant block stamps
  it at `chat.ts:438`, the running tool fold at `chat.ts:449`, and the reveal reads it at
  `chat.ts:648`. It degrades to "scroll to the transcript bottom" when absent.
- **Count + membership: `workingGroup` unchanged.** The pill renders
  `workingGroup(state.members, state.target?.id ?? null)` (`view.ts:KeUWa`): a member
  other than the target with `state === "running" || liveAction !== undefined`. **No
  new wire data.** The avatar order is the roster order (root first).
- **Coexistence: it shows whenever any other member is working** — in **both** the
  rail-expanded and rail-collapsed states — because it is the *durable* typing
  affordance, not a degradation. That supersedes the old "only when the rail is
  collapsed" rule. The same fact is not shown twice: the rail owns the **per-member**
  rows, the pill owns the **aggregate**; that is the TUI's own split between the
  status band and the sidebar. The header chip stays a third, different fact (identity).
- **`--wc-*` only.** The pill uses the v3 tokens (`--wc-1…5`, `--wc-fs-ui`) and the
  `--vscode-*` palette; it is an **L2a** shape (a bordered, interactive card) like the
  other interactive surfaces.

### 1.4 The composer: 7 controls → 2 always, 4 at most

**Now** (`chat.ts:OghJl`…`:nFrit`): `@ selection`, `Mode: …`, a read-only `Model: …`
span (`:l8HCM`), a `spacer`, the hint `Enter to send` (`:JpHql`), `Send`, and `Stop` —
**always** both, even when nothing is running.

**v3.**

| control | verdict | why |
|---|---|---|
| `Send` | **keep** | always visible |
| mode | **keep** as a `.seg` (`Act`/`Plan`) | one mode control; the same `.seg` component as All/Focus |
| `@` | **keep**, low emphasis | the manual re-attach; the chip row is already conditional (`chat.ts:6IpIt`) |
| `Stop` | **conditional** — visible only while `status.running` | Esc already cancels (`chat.ts:RrLdG`); an idle Stop button is chrome |
| `Model: …` | **remove** → the header disclosure | a read-only span that never acts, and the same fact twice |
| `Enter to send` hint | **remove** | the placeholder (`chat.ts:PWRbr`) already says it, in full |

### 1.5 A real visual system

**(a) Spacing: a 4px grid.** `--wc-gap: 8px` is declared (`media/chat.css:MWQb5`) and
**never used**; the sheet carries ~35 distinct `padding` values and gaps of 4/6/7/8/18px.
v3 defines five steps and uses only those:

```
--wc-1: 4px   --wc-2: 8px   --wc-3: 12px   --wc-4: 16px   --wc-5: 24px
```

Rule: rows pad on `--wc-1`/`--wc-2` vertically, bands on `--wc-2 --wc-3`; `--wc-4` is
the between-turn rhythm (today's ad-hoc `18px`, `media/chat.css:YyF7c`); `--wc-5` is
reserved for a card's inner air (the state card). **Density is now one switch**: the
prototype's `dense` mode changes only these token values, which is the proof the scale
*is* the system.

**(b) Type: 10 sizes → 4.** The sheet uses `10 / 11 / 11.5 / 12 / 13px` plus
`0.88 / 0.92 / 0.94 / 0.95 / 1.02em`. v3 defines four:

```
--wc-fs-meta: 11px   --wc-fs-ui: 12px   --wc-fs-body: 13px   --wc-fs-code: .9em
```

`meta` = uppercase section labels, badges, tool meta, rail action lines. `ui` = header
cells, controls, rail rows, tables. `body` = prose (equal to `--vscode-font-size`, so
the panel tracks the editor's font size). `code` is relative so code tracks the body.
Hierarchy comes from **weight + colour + space**; a fifth size is not an option.

**(c) Components: lines vs cards.** Today `.fold` (`media/chat.css:tKorz`), `.code`
(`:yEea1`), `.hunk` (`:RSYck`), `.group` (`:uaEAR`), `.statecard` (`:ulRTF`), peer
messages (`:wMKbd`) **all** carry `border + radius + fill` — that is the wall of cards.
v3 defines **two levels** and says which is which:

- **Line (L1)** — structure: a 2px inset left rule, or a single hairline. *No box, no
  fill.* Thinking folds, tool folds, separators. `details.fold` loses its box and keeps
  its rule: **the fold is a feature and stays; its chrome was the problem.**
- **Card (L2)** — a shape with `--wc-radius`, used **only** for content that is
  *foreign or interactive*: code blocks, tool output, the change-review card, a peer's
  message, the state card, popovers/menus. L2 has exactly **two sub-forms**, and which
  one is used is not a judgement call:
  - **L2a bordered** — the boundary is a **`1px` hairline border**; the fill is
    **optional** (present where the content is a console, absent where the card sits on
    the panel background). → the code block, the change review (plus the accent left
    rule as a modifier), the state card, the popovers and menus (the bordered markdown
    tables are L2a too), the **working pill** (a bordered, interactive card), and the
    **tool output card** (V12, §1.5e — it gains a head, so it gains a border).
  - **L2b recessed** — the boundary is the **fill alone**, with the radius and **no
    border at all**. → **a peer's message** (`.body.peer`) — and nothing else. (The tool
    console *was* here; V12 moved it to L2a to give it a head.)

  A component is exactly one of L1 / L2a / L2b. There is no third treatment, and no
  component may mix them (the pre-v3 sheet did: `.out` was a borderless recess while
  `.code` next to it was a bordered box, and `.body.peer` was a recess with no border at
  all — three treatments for two levels).

The rule to write down: **an L2 boundary means "this is a distinct object you can act
on, or that arrived from somewhere else"; a LINE (or plain text) means everything
else. L2a draws that boundary with a border, L2b with a fill.**

**(d) One accent, five jobs.** `--wc-accent` (`media/chat.css:RAbMj`) currently paints
the target chip, the tool name (`:mPJSD`), the tool's left border (`:EhEGR`), links,
the spinner, the pressed mode, the selected rail row, and the working glyph. v3 writes
the policy down: **the accent means "active now"** —

1. the target chip (identity + the one interactive element in the header),
2. the pressed segment of a `.seg` (the active mode),
3. the running state: the `⠋` glyph (`g-run`), `running…`, the **working band's** `⋯`
   pulse (§1.3), and the **running tool row's** rule and `⚙` mark (§1.5e, V13),
   `⋯` pulse (§1.3),
4. links, the live cursor `▌`, and the **current** item in a list —
   `.cmdmenu .cmd-name` (`media/chat.css:561`), the "where you are / where to go" job,
5. the **change review's** accent left rule — the review card / its `.hunk`
   (`media/chat.css:871`) — the one actionable card in the transcript (§1.5(c)).

**Identity, not accent:** the two `var(--sw, var(--wc-accent))` fallbacks —
`.turn.wcode .rail` (`media/chat.css:191`) and `.who.wcode .av` (`:223`) — paint the
accent **only when a member has no swatch**. They are the *identity* channel's
last-resort default, not an accent paint; the CSS is left as-is.

**Removed to the neutral palette:** tool names become `--vscode-foreground` at weight
600 in mono (identity comes from the `⚙` glyph, not from colour); the tool fold's rule
becomes `--vscode-panel-border`; the context meter becomes monochrome
(`▰` in foreground, `▱` dim) — **V11 only**: the V6 default is the text `ctx 42k` with
no gauge at all (decision 12) — the v2 draft tinted it (`vscode-ui-v2-draft.html:typ0q`).

**Kept:** the six `charts-*` member swatches (`MEMBER_SWATCHES`, `view.ts:Y7N2r`,
`media/chat.css:G573m`). They are **identity**, confined to the rail edge, the turn
rail, the `.who` avatar and the collapsed strip — always at the existing low mix
(`color-mix(… 20%, transparent)`), never as prose colour. Semantic green/red stay for
`+N −M` (`:R1gHN`,`:pmb3R`) and the diff (`:MVZAP`,`:m3duW`) — they are semantic, not
decorative.

**(e) Tool activity: the call is a LINE, the output is a CARD (V12).** The user's read of the
shipped surface: *"the render of tool call and output feel lack of identification."*
Grounded: the tool fold is an **L1 line** (`media/chat.css:413`) whose rule is explicitly
**neutral** (`:421`, comment `:418`) with a neutral `.tname` (`:468`), and the **thinking**
fold is *also* an L1 line (`:430`) — so an action and an aside read alike. The output is a
bare **L2b recess with no header** (`media/chat.css:502`, `chat.ts:481`), unlike the code
card, which has a `.chead` (`media/chat.css:525`).

*The call: identity inside L1, bought by lowering the aside.*

- the tool fold **stays L1** — promoting it to a card would re-create the wall of boxes v3
  removed (the fold is a summary row, not a body);
- `.tname` (the `⚙ {tool}` mark, `media/chat.css:471`) goes **600 → 700**, still
  `--vscode-foreground`, still mono. The **`⚙` glyph is the action mark** and it is on every
  tool, never on a thinking block. **V13:** the mark is neutral for a *complete* tool,
  but it takes the status colour while the tool is running or has failed (below);
- **`thinking` is demoted**: its label drops to `--vscode-descriptionForeground` at weight
  **400** with an **italic** summary; its dim rule (`:430`) stays.
- *Rationale (the level/accent decision in one sentence):* the two rows are separated by
  making the **aside quiet**, which costs no chrome, no accent and no new level — raising the
  tool instead would have to borrow the accent, and a finished tool is not "active now".
*The status: all three states carry colour — GREEN / BLUE / RED (V13, revised by V13r).* A
tool row carries exactly one of three states, and **each state has a colour**. This
**overrides V13's "complete is quiet" rationale**: the user, having seen V13 shipped,
chose colour for the complete state, so the amendment is stated openly (V13r below).

| state | the L1 rule (`border-left`) | the `⚙` mark | the meta slot |
|---|---|---|---|
| **running** | `--wc-accent` | `--wc-accent` | `⠋ running…` in `--wc-accent` (the glyph spins) |
| **error** | `--vscode-errorForeground` | `--vscode-errorForeground` | `✗ failed` in `--vscode-errorForeground` |
| **complete** | **`--vscode-charts-green`** (green) | **`--vscode-charts-green`** | `+N −M · 38ms`, dim, no glyph |

- **Where the colour is expressed:** the **L1 rule** (the row's own edge — the most
  glanceable channel at zero chrome cost) **plus the `⚙` mark** (the row's identity glyph),
  and a **status glyph + word in the meta slot**. **No fill, no box:** the fold stays
  **L1** ("no box, NO fill") and no fourth treatment is introduced.
- **Complete is GREEN (V13r amendment).** *Rationale (one sentence):* the user asked for
  green / blue / red so the three states are readable by **hue alone**, which overrides
  V13's "most tools complete, so green would be a stripe" trade-off — and the risk is
  bounded because the green is the theme's own `--vscode-charts-green`, not a new colour.
- **The semantic colours are VS Code's, so the theme owns them.** `--vscode-charts-green`
  (complete) and `--vscode-errorForeground` (error) are host tokens; the **accent stays
  blue for running** (`--wc-accent`), so **running never reads as complete**.
- **The glyphs are the member-state vocabulary** (`memberGlyph`, `reducer.ts:RNWdV`):
  `⠋` running (it spins, `prefers-reduced-motion` gated), `✗` error. Only the exception
  states carry a glyph.
- **The change review does not clash.** A diff-bearing tool is *complete*, so its fold rule
  is **green** while the review card inside keeps its **accent (blue)** left rule
  (`media/chat.css:871`). They answer different questions at different levels: the fold's
  2px rule is the **row's status**, the card's 3px rule is **"this is actionable"**
  (accent job 5). Blue is never a tool *status* except running, so there is no ambiguity.
- **The yellow `.running-tag` is retired.** `media/chat.css:1107` paints `running…` in
  `--vscode-charts-yellow`; a running tool IS "active now", so it takes the accent like the
  rest of job 3 — a *reconciliation* of an inconsistency, not a new accent job.
- **Accent: §1.5(d) job 3 is CLARIFIED, not extended.** Job 3 already owns "the running
  state"; V13 names the running tool's rule and `⚙` mark as part of it, so the accent still
  has **five** jobs. The complete-state green is **semantic, not the accent**, so the one-accent
  rule holds: blue = running, green = complete, red = error.

*The output: L2b → L2a, because it gains a head.*

```
⚙ bash · output                                          Copy
running 145 tests
```

- the console becomes a **bordered card with a `.chead`** naming its producer and its kind
  (`⚙ {tool} · output`), mirroring the code card's `.chead` (`media/chat.css:525`); the `.mini` `Copy`
  button already has its call site (V8).
- *Rationale:* an anonymous recess cannot carry a header, so identification forces the
  level change — the console moves **L2b → L2a**, and there is still exactly one card
  treatment in the system.
- **Exactly one body per fold.** A diff-bearing tool renders the existing `.review` card
  (L2a) as the body; a plain tool renders the new output card (L2a). Either way the fold's
  body is **exactly one L2a**, the fold stays **L1**, and no card nests in a card.

**(f) Transcript composition: the turn gets depth (V14).** The shipped transcript reads
flat because **every turn is the same shape**: `.turn` is a grid `14px 1fr`
(`media/chat.css:295`), the rail is a 2px `border-left` (`:301`), `you`/`wcode`/`err` differ
**only** by that border's colour (`:305`/`:309`/`:313`), `.who` is an 11px dim label
(`:317`), and `.transcript` uses **one uniform gap** (`--wc-4`, `:286`). The user picked
**direction A** from `design/vscode-transcript-draft.html`; it is folded in here as V14.
Depth comes from **composition, not boxing** — there is **no new level** and no new card:

1. **The turn head.** `.who` becomes a real head: the name at `--wc-fs-ui`/**700** with a
   **hairline rule** running from the name to the right edge (`::after`, an **L1 line**),
   instead of a dim 11px label floating above the body.
   *Rationale: the flat render gives a turn no beginning — a head rule gives the eye an
   anchor and turns a list into a sequence.*
2. **The spine.** The 2px `border-left` becomes a **3px rounded coloured spine** (the turn's
   `--sw` for a member, dim for `you`, red for `err`), i.e. a *shape* rather than a hairline.
   *Rationale: a 2px border reads as a table rule; a 3px rounded spine reads as the turn's
   own edge, which is what makes a turn a unit.*
3. **Conversation alignment.** `you` turns **indent** (`--wc-5`) and take a **quiet fill**
   (`--vscode-textBlockQuote-background`, **neutral, NOT the accent**). **The fill sits on
   the TURN itself** (`.turn.you`), with **no content wrapper and no DOM change**: the
   shipped `renderTurn` emits a class-less content div (`chat.ts:619` `el("div", null)`), so
   a wrapper class would be dead. The turn gets **no left padding**, so the 3px spine sits
   **flush at the fill's own left edge**; the radius clips both.
   *Rationale: alignment is the cheapest "this is a dialogue" signal there is, and the fill
   marks your own words without spending the accent.*
4. **A role-aware rhythm.** A **speaker change** gets a bigger beat (`--wc-4` margin-top on
   `.turn.new`), a **continuation** stays tight (`--wc-2`).
   *Rationale: one uniform gap makes a continuation look like a new speech; spacing that
   tracks who is speaking creates grouping at zero chrome cost.*
5. **The running fold's faint wash** — `color-mix(--wc-accent 6%)` on a `[data-live]` tool
   fold. *Reconciles with V13r:* the wash is the **accent (blue)**, i.e. "active now", which
   is exactly the running state, so the wash and the running rule are the **same** blue and
   never compete with the complete-green or the error-red.

**No wall of cards.** The turn **internals** (the L1 folds, the V12 output card, the change
review) are **unchanged**; V14 touches only the turn's *composition*. The one new *shape* is
the spine, which replaces an existing L1 rule. **No new element and no new wrapper:**
`.turn`/`.rail`/`.who` are already emitted (`chat.ts:568`) and the fill goes on `.turn.you`;
the **only** change to `renderTurn` is the **`.turn.new` class** on the turn (a speaker
change), which the shipped template (`turn ${role}${sw}`) does not emit yet.

### 1.6 What is KEPT / MOVED / REMOVED

| element (shipped) | anchor | verdict |
|---|---|---|
| state dot | `chat.css:jQK5r` | **removed** → merged into the identity state glyph |
| state word (`ready`) | `view.ts:BV3Nc` | **removed** when ready; returns only for `starting`/`stopped`/`crashed` |
| `·` separators | `view.ts:4l8Od`, `chat.css:Il2nW` | **removed** (space separates; the `·` budget is 1 per metadata line) |
| session id | `view.ts:qAmFf` | **moved** → the `▾` disclosure |
| target chip | `view.ts:U2map` | **kept** in Focus, `hidden` in All (`chat.ts:YoBUD`) — **echo 1 of two** (identity); the working pill is echo 2 |
| plan chip | `view.ts:bsBjE` | **removed** (duplicate of the composer's mode control) |
| `running…` cell | `view.ts:pQS7R` | **removed** (the glyph + the fold tag carry it) |
| `lastError` cell | `view.ts:DBXdl` | **removed** from the header; the failure **already** renders as the transcript's `error` block (`reducer.ts:306`, `.body.error` `media/chat.css:348`) — nothing new is added |
| model cell | `view.ts:8Ft2X` | **moved** → the disclosure |
| `ctx N` cell | `view.ts:cj7nC` | **kept** → a monochrome meter; the window is ✗ new |
| `{n} members` cell | `view.ts:vpYZ3` | **removed** (the rail's head shows the count) |
| `#modes` band | `chat.ts:XilT1` | **moved** → into the header |
| `#ribbon` + "Show all" | `chat.ts:et41X`, `:2Ujpw` | **removed** (duplicate control; the sentence goes) |
| `.roster` rail | `chat.ts:iQ1Ve` | **kept** — the team's ONE home; the row is re-composed |
| `.group` box | `chat.ts:aee9V` | **replaced** by the **working pill** (an **in-flow band** above the composer, never an overlay: avatars + `{n} working` + a pulse, a button that reveals the live step) |
| `memberGlyph` `✓ ✗ ○ ⠋` | `reducer.ts:RNWdV` | **kept, now rendered** (header + rail) |
| `.chipbtn`, `.mini` | `chat.css:Cnla7`, `:aLUf2` | **removed** (dead: the former) / **kept** (`.mini` gets a real call site: the code card's `Copy`) |
| `Model:` span | `chat.ts:reM8I` | **removed** → the disclosure |
| `Enter to send` hint | `chat.ts:JpHql` | **removed** (the placeholder says it) |
| `Stop` always-on | `chat.ts:nFrit` | **conditional** (running only) |
| `--wc-gap` | `chat.css:MWQb5` | **replaced** by `--wc-1…5` |
| tool output `<pre class="tool-output">` | `chat.ts:481`, `chat.css:502` | **promoted** L2b → **L2a**: it gains a `.chead` (`⚙ {tool} · output` + `Copy`), so it is no longer anonymous |
| `.tname` (the `⚙ {tool}` mark) | `chat.css:471` | **kept**, weight 600 → **700** (identity by weight, not accent) |
| `thinking` fold label | `chat.css:430` | **demoted**: `--vscode-descriptionForeground`, weight 400, italic summary (so a tool no longer reads like an aside) |
| tool fold rule (`details.fold.tool`) | `media/chat.css:421` | **kept L1**, but the colour becomes the **STATUS**: **green** (complete, V13r) / **accent blue** (running) / **error red** |
| `.fold .running-tag` (yellow) | `media/chat.css:1107` | **recoloured** to `--wc-accent` + the `⠋` glyph; the yellow is retired (folded into accent job 3) |
| tool meta slot | `chat.ts:575` | **extended**: `⠋ running…` / `✗ failed` for the exception states; the done `+N −M · 38ms` is unchanged |

### 1.7 Frictions (where v3 fights the shipped code)

1. **The two-row header is structural, not cosmetic.** `PanelHeader` has `r1`/`r2`
   (`view.ts:46-49`), `panelHeader` fills both, `headerSignature` (`chat.ts:6QLvB`) and
   `renderHeader` (`chat.ts:hcKrh`) walk both, and `editors/vscode/test/view.test.ts`
   asserts the `r2` cells. v3 changes the type to one `cells` array — a small,
   mechanical diff, but it is not free.
2. **The `▾` disclosure is a new popover.** The panel has exactly one popover today
   (the target menu, `.menu` `media/chat.css:8JPZU`) with the APG roving-tabindex
   pattern (`chat.ts:kMQ7o`). v3 reuses that pattern; the second popover must not
   invent a second one.
3. **No timestamps on the wire.** The v1/v2 drafts render `.stamp` (`vscode-ui-draft.html`),
   but nothing carries time across the protocol. **v3 renders no timestamps** — the
   drafts' stamps are fiction. (`media/chat.css` never had `.stamp`; it is draft-only.)
4. **The `.stamp` / `.sc-head` "dead code"** named in the audit is dead *in the
   drafts*, not in `media/chat.css` — a grep finds neither. Only `.chipbtn` (`:Cnla7`)
   and `.mini` (`:aLUf2`) are dead CSS in the sheet. Recording the correction.
5. **`.body.notice` and `.body.btw` have no CSS rule.** `classNames` emits `body notice`
   and `body btw` (`view.ts:2wb8h`, `:VPBGw`) and neither is styled. **Decision: fold
   them into ONE treatment** — `.body.notice, .body.btw` share a single **L1** rule (a
   dim inset line: `--vscode-descriptionForeground`, a 2px dim left rule, no card),
   because both are host-rendered, non-prose, non-interactive asides and v3 draws
   structure by L1/L2, never by a fourth treatment. The two classes stay **distinct** in
   the contract (the renderer still chooses one), so this is a CSS-only fold. `.body.notice`
   is also where non-error notices land, so the class stops being a no-op.
6. **The context window is ✗ new**, so the meter's denominator does not exist. v3
   designs the *slot* and **the V6 default is the plain text `ctx 42k`**: no `▰`,
   no denominator, no `▰▰▰▱▱ 42 / ?`. `.meter` is a text token, and the `▰▰▰▱▱`
   gauge bar exists **only** behind V11 (the prototype carries it as a labelled
   preview, never as the default). The shape matches what the in-flight review SKETCH
   at `view.ts:89` proposes. **The sketch is not shipped and
   v3 does not assume it.** Same for **effort** (✗ new) in the disclosure.
7. **The working pill is an in-flow band; it does not overlay the transcript.** This
   **supersedes** the earlier floating-pill design. Because the band is a sibling of
   `.transcript` inside `.main` (already a flex column), it needs **no positioned
   wrapper**: `.main` lays out `.transcript` (flex, scrolls) then the band then the
   composer, so the band pushes content instead of covering it, and the
   `nearBottom` auto-stick (`chat.ts:aNXcu`) is untouched. The band is outside the
   scroller, so it still never scrolls away. The reveal (below) needs nothing extra: it
   targets a `data-live` element already in the transcript and calls `scrollIntoView`.

---

## 2. Feasibility (grounded in the shipped code)

`✔ existing` = the data already reaches the webview; `◑ compute` = a pure function over
existing data; `✗ new` = new plumbing or a decision.

| change | mechanism | verdict |
|---|---|---|
| one-row header | `panelHeader` returns `cells` instead of `{r1,r2}`; `renderHeader`/`headerSignature` (`chat.ts:hcKrh`,`:6QLvB`) walk one array; `view.test.ts` updated | ◑ small mechanical change |
| identity state glyph | `memberGlyph` (`reducer.ts:RNWdV`) is already imported by `chat.ts`; the target member is `state.members.find(…)` (as `view.ts:PKgNN` does) | ✔ data exists; ◑ a new call site |
| All/Focus in the header | `mode` already crosses the wire (`webview.ts:nrE1F`); the control is the existing `#modes` markup moved | ✔ (delete `#modes`, `#ribbon`, `renderRibbon`) |
| `▾` disclosure (session/model) | `PanelSessionInfo.id` (`webview.ts:MrwiE`) + `SessionMember.model` (`reducer.ts:lUtUt`) are on the wire; reuse the `.menu` popover + roving tabindex (`chat.ts:kMQ7o`) | ✔ data; ◑ one new popover |
| error already in the transcript | the reducer's `error` case pushes an `error` block (`reducer.ts:306`) **and** sets `status.lastError` (`:310`); `render.ts:114` paints `.body.error` (`media/chat.css:348`) | ✔ **already shipped** — V10 adds nothing; it only drops the header cell (V6) and folds `.notice`/`.btw` |
| rail row: `--sw` edge + state glyph | `memberSwatch` (`view.ts:Kg1gg`) + `memberGlyph` (`reducer.ts:RNWdV`) both exist; `reducer.ts:tAJNC` is unchanged | ✔ CSS + one DOM tweak |
| remove the `.group` box | drop `renderGroup` (`chat.ts:aee9V`); keep the predicate from `workingGroup` (`view.ts:KeUWa`) | ◑ delete only |
| the **working pill** | `workingGroup` (`view.ts:KeUWa`) already returns the rows; the avatars reuse `memberSwatch`/`memberInitial` (`view.ts:Kg1gg`,`:TJpz6`); the pill is an **in-flow band**, a sibling of `.transcript` inside `.main`'s existing flex column (outside the scroller, so it never scrolls away; in flow, so it never overlays); the click is a **client-local scroll** (All) or a mode switch then a scroll (Focus) | ✔ no new wire; ◑ one new band, no wrapper |
| spacing + type scales | new `--wc-1…5`, `--wc-fs-*` in `:root` (`media/chat.css:FGqrn`); every padding/gap/font-size re-pointed | ✔ CSS only |
| line-vs-card | `details.fold` box → rule (`media/chat.css:tKorz`); each component assigned ONE L2 sub-form (L2a bordered: `.code`, `.hunk`/`.review`, `.statecard`, `.menu`/`.pop`, `.body table`, `.pill` when V7 lands, and the **tool output card** when V12 lands; L2b recessed: `.body.peer`) — no third treatment | ✔ CSS only |
| accent policy | `--wc-accent` call sites audited; the tool name + meter + tool rule move to neutral tokens | ✔ CSS only |
| composer 7 → 4 | delete the `Model:` span + hint (`chat.ts:reM8I`,`:JpHql`); gate `Stop` on `status.running` (`chat.ts:nFrit`, `reducer.ts:tDvbE`) | ✔ data exists |
| delete dead `.chipbtn` | `media/chat.css:Cnla7` (+ `:hover`,`:focus-visible`) | ✔ CSS only |
| `.body.notice` / `.body.btw` (folded into ONE L1 rule) | `view.ts:2wb8h`,`:VPBGw` already emit both classes | ✔ CSS only |
| context meter **gauge + denominator** (`▰▰▰▱▱ 42k / 200k`); **the V6 default is the text `ctx 42k`** | `status.contextUsed` is the numerator; **the window is never serialized** (`limits::model_limit` is kernel-internal) | **✗ new (flagged)** — V6 ships `ctx 42k`; the gauge waits on V11 |
| effort in the disclosure | `SetEffort` exists; the current effort is not on the wire | **✗ new (flagged)** — the row is hidden while absent |
| tool status colour (running / error / complete) | the DOM already carries the state: `.fold.tool.error` (from `block.tool.isError`) and `[data-live]` (set when `!tool.done`, `chat.ts:449`); `RenderedTool.done`/`isError` (`render.ts`) need no change | ✔ no new wire; ◑ CSS + the meta content |
| transcript composition (V14: head, spine, alignment, rhythm) | all CSS over the existing DOM: `.turn`/`.rail`/`.who` are already emitted by `renderTurn` (`chat.ts:568`); the `you` fill goes on `.turn.you` (**no wrapper class**: `renderTurn` emits a class-less content div, `chat.ts:619`); the only new hooks are a `.turn.new` marker (a speaker change) and the `.who::after` hairline | ✔ no new wire; ◑ CSS + one class |
| tool output card (L2b → L2a) + the tool mark at weight 700 + `thinking` demoted | `RenderedTool.name` (`render.ts:QWZjR`) and `outputText` (`:iKvzO`) already reach the webview; the head reuses the existing `.card`/`.chead`/`.mini`; the rest is CSS only | ✔ no new wire; ◑ one head |

---

## 3. Phases (independently shippable commits)

**V8 is CSS-only and can lead**; V6 and V7 then land against the scale.

| phase | goal | touches | acceptance |
|---|---|---|---|
| **V6** | **One chrome row.** `panelHeader` → one `cells` array: state glyph + target chip + All/Focus `.seg` + spacer + ctx + `▾` disclosure. Delete `#modes`, `#ribbon`, `renderRibbon`. Move session/model into the disclosure and **drop the `lastError` cell** (the failure already renders in the transcript; see V10). The state word survives only for `starting`/`stopped`/`crashed`, and `crashed` reads `✗ crashed` (the error glyph, not a recoloured spinner). The target chip is hidden in All mode (`chat.ts:YoBUD`); cell 1 is then the root's glyph alone. **The meter default is the text `ctx 42k`** (no gauge, no denominator). | `webview/view.ts`, `webview/chat.ts`, `media/chat.css`, `test/view.test.ts` | one chrome band above the transcript; All/Focus switches mode from the header; no `ready` word in the steady state; the disclosure opens on click and Enter, closes on Escape, and restores focus to its button |
| **V7** | **The team's one home, and the working pill.** The rail row becomes `--sw` edge + state glyph + name + action; the header count and the `.group` box go; the **working pill** is an **in-flow band** above the composer (avatars + `{n} working` + a pulse), a real button that reveals the live step. | `webview/view.ts`, `webview/chat.ts`, `media/chat.css` | the transcript contains no member-count and no `.group` box; a running member shows `⠋` in the rail; the pill is **in flow and never overlays transcript content** at any scroll position or docked width (no `position:absolute`); it shows **whenever any other member is working** (rail expanded **and** collapsed), with an overlapping avatar stack capped at 3 plus `+N`; clicking it scrolls to the newest live step (in Focus it switches to All first); it is keyboard-reachable with a visible focus ring; under `prefers-reduced-motion` the `⋯` is static |
| **V8** | **The visual system.** The two token scales; every padding/gap/font-size re-pointed; `details.fold` box → rule; every component assigned **exactly one** L2 sub-form (`L2a` bordered: `.code`, `.hunk`/`.review`, `.statecard`, `.menu`/`.pop`, `.body table`, `.pill`; `L2b` recessed: `.out`, `.body.peer`) with no third treatment; the accent policy applied; `.chipbtn` deleted; `.body.notice`/`.body.btw` styled; `.mini` given its `Copy` call site. | `media/chat.css`, `webview/chat.ts` | ☑ shipped `3fcd09c` (second-layer APPROVED); a §8 pre-flight pass, no bare `font-size:` px outside `:root`. **V12 later supersedes the L2b assignment here:** `.out` is promoted to L2a and L2b is `.body.peer` only |
| **V9** | **The composer.** `Model:` span and the hint removed; `Stop` gated on `status.running`; the mode is a `.seg` (the same component as All/Focus). | `webview/chat.ts`, `media/chat.css` | idle shows `@`, mode, `Send`; running adds `Stop`; a keyboard-only pass reaches every control with a visible focus ring |
| **V10** | **The notice treatment (no new error row).** The run failure is **already** carried by the transcript's `error` block (`reducer.ts:306` → `.body.error`, `media/chat.css:348`), so V10 adds **nothing** for it: the redundant header cell is removed in V6, and V10 only gives `.body.notice`/`.body.btw` the one folded L1 treatment (friction 5). | `webview/chat.ts`, `media/chat.css` | a notice and a btw render as the same dim inset line; the header is clean; the run failure appears **exactly once**, via the existing `.body.error` block (no duplicate row) |
| **V11** | **(wire, flagged) The gauge + effort.** `contextWindow` + `effort` on the wire fan out to `SessionMember`; the meter renders `▰▰▰▱▱ 42k / 200k`; the disclosure gains an effort row. While absent, both degrade (numerator-only `ctx 42k`; the row hidden). | `wcode-harness`, `wcode-cli`, `wcode-protocol`, the extension | the meter shows `used / window` when known and `ctx 42k` when not; nothing renders `▰▰▰▱▱ 42 / ?` |
| **V12** | **Tool activity identification.** The tool fold keeps its **L1** rule but its `⚙ {tool}` mark goes to weight **700**, and `thinking` is demoted (dim, weight 400, italic); the tool output is promoted **L2b → L2a** into a card whose `.chead` names it (`⚙ {tool} · output`) with a `Copy` `.mini`. | `media/chat.css`, `webview/chat.ts` | ☑ shipped `80ebb2a` (second-layer APPROVED); a tool line reads at full strength and a thinking line reads quiet; the output is a labelled card with a Copy button; a diff-bearing tool still shows **exactly one** `.review` card as its body; no fourth treatment and no nested card |
| **V13** | **Tool status colour.** The tool row's **L1** rule and `⚙` mark take the **status**: **accent** running, **errorForeground** error, **neutral** complete; the meta slot shows `⠋ running…` / `✗ failed` for the exceptions only (complete keeps `+N −M · 38ms`); the yellow `running-tag` is retired to the accent. | `media/chat.css`, `webview/chat.ts` | ☑ shipped `38a3a42`; a running, a failed and a completed tool are distinguishable **by colour alone at a glance**; the fold stays **L1** (no box, no fill); the `⠋` spin is reduced-motion gated. **Superseded on the complete-state colour by V13r** |
| **V13r** | **Tool status colours, revised: GREEN / BLUE / RED.** Complete is no longer neutral: the rule and the `⚙` mark take **`--vscode-charts-green`** (a semantic host token, not the accent). Running stays **accent blue**, error stays **red**. The change review's accent left rule is reconciled as a *different channel* ("actionable", not "status"). | `media/chat.css` | the three states read by **hue alone** (green / blue / red); the accent still has **five** jobs (green is semantic); running never reads as complete; a diff-bearing (complete) tool shows a green row rule with the review's blue actionable rule inside, and the two do not read as two statuses |
| **V14** | **Transcript composition (§1.5f).** The turn head (`.who` → name at `--wc-fs-ui`/700 + a hairline to the right edge); the **3px rounded coloured spine** replacing the 2px `border-left`; **conversation alignment** (`you` indents `--wc-5` + a neutral fill); a **role-aware rhythm** (`--wc-4` on a speaker change, `--wc-2` on a continuation); the running fold's faint accent wash. | `media/chat.css`, `webview/chat.ts` | a speaker change is visually distinct from a continuation; `you` turns are indented and filled; the turn head has a hairline; the spine is 3px and rounded; the turn **internals** are unchanged and **no card is added** (no wall of cards) |

---

## 4. Decisions (settling the open questions)

1. **The header is one row, and it answers three questions only: who, is it working,
   how big is the context.** Everything else is one click behind `▾`. *Amends* the
   two-row header of `vscode-ui-rework-plan.md` §2/§4 and `vscode-ui-v2-plan.md` §0.
2. **The All/Focus control lives in the header.** *Amends* the separate `#modes` band
   (`vscode-ui-v2-plan.md` §0 item 3). The `#ribbon` is deleted with it — its "Show all"
   was the same control twice and its sentence restated the control's own label.
3. **The team has ONE home (the rail) and TWO echoes.** The rail owns the **per-member**
   rows (state glyph + `liveAction`) — that is the detail. Echo (a), the header's target
   chip, owns **identity** (which member you are reading). Echo (b), the **working pill**,
   owns the fact the rail does **not** state even when expanded: the **aggregate count**
   and the **jump affordance** ("reveal the newest live step"). With the rail expanded
   the pill's avatars largely restate the rail's working rows — that redundancy is
   deliberate (the pill is the *durable* typing cue, the rail is the *detail*), but the
   pill's genuinely new content is the count plus the jump, so it is not a pure repeat.
   *Amends* `vscode-ui-rework-plan.md` §5 decision 3, which said "one home, one echo"
   but shipped with three renderings. The header count and the transcript `.group` box
   are removed; the pill is `workingGroup` (`view.ts:KeUWa`) with the predicate
   unchanged and **no new wire data**.
4. **Errors live where they happened — and they already do.** The transcript's `error`
   block (`reducer.ts:306` → `.body.error`, `media/chat.css:348`) is the failure's home;
   v3 only **removes the redundant header cell** (V6) and adds no new row. `status.lastError`
   (`reducer.ts:310`) **stays a reducer field** — it is the status record, not a render
   source — but is no longer rendered separately. A header that holds an error forever
   stops being glanceable.
5. **Lines vs cards, with two named card sub-forms.** A component is exactly one of
   **L1** (a line: a rule, no box, no fill), **L2a** (a bordered shape: a `1px` hairline
   plus `--wc-radius`; the fill is optional) or **L2b** (a recessed shape: the fill plus
   the radius, no border). Folds are L1; the code block, the change review, the state
   card, the popovers, the bordered tables, the working pill and the tool output card are
   L2a; a peer message is L2b and **nothing else**. There is
   no third treatment and no mixing, so no implementer has to guess. One radius token.
6. **Density is a token switch, not a second design.** `[data-density="dense"]` changes
   `--wc-1…5` and `--wc-fs-*` and nothing else — which is also how the system proves it
   is a system.
7. **No timestamps until the wire carries time.** The drafts' `.stamp` is fiction; v3
   renders none.
8. **The accent means "active now"** — target chip, pressed segment, running state,
   links, live cursor. Tool names, the tool rule and the meter are neutral.
9. **Nothing new is assumed on the wire.** V11 is flagged, optional, and degrades:
   `ctx 42k` without a window, no effort row without effort.
10. **In All mode, cell 1 is the root's state glyph and nothing else.** The target chip
    is `hidden` (`chat.ts:YoBUD`), because the merged transcript belongs to nobody in
    particular; per-member identity is carried by each turn's `.who` line
    (`chat.ts:KoM4E`). In Focus mode, cell 1 is the glyph **plus** the chip. The
    header therefore has a defined shape in **both** modes.
11. **A failure is a glyph change, not a colour change.** On `crashed` the header glyph
    is `✗` (`g-err`, `--vscode-errorForeground`) **plus** the word `crashed`; on
    `starting` a spinning `⠋` plus the word. A frozen recoloured spinner is not
    enough, and a healthy session carries no word at all.
12. **The V6 meter is the text `ctx 42k`.** No `▰`, no denominator, no
    `▰▰▰▱▱ 42 / ?` — the gauge form is a V11 preview behind a labelled control, never
    the default, so the plan and the prototype cannot disagree.

---

## 5. Non-goals / frictions

- **No kernel behaviour config** (`AGENTS.md`). V11 adds fields, not policy.
- **No second theme**, no light/dark toggle in the surface: VS Code owns the theme, and
  the prototype's light mode exists only to prove nothing is hardcoded.
- **Not the workbench chrome** (activity bar, title bar, editor tabs, status bar). The
  v1 draft mocked it (`vscode-ui-draft.html`, `.titlebar`/`.activitybar`); v3 shows only
  the wcode surface.
- **Zero remote assets**, `--vscode-*` for colour only, one accent, no `—` in any
  rendered string.
- **Not a new IA.** The panel stays transcript + composer with the rail on the left; the
  sidebar keeps Team + Tasks (`chat.ts:iQ1Ve`,`:X9nZp`). Sessions stays out of scope
  (v2 plan §6).
- **The pickers** (`/model`, `/effort`, `/resume`, the `/` menu) are unchanged; v3 only
  moves *where their current values are displayed*.

---

## 6. Progress

| phase | status |
|---|---|
| V6 one chrome row | ☑ `4c4735d` |
| V7 the team's one home + the working pill | ☑ `8663d89` |
| V8 the visual system | ☑ `3fcd09c` |
| V9 the composer | ☑ `a14b641` |
| V10 the notice treatment | ☑ `d66cc4f` |
| V11a wire gauges + effort | ☑ `8896332` |
| V11b client gauge + effort row | ☑ `f37dba4` |
| V12 tool activity identification | ☑ `80ebb2a` |
| V13 tool status colour | ☑ `38a3a42` |
| V13r tool status colours revised (green / blue / red) | ☐ proposed |
| V14 transcript composition | ☐ proposed |
