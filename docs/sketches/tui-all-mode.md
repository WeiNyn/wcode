# SKETCH — TUI All mode: merge members into the main transcript

- **Status: review-only, NOT real code.** Nothing here compiles and nothing is
  wired in. The developer writes the real code from these signatures and leaves
  **ZERO `SKETCH` markers** in the diff.
- **Implements:** the TUI analogue of the VS Code extension's **All / Focus**
  mode — [`docs/vscode-ui-v2-plan.md`](../vscode-ui-v2-plan.md) §4 decision 2
  (*"All mode is a client-side merge over the existing per-session transcripts —
  no protocol change"*, `:75`) and [`docs/vscode-ui-v3-plan.md`](../vscode-ui-v3-plan.md)
  §1.1. Reference implementation: `editors/vscode/src/render.ts:224`
  `export function renderMerged(state: ViewState, thinking: FoldDefault = "auto"): RenderedBlock[] {`
  over the reducer's arrival `seq` (`editors/vscode/src/reducer.ts:131`
  `  seq: number;`, doc at `:129` *"the only ordering signal on the wire"*; the
  stamp is `reducer.ts:535` `const stamped = added.map((block, i) => ({ ...block, seq: base + i }));`),
  gated by `editors/vscode/src/webview.ts:40` `export type ViewMode = "all" | "focus";`.
- **Design locks this sketch honors** (`vscode-ui-v3-plan.md`): the mode control
  moves **into the header** as a `.seg`, cell 2 (`:149` *"The All/Focus control
  moves **into** the header as a `.seg` (cell 2)."*); in **All** mode **cell 1
  loses the identity chip** and shows the **root member's state glyph alone**
  (`:116` *"All mode has no target, so cell 1 loses the chip."*, `:118` *"in
  **All** mode, cell 1 is **the ROOT member's state glyph alone** (no chip, no
  name)"*); per-turn `.who` lines carry per-member identity in All (`:122`).
- **Form chosen: a new `docs/sketches/*.md`** (Form A; the `docs/sketches/turn-block.md`
  precedent). Justification: the change is spread across **five** files —
  `crates/wcode-tui/src/app.rs`, `ui.rs`, `lib.rs`, `crates/wcode-cli/src/config.rs`,
  `crates/wcode-cli/src/main.rs` — so **no single in-place block sits where the
  code goes**, and a doc lets every `file:line` be cited against the **current
  tree, which this sketch does not edit**. Every cite is `file:line` + a quoted
  snippet against `HEAD` (`991285e`). **No `.rs` file is touched by this sketch.**
- **The amendments are SETTLED** (§7 A1–A9). A1 default = **Focus**; A2
  committed-only merge; A3 display-only, Focus-scoped selection; A4 header chip;
  A5 root-then-members replay (after the seed stamp); A6 `Ctrl-O`; A7
  `/view <all|focus>`; A8 label every block; A9 `Enter` → focused surface. The
  two divergences from the reference are **documented**, not hidden (§7).

---

## 0. The target frame (All mode)

```
WCODE · ALL · session a1b2c3d4 ────────────────────── wcode · ⎇ main   ← pinned head: the .seg (cell 2, after WCODE, §2.6)
────────────────────────────────────────────────────────────────────
 wcode   A  n anchor hashes the line's raw content …                    ← origin head, root turn
 YOU     add a --view flag                                              ← Block::User, root
 w1      read   crates/wcode-cli/src/main.rs  128 ln  12ms             ← a member's block, interleaved
 w1      A  added the flag after the past-tense branch …                 ← w1's turn, in arrival order
 wcode   edit   crates/wcode-cli/src/config.rs  +3 −0   9ms             ← the root's next block
                                                          ⏸ idle       ← cell 1: the ROOT glyph alone (no chip, no name)
```

The **order is arrival order across every surface** (the reducer's `seq`), not
per-surface runs; each block carries a dim **origin head** (`wcode` / `w1` …) that
the Focus view omits. In **Focus** mode the frame is byte-identical to today.

---

## 1. The current state (grounded — this is what the sketch plugs into)

**Rendering today is focused-surface-only; no merged view exists** (`grep
all_mode|merged|ViewMode|interleav crates/wcode-tui` finds only unrelated
"interleaved" doc comments).

- The transcript loop iterates the **focused** surface:
  `crates/wcode-tui/src/ui.rs:585` `    let n = app.transcript().len();` →
  `:587` `        ranges.push(app.focused_mut().append_block_lines(i, width, &mut lines));`
  → `:592` `    let _ = app.focused_mut().append_live_lines(width, &mut lines);`.
- `crates/wcode-tui/src/app.rs:4434` `    pub fn transcript(&self) -> &[Block] {`
  → `        &self.focused().transcript`.
- Members are **separate surfaces** whose events already reach the app:
  `App` holds one `Vec<Surface>` (`app.rs:2386` `    surfaces: Vec<Surface>,`,
  index 0 = root; struct `app.rs:1335`, `transcript` `app.rs:1344`); events route by
  id to the owner (`app.rs:3778` `            AppEvent::Agent(id, event) => {` →
  `:3787` `                    if let Some(idx) = self.surface_index(&id)`).
- The loop merges every surface's stream into **one channel tagged by id** —
  `crates/wcode-tui/src/lib.rs:501` `    let (agent_tx, mut agents) = mpsc::unbounded_channel::<(SessionId, AgentEvent)>();`;
  the receipt **arm** is `:540` `            Some((id, event)) = agents.recv() => {`
  and the handle call is `:550` `                app.handle(AppEvent::Agent(id, event));`
  (the only runtime receipt point — the other `handle(Agent(..))` calls are
  `#[cfg(test)]`) — **but the stream carries no global ordinal, and none is
  stamped anywhere today.**
- Member activity is shown only **outside** the transcript: the working-team strip
  (`app.rs:3035` `pub fn working_team_rows`, drawn `ui.rs:2396` `fn draw_working_team`)
  and the sidebar (`app.rs:3050` `pub fn member_rows`, drawn `ui.rs:2257` `fn draw_sidebar`).
- The turn has **no speaker head** (`ui.rs:838` `fn turn_lines` → no head; D012) —
  the only `WCODE` is the pinned header (`ui.rs:798`
  `        let mut spans = vec![Span::styled("WCODE", dim())];`, in
  `ui.rs:789` `fn session_head_row(head: &SessionHead, width: usize) -> Line<'static> {`).
- The **single content column** the origin head must align to is `CONTENT_COL`
  (`ui.rs:29` `const CONTENT_COL: usize = 7;`, doc at `:25`), shared by the user
  block, the `WCODE` head and every body wrap.

**The view-flag precedent.** `sidebar` is a client-local bit beside `mode`:
`app.rs:2476` `    sidebar: bool,` (*"It is view state like `mode`, not surface
state: `toggle_sidebar` flips this bit and marks dirty, never touching a
surface"*), flipped by `app.rs:4970` `    fn toggle_sidebar(&mut self) {` from
`app.rs:3874` `            Key::Ctrl('b') => self.toggle_sidebar(),`. **`view` is the
second such bit.**

**Mode is an enum already** — `app.rs:2308` `pub enum Mode {` (`Input`/`Browse`) with
field `app.rs:2459` `    mode: Mode,`. All/Focus is **orthogonal**: a *rendering*
choice, so it gets its own `ViewMode` (`Mode` must not grow a variant — browse keys
route off `mode == Mode::Browse`, `ui.rs:190`/`:603`).

**The chrome that carries identity is focused-derived** — this is what blocker 2
(§4) must fix: `app.rs:5006` `    pub fn running(&self) -> bool {` →
`        self.focused().running`, and `app.rs:4934` `    pub fn run_elapsed(&self) -> Option<std::time::Duration> {`.

**The toggle surfaces' registries.**

- Keys: `app.rs:1026` `pub(crate) const KEYS: &[(&str, &str)] = &[` (the `Ctrl-B`
  row is `app.rs:1040` `    ("Ctrl-B", "toggle the sidebar"),`). The handler is
  `fn on_key` (`:3802`), Ctrl arm `:3862`–`:3888`; the Ctrl-B arm is `:3874`.
- Commands: the registry head is `app.rs:729` `const COMMANDS: &[Command] = &[`,
  the `surface` entry is `app.rs:815` `        name: "surface",`, the `width`
  entry is `app.rs:821` `        name: "width",`; registry ends `:844` `];`.
  Dispatch is `app.rs:4000` `    fn run_command(&mut self, cmd: &Command, arg: Option<&str>) {`.
- Config: `config.rs:406` `pub struct TuiConfig {` with `:409` `    pub width: Option<usize>,`,
  resolved into `wcode_tui::TuiSpec` at `config.rs:900` `    let tui = wcode_tui::TuiSpec {`
  and carried on the resolved `Config` (`config.rs:610` `    pub tui: wcode_tui::TuiSpec,`).
- The spec + carrier: `crates/wcode-tui/src/lib.rs:45` `pub struct TuiSpec {`
  (`:47` `    pub width: Option<usize>,`); `Options` (`lib.rs:279`) carries it as
  `lib.rs:297` `    pub tui: TuiSpec,`; `run` is `lib.rs:358` `pub async fn run(`;
  the width seed is `lib.rs:388` `    if let Some(cols) = tui.width {` →
  `        app.set_measure(cols);`.
- CLI: `crates/wcode-cli/src/main.rs:197` `    tui: bool,` (the `--tui` flag, parsed
  `:315` `            "--tui" => a.tui = true,`); both `Options` sites are
  `main.rs:1017` (socket) with `:1028` `                        tui: cfg.tui,` and
  `main.rs:1860` (local) with `:1872` `                tui: cfg.tui,`.

---

## 2. The interface

All new items are marked with their destination file. Signatures are final;
bodies are `todo!()`.

### 2.1 `crates/wcode-tui/src/app.rs` — the mode enum

```rust
/// The client-local view mode (the VS Code extension's `ViewMode`,
/// `webview.ts:40`): `Focus` renders the focused surface's transcript —
/// **today's behavior**; `All` renders the MERGED transcript of the root + every
/// member, interleaved by arrival order (`renderMerged`, `render.ts:224`).
///
/// Orthogonal to [`Mode`] (input vs browse) — this is a RENDERING choice, so it is
/// its own enum. Default `Focus` ⇒ a frame is byte-identical until the user opts
/// in (A1; the `sidebar: bool` off-by-default precedent, `app.rs:2476`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    #[default]
    Focus,
    All,
}

impl ViewMode {
    /// Parse a config/CLI/`/view` token: `all` \| `focus` (case-insensitive,
    /// surrounding whitespace trimmed). # Errors: any other token — the message
    /// names the value and the two accepted words (the loud `[tui] width` rule,
    /// `config.rs:900`).
    pub fn parse(token: &str) -> Result<ViewMode, String> { todo!() }

    /// The header `.seg` label, rendered ONLY in All mode — ONE rule (§2.6): `ALL`
    /// in All, `None` in Focus, so Focus draws no seg and its header stays
    /// byte-identical to today. Returning `None` (rather than a dead `"FOCUS"`
    /// string) makes the rule mechanical: **the seg appears iff `label()` is
    /// `Some`.** Beside `WCODE` (v3 §1.1 cell 2).
    pub fn label(self) -> Option<&'static str> {
        match self {
            ViewMode::Focus => None,
            ViewMode::All => Some("ALL"),
        }
    }
}
```

### 2.2 `crates/wcode-tui/src/app.rs` — new `App` fields

```rust
// in `struct App` (`app.rs:2383`), beside the `sidebar: bool` (`:2476`):
/// The active view mode (All/Focus). View state like `mode`/`sidebar`, NOT
/// surface state — `toggle_view` flips it and marks dirty, never touching a
/// surface. Read by `draw_transcript` (which transcript to build), the header
/// `.seg`, and the chrome routing (`chrome_surface`, §4).
view: ViewMode,
/// The next arrival ordinal to stamp on a block at append time. MONOTONIC over
/// the whole app — every surface draws from it, so it is the merged view's single
/// total order (mirrors the reducer's `ViewState.seq`, `reducer.ts:131`). Starts
/// 0; a replay advances it past every seeded block (§3), so live ordinals are
/// strictly greater than every seeded one.
next_seq: u64,
/// The merged view's scroll offset (lines up from the bottom), in the MERGED line
/// space. Owned by `App` (NOT a surface) so a mode flip never leaks merged scroll
/// into a surface's `scroll`/`last_total` (blocker 3). Used only in `All` mode.
all_scroll: usize,
/// The merged view's scroll clamp, viewport height, and last measured
/// total/width — the All-mode mirror of **all four** of `Surface`'s remaining
/// scroll fields (`app.rs:1385` `max_scroll`, `app.rs:1388` `viewport`,
/// `app.rs:1389` `last_total`, `app.rs:1390` `last_width`) beside `all_scroll`
/// (the mirror of `scroll`, `app.rs:1383`). `Surface::sync_scroll`
/// (`app.rs:2277`) consumes all four for pin-on-growth (`:2281`–`:2283`) and the
/// top clamp (`:2285`–`:2287`); the merged view needs the same four, or it can
/// only clamp (merged-scroll). Used only in `All` mode; `all_viewport` also feeds
/// `App::page` in All (below).
all_max_scroll: usize,
all_viewport: usize,
all_last_total: usize,
all_last_width: usize,
```

Constructor `App::new` (`app.rs:2524`) gains
`view: ViewMode::default(), next_seq: 0, all_scroll: 0, all_max_scroll: 0,
all_viewport: 0, all_last_total: 0, all_last_width: 0,`.

### 2.3 `crates/wcode-tui/src/app.rs` — new `Surface` field

```rust
// in `struct Surface` (`app.rs:1335`), beside `transcript`/`block_revs`/`cache`:
/// Each committed block's arrival ordinal, index-aligned with `transcript` — the
/// order the app saw it. The merged view's merge key. The FOURTH parallel vec
/// (after `transcript`, `block_revs`, `cache`); every push/insert/remove must keep
/// it in lockstep (see §6 friction 1).
seqs: Vec<u64>,
```

### 2.4 `crates/wcode-tui/src/app.rs` — the merge + stamp + accessors

```rust
/// The active view mode.
pub fn view_mode(&self) -> ViewMode { todo!() }

/// Set the view mode; marks the frame dirty. A no-op when unchanged. Called at
/// startup (from `TuiSpec.view`) and by `/view`.
pub fn set_view_mode(&mut self, view: ViewMode) { todo!() }

/// Flip Focus ⇄ All (`Ctrl-O`, A6). # Contracts: side-effect-free beyond the bit
/// + `dirty` (like `toggle_sidebar`, `app.rs:4970`).
fn toggle_view(&mut self) { todo!() }

/// The merged view's iteration order: `(surface index, block index)` over EVERY
/// surface (root included), sorted by the block's arrival ordinal — ties broken by
/// the canonical surface order (`canonical_order`, `app.rs:2637`) then block index,
/// for a stable total order. Owned (no borrow held), so the render loop can then
/// take `append_block_lines_at` per pair. # Contracts: O(n log n) over all blocks;
/// empty when every surface is empty. (In `Focus` mode the caller uses
/// `transcript()` instead and never calls this.)
pub(crate) fn merged_order(&self) -> Vec<(usize, usize)> { todo!() }

/// A surface's display label for the All-mode origin head: the private
/// `Surface.label` field (`app.rs:1339` `    label: String,` — no public accessor
/// exists today, so this new `App`-level reader is the first). `wcode` for the
/// root, `w1`… for members. # Contracts: `si` out of range ⇒ `""`.
pub(crate) fn surface_label(&self, si: usize) -> &str { todo!() }

/// Append `block` to surface `idx` and stamp it with the next arrival ordinal —
/// **THE App-level append chokepoint** (the *ordinal chokepoint*). Every `App`-driven append
/// that bypasses the `AppEvent::Agent` arm routes through here:
/// `submit`'s `Block::User` (`app.rs:3976`) **and** the "already running" transient
/// it emits (`app.rs:3972`), `show_change`'s `Block::Diff` (`app.rs:4585`),
/// `notice`'s `push_notice` (`app.rs:4308`) and `transient`'s `push_transient`
/// (`app.rs:4316`) — five sites, else those blocks keep the `push_block`
/// placeholder `0` and sort to the FRONT of the merged view. (The reference stamps
/// *inside* its append chokepoint, `reducer.ts:535`; the TUI's `Surface`-level
/// chokepoint is `push_block`, `app.rs:1630`, but only `App` owns `next_seq`, so
/// the App-level chokepoint is what stamps.) **ORDER (the off-by-one): clear the
/// empty-state hint FIRST, then measure, then append, then stamp** —
/// `self.surfaces[idx].clear_hint(); let before = self.surfaces[idx].transcript.len();
/// self.surfaces[idx].push_block(block); self.stamp_new(idx, before);`.
/// `push_block` clears the hint internally (`app.rs:1632` `self.clear_hint();`), so
/// measuring before that would count the hint and land `before` one PAST the new
/// block on a hint-present surface (`seed_hint_if_empty`, `app.rs:1641`) — making
/// `stamp_new`'s `before >= len` contract a silent no-op and leaving the block at
/// `seq 0`. Clearing first is idempotent (a no-op when there is no hint), so the
/// delta is exactly the appended block. # Contracts: `idx` out of range ⇒ no-op
/// (defensive; every caller passes `self.focus`).
fn push_on(&mut self, idx: usize, block: Block) { todo!() }

/// Render block `bi` of surface `si` into `out`, matching what
/// `focused_mut().append_block_lines` (`app.rs:1488`) produces for the focused
/// surface — the All-mode render seam. When `origin` is `Some(label)`, a dim
/// origin head row is prepended (and IS part of the returned range). Returns the
/// block's line range. # Contracts: a no-op `0..0` when `si`/`bi` are out of
/// range; **it OWNS the leading blank separator — it must NOT delegate to
/// `append_block_lines`** (separator ownership): that method suppresses the separator for
/// its surface-local `i == 0` (`app.rs:1518`
/// `        if i > 0 && !out.is_empty() && !lines.is_empty() {`), so a surface's
/// FIRST block (`bi == 0`) rendered after another surface's lines would lose its
/// separator. Key the separator on `out` alone — emit one iff `out` is non-empty
/// — reproducing `append_block_lines`' *effect* without its block-local index
/// rule. **The origin head is appended OUTSIDE the `(rev, width)` cache** (§2.5)
/// so a mode flip never shows stale heads nor poisons the Focus cache.
pub(crate) fn append_block_lines_at(
    &mut self,
    si: usize,
    bi: usize,
    width: usize,
    origin: Option<&str>,
    out: &mut Vec<Line<'static>>,
) -> Range<usize> { todo!() }

/// Stamp the blocks `[before..]` of surface `idx` with fresh ordinals from
/// `next_seq`, bumping it past the last. Called from the `AppEvent::Agent` arm
/// (`app.rs:3778`) right after a surface append — the reducer's append site
/// (mirroring the reducer's `pushBlocks`, `reducer.ts:535`) — and from
/// `App::push_on` for the App-level appends that bypass that arm (ordinal chokepoint).
/// # Contracts: stamps EXACTLY the appended tail `[before..]` — no more, no fewer,
/// and a no-op when `before >= transcript.len()` (i.e. the append added nothing);
/// `before` MUST be the true pre-append length, so **both** callers clear the
/// empty-state hint first (`Surface::clear_hint`, `app.rs:1652`) or `push_block`'s
/// internal `clear_hint` (`app.rs:1632`) shifts the length by one and the stamp
/// silently no-ops; keeps `seqs` aligned with `transcript`.
fn stamp_new(&mut self, idx: usize, before: usize) { todo!() }

/// Stamp a freshly-SEEDED surface's blocks with ordinals from `next_seq`, in
/// index order, advancing `next_seq` past them (blocker 4). A seed is a full
/// replay, so it re-stamps the WHOLE surface (its blocks are all replay blocks);
/// used only by `seed_history`, before any live block exists. # Contracts: the
/// caller replays ROOT-FIRST, so every seeded ordinal < every live ordinal (A5).
fn stamp_replayed(&mut self, idx: usize) { todo!() }

/// Scroll up `lines` in the ACTIVE transcript: in All mode adjust `all_scroll`
/// (clamped to the merged content), else the focused surface's `scroll`
/// (today's behavior, `app.rs:4491`). View-aware so the wheel/PgUp actually moves
/// the merged view (blocker 6). # Contracts: marks dirty.
fn scroll_up(&mut self, lines: usize) { todo!() }

/// Scroll down `lines` in the ACTIVE transcript (All ⇒ `all_scroll`, else
/// focused; `app.rs:4497`). `0` is the tail. # Contracts: marks dirty.
fn scroll_down(&mut self, lines: usize) { todo!() }

/// A full page for the ACTIVE transcript: in All mode the merged viewport height
/// (`all_viewport`, §2.2), else the focused surface's page (`app.rs:4486`). Read
/// by `PgUp`/`PgDn` (`app.rs:3884`).
fn page(&self) -> usize { todo!() }

/// Lines scrolled up from the tail of the ACTIVE transcript (`0` = following).
/// In All mode the merged offset `all_scroll`, else the focused surface's `scroll`
/// (`app.rs:4503`). Read by the renderer's windowing (`ui.rs:606`
/// `    let end = total.saturating_sub(app.scroll());`) and by `corner_titles`
/// (`ui.rs:2013` `        if show_scroll && app.scroll() > 0 {`).
pub fn scroll(&self) -> usize { todo!() }

/// Reconcile the ACTIVE transcript's scroll with what the renderer just measured.
/// In All mode mirror [`Surface::sync_scroll`] (`app.rs:2277`) against the MERGED
/// line space via the four `all_*` fields (§2.2) — pin-on-growth (`:2281`–`:2283`)
/// then clamp — so the merged view stays pinned while members append; in Focus
/// mode it is today's `self.focused_mut().sync_scroll(...)` (`app.rs:4515`),
/// byte-identical. # Contracts: in All mode never touches a surface's own
/// `scroll`/`last_total`; marks no dirty (the renderer owns this each frame).
pub fn sync_scroll(&mut self, total: usize, height: usize, width: usize) { todo!() }
```

**`scroll_up`/`scroll_down`/`page`/`scroll`/`sync_scroll` are the whole scroll
surface (blocker 6).** The wheel (`Key::ScrollUp`/`Down`) and `PgUp`/`PgDn` route
through `scroll_up`/`scroll_down`/`page` (`app.rs:3864`–`:3888`); the renderer's
windowing reads `scroll` (`ui.rs:606`) and reconciles via `sync_scroll`
(`ui.rs:599`). **All five** must be view-aware (All ⇒ `all_scroll` + the merged
`total`/viewport; Focus ⇒ the focused surface, today's behavior) or the merged view
never scrolls while the focused offset moves unseen. All is **not** just
`all_scroll`: the per-surface scroll state is **five** fields — `scroll`
(`app.rs:1383`), `max_scroll` (`:1385`), `viewport` (`:1388`), `last_total`
(`:1389`), `last_width` (`:1390`) — consumed by `scroll_up` (`:2267`), `page`
(`:2261`) and `sync_scroll`'s pin-on-growth (`:2281`–`:2283`); All mirrors **all
five** as the `all_*` fields (§2.2, merged-scroll). `set_block_ranges`/`block_at`
are **not** view-aware (browse is Focus-only, §6 friction 3).

### 2.5 `crates/wcode-tui/src/ui.rs` — the origin head + the merged pass

```rust
/// One All-mode origin head row: a dim ` {label}` run padded to [`CONTENT_COL`]
/// (`ui.rs:29`; the `user_head` shape, `ui.rs:741`, without the accent). The
/// label is the owning surface's `label` (`wcode` for the root, `w1`… for
/// members). Pinned to CONTENT_COL so every interleaved block's text still lines
/// up under the single content column.
fn origin_row(label: &str, width: usize) -> Line<'static> { todo!() }
```

**The origin head MUST render OUTSIDE the per-block cache.** `append_block_lines`
returns `self.cache[i].lines` (`app.rs:1517` `        let lines = &self.cache[i].lines;`)
— the cached lines are the block's own render, identical in both modes. Baking an
origin head INTO that cache would make a Focus frame show stale heads (or All
frame lack them) after a mode flip, because the cache is keyed only on
`(rev, width)`, not on the view mode. So `append_block_lines_at` appends the
`origin_row` **before** the cached block lines, never mutating the cache.

The merged pass **replaces** the focused loop in `draw_transcript`
(`ui.rs:585`–`:592`) with a mode branch — **described here, not written**:

```text
if app.view_mode() == ViewMode::All {
    for (si, bi) in app.merged_order() {                 // owned indices, no borrow held
        let origin = app.surface_label(si).to_string();  // owned, so the &mut below is free
        // Do NOT push into `ranges`: `set_block_ranges` is Focus-only (blocker 3).
        let _ = app.append_block_lines_at(si, bi, width, Some(&origin), &mut lines);
    }
    // DISPLAY-ONLY: browse/hit-testing are Focus-scoped (blocker 3) — do NOT call
    // set_block_ranges / set_transcript_hit here (that is why the loop above pushes
    // no range); reconcile all_scroll against the merged `total` via the
    // view-aware `sync_scroll` (below).
} else {
    let n = app.transcript().len();                      // today's path, byte-identical
    for i in 0..n { ranges.push(app.focused_mut().append_block_lines(i, width, &mut lines)); }
    app.set_block_ranges(ranges.clone());
    app.set_transcript_hit(col, start, rows_text);
}
// The live trailer is appended AFTER the committed list in BOTH modes, focused only
// (A2; §6 friction 2).
let _ = app.focused_mut().append_live_lines(width, &mut lines);
```

### 2.6 `crates/wcode-tui/src/ui.rs` — the header `.seg` (A4) + the identity rule (blocker 2)

**The chip lives in the HEADER, not the composer foot** (v3 §1.1: *"The All/Focus
control moves **into** the header as a `.seg` (cell 2)"*, `:149`). The TUI's header
is the pinned head row `session_head_row` (`ui.rs:789`). Append a dim `.seg` run to
its **left** run, after the `WCODE` span (`ui.rs:798`
`        let mut spans = vec![Span::styled("WCODE", dim())];`) and before the
`session` group — the mode is one more datum in the existing left run:

**ONE rule (the *one-seg rule*): the seg is present in `All` mode only — ` · ALL` — and
absent in `Focus`.** There is no ` · FOCUS` run: Focus must stay byte-identical to
today's header. This is mechanical because `ViewMode::label` returns
`Option<&'static str>` (`All ⇒ Some("ALL")`, `Focus ⇒ None`, §2.1) — push the seg
iff it is `Some`:

```rust
// in `session_head_row`'s `left` closure (ui.rs:797), after the WCODE span:
//   if let Some(seg) = head.view.label() {
//       spans.push(Span::styled(format!(" · {seg}"), dim()));
//   }
```

**Threading (the *head-threading* item — the location is settled, so is the mechanism).**
`session_head_row` takes `&SessionHead` (`ui.rs:789`), so the mode rides the head:
add a `view: ViewMode` field to `SessionHead` (`app.rs:164`
`pub struct SessionHead {`), set it in `SessionHead::of` (`app.rs:184`) from
`app.view_mode()`, and read it in `session_head_row` as above. No new parameter and
no `App` borrow in the renderer's head path — the head is already the chrome
snapshot.

```rust
// in `struct SessionHead` (`app.rs:164`), beside `session` (`:166`):
/// The active view mode — `All` draws the ` · ALL` seg; `Focus` draws none
/// (ONE rule, §2.6). Snapshot of `app.view_mode()` (head-threading).
pub view: ViewMode,
```

The `of` constructor (`app.rs:184`) gains one line — `view: app.view_mode(),` —
beside the `session`/`model` fields it already snapshots.

**The identity rule (blocker 2).** In **All** mode the merged transcript is
nobody's particular transcript, so the header/footer **loses the identity chip** and
the **state glyph + head identity reflect the ROOT**, not the focused member
(v3 §1.1 `:116`/`:118`). Today the footer state is focused-derived
(`app.rs:5006` `running`, `app.rs:4934` `run_elapsed`, consumed by `corner_titles`,
`ui.rs:1977`). **`running()` and `run_elapsed()` MUST NOT change** — they are not
chrome: `running()` is the event-loop tick guard (`lib.rs:563`
`            _ = tick.tick(), if app.running() => {` — if it became the *root's*
state in All, a running member under an idle root would stop ticking and its
`run_elapsed` would freeze), and `run_elapsed()` feeds the **sidebar's focused row**
(`ui.rs:2306` `        if focused && let Some(elapsed) = app.run_elapsed() {`— in All
it would show the root's elapsed on the focused member's row). So add **chrome-only**
accessors, used **solely** by `corner_titles`:

```rust
/// The surface whose identity/state the CHROME reflects: the ROOT (index 0) in
/// All mode — the merged transcript is nobody's particular transcript (v3 §1.1) —
/// else the focused surface. # Contracts: infallible (`surfaces` is never empty).
fn chrome_surface(&self) -> &Surface { todo!() }

/// The chrome surface's `Status` (session/model/effort) — the `SessionHead::of`
/// source (blocker 7). The sidebar/loop keep `status()` (`app.rs:4997`, focused).
fn chrome_status(&self) -> &Status { todo!() }

/// True when the CHROME surface is running (the ROOT in All, the focused in
/// Focus). Used ONLY by `corner_titles` (`ui.rs:1988`); the tick guard keeps
/// `App::running()`.
fn chrome_running(&self) -> bool { todo!() }

/// The CHROME surface's run elapsed (the ROOT in All, the focused in Focus). Used
/// ONLY by `corner_titles`; the sidebar keeps `App::run_elapsed()`.
fn chrome_elapsed(&self) -> Option<std::time::Duration> { todo!() }
```

`corner_titles`' match (`ui.rs:1988`
`    let (state, state_style) = match (app.running(), app.run_elapsed()) {`) becomes
`match (app.chrome_running(), app.chrome_elapsed())`.

**The pinned head names the ROOT in All mode (blocker 7).** `SessionHead::of`
(`app.rs:184` `    pub(crate) fn of(app: &App) -> SessionHead {`) snapshots the
**focused** `Status` (`:186` `            session: app.status().session.clone()`,
via `app.rs:4997` `    pub fn status(&self) -> &Status {` → `        &self.focused().status`)
— so the head's `session · model · effort` would still name the focused member in
All. `SessionHead::of` must read `app.chrome_status()` for `session`/`model`/`effort`
(the ROOT in All, the focused in Focus); `project`/`branch` stay
`cwd`/`git` (process-wide, mode-independent). `App::status()` (`app.rs:4997`) itself
stays focused (it is per-surface state other callers read).

The TUI has **no separate target-name chip** to hide (its chrome is the footer state
+ the head + the sidebar `*`); the All-mode "no chip" rule reduces to this chrome
routing, plus the `▤ browse` segment never appearing (browse is a no-op in All, §6
friction 3).

### 2.7 `crates/wcode-tui/src/lib.rs` — the spec + seeding

```rust
// `struct TuiSpec` (`lib.rs:45`), beside `width` (`:47`):
/// The initial view mode (All/Focus), from `[tui] view` / `--view`. `None` = the
/// TUI default (`ViewMode::Focus`, A1). The view analogue of `width` (D016): the
/// user can toggle it live (`Ctrl-O`, `/view`), but the config sets the session's
/// initial mode.
pub view: Option<ViewMode>,
```

**`ViewMode` must be RE-EXPORTED (re-export).** The CLI names it
(`wcode_tui::ViewMode::parse`, §2.8/§2.9) and `TuiSpec.view` types on it, so it
must join the crate's public surface — add it to the re-export at `lib.rs:36`
`pub use crate::app::{` (which already carries `App`, `Block`, `SessionItem`, …).
That same `pub use` also brings `ViewMode` into `lib.rs`'s own scope, so the
`pub view: Option<ViewMode>` field below needs **no** separate `use`.

In `run` (`lib.rs:388`, right where `tui.width` is applied):

```rust
    // The configured initial view mode (`[tui] view` / `--view`), the view analogue
    // of the width seed above.
    if let Some(view) = tui.view {
        app.set_view_mode(view);
    }
```

### 2.8 `crates/wcode-cli/src/config.rs` — the `[tui]` field

```rust
// `struct TuiConfig` (`config.rs:406`), beside `width` (`:409`):
/// The initial view mode: `"all"` or `"focus"`. Validated loudly (an unknown
/// token is a startup error, like an out-of-range `width`); absent = the TUI
/// default (`ViewMode::Focus`, A1).
pub view: Option<String>,
```

Resolved at `config.rs:900` (`let tui = wcode_tui::TuiSpec { … }`):

```rust
    let tui = wcode_tui::TuiSpec {
        width: file.tui.width.map(wcode_tui::validate_measure).transpose()
            .map_err(ConfigError::Tui)?,
        view: file.tui.view.as_deref()
            .map(wcode_tui::ViewMode::parse)
            .transpose()
            .map_err(ConfigError::Tui)?,
    };
```

### 2.9 `crates/wcode-cli/src/main.rs` — the flag

```rust
// in `struct Args`, beside `tui: bool` (`main.rs:197`):
/// `--view all|focus`: force the initial TUI view mode, overriding `[tui] view`.
/// Ignored by the line REPL.
view: Option<String>,
```

Parsed in the flag loop (beside `main.rs:315` `            "--tui" => a.tui = true,`),
in the repo's inline `args.get(i).ok_or(..)` style (e.g. `--model`, `main.rs:251`):

```rust
            "--view" => {
                a.view = Some(args.get(i).ok_or("--view requires all|focus")?.clone());
                i += 1;
            }
```

Folded into the resolved `TuiSpec` **after `merge` and before building `Options`**
(so BOTH `Options` sites — `main.rs:1017` and `:1860` — pick it up for free, and
`--dump-config` reflects it):

```rust
    // `--view` > `[tui] view` > the TUI default. The flag wins (the `--tui` rule).
    if let Some(token) = args.view.as_deref() {
        cfg.tui.view = Some(
            wcode_tui::ViewMode::parse(token).map_err(/* -> a loud startup error */)?,
        );
    }
```

### 2.10 `crates/wcode-tui/src/app.rs` — the `/view` command (A7)

A **required, validated argument** (`/view <all|focus>`), mirroring `/width <cols>`
(`app.rs:821`/`:4033`), failing loudly on an unknown token. Not a bare `/all`.

```rust
// in `COMMANDS` (registry head `app.rs:729`), after the `width` entry (`:821`):
Command {
    name: "view",
    aliases: &[],
    args: Some("<all|focus>"),
    summary: "set the transcript view (all = merged)",
},
```

```rust
// a new arm in `run_command` (`app.rs:4000`), beside `"width"` (`:4033`):
"view" => match arg.map(wcode_tui::ViewMode::parse) {
    Some(Ok(view)) => self.set_view_mode(view),
    Some(Err(e)) => self.transient(e),          // the loud, user-facing message
    None => self.transient("usage: /view <all|focus>"),
},
```

`Ctrl-O` (A6) toggles; `/view <mode>` sets explicitly. Both are client-local (no
`Request`).

---

## 3. The ordering — the arrival ordinal (the design's core)

The design (`vscode-ui-v2-plan.md` §4.2) makes the merge key **the order the client
saw each block** — *"a true arrival-order interleave … superseding V3's per-session
contiguous runs"*. The VS Code stamp lives **inside the reducer's append**
(`reducer.ts:535` `const stamped = added.map((block, i) => ({ ...block, seq: base + i }));`) —
the ordinal is a property of the append the reducer performs, so it belongs at the
reducer's own append site, **not** in the transport. The TUI's analogue is the
`AppEvent::Agent` arm (`app.rs:3778`), which owns the append:

```text
AppEvent::Agent(id, event) => {
    if let Some(idx) = self.surface_index(&id) {
        self.surfaces[idx].clear_hint();                    // NEW: retire the hint FIRST
        let before = self.surfaces[idx].transcript.len();   // NEW: the true pre-append len
        if self.surfaces[idx].apply(event) { self.dirty = true; }
        self.stamp_new(idx, before);                        // NEW: stamps exactly [before..]
    }
}
```

**The App-driven appends must go through ONE chokepoint (the *ordinal chokepoint*).** The receipt
arm is **not** the only place a block lands: `App` methods add blocks to the
*focused* surface directly, bypassing the arm — a submitted `Block::User`
(`submit`, `app.rs:3976`), a shown `Block::Diff` (`show_change`, `app.rs:4585`),
and `/`-command chatter via `notice` (`app.rs:4308`) and `transient`
(`app.rs:4316`, plus the "already running" transient at `:3972` — **five** sites).
Left at `push_block`'s placeholder `0`, every one of those sorts to the **front**
of the merged view. So route each through `App::push_on(idx, block)` (§2.4) —
append, then `stamp_new` — exactly as the reference stamps inside its own append
chokepoint (`reducer.ts:535`). `push_on` calls `self.surfaces[idx].push_block(block)`
(the `Surface`-level chokepoint, `app.rs:1630`) and then `stamp_new(idx, before)`,
so an App-driven append gets a real ordinal, strictly greater than the preceding turn's.

**The off-by-one both seams share — clear the hint BEFORE measuring.** `push_block`
retires the empty-state hint as its first act (`app.rs:1632`
`        self.clear_hint();` → `app.rs:1652` → `:1657`
`            self.transcript.remove(i);`). So on a fresh surface that still holds the
hint (`seed_hint_if_empty`, `app.rs:1641`, seeded every frame from `ui.rs:93`),
`before = transcript.len()` counts the hint as `1`, `push_block` drops it (len
`0`) then pushes the block (len `1`), and `stamp_new(idx, 1)` hits its own
`before >= transcript.len()` no-op — the block keeps `seq 0` and sorts to the
FRONT. **The rule (mechanism (a), both seams):** call `self.surfaces[idx].clear_hint()`
FIRST (idempotent — a no-op when there is no hint), THEN capture `before`, THEN
append, THEN `stamp_new(idx, before)`. `push_block`'s own `clear_hint` then no-ops
and the length delta is exactly the appended blocks. This fixes `push_on`
(§2.4) **and** the Agent arm above — `apply(event)` (`app.rs:3788`
`                && self.surfaces[idx].apply(event)`) appends through the same
`push_block` and has the identical off-by-one. **`apply` may append 0 or >1
blocks:** with the hint cleared first, `before` is the true pre-append length, so
`stamp_new(idx, before)` stamps exactly `[before..]` — no more, no fewer, and a
clean no-op when `apply` appended nothing (`before == transcript.len()`).

**The seed path must stamp too (blocker 4).** A replay does NOT go through the
receipt arm: `replay_history` (`lib.rs:438`) calls `App::seed_history`
(`app.rs:5043` `    pub fn seed_history(&mut self, id: &SessionId, messages: &[AgentMessage]) {`)
→ `:5045` `            && self.surfaces[idx].seed(messages)`, and `seed` (`app.rs:2188`)
`insert_block`s a replay divider at a mid-vec `start` (`app.rs:2253`). If only the
receipt arm stamped, every seeded block would sit at the default seq `0` and the
first live block would collide. So `seed_history` calls `stamp_replayed(idx)` (§2.4)
after a successful `seed`, and `replay_history` seeds **root-first** (`lib.rs:438`
iterates `backends` in list order) — every seeded ordinal < every live ordinal (A5).

**`seqs` must stay in lockstep at EVERY push — including the empty-hint path
(seed-hint lockstep).** `Surface::push_block` (`app.rs:1630`) gains `self.seqs.push(0);`
beside its three siblings (`self.transcript.push`/`block_revs.push`/`cache.push`).
`insert_block` (`:1725`) and the two remove sites (`clear_hint` `:1652`,
`retire_transient` `:1690`) insert/remove `seqs[i]` too — the same lockstep
discipline `retire_transient_keeps_the_three_vecs_in_step` (`app.rs:5417`) already
guards; extend it to four. **And `seed_hint_if_empty` (`app.rs:1641`) pushes its
`Block::Notice` straight onto `transcript`/`block_revs`/`cache` (`:1644`–`:1646`),
NOT via `push_block` — it runs every frame (`ui.rs:93`), so it needs its own
`self.seqs.push(0);` or the four vecs drift by one the first time the hint lands
(its seq stays the placeholder `0`; the hint is guarded to the all-empty case,
§6 friction 7).**

---

## 4. Focus & input under All mode (A9 — the divergence, documented)

**The reference does NOT disable the composer.** In VS Code All mode the composer
renders and submits exactly as in Focus — the only difference is **identity**: it
stays addressed to the **root** (`editors/vscode/src/surface.ts:32`
`  /** `target` is the member the composer is addressed to (null = the root). */`,
`:285` `        this.handlers.onSubmit(message.text, this.target);` — `this.target` is
`null` = root in All mode), and the header shows the root
(`editors/vscode/src/webview/view.ts:92` `  if (mode === "all") return rootMember(state);`).

**The TUI keeps input on the FOCUSED surface (A9).** The routing seam is the loop's
focused-surface resolve — `lib.rs:573` `        let idx = app.focus();` →
`        let (id, backend) = &backends[idx];`. **Resolution (pinned):** All mode is
**rendering-only**; `focus` is untouched and still names the input/command target
(`app.rs:2609` `    pub fn focus(&self) -> usize {` → `app.rs:2614`
`    pub fn focused(&self) -> &Surface {`). **Divergence (documented):** the
reference sends input to the **root**; the TUI sends it to the **focused** surface.
Concretely:

- `Enter` submits to the **focused** surface (A9; the reference's "input → root" is
  the alternative the reviewer may later choose).
- The focus keymap (`Ctrl-N`/`Shift-Tab`, `Alt-1..9`) still moves `focus`; in All
  mode it also moves the basis of the origin heads' emphasis.
- The header `.seg` (`§2.6`) shows the mode; the footer **state glyph reflects the
  ROOT** in All (blocker 2). The per-block origin head (`§2.5`) names every block's
  owner — the reader must never mistake whose turn is whose (v3 §1.1's rule: *"the
  user cannot tell whose transcript they are reading, and will attribute one
  member's words to another"*).

---

## 5. Integration points (each `file:line` + a quoted snippet)

| # | what the sketch plugs into | cite |
|---|---|---|
| 1 | The transcript loop to branch on view mode | `ui.rs:585` `    let n = app.transcript().len();` · `ui.rs:587` `        ranges.push(app.focused_mut().append_block_lines(i, width, &mut lines));` |
| 1b | The focused-indexed consumers — **Focus only** after blocker 3 | `ui.rs:596` `    app.set_block_ranges(ranges.clone());` · `ui.rs:599` `    app.sync_scroll(total, height, measure);` · `ui.rs:640` `    app.set_transcript_hit(col, start, rows_text);` |
| 2 | The focused-only transcript accessor | `app.rs:4434` `    pub fn transcript(&self) -> &[Block] {` |
| 3 | The receipt arm (the reducer's append site — stamp `seq`); clear the hint FIRST | `app.rs:3778` `            AppEvent::Agent(id, event) => {` · `app.rs:3787` `                    if let Some(idx) = self.surface_index(&id)` · `app.rs:3788` `                && self.surfaces[idx].apply(event)` |
| 3d | `push_block` clears the hint internally — the source of the off-by-one (clear before measuring) | `app.rs:1632` `        self.clear_hint();` · `app.rs:1652` `    fn clear_hint(&mut self) {` · `app.rs:1657` `            self.transcript.remove(i);` |
| 3b | The SEED path that must also stamp (blocker 4) | `app.rs:5043` `    pub fn seed_history(&mut self, id: &SessionId, messages: &[AgentMessage]) {` · `:5045` `            && self.surfaces[idx].seed(messages)` · `app.rs:2253` (the `insert_block` divider) |
| 3c | The five App-driven appends that BYPASS the arm — route via `App::push_on`, which clears the hint BEFORE measuring (ordinal chokepoint) | `app.rs:3976` `        self.focused_mut().push_block(Block::User(text.clone()));` (submit) · `app.rs:3972` `                .push_transient("a turn is already running — Esc to cancel");` (submit's rejection) · `app.rs:4585` `        self.focused_mut().push_block(Block::Diff {` (show_change) · `app.rs:4308` `        self.focused_mut().push_notice(text);` (notice) · `app.rs:4316` `        self.focused_mut().push_transient(text);` (transient) |
| 4 | The owner list (merge over all surfaces) | `app.rs:2386` `    surfaces: Vec<Surface>,` · `app.rs:2637` `    pub(crate) fn canonical_order(&self) -> Vec<usize> {` |
| 5 | The parallel-vec lockstep (add `seqs`) | `app.rs:1630` `    fn push_block(&mut self, block: Block) {` · `app.rs:1725` `    fn insert_block(&mut self, at: usize, block: Block) {` |
| 5b | The empty-hint push that BYPASSES `push_block` — needs its own `seqs.push(0)` (seed-hint lockstep) | `app.rs:1641` `    fn seed_hint_if_empty(&mut self) {` · `:1644` `            self.transcript.push(Block::Notice(EMPTY_HINT.into()));` · `:1645` `            self.block_revs.push(0);` · `:1646` `            self.cache.push(CacheEntry::never());` · caller `ui.rs:93` `    app.seed_empty_hint();` |
| 6 | The view-flag precedent (copy its shape) | `app.rs:2476` `    sidebar: bool,` · `app.rs:4970` `    fn toggle_sidebar(&mut self) {` |
| 6b | The FIVE per-surface scroll fields All must mirror as `all_*` (merged-scroll) | `app.rs:1383` `    scroll: usize,` · `app.rs:1385` `    max_scroll: usize,` · `app.rs:1388` `    viewport: usize,` · `app.rs:1389` `    last_total: usize,` · `app.rs:1390` `    last_width: usize,` · consumers `app.rs:2267` `        self.scroll = (self.scroll + lines).min(self.max_scroll);` · `app.rs:2281`–`:2283` (pin-on-growth) |
| 7 | The key handler (add `Ctrl-O`, A6) | `app.rs:3874` `            Key::Ctrl('b') => self.toggle_sidebar(),` · registry `app.rs:1026` `pub(crate) const KEYS` · Ctrl-B row `app.rs:1040` |
| 8 | The `/view` command (A7) | registry `app.rs:729` `const COMMANDS: &[Command] = &[` · `app.rs:815` `        name: "surface",` / `app.rs:821` `        name: "width",` · dispatch `app.rs:4000` `    fn run_command(&mut self, cmd: &Command, arg: Option<&str>) {` |
| 9 | The header `.seg` (A4) + the identity rule (blocker 2) | `ui.rs:789` `fn session_head_row(head: &SessionHead, width: usize) -> Line<'static> {` · `ui.rs:798` `        let mut spans = vec![Span::styled("WCODE", dim())];` · state source `app.rs:5006` `    pub fn running(&self) -> bool {` / `app.rs:4934` |
| 9b | `ViewMode` threading into `SessionHead` — the `.seg` source (head-threading / one-seg rule) | `app.rs:164` `pub struct SessionHead {` · `app.rs:184` `    pub(crate) fn of(app: &App) -> SessionHead {` · constructor `app.rs:2524` `    pub fn new() -> Self {` (the field) |
| 10 | The origin-head shape to mirror + the content column | `ui.rs:741` `fn user_head(text: &str) -> Line<'static> {` · `ui.rs:29` `const CONTENT_COL: usize = 7;` · `ui.rs:838` `fn turn_lines` (no head today) |
| 10b | The per-block cache the origin head must stay OUT of | `app.rs:1517` `        let lines = &self.cache[i].lines;` |
| 11 | `TuiSpec` (the spec field) | `lib.rs:45` `pub struct TuiSpec {` · `lib.rs:47` `    pub width: Option<usize>,` |
| 12 | The spec rides in `Options`; `run` seeds the app | `lib.rs:279` `pub struct Options {` · `lib.rs:297` `    pub tui: TuiSpec,` · `lib.rs:358` `pub async fn run(` · `lib.rs:388` `    if let Some(cols) = tui.width {` |
| 13 | `[tui]` config + resolution | `config.rs:406` `pub struct TuiConfig {` · `config.rs:900` `    let tui = wcode_tui::TuiSpec {` |
| 14 | The resolved config field | `config.rs:610` `    pub tui: wcode_tui::TuiSpec,` |
| 15 | The flag + both `Options` sites | `main.rs:197` `    tui: bool,` · `main.rs:1017`/`:1028` and `main.rs:1860`/`:1872` `tui: cfg.tui,` |

The two `Options` sites need **no** field change — `tui: cfg.tui` already carries the
whole `TuiSpec`, so folding `--view` into `cfg.tui.view` (§2.9) reaches both for
free. **No protocol change** and **no CLI socket change**: the mode is client-local,
exactly as the design states (`vscode-ui-v2-plan.md`:75).

---

## 6. Friction (where the design fights the code)

1. **The stamp has no home on `Surface`.** `push_block` (`app.rs:1630`) is a
   `Surface` method and cannot see a global counter. The stamp is lifted to `App`
   (§3) — at the `AppEvent::Agent` arm (before/`stamp_new`) **and** at the new
   `App::push_on(idx, block)` chokepoint for the App-driven appends that bypass that
   arm (`submit`/`show_change`/`notice`/`transient`, ordinal chokepoint). The block-level
   `seq` becomes an **app-level** append, and the `seqs` vec must be kept in
   lockstep by hand across `push_block`/`insert_block`/`clear_hint`/
   `retire_transient` **and** `seed_hint_if_empty` (`app.rs:1641`, which pushes
   `transcript`/`block_revs`/`cache` directly, seed-hint lockstep). **Resolution:**
   `App::push_on` (§2.4) + `App::stamp_new(idx, before)` (§2.4) + a `seqs.push(0)`
   at each of those five sites. **Order matters (the off-by-one):** both `push_on`
   and the Agent arm must call `clear_hint()` BEFORE capturing `before`, because
   `push_block` clears the hint itself (`app.rs:1632`) — see §3.
2. **Only the focused surface has a live block.** `Surface::live` is per-surface,
   but `append_live_lines` (`app.rs:1560`) is called on the focused surface only
   (`ui.rs:592`). In All mode a **member's** streaming message does not appear until
   it commits as a block (`MessageEnd`); the working-team strip
   (`app.rs:3035 working_team_rows`) is the live-activity view meanwhile.
   **Resolution (A2):** All mode merges **committed** blocks; the **focused**
   surface's live trailer rides the tail.
3. **Scroll/selection are focused-indexed; All is DISPLAY-ONLY (blocker 3 / A3).**
   `scroll` (`app.rs:4503`), `max_scroll`/`viewport`/`last_total`, `ranges`
   (`app.rs:3081` `pub fn set_block_ranges(&mut self, ranges: Vec<Range<usize>>) {`)
   and `block_at` (`app.rs:3589` `        self.focused()` →
   `.ranges.iter().position(|r| r.contains(&line))`) all read the **focused** surface. **Resolution (pinned v1):** All mode owns a dedicated
   app-level offset `all_scroll` (§2.2) applied to the **merged** line space — plus
   the **four mirror fields** `all_max_scroll`/`all_viewport`/`all_last_total`/
   `all_last_width` (the resolution of the merged-scroll item, §2.2), so `sync_scroll` keeps the
   merged view **pinned while a member appends**, exactly as `Surface::sync_scroll`
   does (`app.rs:2281`–`:2283`), not merely clamped. `scroll`/`sync_scroll` become
   view-aware, so a mode flip never leaks merged
   scroll into a surface's `scroll`/`last_total`. `set_block_ranges`/`block_at` are
   **NOT re-threaded**; **browse (`Ctrl-G`), mouse/text selection and `Ctrl-T` stay
   Focus-scoped and are NO-OPS in All.** Cross-surface selection is its own
   follow-on.
4. **Browse / `Ctrl-T` are focused-scoped.** `selected`, `toggle_all_tools`
   (def `app.rs:2671` `    fn toggle_all_tools(&mut self) {`, Ctrl-T arm `:3877`) and
   the text selection index the focused transcript. **Resolution (A3):** they are
   gated OFF in All mode (a no-op, so no cross-surface re-threading in v1).
5. **The origin head must not poison the block cache.** `append_block_lines` returns
   `self.cache[i].lines` (`app.rs:1517`), keyed only on `(rev, width)` — not on the
   view mode. **Resolution:** the origin head renders OUTSIDE the cache (§2.5);
   the cached block lines stay mode-agnostic.
6. **Replay has no arrival order, and bypasses the stamp.** `seed_history`
   (`app.rs:5043`) does not go through `AppEvent::Agent`. **Resolution (blocker 4 /
   A5):** `seed_history` calls `stamp_replayed(idx)` after `seed`, and
   `replay_history` seeds **root-first** — accepting the root-then-members grouping
   (the original interleave is not persisted); every seeded ordinal < every live one.
7. **The empty-state hint is per-surface.** `seed_empty_hint` (called from
   `ui.rs:93`) pushes a `Notice` on the **focused** surface. In All mode with idle
   members the hint should appear only when **every** surface is empty.
   **Resolution:** guard the hint on `view == All ⇒ merged_order().is_empty()`.

---

## 7. Settled amendments (A1–A9) + documented divergences

The reviewer settled every open question; the developer honors these:

- **A1 (was Q1) — default = `Focus`.** The D015 byte-identical-default rule (the
  `sidebar: bool` bit is off by default). `[tui] view="all"` / `--view all` opts
  in. **Divergence (documented):** the reference defaults to **All**
  (`webview.ts:60` `export const DEFAULT_PREFS: ViewPrefs = { mode: "all", …`);
  this divergence is recorded in the forthcoming **D031** ADR (W026 brief + D031
  are written at the record step, §Residual).
- **A2 (was Q2) — committed-only merge.** Member live streams stay in the
  working-team strip; the focused surface's live trailer rides the tail.
- **A3 (was Q3) — display-only, Focus-scoped selection** (§6 friction 3/4): a
  dedicated `all_scroll` **plus the four mirror fields** `all_max_scroll`/
  `all_viewport`/`all_last_total`/`all_last_width` (merged-scroll); `set_block_ranges`/
  `block_at` not re-threaded; browse / mouse / text selection / `Ctrl-T` are no-ops
  in All. Cross-surface selection is a follow-on.
- **A4 (was Q4) — the mode control is in the HEADER** (v3 §1.1 `:149`), a `.seg`
  beside `WCODE` in `session_head_row` (`ui.rs:789`); the composer-foot option is
  dropped. **Mechanism (settled, head-threading):** the mode rides `SessionHead` as a
  new `view: ViewMode` field (`app.rs:164`), set in `SessionHead::of` (`app.rs:184`)
  and read by `session_head_row`; the seg is **All-only** (one-seg rule, §2.6).
- **A5 (was Q5) — replay = root-then-members, AFTER the seed stamp** (blocker 4 /
  §6 friction 6).
- **A6 (was Q6) — `Ctrl-O`** (verified free — no `Key::Ctrl('o')` in `app.rs`).
  Its `KEYS` row lands after the `Ctrl-B` row (`app.rs:1040`).
- **A7 (was Q7) — `/view <all|focus>`**, a required, validated argument mirroring
  `/width <cols>` (`app.rs:821`/`:4033`) and failing loudly on an unknown token;
  `Ctrl-O` is the toggle. Not a bare `/all`.
- **A8 (was Q8) — label EVERY committed block, root included.** **Divergence
  (documented):** the design labels per-**turn** `.who` lines
  (`vscode-ui-v3-plan.md:122`); the TUI attaches an origin head per **block**, which
  is coarser (a `Tool`/`Notice` block carries its surface's label too).
- **A9 (was Q9) — `Enter` targets the FOCUSED surface** (`lib.rs:573
  let idx = app.focus();`); All is rendering-only. **Divergence (documented):** the
  reference keeps the composer addressed to the **root** (`surface.ts:32`/`:285`,
  `view.ts:92`).

**Residual (non-blocking, carried).** The **W026** brief
(`docs/work/W026-*.md`) and the **D031** ADR (`docs/decisions/D031-*.md`) are being
written **at the record step**, alongside this sketch; the A1 divergence (default
`Focus` vs the reference's `All`, `webview.ts:60`) is recorded in D031. This sketch
is the doc of implementation detail and is reconciled to W026. Socket-client parity
is untouched by design but unexercised.

---

## 8. Test skeletons (`todo!()` bodies; behaviour-named)

### `app.rs`
```rust
#[test] fn all_mode_is_off_by_default_and_the_frame_is_unchanged() { todo!() }
#[test] fn toggle_view_flips_focus_and_all_and_marks_dirty() { todo!() }
#[test] fn view_mode_parse_accepts_all_and_focus_case_insensitively() { todo!() }
#[test] fn view_mode_parse_rejects_an_unknown_token_loudly() { todo!() }
#[test] fn view_mode_label_is_some_all_only_and_none_in_focus() { todo!() }
//   one-seg rule: `label()` = Some("ALL") | None → the ONE seg rule; no dead "FOCUS".
#[test] fn a_block_is_stamped_with_the_next_arrival_ordinal_on_append() { todo!() }
//   two events → seqs [0, 1]; the counter advances past the last block.
#[test] fn a_locally_submitted_user_block_is_stamped_after_the_preceding_turn() { todo!() }
//   ordinal chokepoint: `submit` appends a `Block::User` OUTSIDE the receipt arm → it must
//   still get an ordinal STRICTLY GREATER than the preceding turn's, not the
//   placeholder 0 that would sort it to the front. Same for `show_change`/`notice`/
//   `transient` (all via `App::push_on`). NOTE: the preceding turn already cleared
//   the hint, so this test does NOT exercise the off-by-one below.
#[test] fn a_locally_submitted_block_on_a_hint_present_surface_is_stamped() { todo!() }
//   the off-by-one the test above cannot catch: the surface still holds the D5 empty
//   hint (`seed_hint_if_empty`, app.rs:1641; `push_block` clears it internally,
//   app.rs:1632). With a member block already at ordinal N, submit on this
//   hint-present surface → `App::push_on` must clear the hint FIRST, so the new
//   `Block::User`'s ordinal is strictly GREATER than N (a naive `before = len()`
//   would count the hint, no-op `stamp_new`, and leave the block at placeholder 0).
#[test] fn the_agent_arm_stamps_after_clearing_a_present_hint() { todo!() }
//   the same off-by-one at the Agent seam: `apply(event)` (app.rs:3788) appends via
//   `push_block`; on a hint-present surface an incoming event must still ordinal the
//   appended block(s); an event that appends none is a clean no-op.
#[test] fn merged_order_interleaves_surfaces_by_arrival_ordinal() { todo!() }
//   root block (seq 0), member block (seq 1), root block (seq 2) → that order.
#[test] fn merged_order_breaks_seq_ties_by_canonical_surface_then_index() { todo!() }
#[test] fn merged_order_spans_every_surface_root_included() { todo!() }
#[test] fn seqs_stay_in_lockstep_across_push_insert_and_retire() { todo!() }
//   seed-hint lockstep: ALSO cover `seed_hint_if_empty` (app.rs:1641) — seed an empty
//   surface's hint, then push a real block → the four vecs stay index-aligned.
#[test] fn append_block_lines_at_renders_an_arbitrary_surface_with_an_origin_head() { todo!() }
//   separator ownership: a surface's OWN first block (bi == 0) rendered after another
//   surface's lines must still carry its leading blank separator (keyed on `out`,
//   NOT on the block index) — `append_block_lines` would suppress it.
#[test] fn append_block_lines_at_is_a_noop_for_an_out_of_range_index() { todo!() }
#[test] fn the_origin_head_is_not_baked_into_the_block_cache() { todo!() }
//   render All then Focus at the same (rev,width): Focus shows no head.
#[test] fn seeded_blocks_are_stamped_and_the_first_live_block_does_not_collide() { todo!() }
//   seed two surfaces (root-first), then a live block → strictly greater ordinals.
#[test] fn set_view_mode_from_the_spec_seeds_the_initial_mode() { todo!() }
#[test] fn all_mode_does_not_disable_input_and_enter_targets_the_focused_surface() { todo!() }
#[test] fn chrome_surface_is_the_root_in_all_and_the_focused_in_focus() { todo!() }
#[test] fn running_and_run_elapsed_reflect_the_root_in_all_mode() { todo!() }
#[test] fn session_head_view_is_the_app_mode() { todo!() }
//   head-threading: `SessionHead::of` snapshots `app.view_mode()` into `head.view`.
#[test] fn browse_and_toggle_all_tools_are_noops_in_all_mode() { todo!() }
#[test] fn the_all_scroll_offset_does_not_leak_into_a_surface_scroll_on_a_flip() { todo!() }
#[test] fn all_scroll_pins_on_growth_via_the_all_mirror_fields() { todo!() }
//   merged-scroll: append to a member while scrolled up in All → the merged view stays
//   pinned (`all_last_total`/`all_last_width`), then clamps at the top.
#[test] fn run_command_view_sets_the_mode_and_rejects_an_unknown_token() { todo!() }
```

### `ui.rs`
```rust
#[test] fn focus_mode_renders_the_focused_transcript_byte_identically() { todo!() }
#[test] fn all_mode_interleaves_members_with_origin_heads() { todo!() }
#[test] fn the_origin_head_names_the_owning_surface() { todo!() }
#[test] fn the_header_seg_shows_all_only_in_all_mode() { todo!() }
#[test] fn the_header_is_byte_identical_in_focus_mode() { todo!() }
```

### `lib.rs` / `config.rs` / `main.rs`
```rust
#[test] fn tui_spec_view_seeds_the_app_mode_at_startup() { todo!() }
#[test] fn tui_view_parses_all_and_focus() { todo!() }
#[test] fn an_absent_tui_view_leaves_the_default_focus() { todo!() }
#[test] fn an_unknown_tui_view_is_a_loud_config_error() { todo!() }
#[test] fn the_view_flag_overrides_the_config_value() { todo!() }
#[test] fn the_view_flag_reaches_both_options_sites() { todo!() }
```

---

## 9. What this sketch did NOT do (honest)

- **This revision is the first-layer-review amendment pass** (still review-only):
  the blockers — the App-level arrival-ordinal chokepoint (§3, §2.4 `push_on`), the
  `seed_hint_if_empty` `seqs` lockstep (§3, §6), the five-field All scroll mirror
  (§2.2, §2.4 `scroll`/`sync_scroll`), `SessionHead.view` threading (§2.6) and the
  ONE seg rule (§2.1/§2.6) — are folded in **in place**. Still **no `.rs` file is
  touched**. The five items are tagged **by name** in the body — *ordinal
  chokepoint*, *seed-hint lockstep*, *merged-scroll*, *head-threading*, *one-seg
  rule* — so they never collide with the sketch's earlier `(blocker N)` numbering,
  which the rest of the doc still uses.
- **Round-3 fix (the off-by-one the amendments introduced):** `push_block` clears
  the D5 empty-state hint as its first act (`app.rs:1632`), so a `before = len()`
  captured after it lands one PAST the append on a hint-present surface
  (`seed_hint_if_empty`, `app.rs:1641`) and `stamp_new` silently no-ops. **Both
  seams** — `App::push_on` and the `AppEvent::Agent` arm — now clear the hint
  FIRST (mechanism (a), §3): idempotent, so `before` is the true pre-append length
  and `stamp_new(idx, before)` stamps exactly the appended tail (0 ⇒ no-op). Two
  new tests (§8) cover it; the weak test is kept and annotated.
- **No `.rs` file was changed** — this file only. `cargo check`/`test`/`clippy` were
  **not** run (the sketcher writes no working logic; there is nothing to compile).
  Every `file:line` is against `HEAD` (`991285e`) and stays valid because no `.rs`
  file moved.
- **No terminal was run.** The frame (§0) is hand-authored from the design.
- **The display-only All model (A3, §6 friction 3) is reasoned, not measured** — the
  `all_scroll` + the four `all_*` mirror fields (`all_max_scroll`/`all_viewport`/
  `all_last_total`/`all_last_width`) windowing and the `chrome_surface` routing must
  agree with the renderer's `total - scroll` window exactly, or the merged view
  scrolls wrong. Full cross-surface selection is explicitly out of scope (its own
  follow-on).
- **The socket-client path is untouched by design** (client-local merge, no protocol
  change) — I did **not** verify a remote roster behaves identically to the local
  one; both entry points pass `cfg.tui` (`main.rs:1017`/`:1860`), so the spec change
  reaches them, but this is unexercised.
- **All open questions are settled (A1–A9)** — the two divergences from the VS Code
  reference (default mode; input target) are documented in §7, not hidden.
