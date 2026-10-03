# wcode — VS Code extension

A second **client** of the wcode kernel, never a second kernel: it spawns
`wcode serve --stdio` and speaks the same frame protocol as the TUI
(`docs/vscode-extension-plan.md` §3.2).

**Status: P2 — native diffs.** Wire types, the stdio session, the pure reducer,
host-side markdown rendering, the webview chat panel, and native diffs
(reverse-apply the patch to reconstruct the pre-image; open on request).

## Layout

| file | role |
|---|---|
| `src/protocol.ts` | the wire types — the ONE place the serde tags live |
| `src/session.ts` | `WcodeSession`: spawn, NDJSON splitter (stdout only), FSM, restart |
| `src/reducer.ts` | the pure `(state, event) -> state` view model (no `vscode`) |
| `src/render.ts` | pure `ViewState` → blocks + HTML (markdown rendered **here**) |
| `src/diff.ts` | pure `reverseApply` (patch → pre-image), the before-token registry |
| `src/diffProvider.ts` | the `wcode-diff:` provider + the click handler (**the only diff `vscode`**) |
| `src/webview.ts` | the `ToWebview`/`FromWebview` unions, the payload parsers, the throttle |
| `src/panel.ts` | `WebviewPanel`, CSP, `localResourceRoots`, `postMessage` wiring |
| `src/markdown.ts` | `markdown-it` with `html: false` |
| `src/extension.ts` | `activate`/`deactivate`, the four commands, an OutputChannel |
| `src/webview/chat.ts` | the webview script **source** (typed, dependency-free) |
| `src/webview/view.ts` | the webview's pure half (`stateLabel`, `statusSegments`, …) |
| `media/chat.js` | the bundled webview script (esbuild IIFE; **generated**, gitignored) |
| `media/chat.css` | the panel stylesheet (theme variables only; zero remote assets) |
| `scripts/capture.mjs` | capture real frames into `test/fixtures/` |
| `scripts/verify-diff-fixtures.mjs` | check the diff fixtures against the **compiled generator** |
| `test/` | plain-node tests (protocol, splitter, reducer, render, webview, diff, pipeline, live) |

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
(`render.ts`, `diff.ts`, `webview/view.ts`) so `panel.ts`/`diffProvider.ts` are
the only files that import `vscode`.

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

If no model/endpoint is configured, step 4 ends with the status strip reading
`● crashed` and the child's **stderr tail** shown under it — that is the failure
mode the status strip exists to make visible, not a silent empty panel.

## What is verified — and what is NOT

**Verified (headless, `npm test`):** the **host-side** pipeline — protocol frames
→ reducer → rendered HTML — against both a **live** `wcode serve --stdio` child
and the committed fixtures; `reverseApply` round-tripping real generator patches
to their pre-images on disk, and returning `null` on every undecidable patch; the
NDJSON splitter; the `FromWebview`/`ToWebview` payload parsers; the throttle; and
markdown escaping (`html: false`).

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
- **the CSP as a live webview enforces it**, and **the CSS** (never painted).

F5 in the Extension Development Host (the click-path above) is the only way to
exercise them; treat it as an unrun step.
