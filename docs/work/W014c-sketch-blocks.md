# W014 slice c — extracted sketch blocks (verbatim, parked for the slice-c round)

Parked by the orchestrator so slice b could commit without the slice-c blocks (A9).
Each block is byte-identical to what the sketcher wrote, with its pre-extraction anchor.
The slice-c implementer re-inserts each block at its anchor, fills it, and deletes the markers.

## Block 1 — `crates/wcode-tui/src/app.rs` (was :211, inside the `Block` enum)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — transient command chatter. Locked design:
    // `docs/tui-design.md` §4 "Transcript" (the block-kinds line, :187-189: "a
    // command notice is a rendering twin of `Notice` with a shorter life") and
    // §4 "Notices" (D018, :200-214 — the two named lists), plus
    // `docs/decisions/D018-tui-live-thinking-annotation.md` §3/"Mechanism" and
    // W014 §4c (option (a), *Ephemeral blocks*, is the evaluated-and-chosen one).
    // Form B (in-place comment block) per `.wcode/skills/interface-sketch/SKILL.md`.
    //
    // -------------------------------------------------------------------------
    // THE VARIANT TO ADD — inside this enum, beside `Notice` (app.rs:199):
    // -------------------------------------------------------------------------
    //
    //     /// A `/`-command's chatter: its echoed line, replies, usage hints and
    //     /// error lines. A RENDERING TWIN of [`Block::Notice`] — the renderer's
    //     /// arm is byte-identical (see the ui.rs sketch block) — differing only
    //     /// in LIFETIME: it never survives the next submit on its surface
    //     /// (`Surface::retire_transient`, sketched beside `clear_hint`).
    //     ///
    //     /// Per-surface, like every other block: a retire never crosses
    //     /// surfaces, so one surface's `/usage` cannot wipe another's scrollback.
    //     /// Never committed to ctx/session (it is display-only, exactly like
    //     /// `Notice`), and it is NOT the D5 empty-state hint (`clear_hint`
    //     /// matches `Block::Notice` on `EMPTY_HINT`, so the hint keeps its own
    //     /// retirement and stays in neither of D018's lists).
    //     ///
    //     /// `String`, not a borrow: identical to `Notice`/`Error`/`Btw`, so the
    //     /// enum's existing derives (`Clone, Debug, PartialEq`) cover it with no
    //     /// lifetime surprises and no new generic parameter.
    //     ///
    //     /// # Errors / # Panics
    //     /// Not applicable — a data variant.
    //     Transient(String),
    //
    // WHAT IT DOES NOT CHANGE: no existing variant gains or loses a field, the
    // enum stays `pub` with the same derives, and `Block`'s other consumers
    // (save/seeding/hit-testing) see one more variant that their wildcard arms
    // already cover — see the exhaustive-match list in the copy_text sketch block.
    // ==== /SKETCH ====
```

## Block 2 — `crates/wcode-tui/src/app.rs` (was :384, the exhaustive-`Block::` census)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — the exhaustive `Block::` matches that must learn the new
    // variant. Locked design: W014 §4c Integration points ("`Block` is `pub` and
    // `PartialEq`; the copy arm at `app.rs:472` matches `Block::Notice(text) |
    // Block::Error(text) | Block::Btw(text)` and must learn the new variant").
    //
    // -------------------------------------------------------------------------
    // (1) THE COPY ARM — `copy_text` (app.rs:469), sketched in place
    // -------------------------------------------------------------------------
    //     fn copy_text(block: &Block) -> Option<String> {
    //         let text = match block {
    //             Block::User(text)
    //             | Block::Notice(text)
    //             | Block::Transient(text)     // <<< NEW — same arm, same text
    //             | Block::Error(text)
    //             | Block::Btw(text) => text.clone(),
    //             …
    //         };
    //         (!text.trim().is_empty()).then_some(text)
    //     }
    //
    // WHY: `Transient` is a rendering twin of `Notice`, and `Ctrl-Y` /
    // browse-`y` / the glyph copy must be able to yank a `/help` reply the
    // user is still reading. It carries no privacy distinction — chatter is
    // chatter. `copy_text`'s rustdoc list (app.rs:376-383) grows one line:
    // "- `User` / `Notice` / `Error` / `Btw` / `Transient`: the block's text."
    //
    // -------------------------------------------------------------------------
    // (2) THE FULL CENSUS — every `Block::` match the fill-in must consider.
    //   EXHAUSTIVE = a compile error until it is extended.
    //   WILDCARD  = compiles unchanged; listed so nobody hunts for it.
    // -------------------------------------------------------------------------
    //  EXHAUSTIVE #1 — `copy_text`, app.rs:469-511 (`match block`, 5 arms covering
    //     all 8 variants). EXTEND.
    //  EXHAUSTIVE #2 — `ui.rs:740-755` `pub(crate) fn block_lines(block: &Block,
    //     width: usize)` — `match block { Block::User | Block::Turn |
    //     Block::Tool | Block::Notice | Block::Btw | Block::Error |
    //     Block::Diff { .. } | Block::Todos }`. EXTEND. This is the render arm
    //     the design calls the "rendering twin"; see the ui.rs sketch block.
    //  WILDCARD — ui.rs:544-602, the D32 affordance walk
    //     (`match &app.transcript()[i] { Block::Turn(..) => …, Block::Tool(..) =>
    //     …, _ => None }`). A transient row has NO affordance (no expand, no
    //     per-item copy rect) — it is plain text — so the wildcard is CORRECT.
    //     Leave it; do not add an arm.
    //  WILDCARD — app.rs:2657-2674 `toggle_all_tools`
    //     (`match &…transcript[i] { Block::Tool(..) => …, Block::Turn(..) => …,
    //     _ => false }` and the `_ => {}` mutate arm). Correct as-is: the
    //     `filter_map` at app.rs:2653 already only collects `Block::Tool |
    //     Block::Turn` slots, so a transient slot is never visited.
    //  WILDCARD — app.rs:3163-3166 `copy_block`'s item copy
    //     (`match b { Block::Turn(..) => …, Block::Tool(..) => …, _ => None }`).
    //     Correct: a transient block has no sub-items; the whole-block path
    //     (`None => … copy_text`) is what copies it.
    //  NON-MATCH — app.rs:1557-1559 `clear_hint`
    //     (`matches!(b, Block::Notice(t) if t.as_str() == EMPTY_HINT)`). The
    //     guard pins it to the hint text, so a transient block never matches and
    //     the hint keeps its own retirement. No change.
    //  WILDCARD — app.rs:2189-2192 `seed`'s tool-result arm
    //     (`match self.transcript.last_mut() { Some(Block::Turn(turn)) => …,
    //     _ => self.push_block(Block::Tool(tool)) }`). Seeding never produces a
    //     transient block. No change.
    //  TESTS — the `matches!(… Some(Block::Notice(text)) …)` patterns throughout
    //     `mod tests` are `matches!`/`if let`, never exhaustive matches, so the
    //     crate COMPILES with them untouched. But ~19 of them assert a NOTICE
    //     on a block this slice turns into a `Transient` and will FAIL at
    //     runtime — see the test sketch block for the list and the invariant.
    //  OTHER CRATES — none. `Block` is `wcode_tui`'s type; the `Block::` hits in
    //     `wcode-cli`/`wcode-harness` are `ContentBlock`/`WidgetBlock`/other
    //     names, and `wcode-tui`'s own `tests/streaming.rs` and `examples/`
    //     only construct `Block::User`/`Turn`/`Tool`. Verified by
    //     `grep -rln 'Block::' crates/`.
    //
    // -------------------------------------------------------------------------
    // FRICTION
    // -------------------------------------------------------------------------
    // * The two EXHAUSTIVE sites are in different files, so the compiler error
    //   for one does not point at the other. Grep the census, do not rely on
    //   `cargo build` iterating you there — `cargo clippy --workspace
    //   --all-targets` is the gate that catches both (plus the tests).
    // * `block_lines` is the render twin; if a future variant is added and it
    //   copies the `Notice` arm, prefer delegating to the same helper rather
    //   than duplicating the `wrap(...)` call, so "renders exactly like" stays
    //   true by construction.
    // ==== /SKETCH ====
```

## Block 3 — `crates/wcode-tui/src/app.rs` (was :1567, `retire_transient`)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — the splice. Locked design: D018 §3 (transient = command
    // chatter, retired at the next submit on that surface) + W014 §4c
    // (`Surface::retire_transient()` — "the splice, modelled on `clear_hint`").
    // Placed beside `clear_hint` (app.rs:1556), the ONLY existing
    // `transcript.remove` in the crate, because it is the precedent this copies.
    //
    // -------------------------------------------------------------------------
    // THE METHOD TO ADD — on `Surface`, beside `clear_hint`/`insert_block`
    // -------------------------------------------------------------------------
    //
    //     /// Retire every transient block (the `/`-command chatter of the
    //     /// PREVIOUS generation) from this surface, keeping the three parallel
    //     /// vecs — `transcript`, `block_revs`, `cache` (app.rs:1534
    //     /// `fn push_block` — "keeping `cache`/`block_revs` index-aligned with
    //     /// `transcript`. Every transcript push goes through here") — in
    //     /// LOCKSTEP, and keeping every block-indexed cursor on the same block.
    //     ///
    //     /// # Contract
    //     ///
    //     /// * Removes EVERY `Block::Transient` slot, not just the last: a
    //     ///   generation of chatter is `/help`'s echo + its reply + a
    //     ///   `usage:` hint, so a single-index splice would leave orphans.
    //     /// * ONE pass, index-by-index: collect the transient indices from
    //     ///   `transcript`, then remove them from all three vecs at the SAME
    //     ///   index in the SAME step. Removals shift later slots, so the walk
    //     ///   goes in DESCENDING index order (or builds three new vecs in one
    //     ///   forward pass). `Vec::retain` per vec is sound only because the
    //     ///   predicate is index-identical for all three — the W014 brief's
    //     ///   wording ("not `retain`-per-vec") asks for the explicit form, and
    //     ///   the invariant is what a reviewer must check: NO `remove` on one
    //     ///   vec without the same index removed from the other two.
    //     /// * The browse selection lands on the SAME block (D018 §3:
    //     ///   "retirement removes it and clamps the browse selection onto the
    //     ///   same block"). Decrement `selected` once per removed slot BELOW
    //     ///   it, then clamp into range — the pattern `insert_block` already
    //     ///   uses for its shift (app.rs:1698-1703 `if let Some(sel) =
    //     ///   self.selected && at <= sel { self.selected = Some(sel + 1); }`)
    //     ///   and `set_block_ranges` uses for its clamp (app.rs:2826-2831
    //     ///   `if len == 0 { … None } else if i >= len { … Some(len - 1) }`).
    //     ///   A removed slot AT the selection keeps the selection on the block
    //     ///   that followed it. (W014 §4c cites `app.rs:656` for this clamp;
    //     ///   that line is a PICKER clamp — `PickerState::move_selection` — so
    //     ///   use the two transcript precedents above instead.)
    //     /// * `open_turn` is a block index too (app.rs:1276). A transient block
    //     ///   is never a `Block::Turn`, so the open turn's own slot never goes;
    //     ///   decrement it once per removed slot below it, mirroring
    //     ///   `insert_block`'s bookkeeping (app.rs:1693-1697
    //     ///   `if let Some(i) = self.open_turn && at <= i { self.open_turn =
    //     ///   Some(i + 1); }`).
    //     /// * NOT touched: `ranges` (app.rs:1332, the per-block line ranges).
    //     ///   `insert_block` leaves `ranges` stale for the same reason — it is
    //     ///   rewritten every frame by `set_block_ranges` (app.rs:2822) — and a
    //     ///   splice happens at submit time, one frame before the rewrite.
    //     ///   See the friction note.
    //     ///
    //     /// # Errors / # Panics
    //     /// Neither. A no-op when the transcript holds no `Block::Transient`
    //     /// (the common case: the FIRST submit after a fresh surface, and every
    //     ///   submit on a surface that never ran a `/`-command).
    //     ///
    //     /// `&mut self`, returns `()`.
    //     fn retire_transient(&mut self) {
    //         todo!()
    //     }
    //
    // -------------------------------------------------------------------------
    // FRICTION — where the design fights the code
    // -------------------------------------------------------------------------
    // 1. **`Surface` vs `App`.** `submit` (app.rs:3831) is an `App` method, so
    //    the retire is reached as `self.focused_mut().retire_transient()`. The
    //    splice itself MUST be on `Surface` (it touches private per-surface
    //    vecs); an `App`-level door is optional and NOT recommended — it would
    //    hide the per-surface scope that D018's "a retire never crosses
    //    surfaces" makes load-bearing. If a reviewer wants one for symmetry with
    //    `App::notice`, its shape is
    //    `fn retire_transient(&mut self) { self.focused_mut().retire_transient(); self.dirty = true; }`.
    //    Pick ONE; do not add both call paths.
    // 2. **`clear_hint` does NOT clamp `selected`, and this must.** `clear_hint`
    //    (app.rs:1556-1565) removes one block and fixes nothing, which is safe
    //    for the hint only because the hint is the FIRST block of an EMPTY
    //    transcript (nothing is selected yet). A transient block can be
    //    anywhere, and browse may hold a selection on it — hence the decrement
    //    above. This is the one place the model is deliberately stricter than
    //    its precedent.
    // 3. **`push_block`'s `clear_hint()` side effect.** Every push — transient
    //    included — retires the D5 hint first (app.rs:1536 `self.clear_hint();`).
    //    That is HARMLESS here (a `/`-command on a fresh surface SHOULD dismiss
    //    the "type a message" hint, exactly as a normal submit does), but note
    //    the interaction this sketch does NOT need to solve: the hint is a
    //    `Block::Notice`, so it is never matched by a transient splice and never
    //    resurrected (`seed_hint_if_empty` is a no-op unless the transcript is
    //    empty — app.rs:1546).
    // 4. **Can a permanent notice be wrongly retired?** No, by construction:
    //    `⋯ compacted N messages` / `⋯ compaction skipped: <reason>` /
    //    `⋯ retrying (n/m)` / `⏹ aborted` / the changes summary /
    //    `⋯ N earlier message(s)` are all pushed as `Block::Notice` from the
    //    `AgentEvent` arms (app.rs:1847, :1609, :1649, :1653, :1663, :1962) and
    //    the splice only matches `Block::Transient`. The timing question is
    //    therefore also answered: a compaction notice landing while a submit is
    //    in flight is pushed as a `Notice` and CANNOT be retired by the next
    //    submit, because the retire runs at the TOP of `submit` (see the submit
    //    sketch block) — before any new block is pushed — and only matches the
    //    transient variant. The one thing to keep true in the fill-in: never
    //    route a run-event notice through `App::transient`/`push_transient`.
    // 5. **The `History` reply is async.** `/usage` (app.rs:4073) pushes an
    //    `Action::Ask(Request::GetHistory)`; the reply arrives LATER as
    //    `AgentEvent::History` (app.rs:1916 → `render_usage`, app.rs:2043) and
    //    its block is pushed from `render_usage` — NOT from inside `command()`.
    //    So the `/usage` reply cannot be made transient by a call site inside
    //    `run_command`; `render_usage` must push `Block::Transient` itself
    //    (i.e. a `push_transient` twin of `push_notice`, app.rs:2038). This is
    //    the one command whose chatter is emitted outside the command frame,
    //    and the reason the split is "per call site" (W014 §4c) rather than a
    //    flag on `push_notice`.
    // ==== /SKETCH ====
```

## Block 4 — `crates/wcode-tui/src/app.rs` (was :3749, the `submit` call site)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — WHERE the retirement lands. Locked design: W014 §4c
    // ("`fn submit` — call `retire_transient()` on the focused surface **before**
    // pushing `Block::User`, so the cleanup lands with the next generation") and
    // D018 §3 (retired at the next submit on that surface; a retire never
    // crosses surfaces). Beside `submit` (app.rs:3831).
    //
    // THE SHAPE OF `submit` AFTER THE CHANGE (the call site, not the body):
    //
    //     fn submit(&mut self) {
    //         let atoms = std::mem::take(&mut self.input);   // app.rs:3833 …
    //         self.cursor = 0;  self.focused_mut().scroll = 0;  self.dirty = true;
    //         let text: String = atoms.iter().map(Atom::text).collect();
    //         let text = text.trim().to_string();
    //         self.focused_mut().history_index = None;
    //         self.focused_mut().draft.clear();
    //         if text.is_empty() { return; }                 // app.rs:3841 …
    //         // >>> NEW: one call, on the FOCUSED surface only. <<<
    //         // Retire the previous generation of command chatter BEFORE this
    //         // generation's first block lands, so a transcript holds at most
    //         // one generation of `/`-command chatter (D018 §Consequences).
    //         self.focused_mut().retire_transient();           // sketched beside `clear_hint`
    //         self.dirty = true;                              // mirror `App::notice`
    //         if text.starts_with('/') { self.command(&text); return; }   // app.rs:3844 …
    //         if self.focused().running {
    //             self.focused_mut().push_transient(          // sketched below
    //                 "a turn is already running — Esc to cancel",
    //             );
    //             return;
    //         }
    //         self.focused_mut().history.push(text.clone());
    //         self.focused_mut().push_block(Block::User(text.clone()));   // app.rs:3855 …
    //         …
    //     }
    //
    // WHY HERE AND NOT LOWER (the three orderings, and what each costs):
    //
    // * **After the `text.is_empty()` guard** (RECOMMENDED, as drawn above). A
    //   stray `Enter` on an empty buffer is not a generation — it must not wipe
    //   `/help`'s reply the user is still reading.
    // * **Before the `starts_with('/')` dispatch** (RECOMMENDED). This is what
    //   makes "at most one generation" true, and it is what W014 §7.2's human
    //   check describes: "Type `/usage`, then `/help`, then send a real prompt.
    //   The notices disappear as the new turn's blocks arrive" — the `/usage`
    //   reply is already gone when `/help` is echoed. It is also the only
    //   ordering in which the new echo cannot retire ITSELF (the retire runs
    //   first, the block is pushed after).
    // * **Only immediately before `push_block(Block::User)`** (the literal
    //   reading of W014 §4c). This keeps `/usage` + `/help` side by side — two
    //   generations — and contradicts D018's consequence line. REJECT unless a
    //   reviewer amends D018.
    //
    // THE RUNNING GUARD. Its block (app.rs:3851
    // `.push(Block::Notice("a turn is already running — Esc to cancel".into()))`)
    // becomes a TRANSIENT push — D018 §3 names it explicitly ("plus the
    // submit-time `a turn is already running` guard — a rejected submit is
    // chatter, not a run event"). Note it currently bypasses `push_block`
    // (a raw `transcript.push`), so the fill-in must either route it through
    // `push_transient` or push all three vecs — see friction. Being AFTER the
    // retire, it survives the submit that produced it and dies on the next one.
    //
    // -------------------------------------------------------------------------
    // FRICTION — where the design fights the code
    // -------------------------------------------------------------------------
    // A. **`submit` early-returns four times** (empty, command, running, and
    //    the final path). A retire placed anywhere but the top is a behavioural
    //    fork: the command path and the normal path would retire at different
    //    times. One call, before the first fork that can produce a block.
    // B. **The guard bypasses `push_block`.** app.rs:3849-3852 pushes straight
    //    onto `transcript`, so `block_revs`/`cache` are NOT extended for it —
    //    a pre-existing index-parity hole this slice does not introduce but
    //    must not widen. Route the guard through the new `push_transient` (which
    //    goes through `push_block`) rather than a second raw `push`.
    // C. **Per-surface scope.** `self.focused_mut()` only. A retire on every
    //    surface would wipe another surface's `/help` reply while the user
    //    reads it — D018 §3's "a retire never crosses surfaces" is the whole
    //    reason the method is on `Surface` and not on `App::surfaces`.
    // D. **`retire_transient` returns `()`**, so `submit` must set `self.dirty`
    //    itself (or the App-level wrapper does). Otherwise a submit whose only
    //    effect was the retire can leave a stale frame on screen.
    // ==== /SKETCH ====
```

## Block 5 — `crates/wcode-tui/src/app.rs` (was :3866, the command call sites)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — the call-site list. Locked design: D018 §3 + W014 §4c
    // ("`self.notice(line)` in `fn command` → `transient(line)`" and the
    // `run_command` arms). Beside `command` (app.rs:3990).
    //
    // -------------------------------------------------------------------------
    // THE TWO NEW HELPERS TO ADD (the twins this whole slice hangs off)
    // -------------------------------------------------------------------------
    //
    //     /// Push a transient block on the FOCUSED surface — the twin of
    //     /// [`App::notice`] (app.rs:4289 `fn notice(&mut self, text: impl
    //     /// Into<String>) { self.focused_mut().push_notice(text); self.dirty =
    //     /// true; }`). Same shape, same `dirty`, one word different.
    //     fn transient(&mut self, text: impl Into<String>) {
    //         self.focused_mut().push_transient(text);
    //         self.dirty = true;
    //     }
    //
    //     /// On `Surface`, beside `push_notice` (app.rs:2038 `fn push_notice(&mut
    //     /// self, text: impl Into<String>) { self.push_block(Block::Notice(text.
    //     /// into())); }`): the same one-liner pushing `Block::Transient`. This
    //     /// is the door for the sites that are NOT `App` methods —
    //     /// `render_usage` (app.rs:2043) is the one that matters.
    //     fn push_transient(&mut self, text: impl Into<String>) {
    //         self.push_block(Block::Transient(text.into()));
    //     }
    //
    // `impl Into<String>`, not `&str`: identical to both twins, so every existing
    // call site's argument type keeps compiling.
    //
    // -------------------------------------------------------------------------
    // THE CALL SITES THAT SWITCH (every `self.notice(...)` reachable from the
    // `/`-command path). `n` = the anchor, quoted:
    // -------------------------------------------------------------------------
    //  `command` — the echoed `/<command>` line itself:
    //    app.rs:3994  `self.notice(line);`
    //    app.rs:3997  `None => self.notice(format!("unknown command: {name}")),`
    //  `run_command` — the replies and usage hints:
    //    app.rs:4017  `self.notice(format!("theme: {name}"));`
    //    app.rs:4019  `self.notice(format!("unknown theme: {name}"));`
    //    app.rs:4033  `None => self.notice("usage: /effort <level> ('-' clears)"),`
    //    app.rs:4038  `self.notice(format!("width: {} cols", self.measure));`
    //    app.rs:4040  `None if arg.is_some() => self.notice("usage: /width <cols> (40-200)"),`
    //    app.rs:4041  `None => self.notice(format!("width: {} cols (usage: /width <cols>)", …)),`
    //    app.rs:4055  `None => self.notice("usage: /btw <question>"),`
    //    app.rs:4064  `self.notice("usage: /plan [on|off]");`
    //    app.rs:4074  `"verify" => self.notice(self.verify_text()),`
    //    app.rs:4081  `Some(_) => self.notice("usage: /reload [--no-session]"),`
    //    app.rs:4086  `Some("roster") => self.notice(team_text(&self.member_rows())),`
    //    app.rs:4089  `"tasks" => self.notice(tasks_text(&self.tasks)),`
    //    app.rs:4091  `"help" => self.notice(help_text()),`
    //  Reached THROUGH a command (the picker/refusal chatter a `/`-command opens):
    //    app.rs:4489  `self.notice("no changes this run");`
    //    app.rs:4562  `self.notice("no sessions to resume");`
    //    app.rs:4582  `self.notice("no project teams found (add ./.wcode/teams/<name>.toml)");`
    //    app.rs:4597  `self.notice("/team (switch team) is unavailable over a socket");`
    //    app.rs:4614  `self.notice("no models available to pick");`
    //    app.rs:4698  `self.notice(format!("model: {selected}"));`
    //    app.rs:4722  `self.notice(format!("theme: {selected}"));`
    //    app.rs:4803  `self.notice("/reload (rebuild + re-exec) is unavailable over a socket");`
    //    app.rs:4815  `self.notice("/new (start a fresh session) is unavailable over a socket");`
    //  Reached through `/copy` (shared with `Ctrl-Y` — see friction E):
    //    app.rs:4240  `self.notice(format!("copied {chars} chars to the clipboard"));`
    //    app.rs:4242  `None => self.notice("nothing to copy yet"),`
    //  The async reply of `/usage` (NOT inside `command` — see friction F):
    //    app.rs:2055  `self.push_notice(line);`   → `push_transient`
    //  The submit-time guard (in `submit`, not here):
    //    app.rs:3851  `.push(Block::Notice("a turn is already running — Esc to cancel".into()))`
    //
    // -------------------------------------------------------------------------
    // WHAT STAYS PERMANENT — the run-event sites, and WHY
    // -------------------------------------------------------------------------
    // D018's second list is "the record of what the run did", and every one of
    // them is pushed from an `AgentEvent` arm (or from seeding), NOT from the
    // `/`-command path — so they keep `Block::Notice` untouched:
    //    app.rs:1847  `self.push_block(Block::Notice("⏹ aborted".into()));`
    //    app.rs:1851  `self.push_block(Block::Notice(self.changes_summary()));`
    //    app.rs:1891  `self.push_block(Block::Notice(format!("⋯ compaction skipped: {reason}")));`
    //    app.rs:1895  `self.push_block(Block::Notice(format!(… "⋯ compacted …")));`
    //    app.rs:1905  `self.push_block(Block::Notice(format!(… "⋯ retrying …")));`
    //    app.rs:2204  `Block::Notice(format!("⋯ {} earlier message(s)", messages.len())),`
    // and the NON-command UI notices, which are neither list (they are keyboard /
    // mouse affordances, not a generation of chatter) — all of them stay
    // `Block::Notice` and are NEVER reclassified:
    //    app.rs:4240/4242  `copy_last`'s `copied N chars` / `nothing to copy yet`
    //                      — the SAME two lines `/copy` reaches (see friction E);
    //                      only the `/copy` spelling is command chatter
    //    app.rs:3175/3177  `copy_block`'s pair, from browse `y` / a glyph click
    //    app.rs:3340       the drag-copy `copied N chars` notice
    //    app.rs:1548       the seeded D5 hint (`clear_hint` retires it; it is in
    //                      neither of D018's lists)
    //
    // -------------------------------------------------------------------------
    // FRICTION — where the design fights the code
    // -------------------------------------------------------------------------
    // E. **`copy_last` is shared by `/copy` and `Ctrl-Y`.** `run_command`'s
    //    `"copy" => self.copy_last()` (app.rs:4083) lands on the SAME notices
    //    (app.rs:4240/4242) as `Key::Ctrl('y')` (app.rs:3668), but only the
    //    `/copy` spelling is command chatter. Three ways out, in preference
    //    order: (1) give `copy_last` a `transient: bool` parameter (or a
    //    `CommandOrigin` enum) and pass `true` from `run_command`; (2) split it
    //    into `copy_last` (Ctrl-Y, permanent) + `copy_last_transient`; (3) a
    //    command-in-flight flag on `Surface`. Pick ONE and say so in the commit
    //    — (1) is the least invasive. Do NOT make `notice()` itself
    //    context-sensitive: that would silently reclassify every other site.
    // F. **`/usage`'s reply is emitted outside the command frame** (see the
    //    retire block's friction #5): the reply arrives as `AgentEvent::History`
    //    (app.rs:1916) AFTER `command()` returned, so `render_usage`
    //    (app.rs:2055) must push a transient itself. There is no call site in
    //    `run_command` to switch.
    // G. **The pickers opened BY a command** (`/model`, `/theme`, `/changes`,
    //    `/resume`, `/team`, `/surface`) emit their notices from the picker's own
    //    arms, sometimes several frames later (`model: {selected}` is pushed when
    //    the user picks, not when they type `/model`). They are still chatter —
    //    the user asked a question and got an answer — so they switch too; but
    //    note their LIFETIME is then longer than the echo's (the echo dies at
    //    the next submit, the picker's answer when the picker closes and another
    //    submit happens). That is the intended reading of D018's "…", and it is
    //    why the list above is a per-call-site enumeration rather than a rule.
    // H. **`/exit`, `/new`, `/compact`, `/model <id>`, `/effort <level>` emit no
    //    notice at all** — they push an `Action` only — so there is nothing to
    //    switch; a `/`-command's chatter is echo + reply, never the action.
    // ==== /SKETCH ====
```

## Block 6 — `crates/wcode-tui/src/app.rs` (was :5148, the test stubs)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — the two new tests (W014 §7.1). Both bodies are `todo!()`;
    // the NAMES are the assertions the developer keeps (interface-sketch §2.3).
    //
    //     #[test]
    //     fn a_command_notice_is_retired_by_the_next_submit() { todo!() }
    //
    //     #[test]
    //     fn retire_transient_keeps_the_three_vecs_in_step() { todo!() }
    //
    // -------------------------------------------------------------------------
    // `a_command_notice_is_retired_by_the_next_submit` — WHAT BREAKS IF IT FAILS
    // -------------------------------------------------------------------------
    // If the splice is missing or mis-ordered, the transcript keeps a
    // generation of chatter forever (D018's headline defect), OR — if the retire
    // runs at the wrong point in `submit` — the test's own `/usage` reply is
    // retired by the very submit that produced it and the FIRST assertion sees
    // nothing. Arrange:
    //   * `App::new()`; `submit(&mut app, "/usage")` needs a `History` reply to
    //     render one, so drive it directly the way `a_history_reply_updates_usage`
    //     does — `app.handle(AppEvent::Agent(root(), AgentEvent::History {
    //     messages: vec![AgentMessage::user_text("q")] }))` — or use `/help`
    //     (a synchronous reply: `submit(&mut app, "/help")`), which is the
    //     shorter fixture. `/help` is preferred; `/usage` is the async case.
    //   * assert the chatter is THERE as a transient:
    //     `assert!(matches!(app.transcript().last(), Some(Block::Transient(_))))`
    //     — and, for the permanent control, that a run-event notice pushed the
    //     same way is a `Block::Notice` (e.g. push `⋯ compacted` via
    //     `AgentEvent::Compaction { summarized: 1, kept: 0 }`, or assert
    //     `⏹ aborted` after a cancelled run as `cancel_is_sent_once_and_…`
    //     does at app.rs:5350).
    //   * then `submit(&mut app, "a normal line")` and assert the transient is
    //     GONE: `assert!(!app.transcript().iter().any(|b| matches!(b,
    //     Block::Transient(_))))` — NOT merely that `last()` changed, which
    //     would also pass with the block simply buried under a `Block::User`.
    //   * then assert the permanent control SURVIVED the same submit:
    //     `assert!(app.transcript().iter().any(|b| matches!(b,
    //     Block::Notice(t) if t.contains("compacted"))))`.
    //   * optional second half (the "at most one generation" ordering): after
    //     `submit("/help")` then `submit("/tasks")`, only ONE transient
    //     generation remains — the `/tasks` echo — and the `/help` reply is gone.
    //     This is the assertion that fails if the retire is placed only before
    //     `push_block(Block::User)` instead of before the command dispatch.
    //
    // -------------------------------------------------------------------------
    // `retire_transient_keeps_the_three_vecs_in_step` — WHAT BREAKS IF IT FAILS
    // -------------------------------------------------------------------------
    // If the splice removes from one vec and not the others, the renderer walks
    // a `transcript` slot whose `cache`/`block_revs` entry belongs to a DIFFERENT
    // block (a wrong cached render, or a stale `CacheEntry` that never
    // invalidates), and `set_block_ranges`'s `len` check
    // (app.rs:2823-2831) starts clamping against the wrong length. The panic
    // surface is `append_block_lines`'s parity guard (app.rs:1415's doc: "Owns
    // the transcript/cache split borrow itself") — a `ranges`/`cache` desync
    // shows up as a mis-rendered frame, which is why the vec lengths are
    // asserted directly. Arrange:
    //   * a fixture with several blocks INCLUDING at least two transients, one
    //     of them in the MIDDLE (a `… /help` echo followed by a permanent
    //     `Notice` is the case that punishes a `retain`-per-vec or a
    //     last-index-only splice). Push them through `submit`/`transient` so
    //     `push_block` keeps the vecs aligned, then force a browse frame:
    //     `app.handle(AppEvent::Key(Key::Ctrl('g')))` and
    //     `app.set_block_ranges(vec![…one range per block…])` (the pattern at
    //     app.rs:6925) so `selected` is meaningful.
    //   * call the retire directly: `app.focused_mut().retire_transient();`
    //   * assert the invariant:
    //     `assert_eq!(app.focused().transcript.len(), app.focused().block_revs.len());`
    //     `assert_eq!(app.focused().transcript.len(), app.focused().cache.len());`
    //   * assert the selection landed on the SAME BLOCK (D018 §3): capture the
    //     selected block's text (or its `Debug`) BEFORE the retire, then assert
    //     `app.selected()` still addresses a block with that text — NOT the same
    //     index, which is exactly what the splice must change.
    //   * assert the transient blocks are gone and the permanent ones remain.
    //   * optional: assert the D5 hint is untouched when it is the only block
    //     (`App::new()` + retire = no-op).
    //
    // -------------------------------------------------------------------------
    // THE INVARIANT THAT KEEPS THE EXISTING NOTICE TESTS GREEN
    // -------------------------------------------------------------------------
    // ~19 of the ~27 `Block::Notice` assertions in `mod tests` sit on a block
    // this slice reclassifies as `Block::Transient`, and they assert the VARIANT,
    // not just the text — so the retirement TIMING is not what keeps them green,
    // and the fill-in MUST update them. Two separate statements:
    //
    // (a) TIMING — safe by construction. Retirement fires on the NEXT submit,
    //     and every one of these tests performs ONE submit (or none: the
    //     `AgentEvent`-driven ones) before asserting, so `transcript().last()`
    //     still holds the block it just produced. No test does
    //     `submit("/x"); submit("y"); assert last is /x's notice`.
    // (b) VARIANT — NOT safe; these must be edited to accept the twin:
    //     app.rs:5139 a_second_submit_while_running_is_refused   (the guard)
    //     app.rs:5411 verify_renders_the_checklist_and_flags_unfinished
    //     app.rs:5439 verify_when_all_items_are_done_reports_finished
    //     app.rs:5460 verify_without_todos_reports_none
    //     app.rs:5885 a_history_reply_updates_usage
    //     app.rs:5915 a_history_reply_without_usage_reports_none
    //     app.rs:6005 copy_emits_the_last_reply
    //     app.rs:6027 copy_with_nothing_says_so
    //     app.rs:6151 model_without_an_argument_and_no_list_says_so
    //     app.rs:6364 changes_with_nothing_says_so
    //     app.rs:6493 the_team_command_lists_the_member_surfaces
    //     app.rs:6543 the_team_command_switches_or_lists
    //     app.rs:6580 the_tasks_command_lists_the_plan
    //     app.rs:7371 resume_with_no_sessions_says_so
    //     app.rs:7401 reload_rejects_a_bad_argument_without_quitting
    //     app.rs:7413 reload_is_refused_over_a_socket
    //     app.rs:7437 new_is_refused_over_a_socket
    //     app.rs:7489 help_lists_every_command_and_the_keymap_from_the_tables
    //     app.rs:7701 a_bare_slash_does_not_accept_on_enter
    //     (each line is the `fn` line; the `Block::Notice` assertion follows
    //      within a few lines — the anchors shift as this sketch's blocks land)
    //     These stay green WITHOUT edits, because their block is a run event or
    //     a non-command affordance notice:
    //     app.rs:5350 cancel_is_sent_once_and_renders_as_an_abort   (⏹ aborted)
    //     app.rs:5506 seeding_replays_the_conversation_so_far        (divider)
    //     app.rs:6233 the_changeset_summary_appears_when_the_run_settles
    //     app.rs:8022 a_front_inserted_seed_divider_does_not_strand_the_selection
    //     app.rs:8040 seeding_an_existing_transcript_keeps_the_selection_on_the_same_block
    //     app.rs:8335 y_with_nothing_to_copy_pushes_no_action        (Ctrl-Y / browse y)
    //     app.rs:8474 append_block_lines_extends_the_parallel_vecs_on_a_parity_miss
    // Recommended edit shape: `Some(Block::Notice(text))` →
    // `Some(Block::Notice(text) | Block::Transient(text))` where the test only
    // cares about the text, and an explicit `Block::Transient` where the test is
    // ABOUT the new lifetime. No `ui.rs`/`tests/streaming.rs`/`lib.rs` test
    // asserts a command notice (verified by grep), so the blast radius is this
    // one module.
    // ==== /SKETCH ====
```

## Block 7 — `crates/wcode-tui/src/ui.rs` (was :681, the render twin)

```rust
    // ==== SKETCH (review-only, not real code) ====
    // W014 slice c — the render arm. Locked design: D018 §3 ("A transient row
    // renders **exactly** like a `Notice`; retirement removes it, it is not
    // restyled") and `docs/tui-design.md`:207-208 ("A transient row renders
    // **exactly** like a `Notice`"). Beside `block_lines` (ui.rs:740), whose
    // `Notice` arm at ui.rs:750 is the twin this copies.
    //
    // -------------------------------------------------------------------------
    // THE ARM TO ADD — inside `block_lines`'s `match block` (ui.rs:741)
    // -------------------------------------------------------------------------
    //         Block::Transient(text) => wrap(text, width, "   ", "   ", dim()),
    //
    // IDENTICAL TO THE `Notice` ARM IN EVERY RESPECT — this is the whole point
    // of the design, so state it plainly for the reviewer:
    //   * same call: `wrap(text, width, "   ", "   ", dim())`
    //   * same gutter `"   "` (3 spaces) and the same continuation `"   "`
    //   * same style role: `dim()` — the named palette role, not a literal, so
    //     a theme reload restyles both arms together
    //   * same function, same parameter order, same `width` — so the block's
    //     HEIGHT is identical too, which matters because `append_block_lines`
    //     caches per `(rev, width)` and a height difference would desync the
    //     cached `Line`s from `ranges`
    // The two arms should either be written verbatim or — better — share one
    // helper so "renders exactly like" cannot drift:
    //     Block::Notice(text) | Block::Transient(text) => {
    //         notice_row(text, width)          // `fn notice_row(text: &str, width: usize)`
    //     }
    // Either is acceptable; the or-pattern is the form that survives a future
    // edit to the `Notice` arm. NOT acceptable: a different gutter, a different
    // role, an extra glyph, or a shortened life shown as a visual difference.
    //
    // WHAT ELSE IN `ui.rs` CHANGES: nothing. `block_lines` is the only render
    // entry for committed blocks, the affordance walk (ui.rs:544-602) covers
    // `Transient` through its `_ => None` wildcard (a transient row has no
    // expand/copy affordance — it is plain text), and no other `ui.rs` match
    // mentions `Block::Notice`. There is no "transient" styling pass, no fade,
    // no countdown — D018 §"Foreclosed" rules out restyling on retirement since
    // the row is REMOVED, not restyled.
    //
    // -------------------------------------------------------------------------
    // FRICTION — where the design fights the code
    // -------------------------------------------------------------------------
    // * **`block_lines` is the crate's only exhaustive `Block::` render match**
    //   (8 arms, ui.rs:741-755), so this is the ONE place a missing arm is a
    //   compile error — and it is in a different FILE from the other exhaustive
    //   site (`copy_text`, app.rs:469). The grep census in the app.rs sketch
    //   block lists both; a `cargo build` that only reports one is not proof
    //   the other was checked.
    // * **The cache.** `CacheEntry` is pushed per block by `push_block`
    //   (app.rs:1539) and keyed `(rev, width)`; a transient block is cached like
    //   any other while it lives and its slot is dropped by the splice. Nothing
    //   to do here, but a reviewer should note that the splice's job is to keep
    //   `cache` index-aligned — see the `retire_transient` block.
    // * **The seeded hint is a `Notice`, not a `Transient`** (app.rs:1548), so
    //   the empty-state row keeps rendering through the `Notice` arm and the two
    //   lifetimes stay visually identical by accident of the shared helper —
    //   which is the intent, not a coincidence to preserve by hand.
    // ==== /SKETCH ====
```
