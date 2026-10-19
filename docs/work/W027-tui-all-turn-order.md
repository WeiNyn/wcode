# W027 — TUI All mode: order merged turns by last activity, not open

- **Status:** ☑ **done** — `628ba63` (re-stamp on growth) + `ee4a3b7` (the `Error`-arm ordering fix); second-layer review APPROVED; zero `SKETCH` markers; workspace tests + clippy clean.
- **Decisions:** [D032](../decisions/D032-tui-all-turn-order-by-activity.md) (refines the ordering rule of [D031](../decisions/D031-tui-all-focus-view-mode.md))
- **Tracker:** [next-steps.md](../next-steps.md) item 87
- **Related:** [W026](W026-tui-all-mode.md) (shipped All mode)

## 1. The ask

In All mode the merged transcript rendered the orchestrator's turn — which carries
its final reply — **above** the member turns it coordinated, even though the reply
was written last. The user's report: *"orchestrator is the one that finishes the
conversation, but the TUI renders messages from other members after that."* Order
each turn by its **last activity** so the last-written turn sorts last.

## 2. Scope / Non-scope

**Scope**
- Re-stamp a surface's **open `Block::Turn`** ordinal when its content grows (so a
  turn sits at its latest activity, not its open).
- A failing-first `merged_order` test + a headless frame check.
- Record the ordering rule (D032) and add a forward pointer from D031.

**Non-scope**
- **Message-granularity** merge (option C) — the orchestrator's turn keeps bundling
  its delegation rows and its reply into one block.
- The VS Code client's same-class defect (F1 below) — its own `editors/` change.
- Socket-client parity; `Focus` mode (unchanged — only `merged_order` is affected).

## 3. Estimates

| workstream | value | complexity | risk | why |
|---|---|---|---|---|
| Re-stamp on content growth | high | low | med | fixes a visible ordering bug at one seam; the risk is the *when* (which events move a turn) |
| Tests + frame check | med | low | low | the failing-first test pins the rule; a `TestBackend` frame is the human proof |
| Record (D032 + D031 pointer) | low | low | low | a short ADR; D031 is at its 50-line budget |

## 4. Interface & structure (integration points)

- **`crates/wcode-tui/src/app.rs`**
  - The merge key is unchanged: `merged_order` (`:4572`
    `    pub(crate) fn merged_order(&self) -> Vec<(usize, usize)> {`) still sorts by
    `:4581` `                let seq = surface.seqs.get(bi).copied().unwrap_or(0);`.
  - The bug: the ordinal is stamped only at **append** — `stamp_new` (`:4614`), and
    `:4619` `        if before >= len {` → `:4620` `            return;` — while a turn
    grows **in place**: `:2005` `                turn.content.extend(content);`
    (`absorb_message`) and `:2075` `            turn.tools.push(tool);`
    (`turn_start_tool`), each `bump_rev` (`:1622`) but never changing the length.
  - **The seam.** `fn touch_open_turn(&mut self, idx: usize, open: Option<(usize, u64)>)`
    (`:4660`) beside `stamp_new`: if the surface's open turn
    (`Surface::open_turn: Option<usize>`, `:1362`) is still open (`open_turn == Some(i)`)
    and its `rev` changed since the snapshot, assign it `next_seq` and bump.
  - **The call site.** The `AppEvent::Agent` arm (`:3881`–`:3902`): the snapshot
    `let open = self.surfaces[idx].open_turn.and_then(|i| self.surfaces[idx].block_revs.get(i).map(|r| (i, *r)));`
    (`:3891`) is taken **before** `apply` (`:3894`); then `self.touch_open_turn(idx, open);`
    (`:3901`) runs **before** `self.stamp_new(idx, before);` (`:3902`) — an in-apply
    growth precedes a same-apply append (the `Error`-arm fix).
  - **No `Surface` API change, no `push_on` change** (it only appends).
- Invariant: **an ordinal only ever increases** — `next_seq` is monotone (`:4630`
  `        self.next_seq = seq;`), so a re-stamp never collides with a later block.

## 5. Plan

1. Write the failing-first test `a_growing_turn_is_re_stamped_so_all_orders_it_last`
   (a root turn opened, a member block stamped after, the root turn grows again →
   `merged_order` == `[(1,0),(0,0)]`). **Deliverable** the red test; **gate** it fails
   on `HEAD`. ✔
2. Add `touch_open_turn` + the arm snapshot/call. **Deliverable** the helper;
   **gate** the test passes, `cargo test --workspace` green. ✔ (`628ba63`)
3. Fix the `Error`-arm order (swap `touch_open_turn` before `stamp_new`) + its
   failing-first test. **Deliverable** the swap; **gate** the new test red on
   `628ba63`, green after. ✔ (`ee4a3b7`)
4. Record D032 + the D031 forward pointer + the tracker row. **Deliverable** the
   docs; **gate** the record review. ✔

## 6. Quality gates

- `cargo build --workspace` → ok.
- `cargo test --workspace` → 1353 passed (the pre-existing `markdown` `SYNTAX_THEME`
  flake aside — `docs/next-steps.md:82`/`:1202`; `wcode-tui --lib` = 477 in isolation).
- `cargo clippy --workspace --all-targets` → clean.
- `grep -rn 'SKETCH\|todo!()' crates/` → none.

## 7. Testing

### 7.1 Automated
Six tests: `a_growing_turn_is_re_stamped_so_all_orders_it_last` (red on pre-fix code);
`a_turn_that_does_not_grow_keeps_its_ordinal` + `a_todo_push_does_not_move_an_open_turn`
(red when the rev guard is dropped); `a_sealed_turn_does_not_jump_past_the_run_end_notices`
(red when the still-open guard is dropped); `an_error_after_a_live_growth_keeps_the_turn_before_it`
(red on the pre-`ee4a3b7` call order); `focus_frame_is_byte_identical_when_a_member_grows`.

### 7.2 How a human verifies it
Run a team session (`cargo run -p wcode-cli -- --tui`, spawn a member), press
`Ctrl-O` (All). The orchestrator's turn — with its reply — must render **below** the
member turns it coordinated.

## 8. Expected outcome

A turn's position in the merged transcript reflects when it last changed, so the
last speaker is last. `next_seq` stays the single total order; the ordinal only
increases.

## 9. Log

- **2026-10-19** — opened from the user's report. Diagnosed the mechanism
  (`merged_order` sorts by the append-time ordinal; a `Block::Turn` is opened at a
  run's first content and then grows in place, so it kept its open ordinal). The human
  chose fix (A): re-stamp on content growth. `628ba63` implemented it
  (`App::touch_open_turn` after `stamp_new`); the second-layer review found the
  `Error`-arm ordering regression (a turn that grows via `flush_live` jumped past the
  `Block::Error` it preceded); `ee4a3b7` swapped `touch_open_turn` before `stamp_new`
  and added the failing-first test. D032 records the rule; D031 carries a forward
  pointer. **Not run:** a real-terminal team run (the ordering is proven by the
  `merged_order` unit tests + a headless frame render).

## F1 — the VS Code client has the same class of defect (non-scope, tracked)

`editors/vscode/src/render.ts:228` sorts by `seq`, stamped only at append
(`reducer.ts:535`); a `setBlocks` replace "keeps the seqs" (`reducer.ts:529`), so a
grown turn keeps its open `seq` there too. Its own `editors/` change.
