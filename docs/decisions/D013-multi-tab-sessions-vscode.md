# D013 — VS Code: multiple editor tabs, each an independent wcode session

- **Status:** accepted (human sign-off 2026-10-10, this session)
- **Date:** 2026-10-10
- **Work item:** W010 ([`docs/work/W010-multi-tab-multi-session-vscode.md`](../work/W010-multi-tab-multi-session-vscode.md), tracker row 68)
- **Resolves:** `docs/sketches/vscode-extension-team.md:167`/`:335-337` — the
  `TODO(prose)` on **one-panel-retargets vs a panel per member**
- **Amends:** `editors/vscode/README.md:8` ("ONE dockable surface"),
  `docs/sketches/vscode-extension-team.md:141`

## Context

Today the extension is **one wcode session per workspace**: one module-level
`WcodeSession` (`extension.ts:42`), one `SurfaceController`, and a **singleton**
editor panel (`panel.ts:19` `private static current`), so `wcode: Open in Editor`
reveals rather than opens and `/new`, `/resume`, `/reload` all respawn the one
child (`extension.ts:249`). The ask is **several editor tabs in one workspace,
each an independent wcode session** — its own conversation, its own team — so
parallel work runs side by side.

The sketch's own `TODO(prose)` (`vscode-extension-team.md:167`) left the
panel-vs-per-tab question explicitly *unsettled*; this record settles it the
other way (a panel **per tab**) and pins the five sub-decisions the brainstorm
raised (A–E below).

## Decision

### Session model — Option A

**N independent `wcode serve --stdio` children, one per tab. No kernel or
protocol change.**

The stdio transport is deliberately one-client (`crates/wcode-protocol/src/server.rs:117-118`
— *"there is no listener, so no second connection is even expressible"*), and
`WcodeSession` is already instance-only state (`session.ts:139`), so N instances
already work. The alternative — one server hosting multiple *root* sessions via a
`Request::NewSession` — is deferred; it needs a wire change and gives nothing v1
needs. (The eventual "share a team across tabs" path is a per-workspace
`--socket` daemon with N connections, not B-over-stdio; out of scope.)

**Cost, stated honestly:** a tab is one root agent. An *idle* tab is one idle
agent — teams spawn lazily, so the brief's "N agents, N teams" framing overstates
the idle cost. The real cost is N *active* agents, which is exactly the parallel
work the feature exists to allow. Under Option A a worker spawned in tab A lives
in A's child and is **never visible to tab B** — tabs are independent by
construction, not by policy.

### The five sub-decisions (A–E)

| # | Question | Decision |
|---|----------|----------|
| **A** | What is the docked `WebviewView` in a multi-tab world? | **Active-tab mirror *with a session switcher at its head*.** VS Code exposes exactly one `WebviewView` per view id, so the sidebar cannot be N-up; it shows the **active tab's** surface (focus-following, falling back to the last active, then the first) and adds a **switcher list** of open sessions so a tab can be focused *from the sidebar*. The sidebar is reframed from "the primary home" to **the index** — it no longer pretends to be the only surface. Re-binding is the existing `surface.detach`/`attach` seam (`surface.ts:146`/`:158`). |
| **B** | How is a tab opened? | **A distinct `wcode.newSession` command** (opens a fresh session in a new tab) is **added**; `wcode.openInEditor` keeps its name but becomes "**open this session in an editor tab**" (ensure + focus an editor tab for the active session), no longer a reveal-only singleton. |
| **C** | Tab labels | **Sticky.** The label is set **once** from the root transcript's first user message (clipped), then **never re-derived**; *before* the first message it is the `--resume` path basename if any, else **`wcode N`** (the tab's index). A live-updating title is refused — a moving tab title makes tabs unidentifiable. |
| **D** | Survive a window reload? | **Deferred, recorded as a known gap.** A VS Code window reload / "Restore editors" does **not** currently bring the tabs (or their children) back; a future `WebviewPanelSerializer` keyed on a tab's session args (`["--resume", path]` restores; a fresh tab does not) closes it. Not v1. |
| **E** | `/new` semantics | **`/new` opens a new tab** (confirmed — replacing the current conversation would defeat the feature). `/resume` opens a new tab; `/reload` reloads **this** tab; `/sessions` keeps its meaning (the **members** of this tab's connection). |

### Lifecycle (settled)

- **Closing a tab at its last host stops its child**; `deactivate` stops **every**
  tab's child (the backstop against orphans).
- **A crashed tab stays open** showing its `✗ crashed` surface — the crash surface
  is where the user reads the error; the tab is not auto-closed.
- **Shared vs per-tab state:** global = the `OutputChannel` (prefixed per tab),
  the status bar, the selection source + `lastSelection`, the diff provider.
  Per tab = `hydrated`, `sessionArgs`, `userStopped`, `reviewVerdicts`,
  `ViewState`, `SurfaceController`, `WcodeSession`.
- **The diff `beforeRegistry` stays ONE shared registry**, cleared **only on
  `deactivate`**, never on a per-tab stop (tokens are unique, before-images
  immutable, the 32-cap evicts).

## Consequences

- `extension.ts:42-63`'s eight module singletons move into a per-tab
  `SessionTab` (`src/tab.ts`); `extension.ts` keeps a `Manager`
  (`src/manager.ts`). `panel.ts` loses `private static current`; one panel per
  tab. `surface.ts` needs **no structural change** — it is already multi-host.
- The pure, testable half lands as **`src/tabs.ts`** (routing / label / active
  helpers), covered by `test/tabs.test.ts` and added to the `purity.test.ts` set.
- **Multi-tab activates a concurrency risk already designed for:** N children in
  one workspace is exactly the cross-process case in
  [`workspace-concurrency.md`](../workspace-concurrency.md) (whole-file digest
  CAS). The tool-mediated path is protected; `bash` writes are not (that doc's
  D3). This is a *consequence of shipping*, not a blocker.
- The webview DOM, the reducer (`reducer.ts:117` is already per-session), and the
  wire protocol are **untouched**.
