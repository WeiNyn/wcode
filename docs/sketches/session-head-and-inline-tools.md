# SKETCH — the session-head block + the inline tool render (W011 delta)

- **Status: review-only, NOT real code.** Nothing here compiles and nothing is
  wired in. The developer turns these blocks into real code and leaves **ZERO
  `SKETCH` markers**.
- **Implements:** [`docs/decisions/D009-tui-book-header-and-inline-tools.md`](../decisions/D009-tui-book-header-and-inline-tools.md)
  (the locked decision) over [`docs/tui-design.md`](../tui-design.md) §4
  (just amended: the session-head block at `:198`, tools inline at `:217`, the
  composer head-row drop at `:282`) and the layout detail in
  [`docs/design/tui-turn-notes.md`](../design/tui-turn-notes.md) §17. Work item:
  [`docs/work/W011-tui-book-header-inline-tools.md`](../work/W011-tui-book-header-inline-tools.md).
- **Extends:** [`docs/sketches/turn-block.md`](turn-block.md) — that sketch shipped
  the **`Block::Turn`** model, which is now **in the working tree, uncommitted**
  (not in `HEAD`). Every cite below is `file:line` + a quoted snippet against that
  **working tree**; the sketch edits no `.rs`, so the numbers hold for the reviewer.
- **Form chosen: a new `docs/sketches/*.md`** (not an in-place `.rs` block). The
  delta is spread across **two** files (`app.rs` the block model, `ui.rs` the
  render + D32), so no single in-place block sits "where the code goes"; a doc lets
  every cite point at the current tree, which the sketch does not edit.

---

## 0. The target frame (D009, §17 H1 reading (i))

```
WCODE · session a1b2c3d4 ──────────────────── wcode · ⎇ main   gpt-5-codex · high   ← session head (block 0)
──────────────────────────────────────────────────────────────────────────────────  ← its `─` rule, on the measure
 YOU    why does the anchor move when I reformat?                                    ← Block::User, `YOU` head

 WCODE  An anchor hashes the line's raw content, so indentation is part of its        ← Block::Turn: `WCODE` head + prose
        address.¹
        » read  crates/wcode-cli/src/tools/edit.rs                                    ← tool 1, INLINE at its call
        ✓ read · 128 lines · 12ms
        A reformat reindents the line, moving every anchor below it.²                ← prose continues
        » edit  crates/wcode-cli/src/tools/edit.rs
        ✓ edit · +3 −0 · 9ms
```

One exchange = `Block::User` + `Block::Turn`, adjacent (the shipped between-roles
blank). Inside the turn, prose and each tool's panel are **interleaved in call
order** — the `¹`/`²` marks and the tools sit where the calls were, NOT gathered
into a foot ledger.

---

## 1. Delta A — the session-head block

### 1.1 The type (new; `app.rs`)

`enum Block` today (`crates/wcode-tui/src/app.rs:155`):

```rust
pub enum Block {
    User(String),
    /// One exchange — the whole run's prose + a ledger of its tools. …
    Turn(Turn),
    /// A standalone tool invocation, with no open turn to join …
    Tool(Tool),
    Notice(String),
    Error(String),
    Btw(String),
    Diff { path: String, diff: String },
    Todos(Vec<TodoItem>),
}
```

**Add** a variant (the transcript's first committed block) and one new type:

```rust
/// The scroll-away book header (D009 §1): `WCODE · session <id> ──── project · ⎇
/// branch` (+ `model · effort` far right) over a `─` rule on the measure. The
/// transcript's FIRST committed block; **chrome** — never a browse selection
/// target, never copied.
SessionHead(SessionHead),

/// The header's composed title — the same pieces the composer's (now-dropped)
/// head row read (`ui.rs` `corner_titles` top row). Snapshotted so the block is
/// self-contained like every other `Block` (`User(String)`, `Diff{..}`) and the
/// `(rev, width)` cache holds.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionHead {
    /// The **RAW** session id (matching `Status.session`); `session_head_row`
    /// (in `ui.rs`, where `short_id` lives) shortens it. `None` hides the group.
    pub session: Option<String>,
    /// The cwd (or `wcode`); **muted**.
    pub project: String,
    /// The branch, WITHOUT the `⎇` (the renderer adds it); `None` hides it.
    pub branch: Option<String>,
    /// The model id — the far-right group (D009 §3, option A).
    pub model: String,
    /// The effort id; joins `model` with ` · `; `None` → model alone.
    pub effort: Option<String>,
}
```

### 1.2 Population — `SessionHead::of` (new; `app.rs`)

The fields are the **same source** the composer head row reads today:

- `crates/wcode-tui/src/ui.rs:1540` `fn corner_titles(` → its top row
  (`:1548` `// --- top row: `{project} · ⎇ {branch}`   ·   `{model} · {effort}` ---`)
  builds `tl(join_title(join_title(&project, branch), session))` vs
  `tr(join_title(&model, effort))` (`:1567`-`:1582`). The head re-orders: `WCODE ·
  session` **left**, `project · ⎇ branch` **right**, `model · effort` **far right**.
- The values come from `App`:
  `crates/wcode-tui/src/app.rs:4179` `pub fn status(&self) -> &Status {` (body
  `&self.focused().status`, giving `model`/`effort`/`session`),
  `app.rs:4140` `pub fn cwd(&self) -> Option<&str> {`, and
  `app.rs:4145` `pub fn git(&self) -> Option<&str> {`.

```rust
impl SessionHead {
    /// Snapshot the header from `App`: the focused surface's `Status`
    /// (`session`/`model`/`effort`) + cwd/git — the same source `corner_titles`
    /// read before the head row was dropped. Stores the **raw** `session` id
    /// (NOT shortened). # Contracts: infallible; a missing session/branch/effort
    /// is `None` (its group is omitted by the renderer).
    pub(crate) fn of(app: &App) -> SessionHead { todo!() }
}
```

**B3 / A9 — `of` must NOT call `ui::short_id`.** `crates/wcode-tui/src/ui.rs:1757`
is `fn short_id(id: &str) -> String {` — **private** (no `pub`), so `app.rs`
cannot reach it; a call there does not compile. **Pinned (option b):** `of` stores
the **raw** id and `session_head_row` (§1.5, in `ui.rs`) shortens it via
`short_id`. (Option (a) — `pub(crate) short_id` — was rejected: it leaks a
presentation helper into `app.rs`.)

### 1.3 Insertion + refresh — `App::sync_session_head` (new; `app.rs`)

The head is (re)seated once per frame, exactly as the D5 hint already is — a
**render-side sync** (precedent: `pub(crate) fn seed_empty_hint(&mut self) { self.focused_mut().seed_hint_if_empty(); }`
at `crates/wcode-tui/src/app.rs:4108`, called from `ui::draw` at
`crates/wcode-tui/src/ui.rs:90` `app.seed_empty_hint();`). `ui::draw` calls the new
fn **before** `seed_empty_hint`.

```rust
/// Ensure the focused surface's leading block is a `SessionHead` carrying the
/// current `SessionHead::of(self)`, refreshing its text (and bumping its rev)
/// when the inputs changed; creates the head as block 0 when absent.
/// # Contracts: idempotent; a cheap `String` compare on a steady frame (a no-op,
/// not a re-render). Uses `insert_block` (NOT `push_block`, which would
/// `clear_hint` before the hint is seeded).
pub(crate) fn sync_session_head(&mut self) {
    // let head = SessionHead::of(self);
    // let s = self.focused_mut();
    // match s.transcript.first_mut() {
    //     Some(Block::SessionHead(cur)) if *cur == head => {}          // steady
    //     Some(Block::SessionHead(cur)) => { *cur = head; s.bump_rev(0); }
    //     _ => s.insert_block(0, Block::SessionHead(head)),            // first frame
    // }
}
```

The `(rev, width)` cache maintainers are shipped: `fn bump_rev(&mut self, i: usize)`
(`crates/wcode-tui/src/app.rs:1328`) and `fn insert_block(&mut self, at: usize, block: Block)`
(`app.rs:1384`). cwd/git are set once at startup (`set_cwd`/`set_git`); `session`/
`model`/`effort` move with `/model`/`/effort` → `set_status` (`app.rs:4183`), which
the next frame's `sync_session_head` picks up — **no new refresh point**.

**A2 (Q2) — head liveness, PINNED.** The **snapshot + per-frame `sync_session_head`**
is the choice: called in `ui::draw` **before** `seed_empty_hint`
(`crates/wcode-tui/src/ui.rs:90` `app.seed_empty_hint();`), seated via
`insert_block(0, …)` (`app.rs:1384`), idempotent `String`-compare. The
**live-render alternative is REJECTED** — reading `App` at draw time bypasses the
`(rev, width)` cache (`app.rs:1251` `if self.cache[i].rev != rev || self.cache[i].width != width {`)
for a one-row block. No open question remains.

### 1.4 Chrome — `selectable` + `copy_text` (edits; `app.rs`)

- `crates/wcode-tui/src/app.rs:1337` `fn selectable(&self, _i: usize) -> bool { true }`
  → restore the pre-`Turn` exclusion:
  `!matches!(self.transcript.get(i), Some(Block::SessionHead(_)))`.
  `block_at` (`app.rs:2996`) already filters by `selectable` (`:3006`
  `.filter(|&i| self.focused().selectable(i))`), so a click on the head selects
  **nothing** (pinned: a no-op, not a fall-through); `first_selectable` skips it.
- `crates/wcode-tui/src/app.rs:309` `fn copy_text(block: &Block) -> Option<String>`
  → add as the FIRST arm: `Block::SessionHead(_) => return None,` (a head copies
  nothing, like the retired `TurnHead`).

### 1.5 Rendering — `session_head_lines` / `session_head_row` (new; `ui.rs`)

`crates/wcode-tui/src/ui.rs:638` `pub(crate) fn block_lines(block: &Block, width: usize)`
gets the arm `Block::SessionHead(head) => session_head_lines(head, width),`.

```rust
/// The session-head block's lines (D009 §1; tui-design §4:198): row 0 is the
/// running head, row 1 a full-width `─` rule **on the measure**. Blocks render at
/// `width == measure` (`ui.rs:421` `let measure = if band >= MEASURE_MIN_BAND { MEASURE_MAX } else { band };`),
/// so "on the measure" is automatic — at ≥84 cols the head sits in the centered
/// 68-col column, not full-bleed. # Contracts (A4): EXACTLY 2 lines — row 0 +
/// the rule. `tui-turn-notes.md` §17's "single-rule" (fill-only, 1-line) reading
/// is OVERRIDDEN. No `¹`, no affordance — the head is chrome.
fn session_head_lines(head: &SessionHead, width: usize) -> Vec<Line<'static>> { todo!() }

/// Row 0 (A1): `padded_row(left, right, "─", dim(), width)` (`ui.rs:1512`), where
/// `left` = `[ WCODE reversed ] · session <short_id>` and `right` =
/// `"project · ⎇ branch   model · effort"` — the **3-space gap sits INSIDE the
/// right run** (D009 §3 option A), and `padded_row` supplies the ONE `─` fill
/// between `left` and `right`. The ladder fed to `pick_titles` (`ui.rs:1648`) is:
/// full → drop `session` → drop the branch → truncate with `…` (`truncate`,
/// `ui.rs:1220`). Shortens the raw `head.session` via `short_id` (`ui.rs:1757`,
/// §1.2 B3).
fn session_head_row(head: &SessionHead, width: usize) -> Line<'static> { todo!() }
```

Reuse, do NOT re-add: `selection_style()` (`ui.rs:608`
`Style::default().add_modifier(Modifier::REVERSED)` — the run the speaker head
already uses), `muted()`/`dim()`, `join_title` (`ui.rs:1638`), `short_id`
(`ui.rs:1757`), `padded_row` (`ui.rs:1512`), `pick_titles` (`ui.rs:1648`), `sep()`
(`ui.rs:1683`). NO new `theme.rs` role, NO new glyph (D009 §6; `─` is declared).

### 1.6 Friction — head vs the D5 empty hint (edits; `app.rs`)

`fn push_block` (`crates/wcode-tui/src/app.rs:1352`) calls `self.clear_hint();`
(`:1354`), and `fn seed_hint_if_empty` (`app.rs:1363`) seeds only when
`if self.transcript.is_empty() && self.live.is_none() {` (`:1364`). Once the head is
block 0 the transcript is never empty, so (**A8** — settled):

- `seed_hint_if_empty` must treat "only the head" as empty
  (`matches!(self.transcript.as_slice(), [Block::SessionHead(_)])`);
- `fn clear_hint` (`app.rs:1372`, today
  `if matches!(self.transcript.first(), Some(Block::Notice(t)) if t.as_str() == EMPTY_HINT)`)
  must find the hint at **index 1** (scan past the head).

Ordering: `sync_session_head` runs BEFORE `seed_empty_hint` (`ui.rs:90`), so the
head exists first; the hint then lands at index 1.

---

## 2. Delta B — the inline tool render

### 2.1 The foot ledger today

`crates/wcode-tui/src/ui.rs:725`:

```rust
fn turn_lines(turn: &Turn, width: usize) -> Vec<Line<'static>> {
    let mut out = speaker_lines(Speaker::Wcode, "", width);
    out.extend(turn_prose_lines(turn, width));
    // `ledger_lines` carries its own `── notes ──` rule.
    out.extend(ledger_lines(&turn.tools, width));
    out
}
```

`fn ledger_lines(items: &[Tool], width: usize)` (`ui.rs:818`) draws
`let mut out = vec![ledger_rule(width)];` (`:824`) — the `── notes ──` rule — then
per tool `note_head_row(k + 1, tool, width)` (`ui.rs:850`) + params + body/summary.
`fn ledger_rule(width: usize)` is `ui.rs:898`. This is the **foot ledger D009 §4
removes.**

### 2.2 The inline walk (replaces `turn_lines`'s ledger call)

```rust
/// One exchange: the `WCODE` head over the run's body (prose + tools, inline).
fn turn_lines(turn: &Turn, width: usize) -> Vec<Line<'static>> {
    let mut out = speaker_lines(Speaker::Wcode, "", width);
    out.extend(turn_inline_lines(turn, width));
    out
}

/// The turn's body: the prose/thinking interleaved with each tool's panel, in
/// call order. Walks `turn.content`; at the k-th `ToolCall` it appends the `¹`
/// mark (`footnote_mark`, `ui.rs:805`) to the last PROSE line, then emits
/// `tool_inline_lines(k + 1, &turn.tools[k], width)`. # Contracts: exactly one
/// panel per tool, in call order; a `ToolCall` whose result is not yet in
/// `turn.tools` (mid-run — the assistant message commits BEFORE
/// `ToolExecutionStart`) emits the mark but NO panel — guard with
/// `turn.tools.get(k)`; the `¹` is SUPPRESSED when no prose landed since the
/// previous tool (a text-less round, D009 §4).
fn turn_inline_lines(turn: &Turn, width: usize) -> Vec<Line<'static>> { todo!() }

/// One tool's inline panel: the `» name  target` head row (its `▸`/`▣` toggle,
/// `note_head_row`), then — running, OR expanded-or-errored — the params + the
/// full body + the `✓ {name} · {note} · {ms}` summary. NO `── notes ──` rule.
/// `n` is the tool's 1-based call order (the `¹` ordinal + 1). # Contracts: ≥1
/// line; row 0 is the D32 toggle row; the standalone `Block::Tool` also uses it
/// with `n = 1`. Mirrors the current `ledger_lines` per-item loop, minus the rule.
fn tool_inline_lines(n: usize, tool: &Tool, width: usize) -> Vec<Line<'static>> { todo!() }
```

- The `Block::Tool` arm in `block_lines` (`ui.rs:648`
  `Block::Tool(tool) => ledger_lines(std::slice::from_ref(tool), width),`) changes
  to `tool_inline_lines(1, tool, width)`.
- **B2 / A6 — the orphan `Block::Tool` D32 hit is one row LOW.** With NO
  `── notes ──` rule its head is block-relative line **0**, but the shipped arm
  (`ui.rs:547` `let r = row0 + 1;`, comment `:545` "its head follows the rule")
  still adds the retired rule row. **Change to `let r = row0;`** (mirrored in §3).
- **`ledger_lines` and `ledger_rule` are retired** (`ui.rs:818`/`:898`).
  `note_head_row`/`note_height`/`tool_panel_body`/`tool_summary_row`
  (`ui.rs:850`/`:880`/`:1008`/`:1070`) and the `panel_*` helpers are reused as-is.
- **A3 — `turn_prose_lines` (`ui.rs:736`) is DELETED.** Its two callers go
  (`turn_lines` here, and the D32 walk §3); `turn_inline_lines` owns the
  committed-turn prose walk. Accepted consequence: `content_lines` (`ui.rs:754`)
  stays the LIVE-buffer walk (`live_lines`, `ui.rs:745`) — a **dual walk** (live
  vs committed) is intentional, not a fork to collapse.

### 2.3 Friction — the `¹`-suppression rule vs inline tools

Today the mark rule is `ui.rs:782`-`:790`:

```rust
ContentBlock::ToolCall { .. } => {
    refn += 1;
    if let Some(last) = lines.last_mut() {
        last.spans.push(Span::styled(footnote_mark(refn), link()));
    }
}
```

`content_lines` (`ui.rs:754`) never emits a tool row, so `lines.last_mut()` is
**always a prose line** — the rule is a cheap "append to the last prose". With
tools **inline**, the last line after a tool panel is a **tool row**, so the walk
needs a `tail_is_prose: bool` flag (set true after a `Text`/`Thinking` render,
false after a tool panel) and the `¹` attaches only when it is true. That IS the
D009 §4 "suppressed on a text-less round" rule, now **structural** — this is the
one place the design fights the code.

**`turn.content` cannot distinguish message boundaries.** `turn.content` is every
assistant message's `Text`/`Thinking`/`ToolCall` concatenated
(`app.rs:131` `pub struct Turn {` … `pub content: Vec<ContentBlock>,`); a
`A(tc) → T → A(tc) → …` round with no prose emits no `Text` between two `ToolCall`s.
The `tail_is_prose` flag is the *only* signal that the second call's round was
text-less — there is no per-message marker to lean on. **(A7 — settled:** the
`tail_is_prose: bool` flag IS the discriminator; `turn.content` carries no
per-message boundary to lean on.)

`turn_prose_lines` (`ui.rs:736`) and a would-be `turn_tools_rows` do not exist as a
pair today — **`turn_tools_rows` is NOT in the working tree**; §3 proposes
`inline_tool_offsets` as its inline-aware successor.

---

## 3. The D32 offset math (`ui.rs`, `draw_transcript`)

The per-tool hit map is `crates/wcode-tui/src/ui.rs:507`:

```rust
Block::Turn(turn) => {
    // A `Turn` is: the `WCODE` head (1 row), the prose (…), then the
    // `── notes ──` rule + the ledger rows. …
    let mut hits = Vec::new();
    if matches!(turn.content.first(), Some(ContentBlock::Thinking { .. })) { /* … row0 + 1 … */ }
    let mut off = 1 + turn_prose_lines(turn, measure).len() + 1; // head + prose + rule
    for (k, tool) in turn.tools.iter().enumerate() {
        let r = row0 + off;
        // … push AffordanceHit { block: i, item: Some(k), toggle, copy } …
        off += note_height(tool, measure);      // ui.rs:880
    }
    Some(hits)
}
```

The offsets must **mirror the render walk** (§2.2) — the ledger-rule `+ 1` goes
away and the prose is interleaved with the tools:

```rust
/// The turn-relative row offset (line 0 = the `WCODE` head) of each inline tool's
/// head row, in call order — the D32 mirror of `turn_inline_lines`. # Contracts:
/// one `(k, off)` per tool ACTUALLY emitted (a `ToolCall` with no result is
/// skipped); `off` accounts for every prose/thinking/tool row above it. A drift
/// from the render misaligns EVERY hit below it (see the D32 publish comment,
/// `ui.rs:495`-`:499`, which still names the retired `Block::Notes` — reword it).
fn inline_tool_offsets(turn: &Turn, width: usize) -> Vec<(usize, usize)> { todo!() }
```

Then the arm becomes (the leading-`Thinking` hit above stays at `row0 + 1` — the
head is line 0):

```rust
for (k, off) in inline_tool_offsets(turn, measure) {
    let r = row0 + off;
    if r < window.len() {
        let y = area.y + r as u16;
        hits.push(AffordanceHit {
            block: i, item: Some(k),
            toggle: Rect::new(col.x, y, measure.saturating_sub(2) as u16, 1),
            copy: Rect::new(col.x + note_copy as u16, y, 1, 1),
        });
    }
}
```

`AffordanceHit` is unchanged — `item: Option<usize>` already spans "tool `k`"
(`Some(k)`) and "the leading thinking" (`None`). The step is still
`note_height(tool, width)` (`ui.rs:880`).

**B2 mirror (the standalone `Block::Tool`).** The `Block::Tool` arm
(`ui.rs:547` `let r = row0 + 1;`) drops its `+ 1` → **`let r = row0;`** — with the
`── notes ──` rule gone (§2.2), the standalone tool's head is block-relative line
0. This is the one D32 arm B2 calls out.

---

## 4. Delta C — the composer head-row drop

`crates/wcode-tui/src/ui.rs:1446`:

```rust
fn draw_input_box(frame: &mut Frame, area: Rect, app: &App, view: &InputView) {
    let (tl, tr, bl, br) = corner_titles(app, area);
    let w = area.width;
    let h = area.height;
    let mut row = area.y;
    // Head line — always drawn. A running head is plain: the two titles are
    // separated by spaces, not a rule.
    if h >= 1 { /* … padded_row(&tl, &tr, " ", …) … */ row += 1; }
    if h >= 4 { /* the full-width ─ rule */ row += 1; }
    let foot = (h >= 3).then(|| area.y + h - 1);
    /* the input fills [row, foot); the foot line (gauge · state · folio) at `foot` */
}
```

**Delete the head-line block** (the `if h >= 1 { … row += 1; }`). The composer
keeps the rule, the input, and the foot (`ui.rs:1496` the `padded_row(&bl, &br, "─", dim(), w)`
foot line: gauge left, `⏻ plan`/`▤ browse`/state/`↑ N`/folio right). The row
thresholds re-derive (one row fewer): the rule seats at `h >= 3`, the foot at
`h >= 2`.

**A5 — `fn corner_titles`** (`ui.rs:1540`, today
`) -> (Line<'static>, Line<'static>, Line<'static>, Line<'static>) {`) returns a
**4-tuple** — the top row (`:1548`-`:1583`, `top_levels`/`pick_titles`) and the
bottom row. **DELETE `ui.rs:1548`-`:1583`** (the top-row block) and change the
signature to the FOOT pair only:

```rust
/// The composer's foot-line pair: the context gauge (left) and
/// `[⏻ plan] · [▤ browse] · {state} · [↑ N] <N>/<M>` (right), width-budgeted to
/// `area`. # Contracts: the surviving half of the old 4-tuple; the head-row pair
/// is gone (it is now `SessionHead` in the transcript).
fn corner_titles(app: &App, area: Rect) -> (Line<'static>, Line<'static>) { todo!() }
```

**A5 — callers to update:** `ui.rs:1447`
`let (tl, tr, bl, br) = corner_titles(app, area);` → the 2-tuple (the `tl`/`tr`
locals go too); and the two tests `ui.rs:3722`
`let (tl, _tr, _bl, br) = corner_titles(&app, area);` and `ui.rs:3738`
`let (_tl, _tr, _bl, br) = corner_titles(&app, area);`.

---

## 5. Test skeletons (`todo!()` bodies; behaviour-named)

### `app.rs`
```rust
#[test] fn the_head_leads_the_transcript_and_carries_the_session() { todo!() }
#[test] fn the_head_reads_the_same_source_the_composer_head_row_did() { todo!() }
#[test] fn a_model_change_refreshes_the_head_via_sync_session_head() { todo!() }
#[test] fn the_head_is_not_a_browse_selection_target() { todo!() }
#[test] fn the_head_copies_nothing() { todo!() }
#[test] fn the_empty_hint_seeds_below_the_head() { todo!() }
```

### `ui.rs`
```rust
#[test] fn the_head_renders_row_zero_with_the_session_and_branch_groups() { todo!() }
#[test] fn the_head_rule_spans_the_measure_not_the_band() { todo!() }
#[test] fn the_head_ladder_drops_session_then_the_branch_at_48_cols() { todo!() }
#[test] fn the_head_shows_model_and_effort_far_right() { todo!() }
#[test] fn tools_render_inline_in_call_order_not_at_the_foot() { todo!() }
#[test] fn a_tool_lands_right_after_its_call_not_at_a_later_replys_foot() { todo!() }
#[test] fn the_foot_notes_rule_is_gone() { todo!() }
#[test] fn the_reference_mark_sits_on_the_prose_at_the_call_site() { todo!() }
#[test] fn a_text_less_round_marks_no_prose_but_still_renders_its_tool() { todo!() }
#[test] fn a_toolcall_with_no_result_yet_renders_no_panel() { todo!() }
#[test] fn an_inline_tool_row_is_its_d32_toggle_and_the_offsets_line_up() { todo!() }
#[test] fn the_composer_draws_no_head_row() { todo!() }
```

---

## 6. Friction (design vs the working-tree code)

1. **The head needs `App` data; `block_lines` is pure.** `corner_titles` reads
   `app.status()`/`cwd()`/`git()`, but `block_lines(block, width)` (`ui.rs:638`)
   cannot reach `App`. Resolved by a **self-contained** `SessionHead` +
   `SessionHead::of` snapshot + the per-frame `sync_session_head` (the
   `seed_empty_hint` precedent). *Alternative:* render the head live in
   `draw_transcript` — **rejected (A2):** it bypasses the `(rev, width)` cache
   (`app.rs:1251`) for a one-row block.
2. **Head vs the D5 empty hint** — `push_block`→`clear_hint` and
   `seed_hint_if_empty` assume block 0 is the hint (§1.6).
3. **The `¹`-suppression rule breaks with inline tools** — `lines.last_mut()` is no
   longer always prose; a `tail_is_prose` flag is required (§2.3).
4. **`turn.content` has no message boundary** — a text-less round is invisible
   except via the flag (§2.3).
5. **`corner_titles`' 4-tuple** — dropping the head row changes its return
   contract to `(Line, Line)` and touches three call sites (§4; A5).
6. **D009/spec internal ambiguity** — D009 §1 and `tui-design.md` §4:198 say "one
   row + one `─` rule **on the measure**" (2 lines), while `tui-turn-notes.md` §17
   calls the row's **middle fill** "the only rule" in the *single-rule* reading (1
   line). **Settled (A4): 2 lines** (row + a full `─` rule); §17's reading is
   overridden.
7. **Orphan `Block::Tool`** — `tool_inline_lines(1, …)` with NO `── notes ──`
   rule (D009 §4), and its D32 hit moves from `row0 + 1` to `row0` (§2.2, §3;
   B2/A6).
8. **B3 — `SessionHead::of` cannot call `ui::short_id`** (`ui.rs:1757` is
   private). Resolved (A9, option b): the block stores the RAW id and
   `session_head_row` shortens (§1.2, §1.5).
9. **`tui-design.md` §4 wording drift (designer-flagged, NOT swept)** — the
   Affordances/Thinking bullets still say "a **tool panel**" / "an **assistant
   block**" (pre-`Turn`). With `Block::Turn` the tools are **inline** and there is
   no committed "assistant block". Not a sketch blocker — recorded for spec/code
   coherence.

---

## 7. Open questions (first-layer gate rulings folded in)

**All settled — no open questions remain.**

- **Q1 (A1)** — the head row's `model · effort` grouping: `padded_row(left, right,
  "─", dim(), width)`, `left` = `[WCODE reversed] · session <id>`, `right` =
  `"project · ⎇ branch   model · effort"` (**3-space gap inside the right run**);
  one `─` fill only (§1.5).
- **Q2 (A2)** — head liveness: **pin the snapshot + per-frame `sync_session_head`**
  (§1.3); the live-render alternative is **rejected** (it bypasses the
  `(rev, width)` cache, `app.rs:1251`).
- **Q3 (A3)** — **delete `turn_prose_lines`** (`ui.rs:736`); `turn_inline_lines`
  owns the committed-turn prose walk; the live-vs-committed dual walk is accepted
  (§2.2).
- **Q4 (A4)** — `session_head_lines` returns exactly **2** lines; §17's
  single-rule reading is overridden (§1.5, §6.6).
- **Q5 (A5)** — `corner_titles` returns **`(Line, Line)`**; delete
  `ui.rs:1548`-`:1583`; update `ui.rs:1447`, `:3722`, `:3738` (§4).
- **Q6 (A6)** — orphan `Block::Tool` → `tool_inline_lines(1, …)`, no rule, D32 hit
  at **`row0`** (= B2) (§2.2, §3).
- **B3 (A9)** — the head stores the RAW id; `session_head_row` shortens (§1.2).

---

## 8. What this sketch did NOT do (honest)

- **No `.rs` file was touched** — this doc only. The working tree's `Block::Turn`
  refactor is untouched (`git diff --numstat` for the two files is unchanged from
  before the sketch); every cite is against that tree and stays valid because no
  `.rs` moved.
- **No terminal was run.** The frame (§0) is hand-authored from D009 + §17.
- **The offset math (§3) is reasoned, not measured** — `inline_tool_offsets` must
  mirror `turn_inline_lines` row for row or every D32 hit below misaligns.
- **The "34 drifted tests" are not enumerated** — only the header/inline additions
  and the three `corner_titles` call sites (`ui.rs:1447`/`:3722`/`:3738`) are named.

---

## 9. Amendment coverage (A1–A10)

Every first-layer amendment, mapped to the section that folds it in:

| # | ruling | folded at |
|---|---|---|
| **A1** | head row = `padded_row(left, right, "─", dim(), width)`; `left` = `[WCODE reversed] · session <id>`, `right` = `"project · ⎇ branch   model · effort"` (3-space gap **inside** the right run); ladder full → drop `session` → drop the branch → truncate | §1.5 |
| **A2** | head liveness: **pin** the snapshot + per-frame `sync_session_head` (in `ui::draw` **before** `seed_empty_hint`), `insert_block(0, …)`, idempotent `String`-compare; **reject** the live render (bypasses the `(rev, width)` cache, `app.rs:1251`) | §1.3, §7 |
| **A3** | delete `turn_prose_lines` (`ui.rs:736`); `turn_inline_lines` owns the committed walk; the live-vs-committed dual walk is accepted | §2.2 |
| **A4** | `session_head_lines` = exactly **2** lines; §17's single-rule reading overridden | §1.5, §6.6 |
| **A5** | `corner_titles` → `(Line, Line)`; delete `ui.rs:1548`-`:1583`; update `ui.rs:1447`/`:3722`/`:3738` | §4 |
| **A6** | orphan `Block::Tool` → `tool_inline_lines(1, …)`, no rule, D32 hit at **`row0`** | §2.2, §3 |
| **A7** | `¹`-suppression needs a `tail_is_prose: bool`; `turn.content` has no per-message boundary | §2.3 |
| **A8** | `seed_hint_if_empty` treats `[Block::SessionHead(_)]` as empty; `clear_hint` scans to index 1 | §1.6 |
| **A9** | resolves **B3**: `of` stores the **RAW** id; `session_head_row` shortens via `short_id` | §1.2, §1.5 |
| **A10** | **designer — DONE** (the spec/D009 set: the duplicate gauge line, `:186` `Assistant`→`Turn`, `:90` reword, the D009 Amends widening). **Not a sketch change.** | `docs/tui-design.md`, `docs/decisions/D009-tui-book-header-and-inline-tools.md` |

**All ten amendments are accounted for:** A1, A2, A3, A4, A5, A6, A7, A8, A9 are
folded into the sketch (§1.2–§1.6, §2.2, §2.3, §3, §4); **A10 is designer-owned
and already applied + verified** (see the row above). No open questions remain.

