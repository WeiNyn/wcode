# W014 slice d — sketch blocks (parked, review-only)

- **Sketched by:** `agent:sketcher`, 2026-10-10.
- **Task:** W014 slice d — in-flight thinking is ONE annotation row, not an expanded stream.
- **Locked design:** `docs/decisions/D018-tui-live-thinking-annotation.md` §1; `docs/tui-design.md` §2 glyph table **:60-61** (`| thinking (in-flight annotation) | `···` | dim row `··· thinking <t>` + live cursor `▌` (D018) |` / `| thinking (collapsed row) | `··· thinking` | dim |`) and §5 **Thinking :247-263**.
- **Tree the anchors were taken from:** `12065f8 tui: retire slash-command notices at the next submit (W014c)` — i.e. AFTER slices b and c shipped, with **zero** `SKETCH` markers in the tree (`grep -rc SKETCH crates/wcode-tui/src/app.rs crates/wcode-tui/src/ui.rs crates/wcode-cli/src/main.rs` → `0 0 0`).
- **Why parked, not in place:** slice c's second-layer review is reading the working tree concurrently, so this file is the only artefact of this session — the tree stays byte-stable (mirrors `docs/work/W014c-sketch-blocks.md`).
- **Form:** **B — comment blocks.** Every block below is a pure INSERTION at the anchor named in its heading. No real code line is modified, added to a `mod`, or compiled.
- **How to use:** for each block, insert its fenced text verbatim at the named file + anchor (the real line it goes immediately before), replace the `todo!()` bodies and the `// shape, not working code` sketches with real code, then **delete the `==== SKETCH` / `==== /SKETCH` markers**. Zero markers left is a second-layer precondition.
- **Anchor discipline (D007):** the numbers below locate, the quoted snippets pin. Re-read the snippet before inserting — a slice-d implementer's earlier edit can move a line.
- **A sketch may not compile — that is the point.** No build, test, or clippy was run.

## Anchor index — every integration point this slice touches

| anchor | snippet (verbatim) | role in slice d |
|---|---|---|
| `crates/wcode-tui/src/ui.rs:918` | `pub(crate) fn live_lines(message: &AgentMessage, width: usize) -> Vec<Line<'static>> {` | gains the `elapsed` parameter |
| `crates/wcode-tui/src/ui.rs:920` | `AgentMessage::Assistant { content, .. } => content_lines(content, width, true, false),` | passes it through |
| `crates/wcode-tui/src/ui.rs:929` | `fn content_lines(` | gains the `elapsed` parameter |
| `crates/wcode-tui/src/ui.rs:946-949` | `ContentBlock::Thinking { text } => {` / `if live {` / `// In-flight: stream inline, expanded — as before D33.` / `lines.extend(wrap(text, width, THINK_FIRST, THINK_CONT, thinking()));` | **the live arm that becomes one row** |
| `crates/wcode-tui/src/ui.rs:27` | `const THINK_FIRST: &str = "   ··· ";` | the gutter the row reuses |
| `crates/wcode-tui/src/ui.rs:969-971` | `if live {` / `match lines.last_mut() {` / `Some(last) => last.spans.push(Span::styled("▌", accent())),` | supplies the `▌` — **unchanged** |
| `crates/wcode-tui/src/ui.rs:1071` | `fn thinking_block_lines(` | the committed twin — **unchanged**; the new helper goes above it |
| `crates/wcode-tui/src/ui.rs:1089` / `:1095-1100` | `fn thinking_header_line(` / `if !affordance {` `return Line::from(vec![Span::styled(THINK_FIRST.to_string(), dim()),` `Span::styled(head, dim())]);` | the head the row reuses (no-affordance path) |
| `crates/wcode-tui/src/ui.rs:1442` | `fn wrap(text: &str, width: usize, first: &str, cont: &str, style: Style) -> Vec<Line<'static>> {` | the call the live arm GIVES UP |
| `crates/wcode-tui/src/ui.rs:1748` / `:1754-1763` | `fn corner_titles(app: &App, area: Rect) -> (Line<'static>, Line<'static>) {` / `let (state, state_style) = match (app.running(), app.run_elapsed()) {` … `format!("⠹ running {}", format_ms(d.as_millis() as u64)),` | **the chip word** |
| `crates/wcode-tui/src/ui.rs:1659` | `let (bl, br) = corner_titles(app, area);` | the one caller (inside `draw_input_box`) |
| `crates/wcode-tui/src/ui.rs:1912` | `fn format_ms(ms: u64) -> String {` | the elapsed formatter — reused verbatim |
| `crates/wcode-tui/src/ui.rs:486-489` | `// The in-flight message trails the committed transcript (it is transient, so` / `// it is never a selection target). …` / `let _ = app.focused_mut().append_live_lines(width, &mut lines);` | why the live row needs no affordance work |
| `crates/wcode-tui/src/ui.rs:537-543` + `:601` | `// D32 — publish the affordance cells. …` / `let affordances: Vec<AffordanceHit> = ranges` … `_ => None,` | the affordance walk — **unchanged** |
| `crates/wcode-tui/src/app.rs:1165` | `live: Option<AgentMessage>,` | where the predicate reads from |
| `crates/wcode-tui/src/app.rs:1234` / `:1238` | `live_cache: CacheEntry,` / `run_elapsed: Option<std::time::Duration>,` | cache + the injected clock |
| `crates/wcode-tui/src/app.rs:1348` | `fn bump_live(&mut self) {` | the invalidation R1 calls |
| `crates/wcode-tui/src/app.rs:1359-1366` | `pub(crate) fn append_live_lines(` … `let message = self.live.as_ref()?;` / `let stale = self.live_cache.rev != self.live_rev \|\| self.live_cache.width != width;` / `let rendered = stale.then(\|\| crate::ui::live_lines(message, width));` | the ONE caller of `ui::live_lines`; the passthrough |
| `crates/wcode-tui/src/app.rs:4044` | `pub fn live(&self) -> Option<&AgentMessage> {` | the accessor it sits beside |
| `crates/wcode-tui/src/app.rs:4481` | `pub fn run_elapsed(&self) -> Option<std::time::Duration> {` | the focused clock the chip already reads |
| `crates/wcode-tui/src/app.rs:4488-4493` | `pub fn set_run_elapsed(&mut self, id: &SessionId, elapsed: std::time::Duration) {` / `if let Some(i) = self.surface_index(id) {` / `self.surfaces[i].run_elapsed = Some(elapsed);` / `self.dirty = true;` | **R1 — the guarded `bump_live`** |
| `crates/wcode-tui/src/app.rs:1562-1572` | `AgentEvent::MessageStart { message } => {` … `self.bump_live();` / `AgentEvent::MessageUpdate { message } => {` … `self.bump_live();` | deltas already invalidate — R1 adds only the clock's path |
| `crates/wcode-tui/src/app.rs:1663-1664` | `AgentEvent::AgentStart => {` / `self.running = true;` | what the chip matches on (a test fixture needs it) |
| `crates/wcode-tui/src/lib.rs:274` / `:565` | `const TICK: Duration = Duration::from_millis(120);` / `app.set_run_elapsed(id, started.elapsed());` | the injected clock |
| `crates/wcode-harness/src/loop_.rs:247` / `:259` / `:866-873` | `TextDelta(delta)` / `ThinkingDelta(delta)` / `fn append_thinking(content: &mut Vec<ContentBlock>, delta: &str) { match content.last_mut() { Some(ContentBlock::Thinking { text }) => text.push_str(delta), _ => content.push(ContentBlock::Thinking { text: delta.to_string() }), } }` | the seam that makes `content.last()` the right predicate |
| `crates/wcode-tui/src/ui.rs:5099-5137` | `fn thinking_auto_collapses_on_turn_end() {` … `assert!(live.contains("visible only while the turn streams"), "in-flight thinking streams inline:\n{live}");` / `assert!(!live.contains("··· thinking"), "no collapsed row while in-flight:\n{live}");` | **the test that pins TODAY'S behaviour — must be rewritten** |
| `crates/wcode-tui/src/ui.rs:5139-5140` | `#[test]` / `fn a_rendered_thinking_row_publishes_a_clickable_toggle() {` | the committed-row affordance test — must stay green |
| `crates/wcode-tui/src/app.rs:8493-8501` | `fn a_frame_with_no_delta_does_zero_live_renders() {` … `assert_eq!(r2, r1, "a second frame with no delta renders ZERO times");` | the invariant R1 must not forfeit |

---

## Block 1 — `crates/wcode-tui/src/ui.rs` (insert immediately before `fn content_lines(` at :929)

Four edits, all named inside the block: the `live_lines` signature, the `content_lines` signature, the live arm (:946-949), and the new helper above `thinking_block_lines` (:1071).

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice d — in-flight thinking is ONE annotation row (D018 §1). Locked
    // design: `docs/decisions/D018-tui-live-thinking-annotation.md` §1 and
    // `docs/tui-design.md` §2 glyph table (:60 `| thinking (in-flight annotation)
    // | `···` | dim row `··· thinking <t>` + live cursor `▌` (D018) |`) + §5
    // "Thinking" (:247-263, `   ··· thinking 1.2s▌` … "**No reasoning body, no
    // affordance** — the live block is never a selection target").
    // The elapsed is a PARAMETER (the renderer stays pure — D018: "the renderer
    // stays pure (the elapsed is a parameter, like `width`)"), never a clock read.
    //
    // (a) ui.rs:918 — `live_lines` gains the elapsed (its ONE caller passes it,
    //     app.rs:1366):
    //     pub(crate) fn live_lines(
    //         message: &AgentMessage,
    //         width: usize,
    //         elapsed: Option<std::time::Duration>,
    //     ) -> Vec<Line<'static>> {
    //         match message {
    //             AgentMessage::Assistant { content, .. } => {
    //                 content_lines(content, width, true, false, elapsed)
    //             }
    //             _ => Vec::new(),
    //         }
    //     }
    //
    // (b) ui.rs:929 — `content_lines` gains the same final parameter:
    //     fn content_lines(
    //         content: &[ContentBlock],
    //         width: usize,
    //         live: bool,
    //         thinking_open: bool,
    //         elapsed: Option<std::time::Duration>,
    //     ) -> Vec<Line<'static>> {
    //     TWO test call sites pass four args and must gain the fifth —
    //     ui.rs:4709 and ui.rs:4731 (both `content_lines(…, 40, false, false)`,
    //     the `¹` superscript test). `None` there: neither renders a live row.
    //
    // (c) ui.rs:946-949 — the live arm. TODAY:
    //         ContentBlock::Thinking { text } => {
    //             if live {
    //                 // In-flight: stream inline, expanded — as before D33.
    //                 lines.extend(wrap(text, width, THINK_FIRST, THINK_CONT, thinking()));
    //             } else {
    //                 lines.extend(thinking_block_lines(text, width, thinking_open, i == 0));
    //             }
    //         }
    //     AFTER (shape, not working code — `text` stays bound for the else arm):
    //         ContentBlock::Thinking { text } => {
    //             if live {
    //                 // D018 §1: ONE dim annotation row — no body, no affordance.
    //                 lines.push(live_thinking_line(width, elapsed));
    //             } else {
    //                 lines.extend(thinking_block_lines(text, width, thinking_open, i == 0));
    //             }
    //         }
    //
    // (d) THE NEW HELPER — inserted just above `thinking_block_lines` (ui.rs:1071)
    //     so the two thinking renderers sit together:
    //
    //     /// The IN-FLIGHT thinking row (D018 §1): `   ··· thinking 1.2s` — the
    //     /// committed collapsed header's head (same `THINK_FIRST` gutter at
    //     /// ui.rs:27, same word, same `dim()` role) plus the live elapsed.
    //     /// Reuses `thinking_header_line(width, false, false)` (ui.rs:1089), whose
    //     /// no-affordance path is ui.rs:1095-1100 (`if !affordance { return
    //     /// Line::from(vec![Span::styled(THINK_FIRST.to_string(), dim()),
    //     /// Span::styled(head, dim())]); }`), so with `elapsed == None` the row is
    //     /// BYTE-IDENTICAL to the committed collapsed header — one rule, two lives.
    //     ///
    //     /// NO body: the reasoning text is reachable only after the turn commits
    //     /// (D018 §1, `docs/tui-design.md`:262-263 "the reasoning text is reachable
    //     /// only after the turn commits"). NO `▸`/`▣` affordance: the live block is
    //     /// never a selection target (ui.rs:486-489 "The in-flight message trails
    //     /// the committed transcript (it is transient, so it is never a selection
    //     /// target)").
    //     ///
    //     /// The trailing live cursor `▌` is NOT drawn here — `content_lines`'s
    //     /// existing tail appends it to the LAST line (ui.rs:969-971 `Some(last) =>
    //     /// last.spans.push(Span::styled("▌", accent()))`), which is this row
    //     /// whenever the model is still reasoning.
    //     ///
    //     /// `elapsed == None` prints NO timer (the run has not ticked yet); the
    //     /// caller injects it, never read here.
    //     ///
    //     /// `width` is threaded for symmetry with its sibling and is UNUSED on this
    //     /// path (no affordance geometry, no wrapping) — a deliberate no-op, not an
    //     /// oversight.
    //     fn live_thinking_line(
    //         width: usize,
    //         elapsed: Option<std::time::Duration>,
    //     ) -> Line<'static> {
    //         todo!()
    //     }
    //
    // ONE ROW PER STRETCH. The kernel opens a FRESH trailing `Thinking` block after
    // any text/tool block (`loop_.rs:866-873` `fn append_thinking` — `match
    // content.last_mut() { Some(ContentBlock::Thinking { text }) => text.push_str(delta),
    // _ => content.push(ContentBlock::Thinking { text: delta.to_string() }) }`), so a
    // live message can be `[Thinking, Text, Thinking]`. This arm is PER BLOCK and the
    // design drops the body everywhere in flight, so each reasoning stretch draws ONE
    // row and NO body; an earlier stretch's text stays hidden until commit lands it in
    // the unchanged collapsed row. Say so in the commit — it is the deliberate reading
    // of "no reasoning body in flight", not an accident.
    //
    // WHAT IS **NOT** CHANGED (D018 §1 "Committed thinking is unchanged"): the
    // `else` arm and both committed renderers — `thinking_block_lines` (ui.rs:1071)
    // and `thinking_header_line` (ui.rs:1089) — so a committed block still draws
    // `··· thinking` with `▸`/`▾`/`▣` and expands with `Enter`. `wrap`'s call with
    // `THINK_FIRST`/`thinking()` (ui.rs:949) disappears from the LIVE path; the
    // helper `wrap` itself (:1442) is untouched and still serves the committed
    // expanded body (:1079) and `Notice`/`Btw` (ui.rs:697 / :699).
    // ==== /SKETCH ====
```

---

## Block 2 — `crates/wcode-tui/src/ui.rs` (insert immediately before `let (state, state_style) = match` at :1754, inside `corner_titles`)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice d — the composer-foot chip says *thinking* while the model
    // reasons. Locked design: D018 §1 ("The composer-foot state chip reads
    // `⠹ thinking <t>` instead of `⠹ running <t>` while the live message's last
    // content block is `Thinking`; it reverts at the first text/tool delta. A
    // **word change, not a new glyph** — `⠹` is already the running glyph") and
    // `docs/tui-design.md`:325 (`… ⏸ idle`/`⠹ running 3.1s`/`⠹ thinking 3.1s`/`⠹ btw…` …)
    // + :255-258.
    //
    // THE SITE — `corner_titles` (ui.rs:1748) already takes `&App`, so nothing is
    // threaded and no signature changes. TODAY (ui.rs:1754-1763):
    //     let (state, state_style) = match (app.running(), app.run_elapsed()) {
    //         (true, Some(d)) => (
    //             format!("⠹ running {}", format_ms(d.as_millis() as u64)),
    //             accent(),
    //         ),
    //         (true, None) => ("⠹ running".to_string(), accent()),
    //         // A pending `/btw` is the other in-flight state — accent, like running.
    //         (false, _) if app.asking() => ("⠹ btw…".to_string(), accent()),
    //         (false, _) => ("⏸ idle".to_string(), dim()),
    //     };
    // AFTER (shape, not working code) — ONE word chosen once; the `btw…`/`idle`
    // arms are byte-identical:
    //     // D018: while the live message's last block is Thinking the chip says
    //     // `thinking`; it reverts at the first text/tool delta by itself.
    //     let word = if app.live_thinking() { "thinking" } else { "running" };
    //     let (state, state_style) = match (app.running(), app.run_elapsed()) {
    //         (true, Some(d)) => (
    //             format!("⠹ {word} {}", format_ms(d.as_millis() as u64)),
    //             accent(),
    //         ),
    //         (true, None) => (format!("⠹ {word}"), accent()),
    //         (false, _) if app.asking() => ("⠹ btw…".to_string(), accent()),
    //         (false, _) => ("⏸ idle".to_string(), dim()),
    //     };
    //
    // NO NEW GLYPH, NO NEW FRAME, NO LADDER CHANGE. `⠹` is already the running
    // glyph and `accent()` already the running style, so the width budget is
    // unchanged in kind: `corner_titles` budgets the pair to `width - 1` and
    // `bottom_levels` (ui.rs:1708 "… [`corner_titles`] has already budgeted the
    // pair to `width - 1` …") drops whole tokens, never characters — "thinking"
    // (8) is ONE column wider than "running" (7), so at the tightest widths the
    // foot clips one column sooner; it never re-lays-out. The one-ladder rung and
    // its order (`[↑ N]` → gauge → `⏻ plan`/`▤ browse`) are untouched.
    // `format_ms` (ui.rs:1912) is reused verbatim: no new clock, no new format
    // (it already renders `1.2s` / `12ms` / `1m02s`).
    //
    // WHY THE ACCESSOR IS FOCUSED-SCOPED: `corner_titles` is drawn for the focused
    // surface only (`draw_input_box` at ui.rs:1659, called with the bands' `editor`
    // rect), and both things it matches on are already focused reads —
    // `app.running()` and `app.run_elapsed()` (app.rs:4481
    // `pub fn run_elapsed(&self) -> Option<std::time::Duration> { self.focused().run_elapsed }`).
    // `App::live_thinking()` must therefore delegate to the focused surface, or
    // the chip could read one surface's reasoning while showing another's clock.
    //
    // THE `(true, None)` ARM MATTERS FOR THE TEST: `run_elapsed` is `None` until
    // the first 120 ms tick (lib.rs:274/565), so the chip legitimately reads
    // `⠹ thinking` with no timer before that. A test that wants `⠹ thinking 1.2s`
    // must inject the clock (`app.set_run_elapsed(&root(), Duration::from_millis(1200))`).
    // ==== /SKETCH ====
```

---

## Block 3 — `crates/wcode-tui/src/app.rs` (insert immediately before the `append_live_lines` doc comment at :1352)

Four edits, each with its own named anchor: the `Surface` predicate, the `App` accessor (beside `live()` at :4044), the elapsed passthrough (:1364-1366), and the R1 `bump_live` guard in `set_run_elapsed` (:4488-4493).

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice d — the model seam and the ONE cache-invalidation fix (R1).
    // Locked design: D018 §1 ("'Thinking is active' needs no kernel change: the
    // kernel already separates the deltas … so `content.last()` is `Thinking`
    // exactly while the model reasons") and its R1 note ("`set_run_elapsed` must
    // bump the surface's `live_rev`, or the cached live render … freezes the timer
    // between deltas").
    //
    // (a) THE PREDICATE — on `Surface`, where `live` already lives (app.rs:1165
    //     `live: Option<AgentMessage>,`). NO new field: it reads what the reducer
    //     stores. This is the ONLY place the answer is computed; the renderer needs
    //     no accessor (it walks `content` itself, Block 1's arm).
    //
    //     /// True while the model is reasoning: the live (in-flight) assistant
    //     /// message's LAST content block is `Thinking`. The kernel opens a FRESH
    //     /// trailing `Thinking` block after any text/tool block
    //     /// (`loop_.rs:866-873` `fn append_thinking`), so `content.last()` is
    //     /// `Thinking` exactly while reasoning and flips to `Text`/`ToolCall` at
    //     /// the first text/tool delta — the "reverts" D018 §1 promises.
    //     ///
    //     /// `false` when there is no live message (idle, between `MessageEnd` and
    //     /// the next `MessageStart`), and `false` for a live `User`/`ToolResult`
    //     /// message (only an assistant message is ever live — app.rs:1564).
    //     fn live_is_thinking(&self) -> bool {
    //         todo!()
    //     }
    //
    //     WHERE IT READS: `self.live`, the same `Option<AgentMessage>` the renderer
    //     consumes at app.rs:1364 (`let message = self.live.as_ref()?;`), and the
    //     same field `App::live()` exposes (app.rs:4044). Nothing else. Do NOT add
    //     an `AgentEvent` or a kernel flag: the seam already exists and is the
    //     reason the design says "no kernel change".
    //
    // (b) THE APP-LEVEL READ — inserted beside `live()` (app.rs:4044
    //     `pub fn live(&self) -> Option<&AgentMessage> { self.focused().live.as_ref() }`):
    //
    //     /// The focused surface's [`Surface::live_is_thinking`] — the composer
    //     /// chip's read (`corner_titles`, ui.rs:1748). Focused-scoped so it agrees
    //     /// with `running()`/`run_elapsed()`, which the chip matches on.
    //     pub(crate) fn live_thinking(&self) -> bool {
    //         self.focused().live_is_thinking()
    //     }
    //
    // (c) THE PASSTHROUGH — `append_live_lines` (app.rs:1359) is the ONE caller of
    //     `ui::live_lines`. TODAY (app.rs:1364-1366):
    //         let message = self.live.as_ref()?;
    //         let stale = self.live_cache.rev != self.live_rev || self.live_cache.width != width;
    //         let rendered = stale.then(|| crate::ui::live_lines(message, width));
    //     AFTER (shape) — copy the `Copy` field to a local FIRST, so the immutable
    //     borrow of `self.live` (held by `message`, which the closure captures) and
    //     the later `self.live_cache = CacheEntry { … }` write never overlap:
    //         let elapsed = self.run_elapsed;   // Copy; borrows nothing
    //         let message = self.live.as_ref()?;
    //         let stale = self.live_cache.rev != self.live_rev || self.live_cache.width != width;
    //         let rendered = stale.then(|| crate::ui::live_lines(message, width, elapsed));
    //
    // (d) R1 — `set_run_elapsed` (app.rs:4488) now MOVES a number that is part of the
    //     CACHED live render, so it must invalidate that cache. TODAY:
    //         pub fn set_run_elapsed(&mut self, id: &SessionId, elapsed: std::time::Duration) {
    //             if let Some(i) = self.surface_index(id) {
    //                 self.surfaces[i].run_elapsed = Some(elapsed);
    //                 self.dirty = true;
    //             }
    //         }
    //     AFTER (shape) — ONE guarded bump, inside the existing `if let Some(i)`:
    //             self.surfaces[i].run_elapsed = Some(elapsed);
    //             // D018 R1: the timer is PART of the cached live row, so a tick that
    //             // moves it must re-render — but only while a thinking row is on
    //             // screen (see the guard note below).
    //             if self.surfaces[i].live_is_thinking() {
    //                 self.surfaces[i].bump_live();
    //             }
    //             self.dirty = true;
    //     `bump_live` (app.rs:1348 `fn bump_live(&mut self) { self.live_rev =
    //     self.live_rev.wrapping_add(1); }`) is already a `Surface` method and
    //     `set_run_elapsed` is an `App` method in the same module, so no visibility
    //     change is needed.
    //
    //     **THE GUARD IS THE POINT.** An UNGUARDED `bump_live()` would re-render the
    //     live block on EVERY tick — every 120 ms (lib.rs:274 `const TICK: Duration
    //     = Duration::from_millis(120);`, injected at lib.rs:565
    //     `app.set_run_elapsed(id, started.elapsed());`) — even when the live message
    //     is prose-only and no timer is on screen. That would forfeit the existing
    //     invariant `app.rs:8493` `a_frame_with_no_delta_does_zero_live_renders`
    //     ("a second frame with no delta renders ZERO times") for no visible gain.
    //     Guarded, a tick re-renders ONLY while a thinking row is actually drawn —
    //     which is exactly when the timer text changed.
    //
    //     The CHIP needs no invalidation: it is rebuilt from scratch every frame
    //     (`corner_titles` reads `app.run_elapsed()` live, no cache), so only the
    //     cached TRANSCRIPT row needs the bump.
    //
    // NOT NEEDED: `live_rev` is already bumped at every `MessageStart`/`MessageUpdate`
    // (app.rs:1562-1572 `self.bump_live();`), so deltas keep re-rendering exactly as
    // today; R1 adds the clock's path only. A `set_run_elapsed` for a NON-focused
    // surface is harmless — its cache is simply invalidated, and only the focused
    // surface's row is ever drawn (ui.rs:489).
    // ==== /SKETCH ====
```

---

## Block 4 — the affordance / blocks-line twin: **NOT AFFECTED** (no block, no edit)

Deliberately written as prose, not a block: there is nothing to insert, and a block here would leave a stray marker for no code.

- **The live block publishes no affordance, and needs none.** The D32 affordance walk iterates `ranges` — the ranges of COMMITTED blocks only (`ui.rs:483-485` `for i in 0..n { ranges.push(app.focused_mut().append_block_lines(i, width, &mut lines)); }`) — while the live message is appended separately and its range is DROPPED (`ui.rs:486-489` "The in-flight message trails the committed transcript (it is transient, so it is never a selection target). … `let _ = app.focused_mut().append_live_lines(width, &mut lines);`"). So `&app.transcript()[i]` in the walk (ui.rs:548) can never see the live message, and its `_ => None` wildcard (ui.rs:602) already returns nothing for a non-`Turn`/non-`Tool` block. **No change.** D018's "no affordance" is satisfied structurally, and slice d makes the live row *less* affordance-like than today's expanded body, never more.
- **The "blocks line" (`block_lines`, ui.rs:681) is untouched.** It renders COMMITTED blocks only; the live path is `live_lines`/`content_lines` (Block 1). Slice c's `Block::Transient` arm there is unrelated.
- **The committed thinking renderers are untouched** (`thinking_block_lines` ui.rs:1071, `thinking_header_line` ui.rs:1089) — D018 §1 "Committed thinking is unchanged". The new live helper is a SIBLING that reuses `thinking_header_line`'s no-affordance path (ui.rs:1095-1100), so the two lifetimes share one head by construction rather than by copy.
- **Regression that must stay green:** `ui.rs:5140` `a_rendered_thinking_row_publishes_a_clickable_toggle` (the committed row still publishes exactly one affordance record and expands on click).

---

## Block 5 — tests: two `#[cfg(test)]` skeletons (insert immediately after `fn thinking_auto_collapses_on_turn_end` at :5137 ends, and beside `a_frame_with_no_delta_does_zero_live_renders` at :8493)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice d — tests. W014 §7.1 names the first one ("`in_flight_thinking_is_one_annotation_row`
    // — rewrites `ui.rs:4967`: a `MessageStart` with a `Thinking` block renders
    // **one** dim `··· thinking` row with the elapsed and no body; the body appears
    // only after `Enter` on the committed block"). Bodies are `todo!()`; the NAMES
    // are the assertions the developer keeps (interface-sketch §2.3).
    //
    // Fixtures available in `ui.rs`'s test module: `render(app, w, h)` (:2282),
    // `buffer_text(terminal)` (:2288), `root()` (:2253), `push_thinking(app, text)`
    // (:5017). For the chip, the run must be LIVE: `AgentEvent::AgentStart` sets
    // `running = true` (app.rs:1663-1664) — a `MessageStart` alone leaves the chip
    // at `⏸ idle`. For a timer, inject the clock:
    // `app.set_run_elapsed(&root(), Duration::from_millis(1200))`.
    //
    // -------------------------------------------------------------------------
    // (1) `in_flight_thinking_is_one_annotation_row` — REPLACES the first half of
    //     `thinking_auto_collapses_on_turn_end` (ui.rs:5099-5137), which pins
    //     today's opposite behaviour and MUST be rewritten, not kept. Arrange:
    //       * `AgentStart`, then `MessageStart` with ONE
    //         `ContentBlock::Thinking { text: reasoning }`, then
    //         `set_run_elapsed(&root(), Duration::from_millis(1200))`.
    //       * render 80x24 and assert on the buffer:
    //         - EXACTLY ONE row contains `··· thinking`:
    //           `let rows = text.lines().filter(|l| l.contains("··· thinking")).count();`
    //           `assert_eq!(rows, 1, …)` — this is the whole design; >1 fails, 0 fails.
    //         - the row carries the live elapsed: `text.contains("1.2s")`.
    //         - the body is ABSENT: `!text.contains(reasoning)` — the assertion
    //           that FAILS today (ui.rs:5118-5121 asserts the opposite) and the one
    //           that proves the expansion is gone.
    //         - the live cursor closes it: `text.contains("▌")`.
    //         - NO affordance glyphs on that row: `!text.contains("▸")` and
    //           `!text.contains("▣")` (the committed row draws both — ui.rs:1107-1113).
    //       * then `push_thinking(&mut app, reasoning)` and assert the SECOND half
    //         of the old test unchanged: the body is still hidden, the `··· thinking`
    //         row is present, and `Enter`/click on the selected row reveals the body
    //         (mirror `a_rendered_thinking_row_publishes_a_clickable_toggle`,
    //         ui.rs:5140-5173).
    //
    // (2) `the_foot_chip_reads_thinking_while_reasoning_then_reverts` — the second
    //     behaviour D018 §1 names, and the one no existing test covers:
    //       * `AgentStart` + `MessageStart` with `[Thinking]` + `set_run_elapsed(…, 1200)`
    //         → render → `assert!(text.contains("⠹ thinking 1.2s"))` and
    //         `assert!(!text.contains("⠹ running"))`.
    //       * then a TEXT delta — `MessageUpdate` with
    //         `[Thinking{…}, Text{ … }]` (exactly what the kernel produces:
    //         `append_text` at loop_.rs:857-864 pushes a NEW `Text` block when the
    //         last block is `Thinking`) → render → `assert!(text.contains("⠹ running 1.2s"))`
    //         and `assert!(!text.contains("⠹ thinking"))`. This is the REVERT, and it
    //         is the assertion that fails if the predicate is not `content.last()`.
    //       * note in the test comment: the live message still contains the Thinking
    //         block, so the ROW survives (Block 1's per-block rule) while the CHIP
    //         reverts — assert both, so the two halves cannot drift apart.
    //
    // (3) `live_thinking_tracks_the_last_content_block` (app.rs, near the
    //     `live_cache_tests` module or the reducer tests) — the seam itself, cheap
    //     and exact. Drive `app.handle(...)` and assert `app.live_thinking()`:
    //       * no live message → `false`;
    //       * `MessageStart [Thinking]` → `true`;
    //       * `MessageUpdate [Thinking, Text]` → `false`;
    //       * `MessageUpdate [Thinking, Text, Thinking]` → `true` (the kernel's
    //         fresh block per stretch — loop_.rs:866-873);
    //       * `MessageUpdate [Thinking, ToolCall]` → `false`.
    //
    // (4) `set_run_elapsed_rerenders_the_live_thinking_row` (app.rs, beside
    //     `a_frame_with_no_delta_does_zero_live_renders`, :8493) — the R1 guard,
    //     and the only test that can catch a frozen timer:
    //       * fixture: a live THINKING message (a `start_thinking` twin of the
    //         module's `start`/`update` helpers at :8466/:8475), then
    //         `frame(&mut app, 60)` → note `live_renders()`;
    //       * `app.set_run_elapsed(&root(), Duration::from_millis(1200))` →
    //         `frame` again → assert the render count went UP by one and the drawn
    //         text contains `1.2s` (a frozen cache would show neither);
    //       * the NEGATIVE half (why the guard exists): with a PROSE-ONLY live
    //         message, `set_run_elapsed` must NOT bump — a second frame is a hit
    //         (mirror `a_frame_with_no_delta_does_zero_live_renders`'s
    //         `assert_eq!(r2, r1, …)`).
    // ==== /SKETCH ====
```

---

## Friction — where the design fights the code

1. **The existing test asserts the OPPOSITE and must be rewritten, not merely extended.** `ui.rs:5099-5137` `thinking_auto_collapses_on_turn_end` asserts `live.contains("visible only while the turn streams")` ("in-flight thinking streams inline") and `!live.contains("··· thinking")` ("no collapsed row while in-flight"). Both are inverted by D018 §1. W014 §7.1 already anticipates this ("rewrites `ui.rs:4967`"); the anchor has drifted to **`ui.rs:5099`**. The rewrite is Block 5's test (1); the second half of the old test (commit ⇒ hidden body, `··· thinking` present, `Enter` reveals) stays valid and should be preserved verbatim.
2. **The kernel feeds the thinking body through a path this slice stops using — but only on the live side.** Today the in-flight body is wrapped by `wrap(text, width, THINK_FIRST, THINK_CONT, thinking())` (ui.rs:949), the SAME helper the committed expansion uses (ui.rs:1079). Slice d drops that call from the live arm only. `wrap` itself must stay untouched: it is also `Notice`'s/`Transient`'s/`Btw`'s renderer (ui.rs:697-699) and the committed thinking body's. A reviewer should check the diff removes the call, never the function.
3. **`content_lines` is called with four args from two tests** (`ui.rs:4709`, `ui.rs:4731`). Adding a fifth parameter is a mechanical fix there (`None`), but it is a real compile break in the test module, so it belongs in the same commit (Block 1(b)).
4. **R1 could silently make the clock a no-op.** If `set_run_elapsed` is left alone, the cached row never changes and the timer freezes at whatever the first frame showed — the defect is invisible in a headless `dump` (one frame) and only shows in a live TUI. Block 3(d) is the fix; Block 5's test (4) is the guard. The *unguarded* variant (bump on every tick) trades the freeze for a per-tick re-render of the whole live block — acceptable for one row, but it defeats `a_frame_with_no_delta_does_zero_live_renders` (app.rs:8493) in production, so the guard (`live_is_thinking()`) is the recommended shape.
5. **The number shown is the RUN's elapsed, not the stretch's.** `run_elapsed` is injected from `started` = the run's start (lib.rs:565), so a second reasoning stretch shows the run total. That is the design ("the only honest number (how long have I been waiting)" — D018 Context) and there is deliberately NO per-thinking timer ("**Foreclosed**: … a new clock, or a per-thinking timer" — D018 Consequences). Do not "fix" this into a per-stretch timer; it is foreclosed.
6. **A multi-stretch live message draws one row per stretch, not one row per message.** `[Thinking, Text, Thinking]` renders two `··· thinking` rows (Block 1's per-block arm). This is the faithful reading of "no reasoning body in flight" (the earlier stretch's text must not leak), but it is worth a sentence in the commit so a reviewer does not read it as a bug. If the reviewer prefers exactly one row, the alternative — draw the annotation only for `content.last()` and emit NOTHING for an earlier Thinking block — costs the reader the fact that the earlier stretch happened; state which reading was chosen.
7. **`agent` test fixture ordering.** The chip's match is `(app.running(), app.run_elapsed())`, so a test with only `MessageStart` renders `⏸ idle` — the fixture needs `AgentStart` first (app.rs:1663) and, for a timer, a `set_run_elapsed` call. `ui.rs:4193/4205/4336/4475` show the existing `AgentStart` usage pattern.
8. **Anchor drift already in the tree:** D018's own citations are stale against the current HEAD (it cites `ui.rs:912`/`:923` for `live_lines`/`content_lines` — now `ui.rs:918` / `ui.rs:929` — and `app.rs:1344` for `append_live_lines` — now **:1359** — because slices b/c landed). The snippets are the pin; re-grep before inserting.

## What I could NOT verify

- **No build, no test, no clippy.** A sketch may not compile; every "this fails today" claim above is from reading the current tree, not from running it.
- **The true terminal width budget for `⠹ thinking 12.3s` at 48 cols** was not measured (the one-column difference is an argument, not an observation) — the foot's ladder clip is drawn by `padded_row`/`Paragraph` and is safe, but the visual balance at the tightest sizes is unverified.
- **The live TUI behaviour** (`dump` frames are a single frame, so a frozen timer would not show) — only a real run against a keyless endpoint (`--base-url http://localhost:11434/v1`) shows the timer advance and the chip flip; not run here.
- **Whether any OTHER test asserts the in-flight expanded body** beyond `thinking_auto_collapses_on_turn_end`. I grepped `in-flight`/`live` in `ui.rs` and found only :5102-5125; I did not exhaustively audit `tests/streaming.rs`, the VS Code surface, or the `examples/` fixtures for a live-thinking frame.
- **The exact rendered spacing** (`   ··· thinking 1.2s` vs `   ··· thinking  1.2s`) — the design's example has ONE space before `1.2s`; the helper's shape as sketched produces one. A golden-frame test (Block 5) would pin it; I did not run one.
