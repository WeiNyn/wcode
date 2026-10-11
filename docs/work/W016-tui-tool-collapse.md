# W016 — a collapsed tool renders as one row (with a per-tool outcome note) — brief

- **Status:** **SHIPPED** — second-layer review APPROVED. Implemented by the developer; gates green (`cargo build` · `cargo test --workspace` 1189 passed/4 ignored · `cargo clippy --workspace --all-targets` clean). See §11.
- **Work item:** W016 (`docs/work/W016-tui-tool-collapse.md`)
- **Decisions:** amends [D019](../decisions/D019-tui-tool-params-always-visible.md); proposes a new **D020**
- **Author:** brainstormer (session `agent:brainstormer`)
- **Date:** this session
- **Revision read:** `6c2b300` (`chore(homebrew): wcode v0.5.0`); working tree clean.

## 1. The ask

In the TUI, a **collapsed tool takes too much vertical space**. Today a finished,
collapsed tool is a whole tree (head + a params block + up to a 4-row output
preview + a `… +N more` hint + a `└` terminator — ~9 rows for a `read` or
`bash`). The ask: collapse to **one row**, and on that row show a **per-tool
outcome** instead of a data dump — `read` → lines read, `edit` → `+a −r`,
`write` → lines written, `bash` → exit code, `grep`/`find` → a count — while
**keeping the wall‑clock ms** (`exit 0 · 1.2s`, `+30 −12 · 4ms`). Params must not
vanish: collapsed, the first param's value rides the head (its key dropped), the
rest as `key=value`; expanding (browse `Enter`/`Space`, or `Ctrl-T`) still shows
the `│   key  value` block and the output body.

## 2. Scope / Non-scope

**In scope.**

- The collapsed render of a tool: `crates/wcode-tui/src/ui.rs::tool_inline_lines`
  (head only; params inlined on the head; overflow wraps) — the one structural
  change.
- A **per-tool completion note** derived in the TUI from `output`/`diff`
  (string-derived, like today's `+a −r`/summary), joined with `format_ms`.
- The `write` tool's **output string** gains a line count
  (`crates/wcode-cli/src/tools/write.rs`) so `write`'s note is derivable from
  the diff-less path (the diff is one-hunk/40-line-capped — the true file line
  count is unrecoverable from it).
- The spec amendment (`docs/tui-design.md` §4 + the §5 keymap line) and a
  decision record (proposed **D020**, amending D019).
- Tests: retarget the tests that asserted the collapsed params-rows/preview;
  add the one-row + counting tests; regenerate the `dump` snapshot.

**Non-scope** (explicitly not this work).

- The **kernel** (`crates/wcode-harness`) and the protocol crate — no
  `ToolOutput` field, no wire/`AgentEvent` change (locked decision #1). The
  counts are TUI string parsing of the existing `output`/`diff`.
- The **line REPL printer** (`crates/wcode-cli/src/repl.rs`) and the one-shot
  printer — they print `ToolOutput` verbatim; the `write` output string change
  is the only thing they’ll visibly pick up, by design.
- The **VS Code surface** (`docs/design/vscode-*`) — its own renderer.
- The **`▸`/`▣` glyph vocabulary and D32 hit-map mechanics** — the affix stays on
  the head row (row 0); no new affordance.
- A **config knob** for the note/collapse form — behaviour config is barred by
  `AGENTS.md`; the form is code.
- The **`find` “files” label accuracy under `kind=dir`** — the label is kept per
  the human's words (`12 files`); the miscount caveat is flagged (§5.5, O3), not
  fixed here.
- Splitting `edit`/`edits`'s `+a −r` beyond what `diff_counts` already yields —
  the 40-line diff cap is a pre-existing limit, unchanged. (`ast_edit` carries no
  `diff` field at all — see B1.)

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| W1 | Collapsed one-row head with inline params + overflow wrap (`ui.rs::tool_inline_lines`/`tool_head_row`) | **high** | **med** | **med** | The space win the human asked for; the only structural render change — the overflow-wrap + D32-affix invariant is where it can go wrong. |
| W2 | Per-tool completion note + counting rules (`ui.rs` pure fns) | **high** | **low** | **low** | Pure string parsing of existing `output`/`diff`; the value is legible outcomes; a miss degrades to the existing summary, never a crash. |
| W3 | `write` tool output gains a line count (`write.rs:59`) | **med** | **low** | **low** | One `format!`; the only way to get a truthful “lines written” (the diff is capped); touches a CLI tool string that the model also sees. |
| W4 | D019 amendment + D020 + `docs/tui-design.md` §4/§5 | **med** | **low** | **low** | Params stop being rows while collapsed; the “never hidden” invariant must be restated or it regresses again. |
| W5 | Tests + snapshot reconciliation | **high** | **med** | **low** | The collapsed-path assertions change shape; the `dump` fixture’s collapsed `bash` changes; a lazy update hides a regression. |

Overall: **high value, medium complexity, low–medium risk.** The real work is
choosing the one-row layout and proving it against the letter of D019 (params
never hide) and D031/D010 (`bash` command shown in full, `├`/`│   `/`└` tree,
frameless).

## 4. Interface & structure

### Added

- `crates/wcode-tui/src/ui.rs` — pure helpers:
  - `completion_note(tool: &Tool) -> Option<String>` — the per-tool outcome
    (§5.2 rules), `None` when nothing derivable → caller keeps the existing
    `summary_line` fallback.
  - `count_read_lines`, `parse_wrote_lines`, `parse_exit_code`, `count_grep_hits`,
    `count_find_paths` — the per-tool counters (all pure `&str -> Option<usize/…>`).
  - `collapsed_head_row(tool: &Tool, width: usize) -> Vec<Line<'static>>` — the
    one-row (plus wrap) collapsed form.
- `docs/decisions/D020-tui-collapsed-tool-is-one-row.md` — the decision record.

### Changed

- `crates/wcode-tui/src/ui.rs::tool_inline_lines` (`ui.rs:905`) — the collapsed
  branch stops emitting `panel_param_lines`/`tool_panel_body`/`tree_end_row` and
  returns the single collapsed head instead:

  ```rust
  fn tool_inline_lines(_n: usize, tool: &Tool, width: usize) -> Vec<Line<'static>> {
      let expanded = tool.expanded || tool.is_error;
      let mut body: Vec<Line<'static>> = Vec::new();
      body.extend(panel_param_lines(tool, width));   // ← stays, but only on the expanded path
  ```

  becomes, in effect:

  ```
  let expanded = tool.expanded || tool.is_error;
  if !expanded {
      // a done OR running tool is COLLAPSED: params inline on the head
      let mut out = collapsed_head_row(tool, width);
      if !tool.done {
          if let Some(tail) = last_line(&tool.output) {
              out.push(prefixed(&tree_body_prefix(), &tail, dim()));  // one live-tail row
          }
      }
      return out;                          // no note while running; no `└`
  }
  // expanded (done or running, incl. is_error): the UNCHANGED shape —
  //   head row (name only) + panel_param_lines + tool_panel_body(true) + `└`
  ```

  A **running** tool (`!tool.done`) is **collapsed** too: its head **inlines its
  params**, carries **no note** (`tool_stats` is `""` while `!tool.done`), **no
  `└`**, and exactly **one** live-tail row (A1). The earlier "expanded / running
  path" pseudo-code above was corrected — §5.1 governs.

- `crates/wcode-tui/src/ui.rs::tool_stats` (`ui.rs:1059`) — the note half stops
  being `match diff { … "+a −r" } else { summary_line }` and becomes
  `completion_note(tool)` with the same `summary_line` fallback; the
  `format_ms` join is unchanged:

  ```rust
      let note = match &tool.diff {
          Some(diff) => {
              let (added, removed) = crate::app::diff_counts(diff);
              format!("+{added} −{removed}")
          }
          None => summary_line(&tool.output).0,
      };
  ```

- `crates/wcode-tui/src/ui.rs::tool_head_row` (`ui.rs:1011`) — the left run gains
  the inlined params **on the collapsed path** (`name` + bare first-param value +
  remaining `key=value`); the expanded head is unchanged (name-only, params-empty
  fallback kept at `ui.rs:1029`):

  ```rust
      let label = match tool.target.as_ref().or(tool.path.as_ref()) {
          Some(target) if tool.params.is_empty() => format!("{}  {target}", tool.name),
          _ => tool.name.clone(),
      };
  ```

- `crates/wcode-tui/src/ui.rs::panel_param_lines` (`ui.rs:1214`) and
  `param_syntax` (`ui.rs:1166`) — **unchanged** behaviour, now reached only on
  the expanded path (they are no longer called while collapsed). `param_syntax`
  is reused to highlight the inlined first-param value:

  ```rust
  fn param_syntax(tool: &Tool, key: &str) -> Option<String> {
      match tool.name.as_str() {
          "bash" if key == "command" => Some("sh".to_string()),
          "read" | "write" | "replace" | "edit" | "edits" => {
              code_extension(tool.path.as_deref().or(tool.target.as_deref()))
          }
  ```

- `crates/wcode-cli/src/tools/write.rs:59` — the success output gains the file's
  line count:

  ```rust
                  output: format!("wrote {} bytes to {}", args.content.len(), args.path),
  ```

  → `let lines = super::diff::split_lines(&args.content).len();` and
  `format!("wrote {lines} {line|lines} ({} bytes) to {}", args.content.len(), args.path)`.

- `docs/tui-design.md:245` and `docs/tui-design.md:383-384` — the spec amendment
  (§5.4 wording).

### Integration points (verify before coding; a line number moves if the line changes)

- `crates/wcode-tui/src/ui.rs:42` — `const TOOL_PREVIEW_LINES: usize = 4;` — the
  **collapsed** output-preview cap (reviewer correction: the brief previously
  mislabeled it "expanded"). After this change `tool_panel_body` is only ever
  called with `expanded = true`, so its collapsed branch (`ui.rs:1479`
  `tool_body(&body, width, TOOL_PREVIEW_LINES, …)`) and this const become
  runtime-dead (still compiled — referenced in the fn body, so no dead-code
  warning, and there is no `deny(warnings)`). If the developer removes the
  collapsed branch, delete the const too.
- `crates/wcode-tui/src/ui.rs:905-908` — the collapsed/expanded fork site.
- `crates/wcode-tui/src/ui.rs:1011` `fn tool_head_row(tool: &Tool, width: usize) -> Line<'static> {` and `:1059` `fn tool_stats`.
- `crates/wcode-tui/src/ui.rs:1166` `param_syntax`, `:1214` `panel_param_lines`.
- `crates/wcode-tui/src/ui.rs:1440` `more_hint`, `:1465` `summary_line` — the
  fallback note path.
- `crates/wcode-tui/src/ui.rs:1355` `code_extension` — the first-param highlight.
- `crates/wcode-tui/src/ui.rs:1029-1032` — the params-empty head-target fallback
  (`tool.target.or(tool.path)`); reused for a collapsed tool whose `params` is
  empty.
- `crates/wcode-tui/src/ui.rs:855-868` — `turn_inline`'s `off += lines.len()`
  walk; the collapsed row count changes, so the D32 offsets recompute from the
  same walk (no drift by construction).
- `crates/wcode-tui/src/app.rs:228-258` — the `Tool` struct
  (`name, target, output, done, is_error, expanded, diff, path, duration_ms, params`).
- `crates/wcode-tui/src/app.rs:773-782` — `tool_param_keys` (first-param order:
  `bash`→`command`; `read`→`path`,`offset`,`limit`; `edit`/`write`→`path`;
  `grep`→`pattern`,`path`; `find`→`path`,`glob`,`kind`).
- `crates/wcode-tui/src/app.rs:285` — `diff_counts` (`+`/`-` body lines).
- **Output formats the note parses** (all re-verified this session):
  - `read` (`read.rs:93`) — `out.push_str(&format!("{n}\t{}\n", truncate_line(line)));`
    plus the page note (`read.rs:98`)
    `"… ({remaining} more lines — re-read with offset/limit to continue)\n"` and
    `"(empty file)\n"` (`read.rs:88`).
  - `write` (`write.rs:59`) — as amended above; success also carries a `diff`.
  - `bash` (`bash.rs:477-482`) — output always ends
    `parts.push(format!("exit code: {}", status.code().map_or_else(|| "terminated by signal".to_string(), |c| c.to_string())));`
  - `grep` (`grep.rs:153`) — `out.push_str(&format!("{}:{}:{}{}\n", rel_str, i + 1, line, marker));`
    with `marker = if match_idx.contains(&i) { "  <--" } else { "" }`
    (`grep.rs:152`); sentinels `[grep: truncated at {max} {unit}]` (`grep.rs:143`)
    and `grep: no matches for /…/ in …` (`grep.rs:159`).
  - `find` (`find.rs:102`) — `out.push_str(&rel_str); out.push('\n');`; sentinels
    `[find: truncated at {max} entries]` (`find.rs:91`) and
    `find: nothing matched under …` (`find.rs:107`).
  - `edit`/`edits` — success carries a `diff` field
    (`edit.rs:119`/`edits.rs:130`); a no-op or dry-run has `diff: None`.
  - `ast_edit` — **never sets the `diff` field** (all 14 `ToolOutput`s in
    `ast_edit.rs` are `diff: None`); its diff rides the **output string**
    (`ast_edit.rs:267` `… Diff:\n{diff}\n…`). So it is NOT in the `+a −r` branch
    — it falls back to `summary_line` (B1).
  - `diff` (`diff.rs:12`) — `const MAX_LINES: usize = 40;` and one hunk: the
    `+a −r` counts are **capped** for a large change (pre-existing, unchanged).

## 5. The design (the interface to build)

### 5.1 The collapsed row

One **logical** row, the tool's head. It is one **physical** row whenever the
content fits; when the inlined first-param value overflows it **wraps char-exact
on a `│   `-aligned continuation** (so a long `bash` command still shows in
full — D31/D019). Layout, left→right:

```
   ├ <name>  <first-param value (bare, key dropped, param_syntax highlight)>  <remaining key=value>  …fill…  <completion note · ms>  ▸ ▣
```

- **First param** = `tool_param_keys(name)` order (`app.rs:773`); its value
  prints bare, highlighted by `param_syntax` (`bash command` → `sh`; a file tool
  → `code_extension(path)`), else `body`.
- **Remaining params** = `key=value`, value quoted **iff it contains a space**
  (`glob="**/*.rs"`). Key `dim`, value `body`.
- **`…fill…`** runs to push the right run flush right; **`▸ ▣`** stay at the D32
  columns on row 0 (`note_affordance_cols`, `ui.rs:1083`) so the hit map is
  unchanged.
- **Width tension:** the note is capped at **40** chars and **dropped when
  crowded** — if `left + note + " · " + ms + aff` doesn't fit with ≥1 fill column,
  drop the note but keep ms; if ms still doesn't fit, drop both. (`summary_line`
  can be 80 today; a 40-cap keeps the head legible. The derived notes — `48 lines`,
  `+30 −12`, `exit 0` — are all short.)
- **Running** (`!tool.done`): head (params inlined, no note) + exactly **one**
  live-tail row under `│   ` (`last_line`), **no `└`** (A1). The "one row" rule
  applies to the *done* collapsed state.
- **Params-empty fallback (B2).** When `tool.params` is empty — a reseeded/
  session-replayed tool whose `path` came from `ToolExecutionEnd`, not args —
  the collapsed head inlines `tool.target.or(tool.path)` (the SAME fallback as
  HEAD `ui.rs:1029-1032`), still under the 40-cap note. Without this the target
  drops off a collapsed reseeded tool (a D019 regression).
- **Expanded** (`Ctrl-T`/browse `Enter`/`Space`, or `is_error`): the **unchanged**
  shape — head (name only) + the `│   key  value` params block + the output/diff
  body + `… +N more` + `└`.

### 5.2 The completion note (counting rules)

`completion_note(tool)`, only when `tool.done` and `!is_error`:

| tool | note | derivation | sentinels excluded |
|---|---|---|---|
| `read` | `N line(s)` | count lines matching `^\d+\t` | `(empty file)` (no tab), `… (M more lines …)` (starts with `…`) |
| `write` | `N line(s)` | `wrote (\d+) line` from the amended output | error strings (`write …: …`) |
| `edit`\|`edits` | `+a −r` | `diff_counts(tool.diff)`, driven by `tool.diff.is_some()` (A2) | — |
| `ast_edit` | fallback `summary_line` | **no `diff` field** (B1); optional: parse its embedded diff | — |
| `bash` | `exit N` | last line matching `^exit code: (\d+)$`; `terminated by signal` → fallback | — |
| `grep` | `N hit(s)` | count lines ending `  <--` | `grep: no matches…`, `[grep: truncated at N …]` |
| `find` | `N file(s)` | count non-sentinel path rows | `find: nothing matched…`, `[find: truncated at N entries]` |
| else | `summary_line(&output).0` | first non-blank line (`ui.rs:1465`); the 40-cap is applied in `collapsed_head_row`, not here (A4) | — |

- The note is joined to ms by ` · ` (`tool_stats`), so the head reads
  `48 lines · 12ms`, `exit 0 · 1.2s`, `+30 −12 · 4ms`.
- **Singular/plural** is a nicety: `1 line`/`N lines`, `1 hit`/`N hits`,
  `1 file`/`N files`.
- **`read` “lines read”** = the shown page, not the file's total (a paged read
  caps at `MAX_READ_LINES`); the note reports the page (O4).
- **`grep` “hits”** = match rows (`  <--`), not context rows.
- **Dispatch by name, before the diff (A2).** `completion_note` reads
  `tool.name` first: `"write"` → `parse_wrote_lines(&tool.output)`; the
  diff-driven `+a −r` branch is `edit`/`edits` only and is **unreachable** for
  `write` (its diff is a capped one-hunk preview — it cannot yield the true line
  count). Today `tool_stats` shows `+a −r` for `write`; that is what changes.
- **Sentinel `Option` = `Some(0)` (A5).** `(empty file)` → `0 lines`,
  `grep: no matches…` → `0 hits`, `find: nothing matched…` → `0 files`. A
  **truncation** sentinel does NOT suppress the count — the shown rows count
  (it is merely a non-matching trailing line), mirroring O4.

### 5.3 The `write` output change (required sub-task)

`write.rs:59` becomes, with `let lines = super::diff::split_lines(&args.content).len();`:

```
wrote {N} {line|lines} ({bytes} bytes) to {path}
```

`split_lines("")` = `[]` → `0 lines` for an empty write. The existing test
`creates_parent_directories` (`write.rs:96`, `assert!(out.output.contains("5 bytes"))`)
stays green (bytes are retained). The TUI parses `wrote (\d+) line`.

### 5.4 The D019/W015 amendment — exact wording

Replace `docs/tui-design.md:245` (currently *“A tool's **params rows always
render** — running or done, collapsed or expanded.”*) with:

> A tool’s **params are never hidden**. **Collapsed**, they ride the head row:
> the first param’s value prints **bare** (its key dropped, its syntax highlight
> kept), the rest as `key=value`; the head wraps on a `│   `-aligned
> continuation when it overflows, so the `bash` command still shows in full.
> **Expanded**, they re-draw as the `│   key  value` rows. The `▸`/`▾`
> disclosure and `Ctrl-T` govern the tool’s **output body** only (params are the
> call’s identity and never hide).

Replace `docs/tui-design.md:383-384` (currently *“…every tool’s **output**
(params stay visible; a collapsed tool shows a 4-line preview, a failed tool
always shows its error)”*) with:

> `Ctrl-T` expands/collapses every tool’s **output** (params stay visible —
> inlined on a collapsed tool’s single row, as `│   key  value` rows when
> expanded; a failed tool always shows its error) ·

**D-record or amendment?** Recommend a **new `D020-tui-collapsed-tool-is-one-row`**
that **amends D019** on the *form* clause (“rows”) while preserving D019’s
*invariant* (“params never hide”). D019 stays as the record of the visibility
invariant (it regressed silently once — worth keeping intact); D020 records the
one-row form and the outcome note. (If the human prefers minimal records, an
in-place D019 amendment is viable but loses the “form vs. invariant” separation.)

### 5.5 Sub-decisions — RESOLVED (human sign-off)

- **O1 — the running tool:** **KEEP** one live-tail row; the “exactly one row”
  rule applies to the *done* collapsed state.
- **O2 — the `└` terminator while running:** **DROP** it for a running tool;
  keep `└` only on the expanded path.
- **O3 — `find` label:** **`N files`** now (per the human's words); `N paths`
  remains the honest alternative for `kind=dir`, flagged not fixed.
- **O4 — `read` count meaning:** **shown page**, not the file total (that is
  what “lines read” means); no `+`.
- **`write` output change:** **APPROVED** — `wrote {N} {line|lines} ({bytes} bytes) to {path}`.
- **D-record:** **APPROVED** — a new **D020** amending D019 (invariant preserved,
  form changed).

### 5.6 First-layer review — BLOCKING items + settled amendments (`6c2b300`)

The first-layer gate BLOCKED on B1–B2 and settled A1–A5. The developer MUST honour
these; they override any contradicting prose above.

- **B1 — `ast_edit` has no `diff` field.** Every `ToolOutput` in `ast_edit.rs` is
  `diff: None`; its diff rides the **output string**. The `+a −r` branch is driven
  by `tool.diff.is_some()` (`edit`/`edits` only); `ast_edit` falls back to
  `summary_line`. No test may assert `ast_edit → +a −r`.
- **B2 — `collapsed_head_row`'s params-empty fallback.** When `tool.params` is
  empty, inline `tool.target.or(tool.path)` (HEAD `ui.rs:1029-1032`), still under
  the 40-cap note.
- **A1 — the running form = §5.1, not §4's old pseudo-code.** State map:
  `done && !expanded` → `collapsed_head_row` (params inline), no `└`;
  `!done && !expanded` → `collapsed_head_row` + one `│   ` live-tail row, **no
  note**, **no `└`**; `expanded` (done or running, incl. `is_error`) → name-only
  head + `panel_param_lines` + `tool_panel_body(true)` + `└`, unchanged.
- **A2 — `write` note source.** `completion_note` dispatches by `tool.name`
  BEFORE any `tool.diff` read.
- **A3 — keep `tool_head_row` name-only** (expanded path); the span-aware
  collapsed form lives in `collapsed_head_row`. Do NOT thread params through
  `tool_head_row`.
- **A4 — the 40-cap is collapsed-only.** `completion_note` returns the note
  uncapped; only `collapsed_head_row` applies the cap + drop-when-crowded.
- **A5 — sentinel `Option` = `Some(0)`**; truncation sentinels don't suppress the
  count.

## 6. Plan (deliverable + gate per step)

1. **Amend the spec + open D020.** Edit `docs/tui-design.md:245` and `:383-384`
   (§5.4); write `docs/decisions/D020-tui-collapsed-tool-is-one-row.md`.
   *Deliverable:* amended §4/§5 + the D020 record. *Gate:*
   `grep -n "params are never hidden" docs/tui-design.md` hits; a human reads §4.
2. **`write` output line count.** `write.rs:59` + `let lines = split_lines(…)`.
   *Deliverable:* the new string. *Gate:* `cargo test -p wcode-cli
   tools::write::tests::creates_parent_directories` green; a new CLI unit test
   asserts `wrote 1 line (5 bytes) to …` and a `0 lines` empty-write case.
3. **The counting helpers.** `completion_note` + the five counters (§5.2) with
   unit tests over literal strings (including every sentinel). *Deliverable:*
   pure functions + tests. *Gate:* `cargo test -p wcode-tui ui::` (the new
   `*_note_*`/`counts_*` tests).
4. **The collapsed one-row render.** `tool_inline_lines` fork +
   `collapsed_head_row` (inline params, `param_syntax` highlight, no-space
   quote, overflow wrap, D32 affix on row 0, 40-cap note + drop-if-crowded);
   keep the expanded path's `│   key  value` rows and `└`. *Deliverable:* the
   one-row collapsed tool + a wrapping long-`bash` case. *Gate:*
   `cargo build -p wcode-tui` + the new `a_collapsed_tool_is_one_row` /
   `a_long_bash_command_wraps_collapsed` / `a_collapsed_tool_shows_its_outcome_note`
   tests.
5. **Reconcile the existing tests** (§7.1 table). *Deliverable:* the retargeted/
   removed assertions. *Gate:* `cargo test -p wcode-tui ui::`.
6. **Regenerate the snapshot.** `cargo run -p wcode-tui --example dump`.
   *Deliverable:* updated `target/tui-render/*`. *Gate:* `git diff` (or a fresh
   `cat`) shows the collapsed `bash` as one row with its note.
7. **Full gates + live check** (§6). *Deliverable:* a clean run.

## 7. Quality gates

- `cargo test --workspace` → all pass (tui · cli · harness · protocol).
- `cargo clippy --workspace --all-targets` → clean (`AGENTS.md`).
- `cargo run -p wcode-tui --example dump` → writes `target/tui-render/{80x24,120x40,48x20}.{txt,ansi}`; `120x40.txt` shows the finished **collapsed** `bash` as one row (`├ bash  <note · ms>  ▸ ▣`) with no `│   command` block and no `└`;
  `grep -c '╭' target/tui-render/120x40.txt` = 0.
- Live: `cargo run -p wcode-cli -- --base-url http://localhost:11434/v1` (keyless
  Ollama) — `clear; make` style turns show the one-row collapsed tools with
  `exit 0 · <ms>`; `Ctrl-T` expands to params + output.
- **Regression tests kept green (by name):**
  `a_completed_tool_shows_a_start_line_and_an_end_line` (`ui.rs:4593`),
  `a_completed_tool_shows_its_duration` (`ui.rs:2598`),
  `a_note_is_frameless` (`ui.rs:4862`),
  `the_tool_rows_share_the_content_column` (`ui.rs:4844`),
  `a_tool_diff_renders_when_expanded` (`ui.rs:2623`),
  `enter_expands_the_selected_tool_fully` (`ui.rs:2958`),
  `an_errored_tool_renders_expanded` (`ui.rs:2976`),
  `the_more_hint_pluralizes_one_hidden_line` (`ui.rs:3077`),
  `a_wide_band_publishes_the_note_cells_on_the_centered_column` (`ui.rs:5182`),
  `tool_lifecycle_becomes_one_tool_block` (`app.rs:5095`).

### 7.1 Automated

**New** (each asserts behaviour, not that it runs):

- `ui::a_collapsed_tool_is_one_row` — a finished `read`/`bash` renders **one**
  transcript row (no `│`, no `└`); the second transcript line is the next block.
- `ui::a_collapsed_tool_shows_its_outcome_note` — a `read` with 3 `N\t` lines
  shows `3 lines · <ms>`; a `write` shows `N lines`; a `bash` with a trailing
  `exit code: 0` shows `exit 0 · <ms>`; an `edit` with a diff shows `+a −r`.
- `ui::a_long_bash_command_wraps_on_the_collapsed_head` — a >width `command`
  renders in full across a `│   `-aligned continuation (no `…` clip).
- `ui::a_collapsed_tool_inlines_params_key_dropped` — `read` shows the bare
  `path` value and `offset=40` (first key dropped, the rest `key=value`).
- `ui::counts_*` (unit) — `read` excludes `(empty file)` and the page note;
  `grep` counts `  <--` and ignores the no-match/truncated sentinels; `find`
  ignores the truncation/nothing-matched sentinels; `bash` parses the exit code
  and rejects `terminated by signal`; `write` parses `wrote N line(s)`.

**Retarget / remove** (the one structural break — see the table below):

- `ui::a_done_collapsed_tool_shows_no_target_on_the_head_when_params_render`
  (`ui.rs:4765`) — **must be removed/replaced**: the head now *does* inline the
  command. Its intent (params are never hidden) is preserved by the new
  `a_collapsed_tool_inlines_params_key_dropped`.
- `ui::a_done_tool_shows_its_head_rows_and_a_preview` (`ui.rs:2935`) — the
  collapsed `line-2` preview and `… +15 more lines` hint assertions move onto the
  expanded path.

**Honest breakage analysis (I did not run the suite).** On inspection, only two
existing tests clearly fail structurally: those two above. The orchestrator named
`a_long_diff_is_capped_until_expanded` (`ui.rs:2995`) and
`a_multi_line_tool_output_expands_to_all` (`ui.rs:3057`) too; on the brief's read
their collapsed assertions still hold (`+30 −0` via `diff_counts`; `row 1` via
the note fallback) — **verify by running**; if they fail, retarget identically.
The W015 five-test set: `a_done_collapsed_bash_shows_its_command` (passes — the
value inlines), `a_done_collapsed_read_shows_its_path_and_offset` (passes —
`offset=40` contains `offset`, `40`), `a_done_collapsed_write_shows_its_path`
(passes), `a_done_collapsed_find_shows_path_and_glob` (passes), and the
no-target test (breaks). The expanded-path tests all pass.

### 7.2 How a human verifies it

```sh
cargo run -p wcode-tui --example dump
cat target/tui-render/120x40.txt        # one-row collapsed bash, note · ms, no │/└
grep -c '╭' target/tui-render/120x40.txt   # 0

cargo run -p wcode-cli -- --base-url http://localhost:11434/v1
```

In the live TUI, ask for a turn that runs `bash`, `read`, `edit`, `write`,
`grep`, `find`. **Look for:** each finished tool is **one** row
`├ name  <first value>  [key=value…]  …  <outcome · ms>  ▸ ▣`; the outcome reads
`exit 0`, `48 lines`, `+30 −12`, `3 hits`, `12 files` respectively; `Ctrl-T`
(or browse `Enter`) redraws the `│   key  value` block + the output; a long
`bash` command wraps rather than clipping; a failed tool stays expanded.

## 8. Expected outcome

- A finished, collapsed tool occupies **one row** (the head) — down from ~9.
- The row shows a **per-tool outcome** — `read`/`write` line counts, `edit`/`edits` `+a −r`, `bash` `exit N`, `grep` hits, `find` files (`ast_edit` falls back to its summary; B1) — joined with the wall-clock ms by ` · `.
- Params are **never hidden**: collapsed, the first value rides the head (bare,
  highlighted) and the rest as `key=value`; expanded, the `│   key  value` block
  returns. A long `bash` command wraps, never clips.
- `write`'s output string carries a line count; the model and the REPL see the
  amended one-liner too.
- `docs/tui-design.md` §4/§5 state the one-row rule; D020 (amending D019) exists.
- `cargo test --workspace` green · `cargo clippy --workspace --all-targets` clean
  · the `dump` snapshot shows a one-row collapsed `bash`.

## 9. References

- `docs/decisions/D019-tui-tool-params-always-visible.md` — the “params never
  hide” invariant this change amends in form; the five-test gate.
- `docs/work/W015-tui-tool-params-collapsed.md` — the predecessor (the params
  gate) and its outcome/test set.
- `docs/tui-design.md:236-249` (§4 Tools) and `:383-384` (§5 keymap) — the
  authority being amended.
- `docs/work/tui-render/redesign-v2.md` §3 — the tool-panel mock-ups (params,
  expand/collapse, “runs the real `ui::draw`” frame intent).
- `docs/next-steps.md:95` (item 78 / W015) — the tracker entry; W016 follows.
- `AGENTS.md` (Minimalism; gotchas) and `.wcode/skills/live-verification` — the
  no-config-behaviour rule and the verification discipline.

## 10. Facts I did NOT verify

- **I ran nothing** — no `cargo build`/`test`/`clippy`/`dump`, no TUI launch. All
  line numbers come from a read-only static read at `6c2b300` and may drift; the
  quoted snippets are the durable anchor.
- **Which of the named tests actually fail** — reasoned from the code
  (§7.1), not executed. The break is structural only for the two tests named;
  the others are flagged “verify by running”.
- **The exact `dump.rs` fixture output** and whether `target/tui-render/*` is
  tracked or gitignored (W015 says gitignored — not re-checked).
- **The `running`-tool one-row form** (O1/O2) — a design choice not yet signed
  off; the brief's default (keep one live-tail row) is the recommendation.
- **Whether any app-level (`app.rs`) or integration test asserts a collapsed
  transcript height / `│` count** — the `ui.rs` tests were enumerated by name
  but not every `render(` assertion in the suite was read.
- **`write`'s amended string reaching a model/REPL test elsewhere** — only
  `write.rs:96` was found by grep; a broader assertion elsewhere was not ruled
  out.
- ~~**`ast_edit`'s success carries a `diff`**~~ — **REFUTED by the first-layer
  review (B1):** all 14 `ast_edit.rs` `ToolOutput`s are `diff: None`; its diff
  rides the output string.
- **The next D number** — `docs/decisions/` tops out at D019, so D020 is proposed
  but not reserved.

**This brief ends with a brief.**

## 11. Outcome (shipped)

**APPROVED at second layer.** Implemented by the developer (which hit its turn
limit before reporting; the reviewer verified completeness itself). Gates:
`cargo build` clean · `cargo test --workspace` **1189 passed, 4 ignored** ·
`cargo clippy --workspace --all-targets` clean · `cargo run -p wcode-tui --example
dump` shows the one-row collapsed tools (`grep -c '╭' target/tui-render/120x40.txt`
= 0).

**Commits.**
- `666add1` `cli:` — `write` reports its line count in the success output.
- `f8e8a4f` `tui:` — collapse a finished tool to one row with a per-tool outcome
  note (+ the `dump` fixture).
- `docs:` — this brief, D020, the `docs/tui-design.md` §4/§5 amendment, and the
  tracker (item 79).

**All B1–B2 + A1–A5 landed** (verified by the second-layer review). Zero `SKETCH`
markers remain.

**Deviations.** Exactly the two predicted tests changed
(`a_done_tool_shows_its_head_rows_and_a_preview` retargeted to the expanded path;
`a_done_collapsed_tool_shows_no_target_on_the_head_when_params_render` replaced by
`a_collapsed_tool_is_one_row`). No other test touched.

**Not run (stated plainly).** The interactive live TUI check — the reviewer's
worker lacks a TTY and the local model emitted no tool call under one-shot
prompts; the headless `dump` example (which runs the real `ui::draw`) is the
render proof, and the counters + `write` string are unit-proven.

**Non-blocking follow-ups (reviewer).**
- `tool_panel_body`'s `!tool.done` / `expanded=false` branches are now runtime-dead
  (kept referenced, so no warning); simplifying is a clean follow-up.
- `examples/dump.rs:214,218` carry stale "expand the `read`" comments (pre-existing).
