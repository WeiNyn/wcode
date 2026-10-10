# SKETCH — one block per turn (`Block::Turn`) + the speaker head + the ledger

- **Status: review-only, NOT real code.** Nothing here compiles and nothing is
  wired in. The developer deletes this file's fences into real code and leaves
  **ZERO `SKETCH` markers**.
- **Implements:** [`docs/design/tui-book-directions.md`](../design/tui-book-directions.md)
  §2 **Direction I** (the novel) over [`docs/design/tui-turn-notes.md`](../design/tui-turn-notes.md)
  §2 **A** (one block per turn), §10 **R2** (the one-line ledger), §15 **(b)**
  (the reverse-video speaker head). Grounds the traced bug in that doc's §1.
- **Form chosen: a new `docs/sketches/*.md`** (not an in-place block). Justification:
  the change is spread across **two** files (`app.rs` the block/turn model,
  `ui.rs` the render + D32), so no single in-place block sits "where the code
  goes"; and a doc lets every `file:line` be cited against the **current tree,
  which this sketch does not edit** — so the numbers cannot drift under the
  reviewer. (The skill's Form B is preferred for a one-file change; this is not
  one.) Every cite below is `file:line` + a quoted snippet against `HEAD`
  (`225ac67`).
- **The failure it removes.** The shipped `commit_turn` (`crates/wcode-tui/src/app.rs:1654`)
  folds the apparatus **retrospectively** and **tail-based**:
  `let notes = self.take_trailing_tools();` (`:1661`) walks back and **stops at
  the first text-bearing block** (`take_trailing_tools`, `:1689`:
  `_ => break,`), so a tool lands at the *next* reply's foot and each reply is
  its own `§` section. The `¹` ordinal is **per message**
  (`crates/wcode-tui/src/ui.rs:685` `let mut refn = 0usize;`) while the foot's
  number is **per turn** (`:742` `note_head_row(k + 1, tool, width)`) — the two
  never agree in a multi-round turn. One `Block::Turn` makes all four agree **by
  construction**.

---

## 0. The target frame (Direction I, §2)

```
WCODE · session a1b2c3d4 ──────────────────────────── wcode · ⎇ main   ← H1 session head (its own block, out of scope)
────────────────────────────────────────────────────────────────────
 YOU    why does the anchor move when I reformat?                      ← Block::User, speaker head `YOU`

 WCODE  A  n anchor hashes the line's raw content, so indentation is    ← Block::Turn: `WCODE` head + prose
        part of its address.¹  A reformat reindents the line.²

        ── notes ────────────────────────────────────────────────     ← the ledger rule (R2, kept)
        1  read   crates/wcode-cli/src/tools/edit.rs   128 ln  12ms     ← the ledger (the turn's foot)
        2  edit   crates/wcode-cli/src/tools/edit.rs   +3 −0    9ms
```

**One exchange = `Block::User` + `Block::Turn`, adjacent** (the blank between
them is the shipped between-roles separator). The turn holds **all** the run's
prose and **all** its tools, in call order — so the `¹` and the ledger's `N` are
one series.

---

## 1. The current state (grounded — this is what replaces)

The three blocks that become one:

- `crates/wcode-tui/src/app.rs:133` — `TurnHead {` … the doc says "one per
  assistant reply … It is **chrome**: never a browse selection target and never
  copied."
- `crates/wcode-tui/src/app.rs:140` — `Assistant { content, thinking_open }`.
- `crates/wcode-tui/src/app.rs:164` — `Notes(Vec<Tool>),` "A turn's apparatus,
  re-homed from the flow to its foot".

The commit + fold:

- `crates/wcode-tui/src/app.rs:1654` `fn commit_turn(&mut self, mut content: Vec<ContentBlock>)`
  → `:1661` `let notes = self.take_trailing_tools();` → `:1662` `self.turns += 1;`
  → pushes `TurnHead` + `assistant` + `Notes`.
- `crates/wcode-tui/src/app.rs:1689` `fn take_trailing_tools` — the tail-based
  walk (`:1701` `_ => break,`).

The run boundary is `AgentEvent::AgentEnd` (`crates/wcode-tui/src/app.rs:1544`
`AgentEvent::AgentEnd => { self.flush_live(); self.running = false; self.finished = true;`)
with `AgentStart` (`:1536`). The streaming text lives in `live`
(`:1133` `live: Option<AgentMessage>,`), committed on `MessageEnd`
(`:1461` `self.live = None; self.commit(message);`) through `flush_live`
(`:1715`).

The render + the D32 map:

- `crates/wcode-tui/src/ui.rs:633` `pub(crate) fn block_lines` — arms for
  `TurnHead` (`:638`), `Assistant` (`:645`), `Tool` (`:649`), `Notes` (`:656`).
- `crates/wcode-tui/src/ui.rs:496-558` — the D32 affordance map (per-block, and
  per-`Notes`-item via `note_height`, `:796`).
- `crates/wcode-tui/src/ui.rs:442` `let _ = app.focused_mut().append_live_lines(width, &mut lines);`
  — the live block is appended **after** every committed block.

---

## 2. The block model — `Block::Turn` (one exchange, one block)

### 2.1 The type

```rust
/// One **exchange**: a user prompt → the assistant's answer, as ONE block.
/// Replaces the `TurnHead` + `Assistant` + `Notes` trio. Its head is the
/// speaker (`WCODE`), its body is the whole run's prose, its foot is the
/// **ledger** (the run's tools, in call order). Placement, order and the
/// `¹`↔`N` numbering are correct **by construction** — no retrospective fold.
#[derive(Clone, Debug, PartialEq)]
pub struct Turn {
    /// The exchange number, per surface, monotonic (`1`, `2`, …). The `§N`
    /// series' successor: the folio (`N/M`) still counts this; the rendered
    /// `§N` is gone (the speaker head replaces it, §4).
    pub n: usize,
    /// The exchange's consumed title — the run's FIRST top-level `# h1`
    /// (`take_heading`, app.rs:317). Rendered only if §9's open question keeps
    /// a title; `None` → a bare speaker head. **Never re-consumed.**
    pub title: Option<String>,
    /// The run's prose + thinking, in order — every assistant message's
    /// `Text`/`Thinking` blocks concatenated, each message's `ToolCall` kept
    /// **in place**. The `¹` is emitted at each `ToolCall` (ui.rs:702); its
    /// ordinal IS the ledger's `N` (one per-turn series, §6).
    pub content: Vec<ContentBlock>,
    /// The run's apparatus, in CALL order — the ledger. `tools[k]` renders as
    /// ledger row `k + 1`; its `¹` is the `(k+1)`-th `ToolCall` in `content`.
    pub tools: Vec<Tool>,
    /// The turn's thinking expanded (D33). Set by the `▸` affordance / `Ctrl-T`;
    /// the affordance attaches to the leading `Thinking` block only.
    pub thinking_open: bool,
    /// `true` while the run is in flight; sealed at `AgentEnd` (app.rs:1544).
    /// A flip bumps the block's rev (the `(rev, width)` cache re-renders).
    pub open: bool,
}
```

In `pub enum Block` (`crates/wcode-tui/src/app.rs:128`), **remove** `TurnHead`
(`:133`), `Notes` (`:164`), and the committed-`Assistant` role (`:140`); **add**
`Turn(Turn),`. `Block::Tool` (`:148`) is retired as a *committed* block (a tool
now lives only inside a `Turn`) — see Friction 4 for the test ripple.

### 2.2 Why one block (the seams it must satisfy — cited)

ONE block = ONE range = the whole turn compares/selects/copies as a unit, so the
existing index-keyed machinery is untouched **in shape**:

- **`(rev, width)` cache** — `crates/wcode-tui/src/app.rs:1268` `pub(crate) fn append_block_lines`
  renders one block per `i`; a `Turn` is one `i`. A tool arriving mid-run bumps
  the turn's rev (`bump_rev`, `:1360`) → the whole turn re-renders once.
- **`ranges` / `block_at`** — `crates/wcode-tui/src/app.rs:3007` `fn block_at`
  finds the block whose recorded range contains a row; a `Turn` is one
  contiguous range (head + prose + ledger), so a click anywhere selects it.
- **browse selection** — `crates/wcode-tui/src/app.rs:1368` `fn selectable(&self, i: usize) -> bool {`
  `!matches!(self.transcript.get(i), Some(Block::TurnHead { .. }))`. With the
  head **inside** the `Turn`, the Turn IS selectable — this becomes `true` (the
  head row yields its column 0 to the selection bar, which is fine).
- **`copy_text`** — `crates/wcode-tui/src/app.rs:345` `fn copy_text(block: &Block)`:
  the `Block::TurnHead { .. } => return None,` arm (`:348`) and the
  `Block::Notes(items) => …` arm (`:373`) are **replaced** by one
  `Block::Turn(turn) => prose` arm (§8).
- **D32 expand/copy** — `crates/wcode-tui/src/app.rs:2019` `pub(crate) struct AffordanceHit`
  already carries `item: Option<usize>` (`:2025`) — a `Turn` maps
  `Some(k)` = ledger tool `k`, `None` = the leading-thinking row. **One block
  mixes chrome (the head), selectable prose, and per-tool affordances** — so the
  map computes the turn's internal row offsets (§7), exactly as the shipped
  `Block::Notes` does per note (`ui.rs:506-521` + `note_height`, `:796`).

**Friction (the design fights the code).** The head is **chrome** but shares
line 0 with the prose (the frame: `WCODE  A  n anchor…`); a `Turn` cannot be
"copy yields `None`" (it holds prose) nor "one affordance per block" (it holds
N tools). The resolution is §7's per-row offset walk + §8's prose-only copy.

---

## 3. The assembly — when the exchange is built

**Run boundary = `AgentStart` → `AgentEnd`** (`crates/wcode-tui/src/app.rs:1536`
/ `:1544`). A `Turn` is **opened on the run's first assistant content** (so an
aborted run with no reply leaves no empty turn), **accumulated in place** as the
run progresses (the shipped `Block::Todos` in-place precedent, `:156`), and
**sealed at `AgentEnd`**. There is no fold.

### 3.1 Surface state to add

```rust
// in `struct Surface` (the field list near `live`, crates/wcode-tui/src/app.rs:1133):
/// The index of the in-progress `Block::Turn` (the run's exchange), or `None`
/// when idle. Set when the run's first assistant content lands; cleared at
/// `AgentEnd`. While `Some`, the turn is the LAST block.
open_turn: Option<usize>,
```

### 3.2 The functions

```rust
/// Open (or fetch) the run's `Block::Turn`. On the first call of a run it pushes
/// a fresh, empty `Turn` (numbering it `++self.turns`) and records its index;
/// thereafter it returns the same index. # Contracts: pushes at most ONE turn
/// per run (so `turns` counts *exchanges* and the folio agrees); never called
/// for a non-assistant reply (a `/btw`/error keeps its own block).
fn open_turn(&mut self) -> usize { todo!() }

/// Absorb one assistant message into the run's turn: if the turn has no title,
/// consume the reply's first `# h1` (`take_heading`, app.rs:317); append the
/// message's `content` blocks (Text/Thinking/ToolCall) in order. # Contracts:
/// creates the turn via `open_turn`; a text-less tool-call message still appends
/// its `ToolCall` (so the ledger's `N` aligns) but renders no prose; replaces
/// `commit`/`commit_turn` (app.rs:1638/1654).
fn absorb_message(&mut self, message: AgentMessage) { todo!() }

/// Seal the run's turn at `AgentEnd` (app.rs:1544): flip `Turn.open = false`,
/// clear `open_turn`. # Contracts: idempotent when `open_turn` is `None`;
/// re-numbers nothing (the turn is already in place).
fn seal_turn(&mut self) { todo!() }

/// Append a running tool to the open turn at `ToolExecutionStart`
/// (app.rs:1466): push `Tool { done: false, .. }` into `turn.tools`, bump the
/// turn's rev, return the block index for `last_action`/`call_*`. A no-op
/// (pushes a standalone `Block::Tool`) only if no turn is open.
fn turn_start_tool(&mut self, call_id: &str, name: String) -> usize { todo!() }
```

### 3.3 The `apply()` arm edits (`crates/wcode-tui/src/app.rs:1443`)

- `MessageStart` (`:1445`) / `MessageUpdate` (`:1452`) — unchanged (`live` streams).
- `MessageEnd` (`:1461`) — `self.live = None; self.absorb_message(message);` (was `self.commit(message);`).
- `ToolExecutionStart` (`:1466`) — `self.turn_start_tool(&call_id, name);` (was `push_block(Block::Tool(…))`, `:1473`); keep `last_action`/`call_target`/`call_params`.
- `ToolExecutionUpdate` (`:1487`) / `ToolExecutionEnd` (`:1500`) — mutate the **last tool of the open turn** (`self.open_turn` → `turn.tools.last_mut()`) instead of `self.transcript.last_mut()` (`:1489`/`:1509`). End still records the `(path, diff)` change-set (`:1531`).
- `AgentEnd` (`:1544`) — insert `self.seal_turn();` (before the aborted/changes notices).

### 3.4 What this replaces

- `fn commit` (`crates/wcode-tui/src/app.rs:1638`) and `fn commit_turn` (`:1654`)
  → `absorb_message` / `open_turn`.
- `fn take_trailing_tools` (`crates/wcode-tui/src/app.rs:1689`) → **deleted**.
- `flush_live` (`crates/wcode-tui/src/app.rs:1715`) → calls `absorb_message`.
- `content_has_text` (`crates/wcode-tui/src/app.rs:305`) → kept (it still decides
  whether a message carries visible prose).
- The **`turns` counter** (`crates/wcode-tui/src/app.rs:1136` `turns: usize,`,
  bumped at `:1662` `self.turns += 1;`) → bumped once per `open_turn`, so it now
  counts exchanges; the folio keeps reading it.

### 3.5 Replay (`seed`, `crates/wcode-tui/src/app.rs:1833`)

Rework: an **exchange boundary is a `User` message**. On `User` → `seal_turn()`
then `push_block(Block::User(text))` (the shipped `:1845`); on `Assistant`
(`:1848`) → `absorb_message`; on `ToolResult` (`:1859`) → `turn_start_tool` +
`ToolExecutionEnd`-equivalent (append the tool, done). `seal_turn()` after the
loop, before the divider `insert_block` (`:1887`). So a resumed session rebuilds
one `Turn` per replayed exchange, keeping the `§N`/folio series (`self.turns`).

---

## 4. The speaker head

`YOU` (the `Block::User`) and `WCODE` (the `Block::Turn`) are one **reverse-video
run** — `Modifier::REVERSED`, the shipped selection modifier
(`crates/wcode-tui/src/ui.rs:603` `fn selection_style() -> Style { Style::default().add_modifier(Modifier::REVERSED) }`).
**No role, no glyph** (§15 (b)).

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Speaker { You, Wcode }

impl Speaker {
    fn label(self) -> &'static str { match self { Speaker::You => "YOU", Speaker::Wcode => "WCODE" } }
}

/// The speaker head spans for a line: a `REVERSED` run of `label()` + the gutter
/// to the content column. `first` = the turn/user head row; `cont` = the body
/// continuation gutter (aligns under the text, not the name).
fn speaker_spans(speaker: Speaker, first: bool) -> Vec<Span<'static>> { todo!() }
```

- **`Block::User`** — `crates/wcode-tui/src/ui.rs:635` `Block::User(text) => wrap(text, width, " ❯ ", "   ", user()),`
  → `speaker_spans(Speaker::You, true)` on line 0, `speaker_spans(Speaker::You, false)`
  on the rest. The transcript `❯` is **dropped**; the composer keeps `❯`.
- **`Block::Turn`** — line 0 = `speaker_spans(Speaker::Wcode, true)` + the first
  prose line; continuation = the body gutter.
- **The `§N` row is removed** from the spec (`docs/tui-design.md` §2) — `§` no
  longer has a call site (it is NOT a new glyph; it is a **removal**).

**Friction.** `wrap` (`crates/wcode-tui/src/ui.rs`) takes a single prefix+style;
the head is a **styled** run + gutter, so the User/Turn renderer cannot reuse
`wrap` for line 0 — it needs a head-prefixed variant (pin it as a small helper).

---

## 5. The ledger (R2 — the turn's foot)

One row per tool: `N  name  target  …stats` (the frame); under the shipped
`── notes ──` rule (`crates/wcode-tui/src/ui.rs:814` `fn notes_rule`) — see §9 Q2
for whether the rule stays.

```rust
/// The turn's foot: the `── notes ──` rule (reuse `notes_rule`, ui.rs:814) + one
/// line per tool in `items`, in call order. # Contract: ≥1 line; row `k`'s
/// FIRST line is its D32 toggle row (mirrors `notes_lines`, ui.rs:738).
fn ledger_lines(items: &[Tool], width: usize) -> Vec<Line<'static>> { todo!() }

/// One ledger row: ` {n}  {name}   {target}   {stats}` — `{n}` `dim`, `{name}`
/// `tool_name()`, `{target}` body, `{stats}` right-aligned `dim` (`+a`
/// `diff_add`, `−r` `diff_del`). The row is the tool's toggle/copy target.
fn ledger_row(n: usize, tool: &Tool, width: usize) -> Line<'static> { todo!() }

/// A ledger row's height (1 collapsed; 1 + the unfolded body when expanded) —
/// the offset table the D32 map walks (mirrors `note_height`, ui.rs:796).
fn ledger_row_height(tool: &Tool, width: usize) -> usize { todo!() }
```

- **Reuse the shipped body builders, frameless** (§11): `panel_param_lines`
  (`crates/wcode-tui/src/ui.rs:908`), `tool_panel_body` (`crates/wcode-tui/src/ui.rs:924`),
  `tool_summary_row` (`crates/wcode-tui/src/ui.rs:986`). These render a tool's
  params/output/`✓` summary.
- **Row expand** (§10 R2 flags "a row can't expand in place"): a row's `▸`
  unfolds its body **below** the row (params + full output + the `✓` summary).
  See §9 Q3 (below-row unfold vs retire per-tool expand).
- **Per-tool affordance** — the row is the D32 toggle; the copy cell yields that
  tool's `output` (`copy_block`, `crates/wcode-tui/src/app.rs:2807`). See §9 Q4
  (a visible dim `▸`/`▣` at the row end, the shipped `note_head_row` look
  `ui.rs:774` `let aff = format!("{glyph} ▣");`, or an invisible hit for R2's
  clean row).

**Friction.** R2's row is clean (no glyphs) but D32 needs a hit region; the
shipped look puts `▸ ▣` flush right (`note_head_row`, `ui.rs:766-784`). Pick one
(§9 Q4).

---

## 6. The reference ordinal — one per-turn series

The `¹` is emitted at each `ToolCall` in `content` (`crates/wcode-tui/src/ui.rs:702`
`ContentBlock::ToolCall { .. } => {`, `:709` `last.spans.push(Span::styled(footnote_mark(refn), link()));`).
Today `refn` is **per message** (`ui.rs:685`), because `content_lines` is called
once per committed `Assistant`. In a `Turn`, `turn_lines` walks the **whole
turn's `content` in ONE pass**, so `refn` is naturally **per turn** and equals
the ledger's row number (`k + 1`) — the §1 mismatch is gone.

- `footnote_mark` (`crates/wcode-tui/src/ui.rs:725`) is **kept** (`¹²³…` for 1..=9,
  `[n]` for `n ≥ 10`); it is `link()` (`:1702`).
- **The `¹` suppression rule is KEPT** (the task asks keep-or-drop): a `ToolCall`
  with no preceding prose line emits **no** mark — `crates/wcode-tui/src/ui.rs:708`
  `if let Some(last) = lines.last_mut() {`. A text-less round's tool is still
  **numbered** in the ledger, just unmarked. This is the only rule that keeps a
  `A(tc) → T → … → A(answer)` round's ledger correct.

---

## 7. The D32 affordance map (`crates/wcode-tui/src/ui.rs:496-558`)

The map currently emits, per block, one hit (or one per `Notes`-item, `:502-522`).
For `Block::Turn`, walk the turn's rows and emit:

```text
row 0            : the speaker head — CHROME, no hit (unless a leading Thinking
                   row: emit one hit, item = None, at the thinking row).
rows 1..H_prose  : the prose — no hit.
row  H_prose     : the `── notes ──` rule — no hit.
rows …           : one hit PER LEDGER ROW (item = Some(k)), stepping by
                   ledger_row_height(tool, measure) (mirrors note_height, ui.rs:796).
```

- `AffordanceHit` (`crates/wcode-tui/src/app.rs:2019`) is **unchanged** —
  `item: Option<usize>` (`:2025`) already spans "tool `k`" (`Some(k)`) and "the
  leading thinking" (`None`).
- The toggle rect spans the row through the `▸`; the copy cell is the `▣` — as
  the shipped `Block::Notes` hit builds them (`crates/wcode-tui/src/ui.rs:511-518`).

**Friction.** The offsets are a pure function of `(turn, width)`, recomputed each
frame (never cached — the `(rev, width)` cache holds `Line`s, not geometry,
`ui.rs:493`). A bug in `ledger_row_height` misaligns every hit below it; the
shipped `note_height` (`ui.rs:796`) is the template to mirror exactly.

---

## 8. Copy / select / toggle

- **`copy_text`** (`crates/wcode-tui/src/app.rs:345`) — replace the `TurnHead`
  (`:348`) and `Notes` (`:373`) arms with `Block::Turn(turn) => turn.content`
  text blocks joined (prose only, matching the retired `Assistant` arm, `:353`).
  See §9 Q5 (prose only vs prose + tool outputs).
- **`selectable`** (`crates/wcode-tui/src/app.rs:1368`) — the `TurnHead`
  exclusion goes; every committed block is selectable.
- **`toggle_block`** (`crates/wcode-tui/src/app.rs:2764` `fn toggle_block(&mut self, i: usize, item: Option<usize>)`)
  — add `Block::Turn(turn)`: `Some(k)` flips `turn.tools[k].expanded`; `None`
  flips `turn.thinking_open`. The `Block::Notes`/`Assistant` arms (`:2772`/`:2789`)
  are replaced.
- **`copy_block`** (`crates/wcode-tui/src/app.rs:2807`) — `Some(k)` yields
  `turn.tools[k].output`.
- **`toggle_all_tools`** (`crates/wcode-tui/src/app.rs:2286`) — extend the
  `matches!(block, Block::Tool(_) | Block::Notes(_))` filter (`:2292`) to
  `Block::Turn` (expand/collapse every ledger tool); the `Ctrl-T` binding is
  reused (§14: no new key).
- **`affordance_at`** (`crates/wcode-tui/src/app.rs:2930`
  `fn affordance_at(&self, row: u16, col: u16) -> Option<(usize, Option<usize>, AffordanceKind)>`)
  is **unchanged** — it already threads `item` through.

---

## 9. Spec / glyph, friction, open questions

### Spec amendments (named, NOT made here)
- `docs/tui-design.md` **§2** — the **`§` row is removed** (Direction I / §16);
  the `¹²³` row stays. No new glyph.
- `docs/tui-design.md` **§4 Transcript** — the blocked trio (`TurnHead` /
  `Assistant` / `Notes`) merges into one **`Turn`** (a speaker head + prose +
  a **ledger**); the `Block::User` transcript `❯` becomes the `YOU` head
  (composer unchanged).
- **No new `theme.rs` role** — `WCODE`/`YOU` reuse `Modifier::REVERSED`, `─` is
  declared.

### Friction (design vs code)
1. **"One exchange" is TWO blocks** — `Block::User` (the `YOU` prompt) precedes
   `Block::Turn`; the frame's "one unit" is adjacency + the between-roles blank.
   (Q1: fold the user into the `Turn`?)
2. **The live buffer renders OUTSIDE the turn.** `crates/wcode-tui/src/ui.rs:442`
   `let _ = app.focused_mut().append_live_lines(width, &mut lines);` appends the
   streaming message **after** every committed block — so during a round the
   text is *below* the turn (with its ledger), then folds in at `MessageEnd`.
   A **streaming reflow** remains (the shipped defect's cousin). Fully fixing it
   means rendering the live INTO the open turn — out of scope for this sketch.
3. **Chrome inside a selectable block** — the head shares line 0 with the prose;
   the selection bar paints column 0 of the whole `Turn` range. Resolved by
   keeping the prose-only copy (§8) and the per-row D32 walk (§7).
4. **Retiring blocks ripples.** `Block::Tool` (`crates/wcode-tui/src/app.rs:148`)
   is used by many test helpers and `seed`; `Block::Assistant`/`TurnHead`/`Notes`
   arms in `block_lines` (`crates/wcode-tui/src/ui.rs:638/645/656`) and `copy_text`
   (`app.rs:348/353/373`) all move. A large but mechanical diff.
5. **`take_heading` timing** — the title comes from the run's **first** h1; with
   accumulation this is the first assistant message's (`app.rs:317`), not the
   answer's.
6. **`¹` suppression kept** (§6) — a deliberate non-change.

### Open questions (ranked; the reviewer settles)
- **Q1** — Does the `Turn` absorb the `User` prompt (one block: `YOU` + `WCODE`),
  or stay `[User, Turn]`? (Frame reads either way.)
- **Q2** — Does the ledger keep the `── notes ──` rule (R2 shows it; the task
  frame omits it)?
- **Q3** — Ledger row expand: a **below-row unfold** (my pin) or **retire**
  per-tool expand (`Ctrl-T` shows everything)? R2 flags "a row can't expand in
  place".
- **Q4** — Ledger affordance: a visible dim `▸`/`▣` at the row end (shipped
  `note_head_row` look) or an **invisible** D32 hit (R2's clean row)?
- **Q5** — `copy_text(Turn)`: prose only (my pin) or prose + tool outputs?
- **Q6** — `toggle_block(Turn, None)` (browse `Enter` on the turn): the thinking,
  all-tools, or a no-op?
- **Q7** — Is the **drop cap** (Direction I, the `A`) in this sketch's scope? The
  task lists only "speaker head + ledger"; the cap reuses `Modifier::REVERSED`
  on the first prose cell (§9 of `tui-turn-notes.md`) and the head row leaves
  room for it — flagged, not built.
- **Q8** — Retire `Block::Tool` fully, or keep it for the live/running tool?
- **Q9** — Is the consumed `title` rendered (a quiet subtitle beside `WCODE`) or
  dropped (§15 drops `§N`; the frame shows no title)?

---

## 10. Test skeletons (`todo!()` bodies; behaviour-named)

### `app.rs`
```rust
#[test] fn one_exchange_is_one_turn_block() { todo!() }
//   A(text+tc1) → T1 → A(text+tc2) → T2 → A(answer), then AgentEnd:
//   transcript == [User, Turn]; the Turn's content has tc1 then tc2 in order,
//   its tools are [T1, T2] in call order — one block, one section.
#[test] fn the_turn_holds_all_the_runs_prose_and_all_its_tools() { todo!() }
#[test] fn a_tool_lands_at_its_own_turns_foot_not_the_next_replys() { todo!() }
//   the traced §1 defect: T1 must NOT attach to a later reply.
#[test] fn the_ledger_order_is_the_toolcall_order_which_is_the_tool_order() { todo!() }
#[test] fn a_prose_only_exchange_gets_a_turn_with_an_empty_ledger() { todo!() }
#[test] fn seal_turn_is_idempotent_when_no_turn_is_open() { todo!() }
#[test] fn a_run_with_no_assistant_content_leaves_no_empty_turn() { todo!() }
#[test] fn seed_rebuilds_one_turn_per_replayed_exchange() { todo!() }
#[test] fn turns_counts_exchanges_and_survives_a_seed() { todo!() }
#[test] fn a_running_tool_is_appended_to_the_open_turn() { todo!() }
#[test] fn copy_text_of_a_turn_is_its_prose() { todo!() }
#[test] fn a_turn_is_a_browse_selection_target() { todo!() }
#[test] fn toggle_block_on_a_turn_none_flips_the_thinking() { todo!() }
#[test] fn toggle_all_tools_expands_every_ledger_row() { todo!() }
```

### `ui.rs`
```rust
#[test] fn a_turn_renders_the_wcode_head_then_prose_then_the_ledger() { todo!() }
#[test] fn the_user_block_renders_a_reversed_you_head_not_a_prompt_glyph() { todo!() }
#[test] fn the_reference_ordinal_is_one_per_turn_across_rounds() { todo!() }
//   content with tc1, tc2 → the prose carries ¹ then ²; the ledger rows are 1, 2.
#[test] fn a_text_less_round_emits_no_superscript_but_is_still_numbered() { todo!() }
#[test] fn a_ledger_row_is_its_d32_toggle_row_and_the_offsets_line_up() { todo!() }
#[test] fn an_expanded_ledger_row_unfolds_its_body_below() { todo!() }
#[test] fn footnote_mark_maps_1_to_9_then_falls_back_to_ascii() { todo!() }
```

---

## 11. What this sketch did NOT do (honest)

- **No code was changed** — this file only. `cargo check` is untouched; the
  `file:line` cites are against `HEAD` (`225ac67`) and stay valid because no
  `.rs` file moved.
- **No terminal was run.** The frame (§0) is hand-authored from the design doc.
- **The row-offset math (§7) is reasoned, not measured** — `ledger_row_height`
  must mirror `notes_lines`/`note_height` (`ui.rs:738`/`:796`) exactly or every
  D32 hit below a row misaligns.
- **9 open questions are left for the reviewer** (§9) — this sketch is fillable
  once Q1–Q9 are ruled.
