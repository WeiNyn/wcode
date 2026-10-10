# W010 — VS Code extension: multiple editor tabs, each an independent wcode session — brief

- **Status:** accepted (human sign-off 2026-10-10) — decisions recorded in [D013](../decisions/D013-multi-tab-sessions-vscode.md)
- **Work item:** W010 (tracker row added by the orchestrator; inferred row 68 after row 67)
- **Depends on:** W005/W008/W009 (the surface is being actively reworked) — this change is a **structural** refactor of `extension.ts`/`panel.ts`, orthogonal to the editorial work but it must sequence **after** whatever surface commit is in flight (see §9, "chat.ts/extension.ts were changing under this session")
- **Conflicts with:** `docs/sketches/vscode-extension-team.md:141` and `:167` (the **one-panel** recommendation, tagged `TODO(prose)` — *unsettled*) — this brief **settles it the other way** and must update that sketch
- **Author:** session `agent:brainstormer` (recorded by the orchestrator)
- **Date:** 2026-10-09

---

## 1. The ask

Today the VS Code extension is **one wcode session per workspace**, surfaced by **one dockable `WebviewView`** plus **one editor-tab `WebviewPanel`**, both fed by one `SurfaceController`; the editor tab is an explicit singleton (`panel.ts:19`) so `wcode: Open in Editor` **reveals** rather than opens, and `/new`, `/resume`, `/reload` all **respawn the one child** (`extension.ts:249`). The ask: let a user open **several editor tabs in one workspace, each tab an independent wcode session (its own conversation, its own team)**, so parallel conversations can be worked side by side instead of the single session available today. This brief turns that request into a reviewable plan.

---

## 2. Scope / Non-scope

### 2.1 In scope

- **A per-tab session bundle.** Re-home the eight module-level mutables in `extension.ts` (session, viewState, controller, sessionArgs, userStopped, hydrated, lastSelection, reviewVerdicts; `extension.ts:42-63`) into a per-tab object, one instance per tab.
- **A non-singleton editor panel.** Replace `ChatPanel`'s `private static current` singleton (`panel.ts:19`) with one panel instance per tab, each owning its own `WcodeSession` + `SurfaceController`.
- **Option A, recommended and locked here: N independent `serve --stdio` children** — one per tab, no kernel/protocol change (§4.0 Q1).
- **Tab-aware commands.** `/new` and `/resume` open a **new tab**; `/reload` reloads **this** tab; `wcode.openInEditor` opens a **new** tab; a new `wcode.newSession` (or reuse) starts a fresh tab.
- **The docked sidebar's binding** to one tab at a time (the active tab), and a rule for what it shows when the active tab changes (§4.0 Q3).
- **Shared-vs-per-tab state**, decided per module: the diff `beforeRegistry`, the sticky `lastSelection`, the `OutputChannel`, the status bar (§4.0 Q5).
- **Tab labelling** (a title resolved from the session) and a **VSIX version bump** note (§4.0 Q7, §6).
- **Reconciling the conflicting design intent**: update `docs/sketches/vscode-extension-team.md:141`/`:167` (the `TODO(prose)`) and the "ONE dockable surface" framing in `editors/vscode/README.md:8` to record the multi-tab decision, and open a `DNNN`.

### 2.2 Non-scope (the important list)

- **Option B — one server hosting multiple root sessions** (a `Request::NewSession` + roster/`server.rs` multiplexing over one connection). **Not built**: it is a protocol change above the kernel (§4.0 Q1), it is larger, and the stdio "exactly one client" contract (`server.rs:117-118`) is a deliberate v1 property (`docs/vscode-extension-plan.md:82`). Named as the alternative, deferred.
- **Cross-tab visibility / cross-tab A2A.** A worker spawned in tab A lives in A's child; tab B's child cannot see it. Tabs are independent by construction under Option A. Sharing a team across tabs would need Option B or a `--socket` daemon — out of scope (§4.0 Q6).
- **A `--socket`/daemon mode for the extension.** The socket path stays for a TUI+editor-share-one-session case (`docs/vscode-extension-plan.md:82`), untouched here; the extension keeps spawning its own stdio children.
- **The `WebviewView` — before VS Code resolves it — has no `attach` and no per-tab identity.** This change does not add a second docked view; VS Code exposes exactly one `WebviewView` per contributed view id (§4.0 Q3; a fact I assert from the API, **not verified in-repo** — §9).
- **Any new behavior-config knob.** No `multiTab` setting, no tab-count cap; the behavior is the feature, per `AGENTS.md` ("build the variant you want instead of configuring one").
- **Moving `lastSelection` semantics.** The selection stays a single global sticky value (one active editor); this change does not make the selection per-tab.
- **The webview DOM / reducer.** `reducer.ts` is already per-session (`transcripts: Record<string, Block[]>` at `reducer.ts:117`); the webview still renders ONE tab's `ViewState`. No webview-protocol change.
- **A `/sessions` → "switch tabs" semantic.** `/sessions` keeps its meaning (list the **members** of this tab's connection, `extension.ts:425-437`); tab switching is the editor's own tab bar. Overloading it is refused (§4.0 Q4).
- **Tests in CI.** CI is cargo-only (`.github/workflows/ci.yml:15-43`); this brief does **not** add an extension-tests job — it says how to run them locally (§6). (A separate, larger change would.)
- **The OutputChannel as N channels.** One channel, prefixed per tab; not one channel per tab.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why (one line) |
|---|------------|-------|------------|------|----------------|
| **P0** | Record the decision (sketch `TODO(prose)` + `DNNN` + README + tracker) | high | low | low | The "one panel vs one per tab" question is explicitly unsettled in the sketch; the decision must be written before the refactor (`docs/sketches/vscode-extension-team.md:167`), docs-only, reversible. |
| **P1** | The per-tab bundle (`src/tab.ts`): re-home the eight singletons | high | high | high | The core of the change; getting controller/session/state lifetimes per-tab wrong silently cross-wires conversations — the exact failure the feature exists to remove. |
| **P2** | Non-singleton `ChatPanel` + a `Manager` registry | high | medium | medium | One deliverable (a panel per tab) but many call sites today assume `ChatPanel.current`; a lost panel leaks a live child. |
| **P3** | Tab-aware command routing (`/new`, `/resume`, `/reload`, `openInEditor`) | high | medium | medium | The user-visible behavior; a wrong route replaces a live session, the failure mode the feature removes. |
| **P4** | The sidebar binding (active tab) + status bar + OutputChannel prefix | medium | medium | medium | Re-binding one `WebviewView` host between controllers is an existing `attach`/`detach` seam (`surface.ts:146`/`:158`) but the "active tab" signal is new. |
| **P5** | Tab labels + the diff-registry lifetime | medium | low | medium | A label is small; the shared `beforeRegistry` (`diffProvider.ts:33`) + `clearDiffs()` on a tab stop is the subtle correctness edge. |
| **P6** | Sweep + VSIX bump + F5 | medium | low | medium | The webview was never run (`editors/vscode/README.md:164`); a multi-tab claim is a painted claim, and the VSIX bump procedure is undocumented. |

**Sizing basis (estimates, must be measured):** `extension.ts` is ~709 lines and holds every command handler; `surface.ts` (~298) already supports N hosts per controller; `panel.ts` (~93) is the only singleton. The refactor is **new code in a new module** (a `SessionTab` class) plus mechanical re-pointing of ~30 handler call sites from module globals to `this`. No Rust change; no bundle-size claim beyond "host `out/extension.js` grows by the new module".

---

## 4. Interface & structure

### 4.0 Design decisions (decided vs open)

| Q | question | decision | basis |
|---|----------|----------|-------|
| **Q1** | Session model: N stdio children (A) vs one multi-root server (B)? | **A — N `serve --stdio` children, one per tab.** *Open alternative: B.* | A needs no protocol/kernel change and respects the stdio one-client contract (`server.rs:117-118` *"there is no listener, so no second connection is even expressible"*); B needs a `NewSession` request + roster/server change (`serve_stream`, `server.rs:141`). Trade-off: A costs N× memory (N agents, N LLM connections, N teams) and cannot share a team across tabs; B would, at the cost of a wire change and the `--socket` story. |
| **Q2** | What multiplies? | **N × (`WcodeSession` + `SurfaceController` + `ViewState` + `HydratedSet` + handlers); a non-singleton panel per tab.** | Today one `SurfaceController` broadcasts ONE `ViewState` to every host (`surface.ts:114`, `:236`). Each tab must own a controller so its `ViewState` is private. The eight `extension.ts:42-63` globals move into the per-tab bundle. |
| **Q3** | Which session does the docked `WebviewView` show? | **The active tab** (the tab whose panel most recently gained focus), falling back to the last active, then the first — **with a session switcher at its head** (D013 A): the sidebar is the *index*, not the only surface. | VS Code exposes exactly one `WebviewView` per view id, so the sidebar cannot be N-up. Re-binding = `surface.detach(view)` then `activeTab.controller.attach(view)` (`surface.ts:146`/`:158`). *Open: a picker instead of focus-following.* |
| **Q4** | Tab lifecycle | **Closing a tab at its last host stops its child** (`stopSession`); **`/new` opens a new tab**; **`/resume` opens a new tab**; **`/reload` reloads the current tab**; **`/sessions` keeps its member meaning**. | The ask is multi-tab, so `/new`/`/resume` must NOT replace (today `restartSession` replaces, `extension.ts:249`). `/reload` re-runs the same args (`extension.ts:285-287`) — still this tab. |
| **Q5** | Shared vs per-session state | **Global:** the `OutputChannel`, the status bar, the selection source + `lastSelection`, the diff provider. **Per tab:** `hook` state (`hydrated`, `sessionArgs`, `userStopped`, `reviewVerdicts`), `ViewState`, controller, session. **Diff registry:** stays ONE shared `beforeRegistry` and is cleared **only on `deactivate`**, not on a per-tab stop. | The provider resolves by token and is registered once (`diffProvider.ts:68`); tokens are unique and before-images immutable, so a shared registry with a 32-cap (`diffProvider.ts:33`) is correct — the edge is that `clearDiffs()` (`extension.ts:240`, `:125`) must stop firing per tab. |
| **Q6** | Team/member model | **A tab = one root session with its own team; no cross-tab visibility.** | Under Option A each child owns its roster; a worker spawned in A never reaches B. Stated, not papered over. |
| **Q7** | Tab labels | **`wcode: <label>`**, **STICKY** (D013 C): set **once** from the root transcript's first user message (clipped ~30), never re-derived; *before* that, the `--resume` path basename if any, else **`wcode <n>`** (the tab index). | `WebviewPanel` accepts a title at creation (`panel.ts:34-35`, today the literal `"wcode"`); `panel.title` is settable later. The first-user-message derivation mirrors `/resume`'s picker (`listSessions` labels by first user message). |
| **Q8** | Open a tab: reuse `wcode.openInEditor` or add a verb? | **Add `wcode.newSession`** (a fresh session, a new tab); `wcode.openInEditor` becomes "open **this** session in an editor tab" (D013 B). |
| **Q9** | Survive a window reload? | **Deferred, recorded as a gap** (D013 D): a reload does not restore tabs/children today; a future `WebviewPanelSerializer` (restore only `["--resume", path]` tabs) closes it. |

### 4.1 Added

- **`editors/vscode/src/tabs.ts`** (new, **pure** — add to `purity.test.ts:19-29` list). The testable decisions, kept out of the vscode-importing bundle:
  - `commandAction(command: SlashCommand | undefined): "new" | "reload" | "resume" | "members" | "request"` — the routing `/new`→new tab, `/resume`→new tab, `/reload`→reload this, `/sessions`→members.
  - `tabLabel(state: ViewState, resumedPath: string | null, index: number): string` — first user message (clipped) → `--resume` basename → `wcode <index>` placeholder. **☑ LANDED.** `stickyLabel(current, candidate)` + `isPlaceholderLabel`/`placeholderLabel` encode D013 C (set once, never re-derived); `commandAction` degrades an unresolved name to `request`.
  - `nextActiveTab(tabs: readonly string[], focused: string, lastActive?: string | null): string | null` — focused → last-active → first → `null` (D013 A). **☑ LANDED** with `test/tabs.test.ts` (added to the `purity.test.ts` set); `npm run typecheck` + `npm test` green (234).
- **`editors/vscode/src/tab.ts`** (new — the vscode-importing bundle). A `SessionTab` class owning: `WcodeSession`, `SurfaceController`, `ViewState`, `HydratedSet`, `sessionArgs`, `userStopped`, `reviewVerdicts`, the `SurfaceHandlers` (`extension.ts:545-604` moved here), `start`/`stop`/`restart` (`extension.ts:144-260`), `hydrate` (`:531`), `togglePlan` (`:505`), the member verbs, and its `ChatPanel`. Constructor takes `{ extensionUri, workspaceRoot, output, logLine, onStateChange, onLabel }`.
- **`editors/vscode/src/manager.ts`** (new — vscode-importing). The `Manager`: `Map<vscode.WebviewPanel, SessionTab>`, the single `SurfaceViewProvider`, the single `OutputChannel`/status bar, the global selection listeners (`extension.ts:88-98`), and the active-tab tracking. `openTab(args: string[])`, `activeTab`, `disposeTab`.

### 4.2 Changed

- **`editors/vscode/src/extension.ts` — `activate`** (`extension.ts:65`). Replace the eight module globals (`extension.ts:42-63`) with one `const manager = new Manager(...)`. `context.subscriptions.push(manager, …)`. The `openInEditor` command (`extension.ts:80-82`) becomes `() => manager.openTab([])` instead of `ChatPanel.createOrShow(requireExtensionUri(), surface)`.
- **`editors/vscode/src/extension.ts` — `deactivate`** (`extension.ts:123`). `return session?.stop()` → `return manager.dispose()` (stop **every** tab's child). `clearDiffs()` stays here (global) and is removed from the per-tab `stopSession`.
- **`editors/vscode/src/extension.ts` — `startSession` / `stopSession` / `restartSession`** (`:144`/`:231`/`:249`). Move into `SessionTab` as methods; `restartSession(args)` is now the tab's **initial** start (a fresh tab) or `/reload` (same args). The `next.on("event", …)` reducer fold (`:180-190`) becomes `this.viewState = reduce(this.viewState, event, frame.session)`.
- **`editors/vscode/src/extension.ts` — `runCommand`** (`:270`). `case "new": void restartSession([])` (`:283`) becomes `manager.openTab([])`; `case "resume": void resumeSession(arg)` (`:290`) becomes `manager.openTab(["--resume", …])`; `case "reload": void restartSession(sessionArgs)` (`:287`) stays tab-local.
- **`editors/vscode/src/extension.ts` — the selection listeners** (`:88-98`). The singletons `controller?.setContext(...)` (`:91`, `:96`, `:150`) become `manager.setContextOnAllTabs(lastSelection)`.
- **`editors/vscode/src/panel.ts` — `ChatPanel`** (`panel.ts:18`). Delete `private static current` (`:19`) and the reveal-if-present branch (`:29-32`); `createOrShow(extensionUri, controller)` → `open(extensionUri, controller, title)` returning a fresh `ChatPanel` with the passed `title`; keep `WebviewPanelHost` (`:62`) unchanged.
- **`editors/vscode/src/webviewView.ts` — `SurfaceViewProvider`** (`webviewView.ts:20`). Constructor takes a `() => SurfaceController` (the active tab's), and `resolveWebviewView` (`:23`) attaches to it; the manager re-attaches on active-tab change. `WebviewViewHost` (`:35`) unchanged.
- **`editors/vscode/src/diffProvider.ts` — the registry lifetime** (`diffProvider.ts:33`). Keep the module-level `beforeRegistry`; **remove the per-tab `clearDiffs()`** calls (`extension.ts:240`, `:125` keep only the latter). Document the residual: a closed tab's tokens persist until the 32-cap eviction or `deactivate` — harmless because unreferenced.
- **`editors/vscode/src/extension.ts` — `surfaceHandlers`** (`:545`), `ensureTarget` (`:523`), `focusMember` (`:442`), `peekMember`/`askMember`/`stopMember`, `pickModel`/`pickEffort`/`resumeSession`/`pickConnectionSession`, `reviewChange`, `revealFile` (`:611`/`:628`). Move into `SessionTab`; each `session`/`viewState`/`controller` reference becomes `this.*`.
- **`editors/vscode/package.json`** — a `wcode.newSession` command (`package.json:46-87`) if a distinct "new tab" verb is wanted (otherwise `openInEditor` doubles); `activationEvents` (`:17-26`) gains it; `"version"` (`:5`) is bumped (see §6).
- **`editors/vscode/README.md:8` / `:164`**, **`docs/sketches/vscode-extension-team.md:141`/`:165-167`**, **`docs/vscode-extension-plan.md:28`/`:82`** — record the multi-tab decision (§5 P0/P6).

### 4.3 Integration points (verify before coding)

- `extension.ts:42` — `let session: WcodeSession | undefined;` — **the module singleton** the bundle replaces.
- `extension.ts:43`/`:47`/`:52`/`:57`/`:59`/`:61`/`:63` — `viewState`/`controller`/`sessionArgs`/`userStopped`/`hydrated`/`lastSelection`/`reviewVerdicts` — all become per-tab except `lastSelection` (stays global).
- `extension.ts:74` — `const surface = new SurfaceController(context.extensionUri, surfaceHandlers(), viewState);` — one per tab.
- `extension.ts:78` — `vscode.window.registerWebviewViewProvider("wcode.surface", new SurfaceViewProvider(surface)),` — **one registration**, re-bound to the active tab's controller.
- `extension.ts:81` — `ChatPanel.createOrShow(requireExtensionUri(), surface),` — the reveal-singleton call replaced.
- `panel.ts:19` — `private static current: ChatPanel | undefined;` — the singleton removed.
- `panel.ts:29-32` — `if (ChatPanel.current) { ChatPanel.current.reveal(); return ChatPanel.current; }` — the reveal-short-circuit removed.
- `surface.ts:114` — `export class SurfaceController {` — already multi-host: `hosts` is a `Set` (`surface.ts:120`), `attach`/`detach` (`:146`/`:158`) exist, `broadcast` (`:236`) fans to all hosts — **no structural change needed**, only a single controller per tab.
- `session.ts:139` — `export class WcodeSession extends EventEmitter` — **instance state only**, no module state; N instances already work (grounds Option A).
- `session.ts:264` — `child = spawn(this.options.binary, ["serve", "--stdio", ...], …)` — one child per `WcodeSession`; N tabs = N children.
- `reducer.ts:117` — `transcripts: Record<string, Block[]>;` — already per-session; **unchanged**.
- `diffProvider.ts:33` — `const beforeRegistry = new BeforeRegistry(32);` — the shared registry (Q5).
- `webview.ts:69` — `| { kind: "ready" };` — the per-host handshake; `surface.ts:146-154` captures the host in the listener closure, so a `ready` from the sidebar does not ready a panel — this is what lets one controller serve two hosts, and it is what breaks if two tabs share a controller.
- `commands.ts:37` — `export const SLASH_COMMANDS: readonly SlashCommand[]` — the registry the new `commandAction` reads.
- `package.json:108-117` — the scripts (`test` = `node --test test/*.test.ts`, no packaging script); `package.json:5` — `"version": "0.0.6"`.
- `server.rs:117-118` — *"Exactly one client: there is no listener, so no second connection is even expressible."* — the stdio constraint Q1 relies on.
- `.github/workflows/ci.yml:15-43` — cargo-only; no extension job.

---

## 5. Plan

1. **P0 — record the decision. ☑ DONE** (D013 + the sketch resolution + this brief). Deliverable: a W010 note in `docs/sketches/vscode-extension-team.md` resolving the `:167` `TODO(prose)` to "a panel **per tab**" (Q1=A, Q3=active tab + switcher), **`docs/decisions/D013-multi-tab-sessions-vscode.md`** recording Option A, A–E and the shared diff registry, a line in `editors/vscode/README.md:8` replacing "ONE dockable surface", and the tracker row. Gate: the sketch no longer carries `TODO(prose)`; the README/plan agree; docs-only review (no build).
2. **P1 — the per-tab bundle. ☑ DONE** — `src/tab.ts` (`SessionTab` owning `WcodeSession` + `SurfaceController` + `ViewState` + `HydratedSet` + args/flags/verdicts + every handler) and `src/manager.ts` (`Manager`); `extension.ts` is now a ~60-line composition root. All eight `extension.ts:42-63` singletons are gone (asserted by `test/multitab.test.ts`). Deliverable: `src/tab.ts` (`SessionTab`) + `src/tabs.ts` (pure) with the `extension.ts:42-63` singletons now fields; `extension.ts` holds a `Manager`. Gate: `npm run typecheck` 0; new `test/tabs.test.ts` (pure: `commandAction` routes `/new`→`new`, `/reload`→`reload`, `/resume`→`resume`; `tabLabel` prefers the first user message over the basename); `tabs.ts` added to `purity.test.ts`; `npm test` green.
3. **P2 — non-singleton panel + manager. ☑ DONE** — `ChatPanel.open(...)` (no `static current`), `Manager` maps panels→tabs, `onPanelClosed` stops a closed tab's child (the sole-session case survives for the sidebar). Deliverable: `ChatPanel.open(...)` (no `static current`), `src/manager.ts` owning `Map<panel, SessionTab>`. Gate: `npm run build` writes `out/extension.js`; a source assertion that `ChatPanel` has no `static current` (`grep -n "static current" editors/vscode/src` → nothing); F5 (P6) is the human proof.
4. **P3 — tab-aware commands. ☑ DONE** — `/new` and `/resume` open a NEW tab (`deps.openTab`/`resumeTab`), `/reload` reloads THIS tab, `/sessions` unchanged, `wcode.openInEditor` re-homes the active session; `wcode.newSession` added. Deliverable: `openInEditor`/`/new`/`/resume` open a tab (via `manager.openTab`), `/reload` reloads this tab, `/sessions` unchanged. Gate: `test/tabs.test.ts` covers the routing table; a live `test/multisession.test.ts` (below) proves two independent children.
5. **P4 — sidebar binding + status + logging. ☑ DONE** — the `WebviewViewProvider` follows the active tab (`Manager.rebindView`), the ONE status bar reflects the active tab (`updateStatus`), and every tab log line is prefixed with the tab label (`SessionTab.log`, `tab.ts`; a source guard in `test/multitab.test.ts` asserts `appendLine` is called in exactly one place). Deliverable: the `WebviewViewProvider` follows the active tab (`Manager.setActive` detaches from the old controller, attaches to the new); the one status bar (`extension.ts:68`) reflects the active tab's state; the one `OutputChannel` prefixes lines with the tab label. Gate: `npm run typecheck`; a pure `nextActiveTab` test; the human step.
6. **P5 — labels + diff-registry lifetime. ☑ DONE** — `panel.title` is set from the sticky label (`Manager.onLabelChange` → `ChatPanel.setTitle`); `clearDiffs()` no longer fires per tab (it stays only in `deactivate`). Deliverable: `panel.title` set from `tabLabel` as the first user message lands; `clearDiffs()` no longer called on a per-tab stop; the residual documented in `diffProvider.ts`. Gate: `test/tabs.test.ts` label cases; `grep -n "clearDiffs" editors/vscode/src` shows only `deactivate`.
7. **P6 — sweep + VSIX + F5. ◑ VSIX done; F5 human** — `package.json` is bumped `0.0.6 → 0.0.7`, packaged (`npx @vscode/vsce package` → 9 files / 142 KB, no warnings), and released as a `.vsix` on an `ext-v*` tag ([extension.yml](../../.github/workflows/extension.yml)). The headless gates are green (`npm run typecheck`, `npm test` 244, `vsce package`). Deliverable: docs reconciled (P0 + the plan/README), `package.json:5` bumped `0.0.6 → 0.0.7`, a captured F5 screenshot of two tabs with distinct transcripts. Gate: §6 commands; the F5 click-path in §7.2.

---

## 6. Quality gates

Run **in `editors/vscode`** (this change touches only TypeScript; cargo is a *guard*, not a gate):

```sh
cargo build                              # test/live.test.ts + routing.test.ts need target/debug/wcode
cd editors/vscode
npm install
npm run typecheck && node scripts/check-css.mjs && npm test && npm run build
```
- `npm run typecheck` (`package.json:111` = `tsc --noEmit && tsc --noEmit -p tsconfig.webview.json`) → **no output, exit 0**.
- `node scripts/check-css.mjs` → `check-css: ok (chat.css)`.
- `npm test` (`package.json:113`) → `check-css: ok (chat.css)` then `# pass <n>` / `# fail 0`.
- `npm run build` (`package.json:109` = `node esbuild.mjs`) → `out/extension.js` + `media/chat.js` written; state the `out/extension.js` delta (the new modules).

**Untouched guard (no Rust is touched):**
```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
Both stay clean — a guard, not a gate (`.github/workflows/ci.yml:28-43`).

**The VSIX bump (undocumented — inferred, flag as such).** `package.json:5` is a literal; there is no packaging script (`package.json:108-117`) and no `vsce` dependency. Bump `"version": "0.0.6" → "0.0.7"` and package with `npx @vscode/vsce package` (respecting `.vscodeignore`, which excludes `src/**`, `test/**`, `scripts/**`). **Not verified**: whether `vsce` is available in this environment, and whether any release process expects a tag.

**Regression tests kept green (by name):** `reducer.test.ts` (the per-session fold is unchanged), `purity.test.ts` (the pure set — with `tabs.ts` added), `routing.test.ts` (*"a member id is served; a bogus one is refused (live)"* — member addressing is unchanged within a tab), `cancel.test.ts`, `commands.test.ts`, `live.test.ts`.

---

## 7. Testing

### 7.1 Automated

- **New — `test/tabs.test.ts` (pure).** (a) `commandAction` maps `/new`→`new`, `/resume`→`resume`, `/reload`→`reload`, `/sessions`→`members`, `/model`→`request`; (b) `tabLabel(state, null)` returns the first user message clipped; (c) `tabLabel(initialState(), "/tmp/x.jsonl")` returns `x`; (d) `tabLabel(initialState(), null)` returns `"wcode"`; (e) `nextActiveTab(["a","b"], "b") === "b"`, and a focused id not in the set falls back to the last.
- **New — `test/multisession.test.ts` (live, model-free; `skip` when `target/debug/wcode` is absent, mirroring `routing.test.ts:31`).** Spawn **two** `WcodeSession` instances against the same binary; assert each reaches `state === "ready"` with a **distinct** `rootSessionId` (`session.ts:176`), and that a `SetPlanMode`/`Status` on each is answered independently (the routing control from `routing.test.ts:53`/`:80`). This asserts the **transport** half of Option A: N children, N root ids, no interference.
- **New — `test/tab.test.ts` (structural-lite, if a pure seam exists).** If the `SessionTab` cannot be node-driven (it imports `vscode`), assert only the *pure* neighbours + a source check that `panel.ts` has no `static current`.
- **Kept green.** `reducer.test.ts`, `purity.test.ts` (+ `tabs.ts`), `routing.test.ts`, `commands.test.ts`, `live.test.ts`.

*Honest limit:* the `Manager`↔`SessionTab`↔`ChatPanel` wiring imports `vscode` and **cannot** be exercised under `node --test` (no VS Code/Electron — `editors/vscode/README.md:60-67`). Its proof is F5 (§7.2) plus `npm run typecheck`.

### 7.2 How a human verifies it

```sh
cargo build
cd editors/vscode
npm install
npm run typecheck && node scripts/check-css.mjs && npm test && npm run build
# .vscode/launch.json -> F5 (Run Extension) -> the Extension Development Host opens
```
Then:
1. Cmd/Ctrl+Shift+P → **`wcode: Open in Editor`** twice. Expect **two** editor tabs, each titled `wcode: …` (the first user message once you send one).
2. Send `hello` in tab 1 and `world` in tab 2. Expect **independent** transcripts; a reply in one never appears in the other; the OutputChannel shows `← sessions` twice with **different** root ids.
3. Start a worker (or `/plan`) in tab 1 only. Expect tab 2 to be unaffected; the sidebar follows whichever tab has focus (Q3).
4. Click the other tab. Expect the docked `WebviewView` to switch to **that** tab's session (or, until a tab is focused, the last active).
5. `/new` in tab 1. Expect a **third** tab with an empty transcript, tab 1 unchanged.
6. Close tab 2 (the editor tab's ×). Expect its child to exit (OutputChannel `stopped` for that id); tabs 1 and 3 keep running.
7. Close every tab, then stop the extension. Expect **no** orphaned `wcode serve --stdio` children (`pgrep -f "wcode serve --stdio"` → nothing).

If no model/endpoint is configured, point the child at a keyless endpoint (`--base-url http://localhost:11434/v1`) or set `wcode.path` to a built `target/debug/wcode`; for the crash path a refused port (`http://127.0.0.1:9/v1`) shows `✗ crashed` in that tab only.

---

## 8. Expected outcome

- Opening the surface N times yields **N editor tabs**, each a `ChatPanel` over its own `SessionTab` (own `WcodeSession` + `SurfaceController` + `ViewState`); `panel.ts` has no `static current`.
- `/new` and `/resume` open a **new tab**; `/reload` reloads **this** tab; `/sessions` still lists **this** tab's members.
- `pgrep -f "wcode serve --stdio"` shows **N** children, one per open tab; closing a tab stops its child; `deactivate` stops all.
- The docked `WebviewView` shows exactly one tab's session at a time (the active tab); the one status bar reflects the active tab's state; the one OutputChannel tags every line with its tab.
- `src/tabs.ts` exists (pure, in `purity.test.ts`); `test/tabs.test.ts` and `test/multisession.test.ts` are green; `npm run typecheck`/`check-css`/`test`/`build` all pass; `cargo test --workspace`/`clippy` remain clean (untouched).
- `docs/sketches/vscode-extension-team.md` has no `TODO(prose)` about one-panel-vs-per-tab; a `DNNN` records Option A; `editors/vscode/README.md:8` no longer claims "ONE dockable surface"; `package.json:5` reads `0.0.7`.

---

## 9. References

- `editors/vscode/src/extension.ts` (`:42-63` the singletons, `:74`/`:78`/`:81` the wiring, `:123` `deactivate`, `:144`/`:231`/`:249` start/stop/restart, `:270` `runCommand`, `:442`/`:523`/`:531`/`:545` member/handlers) — the refactor's surface.
- `editors/vscode/src/panel.ts` (`:19` the singleton, `:28` `createOrShow`), `src/surface.ts` (`:114`/`:120`/`:146`/`:158`/`:236`), `src/session.ts` (`:139`, `:176`, `:264`), `src/webviewView.ts` (`:20`/`:23`), `src/diffProvider.ts` (`:33`/`:68`), `src/reducer.ts` (`:117`, `:393`, `:410`), `src/webview.ts` (`:43`/`:56`/`:69`), `src/commands.ts` (`:37`), `src/webview/chat.ts`.
- `editors/vscode/package.json` (`:5` version, `:17-26` activationEvents, `:46-87` commands, `:108-117` scripts), `.vscodeignore`, `editors/vscode/README.md` (`:8`, `:60-67`, `:164`).
- `docs/sketches/vscode-extension-team.md` (`:141`, `:165-167` the conflicting recommendation, `TODO(prose)`), `docs/vscode-extension-plan.md` (`:28`, `:82`), `docs/next-steps.md` (rows 53–56, 62, 65–67 — the surface's history).
- Rust seam: `crates/wcode-protocol/src/server.rs:117-118` (stdio one-client), `:141` `serve_stream`; `crates/wcode-protocol/src/backend.rs:55-61` (`Backend::Local`/`Remote`); `crates/wcode-cli/src/main.rs:68-72`/`:291-296` (`serve`/`--stdio`/`--socket`); `.github/workflows/ci.yml:15-43`.
- Skills: `.wcode/skills/brainstorm-brief/SKILL.md`, `.wcode/skills/live-verification/SKILL.md`, `.wcode/skills/wcode-conventions/SKILL.md`.

---

## Facts NOT verified (named)

- **No `npm`, node, browser, `vsce`, or VS Code host was run** (read-only worker). Bundle sizes, the `out/extension.js` delta, and the VSIX packaging path are **estimates/assumptions**.
- **VS Code exposes exactly one `WebviewView` per contributed view id** — asserted from API knowledge; the repo has one `viewsContributes`+`views` entry (`package.json:37-45`) and I did not test a second. This is load-bearing for Q3.
- **`WebviewPanel.onDidChangeViewState` / `active` and `panel.title` mutability** — asserted from the API, not exercised in-repo (no `title =` anywhere today).
- **The tracker row (68)** is inferred from row 67, not verified against a counter.
- **The VSIX bump procedure** — no documented process, no `vsce` devDependency, no packaging script; `npx @vscode/vsce package` is an assumption (`.vscodeignore` exists).
- **`extension.ts`/`chat.ts`/`render.ts` were changing under this session** (W008/W009 landing); all `file:line` anchors are from the current tree and will drift.
- **Not read in full:** `src/webview/chat.ts`, `src/render.ts`, `src/webview.ts` beyond the cited lines, `tsconfig*.json`, `.vscode/launch.json` — asserted to need no change from the host-side refactor and the per-session reducer.
- **The memory cost of N children** is reasoned (N agents/connections/teams), not measured.
- **Whether a closed panel's `onDispose` (`panel.ts:44-46`) reliably fires before `deactivate`** when the whole window closes — reasoned, not observed; the "stop every child on deactivate" step is the backstop.

The brief ends with the brief.
