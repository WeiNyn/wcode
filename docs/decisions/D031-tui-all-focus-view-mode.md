# D031 — TUI All/Focus view mode: a client-local merge, default Focus

- **Status:** accepted
- **Date:** 2026-10-19
- **Relates to:** the VS Code All/Focus mode — `editors/vscode/src/webview.ts:40`
  `export type ViewMode = "all" | "focus";` and `editors/vscode/src/render.ts:224`
  `export function renderMerged(state: ViewState, thinking: FoldDefault = "auto"): RenderedBlock[] {`;
  the D015 presentation-settings precedent (an off-by-default client-local bit);
  D016 (`[tui] width`, the startup-setting shape this mirrors)
- **Implements:** [W026](../work/W026-tui-all-mode.md); sketch
  [`docs/sketches/tui-all-mode.md`](../sketches/tui-all-mode.md)

## Context

The TUI rendered only the **focused** surface's transcript —
`crates/wcode-tui/src/ui.rs:585` `    let n = app.transcript().len();` over
`crates/wcode-tui/src/app.rs:4434` `    pub fn transcript(&self) -> &[Block] {`
(`&self.focused().transcript`). Team members are separate surfaces
(`app.rs:2386` `    surfaces: Vec<Surface>,`) whose activity showed only in the
working-team strip and the sidebar, never in the transcript. The VS Code client
already offers **All / Focus**: "All mode is a client-side merge over the existing
per-session transcripts — no protocol change" (`docs/vscode-ui-v2-plan.md:75`),
merged by the reducer's arrival `seq` (`editors/vscode/src/reducer.ts:131`,
stamped at `:535`).

Two reference facts forced a choice. The reference **defaults to All**
(`editors/vscode/src/webview.ts:60` `export const DEFAULT_PREFS: ViewPrefs = { mode: "all", …`),
and its composer stays addressed to the **root**
(`editors/vscode/src/view.ts:92` `  if (mode === "all") return rootMember(state);`). The
TUI's own precedent is the opposite: a view bit is **off by default** so the frame
is byte-identical until opted in (the `sidebar: bool`,
`crates/wcode-tui/src/app.rs:2476` `    sidebar: bool,` — "view state like `mode`").

## Decision

Add a client-local `ViewMode { Focus, All }` (`app.rs:2342` `pub enum ViewMode {`),
**default `Focus`**. The frame is byte-identical until the user opts in.

The merge key is a monotonic **arrival ordinal** (`app.rs:2543` `    next_seq: u64,`)
stamped at the append chokepoints — `App::push_on` (`app.rs:4588`
`    fn push_on(&mut self, idx: usize, block: Block) {`) and the `AppEvent::Agent`
arm, both clearing the D5 hint BEFORE measuring, plus `stamp_replayed`
(`app.rs:4627`) for a replay — mirroring the reference's stamp-inside-append. The
order is **total and stable** across every surface.

All mode is **display-only**: browse, mouse/text selection and `Ctrl-T` are
Focus-scoped no-ops; a dedicated `all_scroll` + four `all_*` mirror fields window
the merged view. Input targets the **focused** surface. The mode control is a
header `.seg` shown iff `label()` is `Some` (`ui.rs:837`
`        if let Some(seg) = head.view.label() {`). Selected by `[tui] view` /
`--view all|focus` (`crates/wcode-cli/src/config.rs:413`,
`crates/wcode-cli/src/main.rs:200`), toggled by `Ctrl-O`, set by `/view <all|focus>`.

## Consequences

- **Divergence (documented): default is `Focus`**, where the reference defaults to
  All — so a wcode TUI session is unchanged until opted in (the D015 rule).
- **Divergence (documented): input → the focused surface**, where the reference
  sends to the root. `focus`/`focused` (`app.rs:2609`/`:2614`) stay the
  input/command target; the origin head names every block's owner so the reader
  never mistakes whose turn is whose.
- **Display-only forecloses cross-surface selection** in v1 — a follow-on, not a
  defect: `set_block_ranges`/`block_at` stay Focus-scoped; in All the hit map is
  empty (`clear_hit_map` at the top of every `draw`) so `block_at` returns `None`.
- The header/footer chrome reflects the **root** in All via new `chrome_*`
  accessors; `running()` (`app.rs:5321`), `run_elapsed()` (`app.rs:5225`) and
  `status()` (`app.rs:5312`) keep their **focused** semantics (the tick guard and
  the sidebar depend on them).
- A member's **live** (uncommitted) stream is not merged — it stays in the
  working-team strip; only the focused surface's live trailer rides the tail.
