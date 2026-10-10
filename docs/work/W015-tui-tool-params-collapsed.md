# W015 — TUI tool panels do not render their input when collapsed

**Type:** defect fix (render gate) + spec clarification
**Status:** DONE — implemented `1927aae`, second-layer **APPROVED** (zero blockers),
recorded as [D019](../decisions/D019-tui-tool-params-always-visible.md). See §12 for the outcome.

---

## 1. The ask

> "there's a problem with wcode TUI that the tools don't render their input, specifically bash."

Restated as a defect: a **finished, collapsed, non-error** tool panel renders its
head row and its output preview, but **not its input params** — so a completed
`bash` shows `├ bash` with no `command`, which is the one thing the tool is *for*.
Confirmed as a render gate, not a data problem (the args reach the model and are
already collected into `Tool.params`).

---

## 2. Scope and non-scope

### In scope
- The single gate at `crates/wcode-tui/src/ui.rs:902-905` (`tool_inline_lines`)
  that suppresses `panel_param_lines` for a done, collapsed, non-error tool.
- The head-target inlining rule at `ui.rs:1023-1026` (`tool_head_row`), as the
  dependent that must stay coherent.
- The spec clarification in `docs/tui-design.md` §4 to make "params always
  render" explicit.
- Tests: a new collapsed-path assertion; fixing the `cmd`-vs-`command` fixture
  fidelity gap.

### Non-scope (explicitly NOT this work)
- The **panel frame form**. The shipped render is the **D010 tree** (`├ name`,
  `│   cmd`, `└`), *not* the boxed `╭─ ⚙ bash ─╮` of `redesign-v2.md`; the box
  mockups are intent, not the shipped frame (`ui.rs:4716` `a_note_is_frameless`
  bans `╭`). Do **not** re-introduce the box.
- **Which keys a tool lists** (`tool_param_keys`, `app.rs:773-782`) and the
  `ACTION_KEYS` superset. The bug is *when* params render, not *what* they are —
  **with one exception**: the `cmd`/`command` fixture decision (§4, O2).
- **Copy semantics** — `copy_text` already returns a tool's *full output*
  regardless of collapse (`app.rs:354-358`); untouched.
- The **`¹` ordinal / panel-count contract / D32 hit map** — analyzed in §6 as
  *not* at risk, but not modified.
- The **kernel** (`wcode-harness`), the protocol crate, and the CLI REPL/one-shot
  surfaces.
- The **`⚙`/`✓`/`⧉`→`▣` glyph vocabulary** and `redesign-v2`'s framed-panel pixels.

---

## 3. Value / complexity / risk (per workstream)

| # | workstream | value | complexity | risk | one-line why |
|---|---|---|---|---|---|
| W1 | Relax the param gate (`ui.rs:902-905`) | **high** | **low** | **low** | One boolean removed from a single `if`; the render walk computes its own row offsets, so nothing downstream needs touching. |
| W2 | Confirm/keep the head-target fallback (`ui.rs:1023-1026`) | **medium** | **low** | **low** | The `target` inlining becomes a *fallback* for calls whose args miss the per-tool key list; only `ui.rs:1023` reads it for display. |
| W3 | Amend `docs/tui-design.md` §4 | **medium** | **low** | **low** | The spec frames `Ctrl-T` as governing *output*, but never states params persist collapsed — make it explicit so the gate can't regress. |
| W4 | Tests + fixture fidelity | **high** | **low** | **low** | The W003 gate landed only as the expand-gated variant; the collapsed path is untested today. |
| W5 | Snapshot/dump regeneration | **low** | **low** | **low** | `examples/dump.rs` output changes; regenerate, don't hand-edit. |

Overall: **high value, low complexity, low risk.** The change is one condition
plus a sentence of spec; the real work is choosing the rule (§4) and proving it
(§7).

---

## 4. The design decision — the recommended rule

**Rule: a tool's params rows ALWAYS render — collapsed or expanded, running or
done. The `▸`/`▾` toggle (and `Ctrl-T`) governs only the *output body* (and the
diff/output preview), never the params.**

This holds for **every** tool, with no per-tool exception. Evidence:

1. **The locked design already draws params collapsed.**
   `docs/work/tui-render/redesign-v2.md:193-203` §3(a) "collapsed (80×24)"
   literally draws the param row inside the collapsed panel:
   ```
   ╭─ ⚙ bash ─────────────────────────────── ▸ ⧉ ─╮
   │  cmd  cargo test -p wcode-cli                  │
   │  ✓ 41 passed · 12 lines · 9ms                  │
   ```
   and §3(b) `redesign-v2.md:218` — *"the params row stays visible when expanded
   (never scrolls away)."* Intent is params-visible in **both** states.
2. **D31 requires the full command shown** — `docs/team-and-tui-plan.md:258`:
   *"the **`bash` command is shown in full** (wraps, never clipped)."* A gate that
   hides it on the common (finished, collapsed) path violates D31.
3. **The spec scopes collapse to the *output*.** `docs/tui-design.md:380-381`:
   *"`Ctrl-T` expands/collapses every tool's **output** (a collapsed tool shows a
   4-line preview, a failed tool always shows its error)."* And
   `tui-design.md:240-246` describes a tool's body as *"(`│   ` params +
   output/diff/preview + a `… +N more` hint)."* Params are part of the body's
   head, not the collapsible output.
   (`docs/tui-design.md:244` is the line that already says *"The **`bash` command
   is shown in full** (it wraps, never clips)."*)
4. **No tool has a param worth hiding.** `tool_param_keys` (`app.rs:773-782`)
   bounds params to ≤3 short scalars: `bash` → `command`/`cmd`/`cwd`; `read` →
   `path`/`offset`/`limit`; `edit`/`write` → `path` **only** (the payload is the
   *diff body*, `ui.rs:1250-1269`, not a param); `grep` → `pattern`/`path`;
   `find` → `pattern`. The "long `edit`/`write` payload" worry does **not** apply.
   And `read`'s `offset`/`limit` render **only when the call set them**
   (`call_params` filter-maps present keys, `app.rs:823-828`), so a shown `offset`
   is *signal* (which slice), not noise.
5. **A per-tool "hide while collapsed" list would be config-for-behavior**, which
   `AGENTS.md` (Minimalism) says belongs in code as a hook, not as a table. There
   is no tool that needs it.

**Spec amendment required — yes.** `docs/tui-design.md` is the authority, and
while it is *consistent* with this rule it never *states* it (that omission is
what let the gate land). Add one sentence:

- After `docs/tui-design.md:244` (the *"The **`bash` command is shown in full**
  (it wraps, never clips)."* line), insert:
  > *"A tool's **params rows always render** — running or done, collapsed or
  > expanded. The `▸`/`▾` disclosure and `Ctrl-T` govern the tool's **output
  > body** only (params are the call's identity and never hide)."*
- Optionally tighten `tui-design.md:380-381` to read *"…expands/collapses every
  tool's **output** (params stay visible; a collapsed tool shows a 4-line
  preview…)"*.

### Head-target interaction — confirmed intended
`ui.rs:1023-1026`:
```rust
let label = match tool.target.as_ref().or(tool.path.as_ref()) {
    Some(target) if tool.params.is_empty() => format!("{}  {target}", tool.name),
    _ => tool.name.clone(),
};
```
Once params always render, a done `bash` has non-empty `params`, so the head shows
**just the name** and the value rides the params rows — exactly what the spec
intends ("params carry the target"). The target still inlines **only as a
fallback** when `params` is empty — e.g. a call whose arg key isn't in its
per-tool list (`read` with `file_path` rather than `path`: `tool_param_keys("read")`
misses it so `params` is empty, but `call_target`'s `ACTION_KEYS` catches it,
`app.rs:846-848`), or a reseeded tool. **Keep the guard**; do not remove it. Grep
confirms `tool.target` is read for display at **only** `ui.rs:1023`, plus a
`code_extension` fallback at `ui.rs:1164` and `ui.rs:1342` (syntax highlighting —
unaffected). Nothing else depends on the head target.

---

## 5. Interface and integration points

The defect is one `if`; the surrounding interface is unchanged.

- **The gate (the fix site)** — `crates/wcode-tui/src/ui.rs:900-906`:
  ```rust
  fn tool_inline_lines(_n: usize, tool: &Tool, width: usize) -> Vec<Line<'static>> {
      let expanded = tool.expanded || tool.is_error;
      let mut body: Vec<Line<'static>> = Vec::new();
      if !tool.done || expanded {
          body.extend(panel_param_lines(tool, width));
      }
      body.extend(tool_panel_body(tool, width, expanded));
  ```
  Fix shape: drop the condition — `body.extend(panel_param_lines(tool, width));`
  unconditionally. (`tool_panel_body` still keys the *output/diff preview* off
  `expanded`, so collapse keeps its meaning.)
- **Params producer** — `ui.rs:1208-1225` `panel_param_lines`: returns `Vec::new()`
  iff `tool.params.is_empty()`; otherwise one wrapped `param_row` per `(key,value)`
  (keys aligned, values syntax-highlighted via `param_syntax`, continuation lines
  aligned under the value column, `ui.rs:1183-1189`).
- **Where `params` is populated** — `app.rs:1613-1614` and `app.rs:2042-2043`,
  both `call_params(&self.transcript, &call_id, &name)` (`app.rs:823-828`), which
  scans the turn's `ToolCall{arguments}` via `call_arg` (`app.rs:789-817`).
- **`Tool.params` contract** — `app.rs:253-257`: *"empty when the call carried no
  recognizable argument — the panel then draws no params rows (never an empty key
  row)."* This already frames "no rows" as *no args*, not *collapsed* — consistent
  with the fix.
- **Head row** — `ui.rs:1008-1048` `tool_head_row`; the target rule at `:1023-1026`.
- **Row-offset / `¹` walk** — `ui.rs:814-874` `turn_inline`: `off += lines.len()`
  after each emitted tool (`:864-867`), and the `¹` mark at `:855-859`. Both are
  computed from the same walk that calls `tool_inline_lines`, so added param rows
  shift `off` in lockstep — **no drift** (contract at `ui.rs:818-825`).

---

## 6. Blast radius — does it re-introduce what D31/D009 fixed?

**No.** Checked against the three original complaints and the contracts:

- **Duplication ("the name printed twice")** came from a `1 » name` head *plus* a
  separate `✓ name` row (D010 Context). Our change adds **key/value** rows, never
  the name; the head still prints it once (`ui.rs:1007` "The name prints ONCE").
  Tests `a_done_tool_shows_its_head_rows_and_a_preview` (`ui.rs:2941`) and
  `a_completed_tool_shows_a_start_line_and_an_end_line` (`ui.rs:4595`) assert
  `matches("bash").count() == 1` and stay green (their fixtures pass
  `arguments:{}`, `ui.rs:2899`, so `params` is empty anyway).
- **Clipping (the 60-char target)** was the *head* path. The params path **wraps**
  (`param_row`, `ui.rs:1183-1189`), so promoting params to always-visible *removes*
  dependence on a clipped head target — the opposite of a regression.
- **The `¹` ordinal** (`ui.rs:855-859`) is per-`ToolCall`, independent of params.
  Untouched.
- **One-panel-per-tool contract** (`ui.rs:811-816`): emission happens at
  `turn_inline`'s `ToolCall` arm (`ui.rs:863-868`) exactly once per call whose
  result exists; adding rows *inside* the panel changes no count.
- **D32 hit map / affordance cells**: the `▸`/`▣` live on the **head row (row 0)**,
  which stays first after params are prepended as *body* rows;
  `inline_tool_offsets` (`ui.rs:823`) recomputes from the same walk.
  `a_wide_band_publishes_the_note_cells…` (`ui.rs:5036`) and
  `a_note_toggle_and_copy_target_the_note` (`ui.rs:4793`) target row 0 and are
  unaffected.
- **Residual risk = test churn + snapshots**, not behavior: any test asserting an
  exact transcript height or a `└`/`│` count on a *collapsed* tool, and the
  `examples/dump.rs` snapshot (`target/tui-render/*`). Audit these (plan step 4).

---

## 7. Numbered plan (deliverable + gate per step)

1. **Amend the spec.** Edit `docs/tui-design.md` §4 with the one-sentence "params
   always render" rule (§4). *Deliverable:* spec line added. *Gate:* human reads
   the amended §4; `grep -n "always render" docs/tui-design.md` hits.
2. **Drop the gate.** Change `ui.rs:902-905` to call `panel_param_lines`
   unconditionally; keep `tool_panel_body`'s `expanded` keying for the output.
   *Deliverable:* the condition gone. *Gate:* `cargo build -p wcode-tui`.
3. **Keep the head fallback.** Confirm `ui.rs:1023-1026` still compiles/renders; no
   code change expected (documented in the commit). *Deliverable:* a comment citing
   the fallback intent. *Gate:* review + the step-5 test on the `params`-empty
   path.
4. **Reconcile the existing tests.** (a) Retarget
   `a_note_shows_its_params_when_expanded` (`ui.rs:4674`) — its `Ctrl-T` is now
   redundant; either drop the keystroke to prove the *collapsed* path, or keep it
   as an idempotency check. (b) Regenerate the `examples/dump.rs` snapshot.
   *Deliverable:* updated tests + snapshot. *Gate:* `cargo run -p wcode-tui
   --example dump` then `git diff target/tui-render/` shows only the added param
   rows.
5. **Add the collapsed-path tests** (§8, named). *Deliverable:* green new tests.
   *Gate:* `cargo test -p wcode-tui ui::`.
6. **Full quality gate + live check** (§8). *Deliverable:* clean run. *Gate:* the
   commands below.

---

## 8. Quality gates, automated tests, human check

**Exact commands (must all pass):**
```sh
cargo test --workspace
cargo clippy --workspace --all-targets     # must be clean (AGENTS.md)
```

**Automated tests to add/rename (name them):**
- `ui::a_done_collapsed_tool_shows_its_params` — push a **finished** `bash` with
  `arguments:{"command":"cargo test -p wcode-cli","cwd":"/w"}`, render at 80×20
  **without** `Ctrl-T`, assert the `command` value (and `cwd`) appear and the
  `├ bash` head is present. This is the W003 gate `a_tool_panel_shows_its_params`
  (`docs/work/W003-tui-redesign.md:142`) landed as the real thing, not the
  expand-gated `a_note_shows_its_params_when_expanded` (`ui.rs:4674`).
- `ui::a_done_collapsed_tool_shows_no_target_on_the_head_when_params_render` —
  assert the head is `├ bash` (no inlined command) once params carry it, locking
  the §4 interaction.
- `ui::a_done_collapsed_read_shows_path_and_its_offsets` — a `read` with
  `path`+`offset` renders the `path` and the `offset` rows collapsed (proves the
  rule is per-tool, not bash-only).
- `app::call_params_returns_command_and_cwd_for_bash` — **fix the fixture** to the
  production key `command` (today `app.rs:8775`/`ui.rs:4614` use the alias `cmd`,
  while production carries `command`, `ui.rs:4561`, `ui.rs:3253`); the alias
  fixture means `param_syntax`'s `bash if key == "command"` branch (`ui.rs:1162`)
  is never exercised and the printed key label differs (`cmd` vs `command`).
- Keep green (regression, by name): `a_completed_tool_shows_a_start_line_and_an_end_line`,
  `a_done_tool_shows_its_head_rows_and_a_preview`, `a_note_shows_the_full_command_unclipped`
  (`ui.rs:4654`), `a_note_is_frameless` (`ui.rs:4716`), `the_tool_rows_share_the_content_column`
  (`ui.rs:4697`), `a_wide_band_publishes_the_note_cells_on_the_centered_column` (`ui.rs:5036`).

**Human / live check** (`live-verification` skill):
```sh
# Snapshot the render (no TTY needed), diff against the amended §4:
cargo run -p wcode-tui --example dump
cat target/tui-render/120x40.txt     # a finished bash shows its FULL command, collapsed
cat target/tui-render/80x24.txt      # the params row under `│`, no `╭` frame

# Live against a keyless local endpoint:
cargo run -p wcode-cli -- --base-url http://localhost:11434/v1
```
Watch for: a **completed** `bash` panel showing `│   command  …` (and `cwd`)
**without** pressing `Ctrl-T`; `Ctrl-T` still collapsing/expanding the **output**
while the params stay put; `▸`/`▣` still on the head row and still clickable; the
`└` terminator present.

---

## 9. Expected outcome

- A finished, collapsed `bash` (and every other tool) shows its input params —
  `command` (wrapped in full, never clipped) and `cwd` — beneath `├ bash`.
- The `▸`/`▾`/`Ctrl-T` toggle still governs only the output/diff preview; params
  never hide.
- The head row shows the target only as a fallback (params empty); the `¹` ordinal,
  one-panel-per-tool contract, and D32 hit map are unchanged.
- `docs/tui-design.md` §4 states the rule; `examples/dump.rs` snapshot reflects it.
- `cargo test --workspace` green · `cargo clippy --workspace --all-targets` clean.

---

## 10. Facts NOT verified (by the brief author)

- **Nothing was run** — read-only worker; no `cargo build`/`test`/`clippy`/`dump`,
  no TUI launch. All line numbers are from a static read at this revision and **may
  have drifted**; the quoted snippets are the durable anchor.
- **Whether the current test suite actually passes today**, and whether any
  *existing* test asserts an exact transcript height/`└` count that step 4 would
  break — the named tests were read, but not every `render(` assertion enumerated.
- **`examples/dump.rs`'s exact current output** and whether its committed/expected
  snapshot lives in `target/` (gitignored) or is compared anywhere.
- **The `target/tui-render/*` files' tracked vs ignored status** — not checked.
- **The `cmd`-vs-`command` intent** — see O2.
- **`wcode-protocol`/kernel behavior on tool args** — taken as given that
  `arguments` reaches the TUI intact (it must, since `call_params` reads them).
- **`Tool.target`'s doc string** (`app.rs:230-234`) still says *"Rendered on the
  `»`/`✓` line"* — those glyphs are retired; looks stale but not traced to when it
  diverged.

### Orchestrator addendum (verified by recon, 2026-02)
- The gate is **not** a W012 regression: it is already present one commit earlier
  (`git show 903dbdd^:crates/wcode-tui/src/ui.rs` lines 893-901), i.e. it shipped
  with the original D31 tool-panel landing. It has never matched §3(a) of the
  redesign mock-up.

---

## Open questions for the human (sign-off gate)

- **O1.** Does a `DNNN` decision record get written, or is a §4 spec amendment +
  this work item enough? *Orchestrator leans:* a short `D019-tui-tool-params-always-visible`
  — the "params never hide" rule is precisely the kind of invariant that regressed
  silently once already, and `AGENTS.md` routes decisions to `docs/decisions/`.
- **O2.** Display the params key as the literal JSON key (`command`, production) or
  the design alias (`cmd`, `redesign-v2.md:250`)? *Orchestrator leans:* the
  **literal** key — `call_params` already prints whatever key the call carried, so
  `command` is what a real call shows; normalize the **fixtures** to `command` so
  the production path and `param_syntax`'s `bash` branch are actually exercised.

**This brief ends with a brief.**

---

## 11. Sign-off and orchestrator addendum

**Human rulings (2026-02, this session):**
- **O1 — YES**: write `docs/decisions/D019-tui-tool-params-always-visible.md`.
- **O2 — follow the orchestrator's recommendation**: the params key is the
  **literal JSON key** the call carried (so `command`, not `cmd`); normalize the
  test **fixtures** to the production key.
- **Explicit instruction:** "make sure we also fix other tool such as
  read/edit/write/etc" — the rule must be proved for every tool, not just `bash`.

**Orchestrator addendum — a SECOND defect, found while verifying the ruling.**

Cross-checking every `tool_param_keys` list against the real `*Args` structs
turned up a dead list entry that makes one tool's params *impossible* to render:

| tool | `tool_param_keys` (`app.rs:774-781`) | real `*Args` fields | verdict |
|---|---|---|---|
| `bash` | `command`, `cmd`, `cwd` | `command`, `timeout_secs`, `background` | `command` ✓; **`cmd`, `cwd` are dead** (never in a real call) |
| `read` | `path`, `offset`, `limit` | `path`, `offset`, `limit` | ✓ exact |
| `edit`/`write` | `path` | `path` + payload | ✓ (`payload` is the diff body, by design) |
| `grep` | `pattern`, `path` | `pattern`, `path`, `glob`, … | ✓ subset |
| **`find`** | **`pattern`** | `path`, `glob`, `kind`, `max`, `no_ignore` | **✗ BROKEN — `FindArgs` has no `pattern` field** |

`crates/wcode-cli/src/tools/find.rs` `FindArgs`:
```rust
pub struct FindArgs {
    /// Directory to search; defaults to the working directory.
    pub path: Option<String>,
    /// Comma-separated glob filters, e.g. "**/*.rs", "!tests/**", "Cargo.toml".
    pub glob: Option<String>,
    ...
```
So `call_params(…, "find")` always yields an empty vec, and a `find` panel
renders **no params rows even after the gate fix** (it falls back to the head
target via `ACTION_KEYS`, which does carry `path`). Fix: `find` →
`["path", "glob", "kind"]`, and drop bash's dead `cmd`/`cwd` so the list is the
literal-arg rule the human just affirmed.

This is the *same logical change* ("a tool's input actually renders") and is
therefore folded into this work item, not opened separately.

---

## 12. Outcome — shipped in `1927aae`, second-layer APPROVED

**One commit:** `1927aae` `tui: always render a tool's params, collapsed or not (D019)`
— touching exactly `crates/wcode-tui/src/ui.rs`, `crates/wcode-tui/src/app.rs`,
`docs/tui-design.md`. Zero `SKETCH` markers.

**Landed:**
- `ui.rs:908` — `body.extend(panel_param_lines(tool, width));`, unconditional;
  the `if !tool.done || expanded` gate is gone. `tool_panel_body(tool, width, expanded)`
  still keys the output/diff preview off `expanded`.
- `app.rs:773-784` — `bash` → `["command"]` (dead `cmd`/`cwd` dropped),
  `find` → `["path","glob","kind"]` (the nonexistent `pattern` dropped).
- `ui.rs:1030` — the `params.is_empty()` head-target fallback kept unchanged,
  with a comment naming its new role.
- `docs/tui-design.md:244-246` + `:383` — the rule stated (params always render;
  `Ctrl-T` governs the output body), and the keymap line tightened.
- D019 written; this record.

**Tests added (all fail if the gate is restored — verified by reasoning, not
re-reverting):** `a_done_collapsed_bash_shows_its_command`,
`a_done_collapsed_tool_shows_no_target_on_the_head_when_params_render`,
`a_done_collapsed_read_shows_its_path_and_offset`,
`a_done_collapsed_write_shows_its_path`, `a_done_collapsed_find_shows_path_and_glob`
(the last also pins defect #2: `**/*.rs` cannot come from the head fallback).
Fixtures normalized `cmd` → `command`; `call_params_returns_cmd_and_cwd_for_bash`
→ `call_params_returns_command_for_bash`.

**Gates:** `cargo build -p wcode-tui` ✓ · `cargo test --workspace` ✓ (tui 405,
cli 450, harness 184, protocol 42) · `cargo clippy --workspace --all-targets`
clean ✓. Render proof via `cargo run -p wcode-tui --example dump`:
`target/tui-render/120x40.txt` shows the finished **collapsed** `bash` as
`├ bash  <stats>  ▸ ▣` over `│   command  cargo test --workspace 2>&1 | tail -40`,
`grep -c '╭'` = 0.

**One un-dispatched change, reviewed and accepted:** `a_note_shows_the_full_command_unclipped`
(`ui.rs:4659`) now reassembles tokens (`text.split_whitespace().collect()`) —
renaming the fixture `cmd`→`command` widened the param key column 3→7, so at 80
cols `avail` fell 68→64 and the 67-char command hard-wraps (`ui.rs` `wrap_styled`
`chars.chunks(64)`). The token cannot be fabricated from a clipped row (a 60-char
clip drops `ays`), so the test still proves "shown in full".

**Not verified (honest gaps):** no live endpoint was reachable (ollama not
running; the refused-port path also refused), so no by-hand TUI run — the `dump`
example, which drives the real `ui::draw`, is the strongest no-TTY evidence. The
`▸`/`▣` click path was exercised by the existing green tests, not by hand.

**Residuals (non-gating, recorded in D019):** two test-hygiene leftovers still
name the retired `cmd` alias — the `push_bash_panel` doc at `ui.rs:4637` and the
`panel_tool("bash", vec![("cmd".into(), …)])` fixture in `app.rs` (no assertion
reads it). Cosmetic.
