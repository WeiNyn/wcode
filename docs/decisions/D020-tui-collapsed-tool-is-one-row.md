# D020 — A collapsed tool is ONE row, with a per-tool outcome note

- **Status:** accepted (human sign-off — W016)
- **Date:** this session
- **Relates to:** D019 (the "params never hide" invariant this amends in *form*),
  D010 (the tool tree row), D009 (inline tools in call order), D031 (the tool
  panel — the `bash` command shown in full), D032 (the `▸`/`▣` affordances)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §4 (**Tools**) and the §5
  keymap line — *"`Ctrl-T` expands/collapses every tool's **output** (params
  stay visible; …)"*
- **Amends:** [`D019`](D019-tui-tool-params-always-visible.md) — the **form**
  clause only (rows → one row). D019's **invariant** ("params never hide") is
  preserved intact and remains the standing record of it.

## Context

D019 fixed a real defect: a finished, collapsed tool rendered its output preview
but **not** its input params — a `bash` showed neither the command nor a target.
D019's fix rendered the params block unconditionally, so a finished tool became a
whole tree: head + `│   key  value` params + a (up to 4-row) output preview + a
`… +N more` hint + a `└` — **~9 rows for a `read` or a `bash`**.

The params *did* need to stay visible (the call's identity; the `bash` command
must show in full). But nine rows per finished tool is too much vertical space
for the most common state. The fix is **form**, not policy: keep params visible,
shrink the collapsed rendering to **one row**.

## Decision

1. **A collapsed tool is ONE row** (the head). It carries the tool's name, its
   params **inlined**, and a per-tool **outcome note** joined with the wall-clock
   ms. There is no `│   ` params block, no output preview, no `… +N more` hint,
   and no `└` while collapsed.
2. **Params are never hidden** (D019's invariant, preserved). Collapsed, the
   **first** param's value prints **bare** — its key dropped, its syntax
   highlight kept (`param_syntax`) — and the remaining params print `key=value`
   (the value quoted iff it contains a space). **Expanded**, they re-draw as the
   `│   key  value` rows, byte-identical to D019's shape.
3. **The disclosure governs the output body only** — unchanged from D019. The
   `▸`/`▾` affordance and `Ctrl-T` (or browse `Enter`/`Space`) expand/collapse the
   output/diff body; they never touch the params. A failed tool is forced
   expanded, so a failure is never hidden.
4. **An over-long head wraps, never clips.** When the inlined value overflows,
   the head wraps char-exact onto a `│   `-aligned continuation, so a long `bash`
   command still shows in full (D031/D019).
5. **The collapsed head shows a per-tool outcome note:** `read`/`write` → `N
   line(s)`; `edit`/`edits` → `+a −r`; `bash` → `exit N`; `grep` → `N hit(s)`;
   `find` → `N file(s)`; anything else (incl. `ast_edit`, which carries no `diff`
   field) falls back to the output's first non-blank line. The note is derived in
   the view from `output`/`diff` — **no kernel/wire/`ToolOutput` change**.
6. **A running tool is collapsed too:** its head inlines its params and shows no
   note, plus exactly **one** `│   ` live-tail row (`last_line`); **no** `└`.

## Mechanism

- `tool_inline_lines` (`crates/wcode-tui/src/ui.rs`) forks on `expanded`
  (`tool.expanded || tool.is_error`): expanded → the D019 shape
  (`tool_head_row` + `panel_param_lines` + `tool_panel_body(true)` + `└`);
  collapsed → `collapsed_head_row`, plus one live-tail row while running.
- `collapsed_head_row` builds the multi-span head (name + bare first value +
  `key=value` rest), appends the `▸ ▣` affix pinned at `note_affordance_cols`
  (the D32 hit map is unchanged), and applies the note's **40-char cap +
  drop-when-crowded** rule (the note, then the ms, then both). It inlines
  `tool.target.or(tool.path)` when `params` is empty (the reseeded-tool fallback).
- `completion_note` dispatches by `tool.name` **before** any `tool.diff` read
  (so `write` reports its written line count, not its capped diff), and the five
  pure counters (`count_read_lines`, `parse_wrote_lines`, `parse_exit_code`,
  `count_grep_hits`, `count_find_paths`) parse the existing `output` strings.
- `tool_stats` (the expanded head) keeps the `summary_line` fallback and the
  `· ms` join; it now reads `completion_note` first.
- `crates/wcode-cli/src/tools/write.rs` — the success output gains a line count
  (`wrote {N} {line|lines} ({bytes} bytes) to {path}`) so `write`'s note is
  derivable from the diff-less path (its diff is a capped one-hunk preview).

## Consequences

- A finished, collapsed tool occupies **one row** (down from ~9):
  `├ bash  cargo test --workspace  exit 0 · 1.2s  ▸ ▣`.
- The row shows a legible **outcome** — line counts, `+a −r`, `exit N`, hit/file
  counts — instead of a data dump, joined with the wall-clock ms by ` · `.
- Params are still visible; a long `bash` command still shows in full (it wraps).
- `Ctrl-T` still toggles the output body only, so the toggle keeps one meaning.
- **Foreclosed:** a per-tool "show a collapsed preview" list; a config knob for
  the collapse form (behaviour is code, per `AGENTS.md`); re-introducing the
  boxed `╭─ ⚙ bash ─╮` frame.
- Gate: `cargo test --workspace` green · `cargo clippy --workspace --all-targets`
  clean · the `dump` snapshot shows a one-row collapsed `bash` with its note and
  no box glyphs.
