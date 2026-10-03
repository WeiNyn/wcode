# wcode — VS Code extension

A second **client** of the wcode kernel, never a second kernel: it spawns
`wcode serve --stdio` and speaks the same frame protocol as the TUI
(`docs/vscode-extension-plan.md` §3.2).

**Status: P1a — the spine.** Wire types, the stdio session, the pure reducer,
markdown rendering, and a minimal extension host entry point. No webview panel
yet (P1b).

## Layout

| file | role |
|---|---|
| `src/protocol.ts` | the wire types — the ONE place the serde tags live |
| `src/session.ts` | `WcodeSession`: spawn, NDJSON splitter (stdout only), FSM, restart |
| `src/reducer.ts` | the pure `(state, event) -> state` view model (no `vscode`) |
| `src/markdown.ts` | `markdown-it` with `html: false` |
| `src/extension.ts` | `activate`/`deactivate`, `wcode.start`/`stop`/`restart`, an OutputChannel |
| `scripts/capture.mjs` | capture real frames into `test/fixtures/` |
| `test/` | plain-node tests (splitter, reducer, markdown, live spine) |

## Develop

```sh
npm install
npm run typecheck     # tsc --noEmit
npm test              # node --test (plain node, no VS Code / Electron)
npm run build         # esbuild -> out/extension.js
npm run capture       # refresh the captured fixtures from target/debug/wcode
```

F5 in VS Code launches the Extension Development Host (the `Run Extension`
configuration is the default); run **wcode: Start Session** and watch the
`wcode` OutputChannel.

The tests run under **plain `node --test`** with Node's built-in TypeScript
type stripping — no `vscode`, no display, no Electron. That is why the sources
avoid TS-only runtime syntax (no enums, no namespaces, no parameter properties)
and use `import type` for type-only imports.

The `wcode` binary is resolved from the `wcode.path` setting, then `PATH`. The
live spine test skips (does not fail) when `target/debug/wcode` is absent.
