# D008 — syntax highlighting runs host-side on Prism (the webview stays dependency-free)

- **Status:** accepted
- **Date:** 2026-10-09
- **Supersedes / relates to:** closes the deferred note at `editors/vscode/src/markdown.ts:7`
  ("P1 ships fenced code WITHOUT token highlighting"); relates to
  [W009](../work/W009-syntax-highlighting.md) and
  [`docs/plans/vscode-ui-editorial-plan.md`](../plans/vscode-ui-editorial-plan.md) §5.

## Context

Three surfaces render code as flat escaped text: the fenced-code rule
(`editors/vscode/src/markdown.ts:26-31`), the tool output recess (`src/webview/chat.ts`
`rawOutput`) and the tool input/diff body (`src/webview/chat.ts` `reviewBlock`'s `.txt` span).
The TUI already highlights fenced code with `syntect` (`crates/wcode-tui/src/markdown.rs:583`),
but Rust `syntect` is not usable in a webview.

The webview is **stated** to be dependency-free and to receive pre-rendered HTML
(`src/render.ts:9`, `src/webview.ts:10`, `esbuild.mjs:5`, `README.md:37`), so the highlighter
cannot live there. Two constraints bound the choice: **zero remote assets** (the extension is
packaged), and the colour budget — the surface colours with `--vscode-*` plus the ONE accent
`--wc-accent` (`media/chat.css`).

## Decision

Syntax highlighting is **host-side**: a new pure module `editors/vscode/src/highlight.ts`
(`highlight(code, lang) → string | null`, `languageForPath`, `languageForTool`) wraps **Prism**
(`prismjs`, bundled into `out/extension.js` alongside `markdown-it`) with a **curated** grammar
set matched to wcode's tools + the TUI's bundled syntaxes; **no `Prism.highlightAuto`**. The host
pre-highlights and carries the HTML to the webview as two new optional `RenderedTool` fields
(`outputHtml`, `diffLinesHtml`); the webview only inserts it (Prism escapes its own output).
Tokens are coloured by a `.token.*` palette mapping Prism classes to `--vscode-symbolIcon-*`,
each with a `--vscode-editor-foreground` fallback.

## Consequences

- The **webview gains no dependency** and `media/chat.js` does not grow; the cost lands on the
  host bundle `out/extension.js` (measured in W009 P1).
- The first new dependency since `markdown-it`.
- `--vscode-symbolIcon-*` is a fresh palette: where the family is absent the fallback collapses
  every token to one colour — a **silent no-op** F5 must rule out (W009 P6).
- Per-line diff highlighting re-joins + re-splits the hunk so multi-line constructs keep
  colour; a continuation line carries its own re-opened spans.
- **Forecloses:** a webview-side highlighter, `highlightAuto`, and a second accent.
