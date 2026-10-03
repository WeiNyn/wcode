# VS Code surface — v2 plan (one dockable surface)

**Status:** design approved (the sample `design/vscode-ui-v2-draft.html`); V1 not started.
**Companion docs:** the sample is [`design/vscode-ui-v2-draft.html`](design/vscode-ui-v2-draft.html);
the rationale is [`design/vscode-ui-exploration.md`](design/vscode-ui-exploration.md);
the shipped P1–P5 rework is [`vscode-ui-rework-plan.md`](vscode-ui-rework-plan.md).

---

## 0. What changed in v2

The first test pass produced five changes plus three additions:

1. **One surface, dockable anywhere** — the whole UI (team + tasks + transcript +
   composer) lives in **one webview** that can sit in the primary/secondary
   sidebar, the bottom panel, or an editor tab. The native `wcode.members` tree is
   retired; the team list moves **into** the surface.
2. **The team list is inside the surface** (with Tasks).
3. **All / Focus view mode** — All shows every member's activity; Focus shows only
   the focused member's.
4. **Run-following folds** — a thinking/tool fold opens while its step is RUNNING
   and collapses when it is DONE.
5. **Bordered tables** — re-add the markdown table rules (the P1 rework dropped
   them; a real regression).
6. **Pickers on the surface** — `/model`, `/effort`, `/resume` (+ a `/` menu).
7. **Context gauge** — `▰▰▰▱▱ used / window`.
8. **`/resume` semantics** — filter to the **current cwd**; label rows by the
   **first user message**, never the raw id.

## 1. Surface read (design-taste §0)

> Reading this as: a **web screen** (the VS Code webview), for a developer driving
> a multi-agent session inside VS Code, in **VS Code's own theme-variable
> language**, with the dominant constraint that the surface uses only `--vscode-*`
> tokens and zero remote assets.

Dials unchanged: `VARIANCE 3 · MOTION 2 · DENSITY 8`; one accent.

## 2. Feasibility (grounded)

| change | mechanism | verdict |
|---|---|---|
| one dockable surface | a **`WebviewViewProvider`** (the view moves between primary/secondary sidebar and the bottom panel; VS Code handles the move) **+** a **`WebviewPanel`** (an editor tab), both serving the SAME `media/*` + contract | ✗ new host wiring (retire `roster.ts`; split `panel.ts`) |
| team + tasks inside | `RenderedState.members`/`todos` already cross to the webview | ✔ data exists; ✗ render (the P5 tree model moves into the webview) |
| All / Focus | client-local mode; **All** needs a MERGED transcript across `ViewState.transcripts` (per session today) | ✗ a new merged-view model |
| run-following folds | derive `open` from the block's `live`/`tool.done` (open while live, collapsed once done) with a user override | ✔ small change to `chat.ts` |
| bordered tables | re-add `.body table`/`th`/`td` (dropped in P1) | ✔ CSS only |
| `/model` picker | `SetModel` exists; the **selectable-model list** is not on the wire | ✗ new wire field |
| `/effort` picker | `SetEffort` exists; the **current effort** is not on the wire | ✗ new wire field |
| context gauge | `Usage.input_tokens` is the numerator; the **window** (`limits::model_limit`) is never serialized | ✗ new wire field |
| `/resume` picker | the wire `SessionInfo` is `{id, model, state}` — no cwd, no preview | ✗ new wire surface (or the extension reads the sessions dir) |
| `/resume` cwd filter | sessions record `cwd` in the header (`session.rs:17`); the listing (`session_groups::list_groups`) does not filter | ✗ a CLI listing change (benefits the TUI too) |
| `/resume` label | the TUI label is `id · age · first line` (`app.rs:744`) | ✗ lead with the user message, drop the id |

## 3. Phases

| phase | goal | touches | acceptance |
|---|---|---|---|
| **V1** | **Surface polish.** Bordered tables; run-following folds (open while running, collapse when done, user override preserved). | `media/chat.css`, `webview/chat.ts` | a table renders bordered; a live fold is open and a finished one is collapsed; a manual toggle still sticks |
| **V2** | **One dockable surface.** A shared webview served by a `WebviewViewProvider` (sidebar/panel, user-movable) and a `WebviewPanel` (editor); the team + tasks render inside; the native tree is retired; a responsive layout. | `extension.ts`, new `surface.ts` (the view provider), `panel.ts` (the panel), `roster.ts` (retired), `webview/chat.ts`, `media/chat.css`, `package.json` | the surface opens in the sidebar (movable to the panel/secondary) and as an editor tab; the team + tasks render inside; no native tree |
| **V3** | **All / Focus.** A client-local mode; All = the merged activity of every member; Focus = the target. | `reducer.ts` (a merged view), `webview/view.ts`, `webview/chat.ts`, `media/chat.css` | All interleaves members (labeled); Focus shows only the target |
| **V4** | **Wire additions + pickers + gauge.** A model list, the current effort, and a context window on the wire; the `/model` `/effort` pickers, the `/` command menu, and the header gauge. | `wcode-harness` (a `Status`/`Sessions` field or a new event), `wcode-cli`, `wcode-protocol`, the extension | the gauge shows `used / window`; `/model` and `/effort` set and reflect |
| **V5** | **`/resume`.** The CLI listing filtered by cwd + a first-user-message preview; a wire surface for the extension + the picker. | `wcode-cli` (`session_groups`), `wcode-tui` (label), `wcode-protocol` (a `Request`), the extension | `/resume` lists only this cwd's sessions, labeled by the user message; picking re-execs/resumes |

Phases are independent commits; V1 is small and unblocks the visible regressions;
V2 is the architectural core; V4/V5 carry the wire additions.

## 4. Decisions

1. **One `WebviewView` + one `WebviewPanel`, one webview source.** The view is
   user-movable (primary/secondary sidebar ↔ panel) for free; the editor tab is a
   `WebviewPanel` from the same `media/*` + contract. The native tree is retired.
2. **All mode is a client-side merge** over the existing per-session transcripts —
   no protocol change (the extension already folds every session's events).
3. **Run-following folds:** a **tool** fold auto-opens while running and collapses
   once done, with a manual override that wins until the next state change (the
   override records the phase it was made in, so it expires at the running→done
   transition; a done fold's override sticks). A **thinking** fold auto-opens while
   its assistant block is live and collapses when done, with **no** manual override
   — the block is host-rendered and replaced on every update, so a client toggle
   could not survive; the auto behaviour is the whole feature there.
4. **The `/` menu is client-side**, routing to the pickers and the existing host
   commands; the pickers reuse the one overlay component.
5. **`/resume` cwd-filter + preview land in the CLI first** (the TUI benefits),
   then the extension gets the list over the wire.

## 5. Non-goals

- No new kernel behavior config (`AGENTS.md`); the wire additions are fields/events,
  not policy.
- Not the VS Code workbench chrome (the activity bar/editor tabs/status bar).
- No second theme or a light/dark toggle in the surface (VS Code owns the theme).

## 6. Progress

| phase | status |
|---|---|
| V1 | ☑ `3332998` — bordered tables + run-following folds; second-layer APPROVED (145 tests green) |
| V2 | ☑ `7c29470` — one dockable surface (WebviewView + editor panel; tree retired); second-layer APPROVED (131 tests green) |
| V3 | ☑ `51469b0` — All / Focus view mode (merged transcript, origin-bounded turns); second-layer APPROVED (135 tests green) |
| V4 | ☐ not started |
| V5 | ☐ not started |
