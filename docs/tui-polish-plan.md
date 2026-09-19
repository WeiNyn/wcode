# wcode TUI — polish pass: tool output, theme, keys (plan)

Status: **☑ landed — reviewed.** Companion to [`tui-plan.md`](tui-plan.md) (the project),
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

- `Tool` gains `expanded: bool` (default `false`). Collapsed, a finished tool
  shows a header with its 80-char summary plus a `TOOL_PREVIEW_LINES = 4` preview
  of the lines **after** the summary (a line is never shown twice), then a dim
  `… +N more line(s) · Ctrl+O` when expansion reveals more. Expanded, the summary
  moves off the header and the whole output is drawn, wrapped — nothing is left
  unreachable.
- A running tool keeps its one-line tail while collapsed; expanded, it shows the
  last `TOOL_EXPANDED_LINES = 200` lines of the live output (tail — it grows
  downward).
- **An errored tool renders expanded**, so a failure is never hidden; a tool that
  finishes with `is_error` forces `expanded = true`.
- A diff body collapses to `TOOL_DIFF_PREVIEW_LINES = 8` diff lines + the hint;
  expanded, up to 200. The `+a −r` footer and `diff_counts` summary stay.
  `/changes` still re-shows the full diff — that path is **uncapped**.
- Body lines **wrap** at the transcript content width, char-exact (`wrap_input`:
  spaces kept, an over-long token hard-broken) under the 5-space gutter, instead
  of clipping at the pane edge. (It counts chars, not display columns — see
  [Known limitations](#known-limitations).)
- **Paste chips are not touched.** `tui-input-plan.md` (B7) decided the
  transcript is the record of truth and never collapses a large paste; that
  stands — this pass is about *tool* output only.
- **Update (later):** the input-mode `Ctrl-O` last-tool shortcut was retired in
  the browse pass — a shortcut whose target is invisible is the defect — and
  per-block toggling moved to browse mode's `Enter`/`Space` (`Ctrl-G` to enter).
  `Ctrl-T` (all tools) is unchanged. See [`tui-browse-plan.md`](tui-browse-plan.md).

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

One logical change each; `cargo test --workspace` and `cargo clippy --workspace
--all-targets -- -D warnings` clean at every commit, each with a headless
rendered-frame assertion proving the behavior.

| commit | subject |
|--------|---------|
| `0f51576` | `docs: plan the TUI polish pass` |
| `f84489e` | `tui: expand/collapse tool output (Ctrl+O)` |
| `9afafc2` | `tui: a central theme with named color roles` |
| `f7bbfaa` | `tui: keys — help overlay, surface jumps, detail toggle` |
| `0371dc4` | `tui: expand a single-line tool output in full` |
| `ad6f5d6` | `docs: drop a stale palette fragment in the design spec` |
| `de0933f` | `tui: close the tool-output test gaps` |
| `7031582` | `tui: pluralize the tool-output hint` |

## Review

**APPROVE** — every rendered-frame test goes red under its own regression. Two
blockers were found and fixed before approval:

1. **A single-line tool output was unreadable.** `body_after_summary` dropped the
   first line, so a `bash` one-liner / single-line JSON had an empty body: `Ctrl-O`
   revealed nothing and the tail was unreachable *even expanded* (measured: a
   299-char line exposed 79 chars, 220 lost). Fixed in `0371dc4` — expanded now
   drops the summary and draws the whole output.
2. **A stale palette fragment.** `tui-design.md` §1.3 carried leftover lines
   asserting a 256-color fallback the code does not have. Fixed in `ad6f5d6`.

Four vacuous tests (the `/changes` uncap, a multi-tool `Ctrl-O`, a resumed errored
tool, and a wrapped running-tool body's scroll height) were tightened in
`de0933f`; the singular hint wording in `7031582`.

## Known limitations

Landed and reviewed; two things are deliberately left as-is.

1. **Wide characters can still clip.** Tool bodies wrap via `wrap_input`, which
   counts **chars, not display columns**, so a CJK / emoji output can overflow the
   pane edge by its width delta. Pre-existing and shared with the transcript
   (tracked as the wide-character open question in
   [`tui-input-plan.md`](tui-input-plan.md)); not fixed here.
2. **A redundant seed assignment.** On a resumed session, `seed` sets a tool's
   `expanded: *is_error`, which the renderer's `|| tool.is_error` short-circuit
   already covers. The end-to-end behavior (a replayed failure renders expanded)
   is tested; the assignment on its own is not — belt-and-suspenders, not
   load-bearing.

## Status

| # | item | status |
|---|------|--------|
| 1 | plan + tracker | ☑ done |
| 2 | tool output expand / collapse | ☑ done |
| 3 | central theme | ☑ done |
| 4 | keys: help overlay, surface jumps, detail toggle | ☑ done |
| 5 | review blockers (single-line output · stale palette fragment · test gaps) | ☑ done |

Legend: ☑ done · ◐ in progress · ☐ todo.
