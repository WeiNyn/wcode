# wcode TUI — polish pass: tool output, theme, keys (plan)

Status: **planned**. Companion to [`tui-plan.md`](tui-plan.md) (the project),
[`tui-design.md`](tui-design.md) (the visual spec) and
[`tui-input-plan.md`](tui-input-plan.md) (the input band). This is a small
presentation pass over the shipped TUI — `wcode-tui` only; the kernel and the
protocol are untouched.

## Problem

Three things make the TUI hard to live in.

1. **Tool output collapses to one line.** `tool_lines` renders a finished tool
   as `✓ name · <first 80 chars>`; everything after the first line is dropped. A
   `bash` / `read` / `grep` result is unreadable — the only way to see it is
   `/changes` (mutating tools only) or re-running the call. This was promised and
   never built: `tui-plan.md` P1 lists "Thinking + tool blocks styled and
   **collapsible**", and `tui-design.md` §6 still carries the expand question
   open. This pass discharges that P1 commitment.
2. **Colors are ad-hoc.** Eight style fns (`dim`, `muted`, `state_style`,
   `accent`, `added_style`, `removed_style`, `error_style`, `code_style`) each
   inline a `NO_COLOR` branch, and the palette is bold/dim/cyan/yellow/red. There
   is no `Theme` to change a role in one place.
3. **The keymap is undiscoverable.** `Ctrl-N` is the only way to change surface,
   there is no help overlay, and the composer has no word/line editing at all.

## Design

### Tool output — expand / collapse (`Ctrl+O`, `Ctrl+T`)

- `Tool` gains `expanded: bool` (default `false`). A finished tool shows a header
  line (as today) plus a body: the first `TOOL_PREVIEW_LINES = 4` output lines,
  then a dim `     … +N more lines · Ctrl+O` when the output is longer. Expanded,
  up to `TOOL_EXPANDED_LINES = 200` lines (still `… +N more` past that).
- A running tool keeps its one-line tail while collapsed; expanded, it shows the
  last 200 lines of the live output (tail — it grows downward).
- **An errored tool renders expanded**, so a failure is never hidden; a tool that
  finishes with `is_error` forces `expanded = true`.
- A diff body collapses to `TOOL_DIFF_PREVIEW_LINES = 8` diff lines + the hint;
  expanded, up to 200. The `+a −r` footer and `diff_counts` summary stay.
  `/changes` still re-shows the full diff — that path is **uncapped**.
- Body lines **wrap** at the transcript content width through the existing `wrap`
  helper (5-space continuation gutter) instead of clipping at the pane edge.
- **Paste chips are not touched.** `tui-input-plan.md` (B7) decided the
  transcript is the record of truth and never collapses a large paste; that
  stands — this pass is about *tool* output only.

### Theme — named roles (`theme.rs`)

- A `Theme` of named `Style` fields (`accent, dim, muted, border, user, body,
  error, success, warn, code, heading, link, tool_name, thinking, diff_add,
  diff_del`), with `Theme::colored()` and `Theme::plain()` — the `NO_COLOR`
  fallback (no `fg`, only bold/italic/dim; byte-identical to today's branches).
  `theme()` picks one through the existing `OnceLock` / `NO_COLOR` check.
- **Named ANSI colors only, no truecolor**, so it works on any terminal. Richer
  but still restrained: thinking = dim + italic, tool names = blue, `✓` = green,
  `✗` = red, inline code = yellow, links = blue + underline, headings = bold.
  Assistant prose stays default and **dim stays the workhorse** (`tui-design.md`
  §1.3); the body text is not colorized.
- **Scope.** This is a structural refactor onto named roles — **not** the P4
  configurable-theme feature and **not** the P2 truecolor detection
  (`COLORTERM`, 256-color fallback). No config, no detection, no env knob beyond
  the existing `NO_COLOR`; P2/P4 stay open.

### Keys

- `Ctrl-O` — expand / collapse the focused surface's **last** tool.
- `Ctrl-T` — expand / collapse **all** tool output at once.
- `Alt-1..9` — focus surface N (0 / absent is a no-op). `Shift-Tab` (`BackTab`)
  — previous surface; `Ctrl-N` stays next. `Tab` keeps accepting completion, and
  `BackTab` only leaves the composer when the completion popup is **closed**.
- `F1` — a help overlay listing the keymap; `/help` gains the same table. A
  single `KEYS` table feeds both. The overlay dismisses **only** on `Esc` / `F1`
  — every other key is ignored (a modal that eats a keystroke silently is worse
  than one you must dismiss deliberately).
- `Ctrl-B` — toggle the team sidebar (`SIDEBAR_MIN_WIDTH` auto-show still applies
  when it is shown).
- Composer readline editing: `Ctrl-W` delete the previous word · `Ctrl-U` delete
  to the start of the current line · `Ctrl-K` delete to the end of the current
  line · `Ctrl-A` / `Ctrl-E` move to the start / end of the input. All
  `Atom`-aware — a paste chip is one unit, never split.

## Commits

One logical change each; tests + `cargo clippy --workspace --all-targets` clean
at every commit, and a headless rendered-frame assertion proving the behavior.

1. `docs: plan the TUI polish pass` — this doc + the tracker row.
2. `tui: expand/collapse tool output (Ctrl+O)`.
3. `tui: a central theme with named color roles`.
4. `tui: keys — help overlay, surface jumps, detail toggle`.

## Status

| # | commit | status |
|---|--------|--------|
| 1 | plan + tracker | ☑ done |
| 2 | tool output expand / collapse | ☑ done |
| 3 | central theme | ☑ done |
| 4 | keys: help overlay, surface jumps, detail toggle | ☐ todo |

Legend: ☑ done · ◐ in progress · ☐ todo.
