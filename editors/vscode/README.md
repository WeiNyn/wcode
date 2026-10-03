# wcode — VS Code extension

A second **client** of the wcode kernel, never a second kernel: it spawns
`wcode serve --stdio` and speaks the same frame protocol as the TUI
(`docs/vscode-extension-plan.md` §3.2).

**Status: P1b — the webview chat panel.** Wire types, the stdio session, the
pure reducer, host-side markdown rendering, and the webview panel (chat surface,
tool timeline, status strip). Diffs are P2.

## Layout

| file | role |
|---|---|
| `src/protocol.ts` | the wire types — the ONE place the serde tags live |
| `src/session.ts` | `WcodeSession`: spawn, NDJSON splitter (stdout only), FSM, restart |
| `src/reducer.ts` | the pure `(state, event) -> state` view model (no `vscode`) |
| `src/render.ts` | pure `ViewState` → blocks + HTML (markdown rendered **here**) |
| `src/webview.ts` | the `ToWebview`/`FromWebview` unions, the payload parser, the throttle |
| `src/panel.ts` | `WebviewPanel`, CSP, `localResourceRoots`, `postMessage` wiring |
| `src/markdown.ts` | `markdown-it` with `html: false` |
| `src/extension.ts` | `activate`/`deactivate`, `wcode.start`/`stop`/`restart`, an OutputChannel |
| `src/webview/chat.js` | the webview script **source** (vanilla JS) |
| `media/chat.js` | the bundled webview script (esbuild IIFE; generated) |
| `media/chat.css` | the panel stylesheet (theme variables only; zero remote assets) |
| `scripts/capture.mjs` | capture real frames into `test/fixtures/` |
| `test/` | plain-node tests (protocol, splitter, reducer, render, webview, pipeline, live) |

`media/chat.js` is a **build artifact** of `src/webview/chat.js`: esbuild cannot
write a bundle over its own entry without nesting the IIFE on a rebuild, so the
source and the output are separate files.

## Develop

```sh
npm install
npm run typecheck     # tsc --noEmit
npm test              # node --test (plain node, no VS Code / Electron)
npm run build         # esbuild -> out/extension.js AND media/chat.js
npm run capture       # refresh the captured fixtures from target/debug/wcode
```

The tests run under **plain `node --test`** with Node's built-in TypeScript
type stripping — no `vscode`, no display, no Electron. That is why the sources
avoid TS-only runtime syntax (no enums, no namespaces, no parameter properties),
use `import type` for type-only imports, and keep the rendering a pure function
(`render.ts`) that `panel.ts` calls.

The `wcode` binary is resolved from the `wcode.path` setting, then `PATH`. The
live tests skip (do not fail) when `target/debug/wcode` is absent.

## Click-path (manual test)

1. **Once:** `cd editors/vscode && npm install`.
2. Open the **repo root** (`wcode`) in VS Code.
3. Press **F5** → the **Run Extension** configuration (`.vscode/launch.json`).
   Its `preLaunchTask` runs `npm run build` in `editors/vscode`, then a second
   **Extension Development Host** window opens.
4. In that window: **Ctrl/Cmd+Shift+P** → **`wcode: Start Session`**.
   - Expect: a **wcode** panel opens beside the editor; its status strip reads
     `● starting` then `● ready`, followed by the **root session id**. With no
     prior session the transcript shows *“No messages yet.”*; a resumed session
     shows its history (the panel hydrates via `Request::GetHistory`).
   - The **wcode** OutputChannel (View → Output → `wcode`) logs
     `starting: … serve --stdio` and then every frame (`← sessions`, `← history`, …).
5. Click the **input** at the bottom (it is focused when the panel opens), type
   `hello`, press **Enter**.
   - Expect: your text appears at once as a **you** block (an optimistic echo —
     the session streams only the assistant's reply), then **wcode** streams a
     reply; tool calls appear as collapsible `⚙ name …` rows (click to expand).
   - **Shift+Enter** inserts a newline; **Esc** (or the *Cancel* button) sends
     `Request::Cancel`.
6. **`wcode: Stop Session`** stops the child (status → `● stopped`);
   **`wcode: Restart`** stops and starts again.

If no model/endpoint is configured, step 4 ends with the status strip reading
`● crashed` and the child's **stderr tail** shown under it — that is the failure
mode the status strip exists to make visible, not a silent empty panel.

## What is verified — and what is NOT

**Verified (headless, `npm test`):** the whole **host-side** pipeline —
protocol frames → reducer → rendered HTML — against both a **live**
`wcode serve --stdio` child and the committed fixtures; the NDJSON splitter; the
`FromWebview` payload parser; the throttle; and markdown escaping (`html: false`).

**NOT verified:** the panel has **never been seen rendering**. There is no
webview, no VS Code, and no display in this environment, so four links are
unexercised:

- **HTML → DOM** — `media/chat.js` painting the rendered blocks (including the
  tool timeline, expand/collapse, and the empty/loading/error states);
- **the webview ⇄ host `postMessage` hop** — the `ready` handshake, `submit`,
  `cancel`, `steer`;
- **the CSP as a live webview enforces it** — it is the documented pattern and
  admits its own `media/*` by construction, but nothing here has loaded it; and
- **the CSS** — `media/chat.css` has never been painted (it uses only
  `var(--vscode-*)` theme variables and no remote assets).

F5 in the Extension Development Host (the click-path above) is the only way to
exercise them; treat it as an unrun step.
