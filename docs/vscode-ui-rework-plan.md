# VS Code surface — UI/UX rework plan

**Status:** design locked; P1 not started.
**Companion docs:** the *why* is [`design/vscode-ui-exploration.md`](design/vscode-ui-exploration.md);
the *what* is the prototype [`design/vscode-ui-draft.html`](design/vscode-ui-draft.html);
the transport/client spine is [`vscode-extension-plan.md`](vscode-extension-plan.md)
(P0–P3 shipped). This plan refines plan §3.2/§3.3/§3.4/§3.5 into a UI-only rework.

---

## 0. Surface read (design-taste §0)

> Reading this as: a **web screen** (the VS Code webview panel), for a developer
> driving a multi-agent session inside VS Code, in **VS Code's own theme-variable
> language** (`--vscode-*`), with the dominant constraint that the panel uses
> **only `--vscode-*` tokens and zero remote assets**.

**Dials:** `VARIANCE 3` (a tool panel, enough recomposition to fix the flatness,
not a landing page) · `MOTION 2` (the spinner `⠋`, the live cursor `▌`, both
reduced-motion gated) · `DENSITY 8` (a work surface).

**Accent:** exactly one (`--vscode-charts-blue`), for wcode identity (target,
tool names, links). Member *state* is semantic (green done / red error / muted
idle), never decorative. Glyphs are the locked TUI vocabulary
(`❯ ⚙ ✓ ✗ ⋯ ▌ ⠋ ● ○`), reused deliberately so the editor surface and the
terminal read as one product.

## 1. What we are reworking

The extension contributes three surfaces; the rework touches the first two:

1. **The webview panel** (`media/chat.css` + `webview/chat.ts` + the pure
   `webview/view.ts` / `render.ts`) — the primary, expressive surface. **P1–P4.**
2. **The native sidebar tree** (`roster.ts`, the `wcode.members` view) — the
   durable "who". **P5.**
3. The activity bar / editor tabs / status bar in the draft are **VS Code's own
   chrome** (decorative in the mock); we do not build them.

## 2. The design is the spec

The prototype (`design/vscode-ui-draft.html`) is the visual/behavioral spec: the
two-row panel header, the turn-railed transcript, the folded thinking/tool rows,
the header-tagged code blocks, the composer control surface, and six screens
(chat · review · team · plan · approval · states). The exploration doc is the
rationale and the improvement levers, in order: typography → spacing → color →
motion → recomposition → new structure.

## 3. Feasibility: screen / element → data → verdict

Grounded in the current code. `✔ existing` means the data already reaches the
panel; `◑ compute` means a pure function over existing data; `✗ new` means new
plumbing (protocol / kernel / client I/O) or a decision.

| draft element | data source | verdict |
|---|---|---|
| header r1: state dot, `ready`, session id, target chip, plan chip | `PanelSessionInfo` + `RenderedState.status`/`target` (`panel.ts:b4faR`, `render.ts:59Wvn`) | ✔ existing |
| header r2: model · effort | target member `model` (`reducer.ts:xSLIN`); **effort is not on the wire** | ◑ model; ✗ effort (show model only) |
| header r2: context gauge `▰▰▰▱▱ 42k / 128k` | `contextUsed` exists (`reducer.ts:2BU6k`); **a context window is not exposed** | ◑ numerator; ✗ denominator |
| header r2: cache clock `⧗ 4m` | not tracked or on the wire | ✗ new (out of scope) |
| transcript turn grouping + rails + avatars | flat `RenderedBlock[]` (`render.ts:qFotY`) | ◑ compute (group into turns) |
| thinking fold | `thinking` ContentBlock → `<details>` (`render.ts:qga8T`) | ✔ existing |
| tool fold: `⚙ name target +N −M · 38ms` | `ToolBlock` name/summary/path/durationMs/diff (`reducer.ts:aW4oH`) | ◑ the `+N −M` is computed from `diff` |
| code block with a filename header | assistant markdown `<pre>` (rendered host-side) | ◑ wrap `<pre>` with a header in `render.ts` |
| working group (live subagents) | `members[].liveAction` + `state` (`reducer.ts:xSLIN`) | ◑ compute |
| composer: textarea, Send/Stop | shipped (`chat.ts:4096f`) | ✔ existing |
| composer: `Mode: Plan/Act` | `status.planMode` + `SetPlanMode` (`extension.ts:V7Mh6`) | ✔ existing |
| composer: `@file#line` chips, `/` + `@` menus | VS Code editor selection (client-side) + inline into `Submit{text}` | ✗ new client logic |
| change review: in-panel diff, accept/reject | `ToolBlock.diff` + `diff.ts::reverseApply` + `BeforeRegistry` (`diff.ts:yhTJL`, `:8OH6p`) | ◑ build on existing; **reject = a real revert** |
| team screen: agent map + member detail | roster + per-member transcript + `peek`/`ask`/`stop` already wired (`extension.ts:7t0K6`, `:U0F8O`, `:wmsB4`) | ◑ re-present existing verbs |
| plan review screen | `SetPlanMode` + a plan rendered from the assistant text | ◑ presentation |
| approval screen (P4 gate) | needs a CLI `Hooks` impl + one Request/event pair | ✗ new (kernel; P6, optional) |
| sidebar: Team / Tasks / Sessions | roster ✔, todos ✔, **on-disk sessions list ✗** (plan §6 open) | ◑ Team+Tasks; ✗ Sessions |

**Frictions named (not papered over):** (a) a wcode presentation diff is
**exactly one hunk** (`diff.ts:iwLrr`) — the draft's "2 of 2" hunks do not map to
it, so in-panel review is **one change per tool call**; (b) there is **no context
window** on the wire, so the gauge shows used tokens (a `▰▱` bar needs a window —
either a new `limits` field or a fallback); (c) **effort is not on the wire**, so
the header shows the model only; (d) the sidebar's Sessions section needs on-disk
session listing, a separate item.

## 4. Phases

| phase | goal | touches | acceptance |
|---|---|---|---|
| **P1** | **Panel chrome + transcript + states.** The two-row header, the turn-railed transcript, folded thinking/tool rows with `+N −M · ms`, header-tagged code blocks, the six empty/error states — to the draft. | `webview/view.ts`, `render.ts`, `webview/chat.ts`, `media/chat.css`, their tests | visual parity with the draft's Chat + States screens against the existing snapshot; pure helpers unit-tested; no protocol change |
| **P2** | **Composer control surface.** Context chips (`@file#line` from the editor selection) and a mode (Plan/Act) control mapping to plan-mode, plus a read-only model display. The context **gauge** (needs a context window) and a **model picker** (needs a selectable-model list) are NOT on the wire — deferred (see §6). | `chat.ts`, `view.ts`, `media/chat.css`, `extension.ts` (selection → submit), `webview.ts` (a new `FromWebview` arm) | a selection becomes a chip and rides into `Submit{text}`; the mode control flips plan-mode and reflects `status.planMode` |
| **P3** | **Working group + target switcher.** The live subagent rows and the header target chip opening a member list. | `view.ts`, `chat.ts`, `media/chat.css` | a running member appears with its `liveAction`; the chip retargets (hydrating) |
| **P4** | **Change review** (in-panel). A pure review model over a tool's diff; Accept keeps the applied edit, Reject reverts via `reverseApply`; "Open native diff" as the escape hatch. | new `src/review.ts` (+ tests), `chat.ts`, `media/chat.css`, `extension.ts` | a diff renders as a hunk; Accept is a no-op, Reject restores the before-image and says so |
| **P5** | **Sidebar rework.** Team / Tasks sections to the draft; Sessions deferred to its own item. | `roster.ts`, `reducer.ts` (tree model), `package.json` | the tree shows sections + `☑ done/total`; still push-only |
| **P6** | **Approval gate (P4 kernel)** — optional. A CLI `Hooks` impl that asks the client and blocks; one Request/event pair; the pending-queue UI. | `wcode-cli`, `wcode-protocol`, `chat.ts` | a risky edit blocks until approved/rejected; a rejection is fed back as a tool error |

Phases are independent commits; P1 is the foundation the rest extend.

## 5. Decisions (settles the exploration's §4)

1. **Panel home: editor tab** (the shipped webview panel), not the secondary
   sidebar — width for diffs and "a webview panel", as the plan said.
2. **Approval: design the inline card, ship the plan-gate.** The plan-gate
   (`SetPlanMode`) is the supported gate today; P6's card is a strict superset, so
   the card may be sketched now and wired only when P6 lands.
3. **Team shown once in the sidebar, the header chip is the active target.** The
   sidebar is the durable roster; the header chip names and switches the one being
   read. One roster source, two affordances — not duplication.
4. **Change review is in-panel primary; the native diff is the escape hatch.**
   The P2 `vscode.diff` provider stays and is linked from the review bar.
5. **One mode control.** Map the composer's `Mode:` onto wcode's existing
   plan-mode toggle (Plan/Act). No second, Cline-style toggle.

## 6. Non-goals / frictions

- **No kernel behaviour config** (`AGENTS.md`): the P6 gate is code (a `Hooks`
  policy), never a config file.
- **No new tokens beyond `--vscode-*`** and the one accent; **zero remote assets**
  (the panel CSP already forbids them — `panel.ts:BIgrz`).
- **Not the workbench chrome** (activity bar, editor tabs, status bar) — VS Code's.
- The Sessions sidebar section and the cache clock are **out of scope** here; they
  need new plumbing and get their own items if wanted.
- **Deferred to a protocol item:** the header **context gauge** (`▰▰▰▱▱ N / M`) needs a
  context window on the wire — `Usage.input_tokens` is the numerator only; the
  kernel has `limits::model_limit` but never serializes it to clients. A **model
  picker** (`Model: … ▾`) needs the selectable-model list, also not on the wire
  (`SetModel` exists; the list does not). P2 ships the model as a **read-only**
  display and the used-token count in the header; the gauge and the picker wait on
  a small `Request`/event addition.

## 7. Progress

| phase | status |
|---|---|
| P1 | ☑ `f276183` — panel shell + transcript + states; second-layer APPROVED (122 tests green) |
| P2 | ☑ `ae10444` — composer controls (selection chip, Plan/Act mode, model); second-layer APPROVED (125 tests green) |
| P3 | ☑ `b9ffdba` — working group + interactive target switcher; second-layer APPROVED (129 tests green) |
| P4 | ☑ `38099bc` — in-panel change review (Accept/Reject, reverse-apply revert); second-layer APPROVED (137 tests green) |
| P5 | ☑ `5f6e81a` — sidebar Team + Tasks sections (one view, section roots); second-layer APPROVED (142 tests green) |
| P6 | ☐ not started (optional) |
