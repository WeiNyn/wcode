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
(`ui.rs:1161`) inlined the first param's value and, when the head overflowed,
**wrapped it char-exact onto `│   `-aligned continuation rows with no row
budget** (`ui.rs:1232-1247`). Its doc promised it — *"A too-long head WRAPS
char-exact onto a gutter-aligned continuation (never a `…` clip), so a long
`bash` command shows in FULL (D31/D019)"*.

The asymmetry is what floods: the **stats** are bounded (the note is capped at 40
chars, the fallback summary at 80 — `ui.rs:1201`, `ui.rs:1886`) while the
**params** are unbounded, so one heredoc `bash` command or one long `sed` script
turns a "one row" tool into ten. Meanwhile the neighbouring surfaces already
carry the right rule: the settings detail field clips with `…`
(`ui.rs:432`, *"An over-long value clips with `…` (clip-not-wrap, D027) — no
silent shortening"*).

Two further defects sat in the same code:

- Both the collapsed head (`ui.rs:1232-1247`) and the expanded params block
  (`wrap_styled`, `ui.rs:1783-1803`) flatten a value into **chars**, so a
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
   remaining line count, singular `line` for one), and that tail is **priced into
   the budget**. No `\n` char may reach a `Span` — not in the head, not in the
   params block.

4. **Degradation is a fixed ladder**, extending the existing A4 rule instead of
   inventing a second one: **(a)** drop the note → **(b)** drop the ms → **(c)**
   drop the extra `key=value` params, from the tail → **(d)** drop the ` … +N
   lines` tail → **(e)** **clip the first value with a floor of 8 columns**.
   The **name never drops** — the name is the only part that says *what ran*.

5. **Expansion is the complete record.** `param_row` splits the value on `\n` and
   renders each logical line as its own wrapped run, continuation rows keeping
   the `value_col` alignment. Nothing is elided once expanded.

6. **The spec says "never `LOST`", not "never hidden"** (D019's invariant,
   restated): a param's value is elided in the collapsed head and complete on
   expansion, so no information is unreachable. `Ctrl-T` still governs the
   **output body** only.

## Mechanism

- `collapsed_head_row` returns a single `Line<'static>` — the "one row" contract
  is now the type, not a doc comment. It flattens the first value to its first
  logical line, prices ` … +N lines`, walks the ladder above, and reuses
  `truncate` (the existing `…` clip) for the last resort. The wrap-to-continuation
  branch and `spans_from_chars` (its only caller) are **deleted**.
- `param_row` maps each `value.split('\n')` line through `wrap_styled`, so a
  heredoc command keeps its line structure; an empty value still yields one row.
- `ui.rs:1159-1160` and `ui.rs:1120-1121` are reworded: the head elides, it does
  not show the command "in full" — expansion does.

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
