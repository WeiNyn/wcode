# D038 — A collapsed tool head is ONE row: elide, never wrap

- **Status:** accepted (human sign-off; design fixed before implementation)
- **Date:** this session
- **Relates to:** [`ui.rs`](../../crates/wcode-tui/src/ui.rs) (`collapsed_head_row`,
  `param_row`, `truncate`), [`tui-design.md`](../tui-design.md) §4 (**Tools**)
- **Amends:** [`D020`](D020-tui-collapsed-tool-is-one-row.md) — its mechanism
  clause 4 ("an over-long head wraps, never clips") and its `│   `-continuation
  shape. D020's *decision* ("a collapsed tool is one row") is what this D038
  finally **enforces**; the wrapper was the loose end.
- **Amends:** [`D019`](D019-tui-tool-params-always-visible.md) — the **form**
  clause only, again: "params are never hidden" becomes "params are never **lost**
  — elided in the collapsed head, complete on expansion".

## Context

D020 said a collapsed tool is one row. It was not: `collapsed_head_row`
(`ui.rs:1225` today) inlined the first param's value and, when the head
overflowed, **wrapped it char-exact onto `│   `-aligned continuation rows with no
row budget** — that loop lived at `ui.rs:1232-1247` on the pre-D038 tree
(`8c9ccf0`) and is **deleted**, not moved. Its doc promised it — *"A too-long
head WRAPS char-exact onto a gutter-aligned continuation (never a `…` clip), so a
long `bash` command shows in FULL (D31/D019)"* (`ui.rs:1158-1160` there).

The asymmetry is what floods: the **stats** are bounded (the note is capped at 40
chars — `ui.rs:1271`; the fallback summary at 80 — `ui.rs:2032`) while the
**params** are unbounded, so one heredoc `bash` command or one long `sed` script
turns a "one row" tool into ten. Meanwhile the neighbouring surfaces already
carry the right rule: the settings detail field clips with `…`
(`ui.rs:442-444`, *"An over-long value clips with `…` (clip-not-wrap, D027) — no
silent shortening"*).

Two further defects sat in the same code:

- Both the collapsed head (that deleted `ui.rs:1232-1247` loop, with
  `spans_from_chars` at `ui.rs:1287`) and the expanded params block
  (`wrap_styled`, `ui.rs:1926-1947`) flatten a value into **chars**, so a
  multi-line `command` (a heredoc, `sh -c`) emitted a literal `\n` **into a
  `Span`** — a zero-width cell that mis-sizes the row.
- The expanded block therefore did **not** render the complete record it claimed
  to; a two-line command printed as one run-on row.

## Decision

1. **A collapsed tool head is exactly ONE row, always.** There is no continuation
   row and no row budget to blow: the head is `prefix + name + first value +
   key=value … + fill + stats`, and it is emitted as one `Line`.

2. **An over-long value is elided with `…`** — clip-not-wrap, the same rule as
   D027. An elision is always **marked**: a cut without a `…` would be a silent
   lie about what the tool was called with.

3. **A multi-line value shows its first line plus ` … +N lines`** (`N` = the
   number of lines FOLLOWING the first — a trailing newline is a terminator, not
   a line). That tail is **priced into the budget** and is **never a rung**: the
   last rung reserves it, so no width can reach an unmarked head. No `\n` char may
   reach a `Span` — not in the head, not in the params block.

4. **Degradation is a fixed ladder of FOUR rungs** (`A4`, extended — one rule,
   not two): **(a)** drop the note → **(b)** drop the ms → **(c)** drop the extra
   `key=value` params, from the tail → **(d)** **clip the first value into what the
   fold mark leaves, with a floor of 8 columns**. The **name never drops** — the
   name is the only part that says *what ran*.

   The invariant the fourth rung codes: **exactly ONE `…` per collapsed head row,
   present whenever anything was folded or clipped — at every width.** A fold and
   a clip never stack: when the first line has to clip, the clip's `…` REPLACES the
   ` … +N lines` tail, so a row never reads `… … +2 lines`. (An earlier draft had a
   fifth rung that shed the tail before clipping; at the 40-col measure floor that
   printed a folded two-line command with no mark at all — the silent cut §2
   forbids. It is gone.)

5. **Expansion is the complete record.** `param_row` splits the value on `\n` and
   renders each logical line as its own wrapped run, continuation rows keeping
   the `value_col` alignment. Nothing is elided once expanded.

6. **The spec says "never `LOST`", not "never hidden"** (D019's invariant,
   restated): a param's value is elided in the collapsed head and complete on
   expansion, so no information is unreachable. `Ctrl-T` still governs the
   **output body** only.

7. **Accepted edge — the 8-column floor overruns a very narrow band.** When
   `width - head - gap - tail` is below `MIN_TOOL_VALUE_COLS` (`ui.rs:48`), the
   value keeps 8 columns and the row overruns; the band clips it (documented at
   `ui.rs:44-47`). Reaching it needs a band narrower than `name + 13`, i.e. below
   the 40-col measure floor for any real tool name — a truncated `…`, never a
   crash, never a wrap. Accepted, not fixed: the alternative is a mark-less head,
   which is the defect this record closes.

## Mechanism

- `collapsed_head_row` (`ui.rs:1225`) returns a single `Line<'static>` — the "one
  row" contract is now the type, not a doc comment. It flattens the first value to
  its first logical line (`fold_value`, `ui.rs:1343`), prices ` … +N lines`, walks
  the ladder above, and reuses `truncate` (`ui.rs:2043`, the existing `…` clip) for
  the last rung. The wrap-to-continuation branch and `spans_from_chars` (its only
  caller) are **deleted**.
- `param_row` (`ui.rs:1725`) maps each `value.lines()` line through `wrap_styled`,
  so a heredoc command keeps its line structure; an empty value still yields one
  row. `fold_value` is the ONE `lines()`-based fold, shared with
  `extra_param_run` (`ui.rs:1359`), so the folded count can never disagree with
  what expansion draws.
- The "in FULL" head doc and the "params are never hidden" line — `ui.rs:1158-1160`
  and `ui.rs:1120` on the pre-D038 tree — are reworded (`ui.rs:1199-1224` and
  `ui.rs:1174`): the head elides, it does not show the command "in full" —
  expansion does.

## Consequences

- A finished, collapsed tool costs **one row** whatever its arguments:
  `   bash  sh -c 'for i in 1 2 3…  exit 0 · 812ms`.
- A run of eight `bash` calls is eight rows instead of up to eighty — the
  transcript's row cost becomes a function of the call **count**, not of the
  call **text**.
- The `bash` command is still reachable in full: `Ctrl-T` / browse `Enter`
  expands the params block, which now honours real line breaks.
- **Foreclosed:** a config knob for the head's clip width (behaviour is code, per
  `AGENTS.md`); a second, params-specific text-container rule beside `truncate`;
  keeping the wrapper behind a threshold ("wrap up to N rows, then clip") — a
  row budget by another name.
- Gate: `cargo test --workspace` green · `cargo clippy --workspace --all-targets`
  clean · a live render count of one 8-call turn's tool region before/after.
