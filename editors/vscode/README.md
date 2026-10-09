# wcode — VS Code extension

A second **client** of the wcode kernel, never a second kernel: it spawns
`wcode serve --stdio` and speaks the same frame protocol as the TUI
(`docs/vscode-extension-plan.md` §3.2).

**Status: the webview surface.** Wire types, the stdio session, the pure
per-session reducer, host-side markdown rendering, native diffs, and the team
surface — all rendered **inside one webview**: a dockable `WebviewView`
(`wcode.surface`, in the Activity Bar) plus an editor-tab `WebviewPanel`
(`wcode: Open in Editor`), both fed by the one `SurfaceController`. The masthead
(identity + status + a dim team caption), the transcript, the composer and the live
type-line are webview DOM
(`src/webview/chat.ts`) — rendered in-webview, not by a VS Code `TreeView`.

## Layout

| file | role |
|---|---|
| `src/protocol.ts` | the wire types — the ONE place the serde tags live |
| `src/session.ts` | `WcodeSession`: spawn, NDJSON splitter (stdout only), FSM, restart |
| `src/reducer.ts` | the pure `(state, event) -> state` view model (no `vscode`) — incl. `memberGlyph` (the state icon) and `memberViews` (the team-caption rows) |
| `src/render.ts` | pure `ViewState` → blocks + HTML **for a target member** |
| `src/surface.ts` | `SurfaceController` — the ONE state/throttle/message owner both hosts attach to |
| `src/webviewView.ts` | the docked `wcode.surface` `WebviewViewProvider` |
| `src/panel.ts` | the editor-tab `WebviewPanel` host (`wcode: Open in Editor`) |
| `src/diff.ts` | pure `reverseApply` (patch → pre-image), the before-token registry |
| `src/diffProvider.ts` | the `wcode-diff:` provider + the click handler (**the only diff `vscode`**) |
| `src/webview.ts` | the `ToWebview`/`FromWebview` unions, the payload parsers, the throttle |
| `src/markdown.ts` | `markdown-it` with `html: false` |
| `src/commands.ts` | the `/`-command registry (pure) — the ONE list the menu and the host share |
| `src/sessions.ts` | the `/resume` picker's data (session files: groups + legacy) |
| `src/startup.ts` | the pure "the child died at startup" classifier |
| `src/review.ts` | the pure change-review model (a hunk + per-call verdicts) |
| `src/extension.ts` | `activate`/`deactivate`, the commands, the surface wiring, an OutputChannel |
| `src/webview/chat.ts` | the webview script **source** (typed, dependency-free) |
| `src/webview/view.ts` | the webview's pure half (`stateLabel`, `panelHeader`, `turns`, `teamCaption`, …) |
| `media/chat.js` | the bundled webview script (esbuild IIFE; **generated**, gitignored) |
| `media/chat.css` | the panel stylesheet (theme variables only; zero remote assets) |
| `scripts/capture.mjs` | capture real frames into `test/fixtures/` |
| `scripts/verify-diff-fixtures.mjs` | check the diff fixtures against the **compiled generator** |
| `test/` | plain-node tests (protocol, splitter, reducer, render, webview, diff, routing, live) |

`media/chat.js` is a **build artifact** of `src/webview/chat.ts`: esbuild cannot
write a bundle over its own entry without nesting the IIFE on a rebuild, so the
source and the output are separate files.

## Develop

```sh
npm install
npm run typecheck     # tsc --noEmit, then the webview's own program (tsconfig.webview.json)
npm test              # node --test (plain node, no VS Code / Electron)
npm run build         # esbuild -> out/extension.js AND media/chat.js
npm run capture       # refresh the captured fixtures from target/debug/wcode
npm run verify:diff-fixtures   # diff fixtures vs. the compiled `diff.rs` (needs rustc)
```

The tests run under **plain `node --test`** with Node's built-in TypeScript
type stripping — no `vscode`, no display, no Electron. That is why the sources
avoid TS-only runtime syntax (no enums, no namespaces, no parameter properties),
use `import type` for type-only imports, and keep the pure halves pure
(`reducer.ts`, `render.ts`, `diff.ts`, `review.ts`, `commands.ts`, `sessions.ts`,
`startup.ts`, `webview/view.ts`, `webview.ts`). The only **non-entry** `vscode`
importers are `panel.ts`, `webviewView.ts`, `surface.ts` and `diffProvider.ts`
(`extension.ts` is the composition root).

`DOM` is available in the **webview's** program only (`tsconfig.webview.json`);
the host program excludes `src/webview`, so a stray `document`/`window` in a host
file is a compile error rather than a runtime crash.

The `wcode` binary is resolved from the `wcode.path` setting, then `PATH`. The
live tests skip (do not fail) when `target/debug/wcode` is absent.

## Click-path (manual test)

1. **Once:** `cd editors/vscode && npm install`.
2. Open the **repo root** (`wcode`) in VS Code.
3. Press **F5** → the **Run Extension** configuration (`.vscode/launch.json`).
   Its `preLaunchTask` runs `npm run build` in `editors/vscode`, then a second
   **Extension Development Host** window opens.
4. In that window: **Ctrl/Cmd+Shift+P** → **`wcode: Start Session`**.
   - Expect: the **wcode** view opens (the Activity Bar container, plus the
     editor tab on `wcode: Open in Editor`); its **masthead** shows the identity
     glyph + `orchestrator` on the left and `ctx N` + the mode toggle on the right
     (a `starting`/`stopped`/`crashed` session also carries the state word). With
     no prior session the transcript shows *“No messages yet.”*; a resumed session
     shows its history (the surface hydrates via `Request::GetHistory`).
   - The **wcode** OutputChannel (View → Output → `wcode`) logs
     `starting: … serve --stdio` and then every frame (`← sessions`, `← history`, …).
5. Click the **input** at the bottom (it is focused when the view opens), type
   `hello`, press **Enter**.
   - Expect: your text appears at once as a **you** block (an optimistic echo —
     the session streams only the assistant's reply), then **wcode** streams a
     reply; tool calls appear as collapsible `⚙ name …` rows (click to expand).
   - **Shift+Enter** inserts a newline; **Esc** (or the *Cancel* button) sends
     `Request::Cancel`.
6. **`wcode: Stop Session`** stops the child (status → `● stopped`);
   **`wcode: Restart`** stops and starts again.
7. **Open a diff.** Ask wcode to change a file, then click the **show diff**
   button on that tool row.
   - Expect: VS Code's **native diff editor** opens, titled
     `wcode: <basename> (before → after)` — left is the reconstructed
     before-image (a `wcode-diff:` virtual document), right is the file on disk.
   - If the patch **cannot be reversed exactly** (truncated at the generator's
     40-line cap, a foreign patch, or the file has changed since the edit), the
     same click opens the **patch text** read-only, titled
     `wcode: <basename> (patch only)` — never an enabled button that opens
     nothing.
   - **`wcode: Open Diff`** (Ctrl/Cmd+Shift+P) opens the most recent diff.
   - A tool call that touched a file but changed **no line** shows **no** button:
     no diff is a normal case, not an error.
8. **The team surface.** Open the **wcode** view (the `❯_` Activity Bar icon).
   The webview draws the team as a **dim caption** under the masthead: one row per
   served session — `orchestrator` (the root) plus each worker — carrying a state
   glyph (`memberGlyph`) and, while it runs, the action it is on.
   - **Click a caption row (or focus it and press Enter/Space)**: the surface
     **retargets** — the masthead's identity names that member, the row is marked
     `sel`, and the transcript switches to *that member's* conversation (hydrated
     via `GetHistory` addressed to its id). Nothing already streamed is lost.
   - **Type in the composer**: the message is `Submit`ted **to the target
     member**, and your text is echoed into that member's transcript.
   - **The composer's mode control** (`Act`/`Plan`, a text toggle) toggles plan
     mode — optimistic, settled on `Ack`, reverted on `Error`. The same toggle is
     the palette command **`wcode: Toggle Plan Mode`**.
   - With **no session running**, the view shows *“No session. Run wcode: Start
     Session.”* — never a blank caption.

If no model/endpoint is configured, step 4 ends with the masthead reading
`✗ crashed` and the child's **stderr tail** shown in the transcript — that is the
failure mode the empty-state card exists to make visible, not a silent empty view.

### Not yet wired — the member menu verbs

`wcode.member.focus` / `wcode.member.peek` / `wcode.member.ask` /
`wcode.member.stop` are registered, but the **only** contributed menu is
`view/title` → `wcode.openInEditor`, so a palette invocation passes **no member
id** and they are **inert** (`src/extension.ts`, the *“INERT until a rail-row
menu lands”* note). Wiring a menu that supplies the member id — a
`view/item/context`, or an in-webview caption-row menu — is a **follow-up**,
deliberately not done in this pass.

## What is verified — and what is NOT

**Verified (headless, `npm test`):** the **host-side** pipeline — protocol frames
→ reducer → rendered HTML — against both a **live** `wcode serve --stdio` child
and the committed fixtures; `reverseApply` round-tripping real generator patches
to their pre-images on disk, and returning `null` on every undecidable patch; the
NDJSON splitter; the `FromWebview`/`ToWebview` payload parsers; the throttle; and
markdown escaping (`html: false`); and, LIVE and **model-free**, that a request
addressed to `agent:<name>` is **validated against the live roster** — the proof
is the CONTROL (a bogus id refused with `unknown session …`); a real one is
answered (`Status` correlated by `reply_to`, `SetPlanMode` → `Ack`, `Define` →
`Spawned` growing the roster). The reply's own `session` field is an **echo** of
the request, not evidence.

**The diff fixtures are real generator output.** They are transcribed from
`diff.rs`'s own tests and its construction, and
`npm run verify:diff-fixtures` compiles `crates/wcode-cli/src/tools/diff.rs`
standalone, drives it with the fixtures' own inputs, and compares the bytes —
10/10 match.

**NOT verified:** no webview and no VS Code was ever run here, so these links are
unexercised:

- **HTML → DOM** — `media/chat.js` painting the rendered blocks (tool timeline,
  expand/collapse, the empty/loading/error states);
- **the webview ⇄ host `postMessage` hop** — the `ready` handshake, `submit`,
  `cancel`, `steer`, `open-diff`;
- **`vscode.diff` + the `wcode-diff:` provider** — the native diff editor, the
  title, the left/right URIs, and the provider resolving a token to the
  before-image. `reverseApply` (the hard part) is tested; the plumbing that
  hands its result to VS Code is not;
- **the CSP as a live webview enforces it**, and **the CSS** (never painted);
- **the team caption drawn inside the webview** — the DOM that paints the caption
  rows (the provider's *pure* half — `memberGlyph`, `memberViews` — and the live
  roster it is fed are tested; the webview that draws them is not), the
  click/keyboard retarget wiring, and the icon/theme rendering;
- **a real `Submit` turn IN A WORKER.** Every verified verb is **model-free**
  (`Status`, `SetPlanMode`, `Define`); a `Submit` addressed to a member takes the
  same `handle.ask` path, but no worker turn was ever run here, so "the composer
  runs a turn in that member" remains **code-traced, not observed**. A busy
  member's `Submit` **deferral** (queued to the next run boundary) is likewise
  untraced.

F5 in the Extension Development Host (the click-path above) is the only way to
exercise them; treat it as an unrun step.
