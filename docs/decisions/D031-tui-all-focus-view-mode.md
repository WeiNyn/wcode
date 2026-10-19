# D031 — TUI All/Focus view mode: a client-local merge, default Focus

- **Status:** accepted
- **Date:** 2026-10-19
- **Relates to:** the VS Code All/Focus mode (`editors/vscode/src/webview.ts:40`
  `export type ViewMode = "all" | "focus";`, `render.ts:224` `renderMerged`); D015, D016
- **Implements:** [W026](../work/W026-tui-all-mode.md); sketch
  [`tui-all-mode.md`](../sketches/tui-all-mode.md)

## Context

The TUI rendered only the **focused** surface's transcript — `ui.rs:597`
`        let n = app.transcript().len();` over `app.rs:4545`
`    pub fn transcript(&self) -> &[Block] {`; members are separate surfaces
(`app.rs:2444` `    surfaces: Vec<Surface>,`) seen only in the strip/sidebar. The VS
Code client already offers **All / Focus** — a client-side merge with no protocol
change (`docs/vscode-ui-v2-plan.md:75`), ordered by the reducer's arrival `seq`
(`editors/vscode/src/reducer.ts:131`, stamped `:535`).

Two reference facts forced a choice: it **defaults to All** (`webview.ts:60`
`DEFAULT_PREFS = { mode: "all", …`) and its composer is addressed to the **root**
(`view.ts:92`). The TUI precedent is the opposite: a view bit is off by default so
the frame is byte-identical until opted in (`app.rs:2534` `    sidebar: bool,`).

## Decision

Add a client-local `ViewMode { Focus, All }` (`app.rs:2342` `pub enum ViewMode {`),
**default `Focus`**. The merge key is a monotonic **arrival ordinal** (`app.rs:2543`
`    next_seq: u64,`) stamped at the append chokepoints — `App::push_on` (`app.rs:4588`)
and the `AppEvent::Agent` arm (both clear the D5 hint BEFORE measuring), plus
`stamp_replayed` (`app.rs:4627`) for a replay.

All is **display-only**: browse/mouse/text selection and `Ctrl-T` are Focus-scoped
no-ops; a dedicated `all_scroll` + four `all_*` mirror fields window the merged view.
Input targets the **focused** surface. The mode control is a header `.seg` shown iff
`label()` is `Some` (`ui.rs:837`); selected by `[tui] view`/`--view` (`config.rs:413`,
`main.rs:200`), toggled by `Ctrl-O`, set by `/view <all|focus>`.

## Consequences

- **Divergence (documented): default `Focus`**, where the reference defaults to All —
  a session is unchanged until opted in (the D015 rule).
- **Divergence (documented): input → the focused surface**, where the reference sends
  to the root; `focus`/`focused` (`app.rs:2695`/`:2700`) stay the input target, and the
  origin head names every block's owner.
- **Display-only forecloses cross-surface selection** in v1: in All the hit map is
  empty, so `block_at` returns `None`.
- Chrome reflects the **root** in All via new `chrome_*`; `running()` (`app.rs:5321`),
  `run_elapsed()` (`:5225`) and `status()` (`:5312`) keep their **focused** semantics.
- A member's **live** stream is not merged — only the focused surface's trailer rides the tail.
