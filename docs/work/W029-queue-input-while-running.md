# W029 — queue input while a turn is running — brief

- **Status:** done — landed in `bcb1297`, second-layer APPROVED with the follow-ups folded in `f3ddef9`
- **Work item:** W029
- **Decisions:** locked by the human (queue TUI-locally; auto-dequeue on `AgentEnd`; Esc keeps the queue; `/`-commands stay immediate). This brief settles the six design questions.
- **Author:** session `agent:brainstormer`
- **Date:** 2026-07-05

## 1. The ask

Typed input submitted while a turn is in flight is **destroyed**, not queued. `Surface::submit` (`app.rs:4055`) takes the text off the buffer at `let atoms = std::mem::take(&mut self.input);` (`app.rs:4057`) **before** the running-guard, then the guard returns without ever using the text:

```rust
if self.focused().running {
    let idx = self.focus;
    self.push_on(idx, Block::Transient("a turn is already running — Esc to cancel".into()));
    return;
}
```
(`app.rs:4081-4088`). So the message is dropped *and* the buffer is already empty. The fix: queue it, and run it when the current turn ends. The kernel is already correct (it defers a second `Submit`, `actor.rs:79`), so this is a **TUI-only** change.

## 2. Scope / Non-scope

**In scope.**
- A per-`Surface` FIFO of pending prompts; `Enter` while running enqueues (and loses nothing).
- Auto-dequeue: the head starts as a normal run when the surface's `AgentEnd` fires.
- A queued item is **not** rendered as a transcript block until it starts (so ordering is trivial and no `block_lines` arm is added); the queue is shown as a foot-line count.
- Esc while running keeps the queue; a forced quit drops it; Esc while idle with a non-empty queue drops it with a notice.
- One new `Action` variant so a dequeue starts on the *surface that owns the queue*, not the focused one.

**Non-scope.**
- **Kernel/protocol changes** — none (see §7). The actor already defers a `Submit`, and `Backend::send` needs no new request.
- **Reordering/editing a queued item** (up/down/delete within the queue) — a later affordance; this change only enqueues, counts, drops, and starts.
- **Rendering a queued block in the transcript** — decided against (§3, Q3); the count + dequeue-time block suffice.
- **A queue in the REPL / one-shot / headless paths** — the REPL is serial by construction (`repl.rs:825-845`) and never loses input; nothing to queue.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P1 | `Surface.queue` + `submit` enqueue path | high | low | low | one field + replacing a discard return with a `push_back`; the text is already in hand |
| P2 | `AgentEnd` auto-dequeue (`App::dequeue_next` + `Action::SubmitTo`) | high | med | med | the start must target the queue's own surface; the focused-`Submit` dispatch (`lib.rs:586-588`) is the trap |
| P3 | Foot-line count + Esc-idle drop + docs | med | low | low | one `corner_titles` span and one `interrupt` branch; a designer-owned glyph |

## 4. Interface & structure

### 4.1 Added

- **`Surface.queue: std::collections::VecDeque<String>`** — on `Surface` (`app.rs:1347`), next to `running` (`app.rs:1370` `running: bool,`). Initialized empty in `Surface::new` (`app.rs:1466`). Holds the trimmed prompt text, FIFO.
- **`App::dequeue_next(&mut self, idx: usize)`** — the App-level start (the `Surface` cannot push an `Action`; `Surface::apply` returns only `bool`, `app.rs:1791`): pop the head, `push_on(idx, Block::User(text))`, set `running = true` / `finished = false` / `cancelled = false`, clear `changes`, push `Action::SubmitTo(id, text)`. It is the *same* body as `submit`'s idle path (`app.rs:4089-4097`), factored so the two cannot drift.
- **`Action::SubmitTo(SessionId, String)`** (`app.rs:115-134`) — mirrors the existing `Action::AskTo(SessionId, Request)` (`app.rs:131`), whose doc explains the target-not-focused contract. Needed because a dequeue can fire for a surface the user is not looking at.
- **`App::queued_count() -> usize`** — the focused surface's queue length, for the foot line.

### 4.2 Changed

**`crates/wcode-tui/src/app.rs`**
- `Surface::submit` (`app.rs:4055-4098`) — the running-guard arm (`app.rs:4081-4088`) becomes: push `text` onto `self.focused_mut().queue` (the transient "already running" notice is **removed** — the foot-line count replaces it), `self.dirty = true`, `return`. The `std::mem::take` at `app.rs:4057` stays (it feeds `text`); the bug is that the guard discards `text`, and now it does not.
- `App::handle` `AppEvent::Agent` arm (`app.rs:3872-3905`) — capture `let ended = matches!(&event, AgentEvent::AgentEnd);` before `apply` consumes it, and after `stamp_new` (`app.rs:3902`) call `if ended { self.dequeue_next(idx); }` guarded on a non-empty queue. This is the `AgentEnd` seam (`app.rs:1902-1917`).
- `Surface::interrupt` idle branch (`app.rs:4019-4023`) — when the focused surface is idle **and** its queue is non-empty, drop the queue and push a `Block::Transient`/`Notice` "dropped N queued message(s)".
- `App::handle` `AppEvent::Key` → `submit` routing is unchanged (`app.rs:3965` `Key::Enter => self.submit(),`).

**`crates/wcode-tui/src/lib.rs`**
- Action dispatch (`lib.rs:584-588`) — add a `Action::SubmitTo(id, text)` arm that resolves the backend by id (exactly like `AskTo`, `lib.rs:597-610`) and `backend.send(Request::Submit { text })`.

**`crates/wcode-tui/src/ui.rs`**
- `corner_titles` (`ui.rs:2018-2076`) — the `br` closure (`ui.rs:2039-2067`) gains a `⇥ {n} queued` span (dim) before `state` when `app.queued_count() > 0`. Bytes and glyph are the designer's call (see §9).

**`docs/tui-design.md`**
- §2 glyph table (`docs/tui-design.md:69-94`) — declare the queued glyph.
- §4 composer foot line (`docs/tui-design.md:346-354`) — add the queued segment to the `⏻ plan · ▤ browse · {state} · ↑ N` grammar.

### 4.3 Design questions — settled

1. **Where the queue lives.** `Surface.queue: VecDeque<String>` (§4.1), beside `running` (`app.rs:1370`). Per-surface because `running` is per-surface (`app.rs:1370`, cleared at `app.rs:1905`).
2. **How it renders.** Not as a block while queued. A foot-line count `⇥ N queued` (`ui.rs` `corner_titles`). On dequeue it becomes a real `Block::User` via `push_on` — so it is pushed **exactly once** (no double-push: nothing was pushed at enqueue time). Why TUI-local, not the kernel's `Submit` deferral: the TUI's `running`/`live`/`open_turn` bookkeeping (`app.rs:1357-1382`) assumes a **single** in-flight run; a second `AgentStart` before the matching `AgentEnd` (`app.rs:1894-1905`) would clear `finished`/`failed` and flip `running` mid-run — a second `AgentStart` cannot occur if the queue only starts one run at a time.
3. **Ordering.** Moot by construction: the `Block::User` is created at dequeue time, so it lands *after* the finished turn — nothing moves, nothing to re-stamp beyond the normal `stamp_new`.
4. **Esc while idle with a non-empty queue.** Drop the queue with a notice (§4.2). Under "auto-dequeue on every `AgentEnd`" this state is nearly unreachable, but defining it prevents an invisible stuck queue. (Second-Esc abandon, `app.rs:4016-4018`, is unchanged and drops the queue with the process.)
5. **Multi-surface.** A queue is per-surface; a dequeue starts a run on **that** surface only, so it may start while another surface runs — they are independent sessions (the same reason `app.rs:1370` is per-surface). Hence `Action::SubmitTo`, not `Action::Submit` (which targets the focused surface, `lib.rs:582`).
6. **Kernel/protocol.** None (§7).

### 4.4 Friction with the existing lifecycle (flagged)

- There is **no `fn finish`**; the "finish" lifecycle is the `AgentEnd` arm (`app.rs:1902-1917`) calling `flush_live` (`app.rs:2081`) then `seal_turn` (`app.rs:2043`). The dequeue must run **after** both — i.e. from `App::handle` after `apply` returns (`app.rs:3894-3902`), not inside `Surface::apply`, because starting a run needs an `Action` and the `Surface` has no action channel.
- `Action::Submit` targets the **focused** surface (`lib.rs:582-588`); a member's turn can end while the root is focused, so `SubmitTo` is required.
- `push_on` (`app.rs:4599`) keeps the four parallel vecs in step (`transcript`/`block_revs`/`cache`/`seqs`); the dequeue **must** go through `push_on` (not a raw `push_block`) so the `seqs` ordinal is stamped — cite `app.rs:4591-4607`.

## 5. Plan

1. **Queue field + enqueue.** Deliverable: `Surface.queue` (empty in `new`); `submit`'s running-guard enqueues instead of discarding. Gate: `enter_mid_run_queues_and_leaves_the_buffer_empty_but_loses_nothing`.
2. **Auto-dequeue.** Deliverable: `App::dequeue_next` + `Action::SubmitTo` + the `AgentEnd` hook + the `lib.rs` arm. Gate: `enter_mid_run_then_agent_end_runs_the_queued_text_exactly_once`.
3. **Esc/quit rules.** Deliverable: Esc-while-running keeps the queue; Esc-while-idle drops it with a notice; `abandon_run` drops it. Gate: `esc_keeps_the_queue`, `esc_idle_drops_the_queue_with_a_notice`.
4. **`/`-commands stay immediate.** Deliverable: no change to `app.rs:4077-4080` (`if text.starts_with('/')` runs before the running-guard). Gate: `a_slash_command_mid_run_is_not_queued`.
5. **Foot-line count + accessor.** Deliverable: `App::queued_count` + the `corner_titles` span. Gate: `queued_count_shows_on_the_foot_line` (a `render_plain` assertion).
6. **Docs.** Deliverable: `docs/tui-design.md` glyph + foot-line rows; a `docs/next-steps.md` row; the W029 work item. Gate: the design doc names the glyph; `cargo run -p wcode-tui --example dump` still writes its frames.

## 6. Quality gates

- `cargo test --workspace` → all pass.
- `cargo clippy --workspace --all-targets` → clean.
- `cargo test -p wcode-tui --lib` → the reducer tests pass (the primary gate for this change).
- **Known-flaky, diff-orthogonal:** `markdown::tests::a_rust_fence_has_multiple_distinct_styles` / `install_syntax_theme_swaps_the_process_theme` (`docs/next-steps.md` W021 residuals, a process-global `SYNTAX_THEME`); re-run before reporting a failure. The `wcode-tui --lib` run is exactly where it bites.
- **Regression tests:** see §11 for the one that must be REPLACED, not kept.

## 7. Automated tests (fail if queueing is wrong)

- `enter_mid_run_queues_and_leaves_the_buffer_empty_but_loses_nothing` — submit "first", then "second": `app.input()==""`, `queued_count()==1`, `take_actions()` has only the first `Submit` (no second run started).
- `enter_mid_run_then_agent_end_runs_the_queued_text_exactly_once` — after `AgentStart`+`AgentEnd` for the surface, exactly one new `Block::User("second")` appears and exactly one `Action::SubmitTo` is emitted; a **second** `AgentEnd` emits none (the queue drained).
- `a_slash_command_mid_run_is_not_queued` — submit "first", then `/help`: `queued_count()==0`, and the command's chatter is present (the `app.rs:4077` ordering is untouched).
- `esc_keeps_the_queue` — submit "first", queue "second", Esc (`interrupt`): `queued_count()==1` still.
- `esc_idle_drops_the_queue_with_a_notice` — with a non-empty queue and `running==false`, `interrupt`: `queued_count()==0` and a notice names the drop.
- `dequeue_targets_the_queue_s_own_surface` — two surfaces, a run ends on the non-focused one: the emitted action is `SubmitTo(<that id>, …)`, not `Submit`.
- `queued_count_shows_on_the_foot_line` — `render_plain` contains `⇥ 2 queued` only while the queue is non-empty.
- `a_multi_surface_dequeue_starts_while_another_is_running` — surface A running, surface B's run ends with a queued item: B starts; A's `running` is untouched.

### 7.2 How a human verifies it

Offline (no endpoint) — `examples/demo.rs` streams a scripted turn and is the fastest mid-run repro:

```sh
cargo run -p wcode-tui --example demo
# type a line, Enter  → the turn streams
# type a SECOND line, Enter while it is still running  → the foot line shows `⇥ 1 queued`
# the second prompt must appear as a ` YOU` block AFTER the first turn ends, and run.
```
Look for: `⇥ 1 queued` on the composer foot line, then a second ` YOU` block after the first exchange — never a bare "a turn is already running" and never a vanished prompt.

Live, against the repo's keyless endpoint (`.wcode/skills/live-verification/SKILL.md` §1), for a slow turn:

```sh
cargo run -p wcode-cli -- --tui --base-url http://localhost:11434/v1
# (or installed: wcode --tui --base-url http://localhost:11434/v1)
```
Ask for a long answer, then type a second prompt and Enter mid-stream; watch `⇥ 1 queued` and the eventual second ` YOU`.

Deterministic frame check — extend the `dump.rs` fixture (`examples/dump.rs:98-230`) with a surface that is `running` **and** has a queued item, then diff the frames:

```sh
cargo run -p wcode-tui --example dump     # writes target/tui-render/120x40.txt
```
Look for: the `120x40.txt` foot line carries the queued segment; the transcript shows the running turn but **no** queued block yet.

## 8. Expected outcome

- `Enter` while a turn runs enqueues the prompt; the buffer empties as today, but nothing is lost.
- When the run ends, the queued prompt starts as a normal run on its own surface — one ` YOU` block, one `Submit`, no double-push.
- The composer foot line shows `⇥ N queued` while a queue is non-empty, and nothing otherwise.
- Esc cancels the run but keeps the queue; a second Esc (abandon) or Esc-at-idle drops it, with the idle drop announced.
- `/`-commands typed mid-run still run immediately.
- No kernel or protocol file changes.

## 9. References

- `crates/wcode-tui/src/app.rs` — `Surface` (`:1347-1444`), `submit` (`:4055-4098`), the `AgentEnd` arm (`:1902-1917`), `App::handle` (`:3850-3912`), `interrupt` (`:4011-4026`), `push_on` (`:4599`), the `Block` enum (`:200-237`).
- `crates/wcode-tui/src/lib.rs` — the action dispatch and the focused-`Submit` trap (`:580-616`).
- `crates/wcode-tui/src/ui.rs` — `block_lines` (`:737-759`), `corner_titles` (`:2018-2076`).
- [`docs/tui-design.md`](tui-design.md) — §2 glyph table (`:69-94`), §4 composer foot line (`:346-354`).
- `crates/wcode-tui/examples/dump.rs` — the headless frame dumper; `examples/demo.rs` — the offline interactive demo.
- `crates/wcode-harness/src/actor.rs` — `:79` "another `Submit` | deferred, becomes the next run" (the reason no kernel change is needed).
- [`.wcode/skills/live-verification/SKILL.md`](../.wcode/skills/live-verification/SKILL.md) — the keyless-endpoint recipe.

## 10. Facts the author did NOT verify

- **The exact dequeue glyph/segment** (`⇥ N queued`) — the designer owns §2/§4; one is proposed, not fixed.
- **Whether the foot line's width budget drops the queued segment** — `corner_titles` drops least-important-first (`ui.rs:2014-2017`); the segment was not added to the drop ladder in the estimate of which level it joins.
- **A full live TTY run** — the reducer and examples were read; the TUI was not run and a real turn was not timed.
- **`esc_idle_drops` reachability** — argued nearly unreachable under "auto-dequeue on every `AgentEnd`"; the exact sequence was not constructed.
- **The `app.rs` line for `Key::Enter => self.submit()`** is `:3965` (verified by grep), but not every key that can reach `submit` was traced.

## 11. Orchestrator pre-review notes (corrected before sign-off)

Three facts in the draft above are wrong or incomplete. They do not change the design, but the plan must absorb them:

1. **`a_second_submit_while_running_is_refused` must be REPLACED, not kept.** The brief's §6 originally listed it as a regression test to keep green; it is not. It asserts the *discard* behaviour this work removes:
   ```rust
   // crates/wcode-tui/src/app.rs:5626
   fn a_second_submit_while_running_is_refused() {
       …
       assert!(matches!(
           app.transcript().last(),
           Some(Block::Transient(t)) if t.contains("already running")
       ));
   ```
   Step 1 rewrites it (new name, e.g. `a_second_submit_while_running_is_queued`) — it is the *proof* the bug is fixed, so deleting it outright would lose the pin.
2. **`push_on`'s doc comment must be edited too.** `app.rs:4591-4592` names the transient among `push_on`'s callers: "(`submit`'s `Block::User` and the "already running" transient, …)". Removing the transient orphans that sentence; §4.2's "the transient notice is removed" has a third site.
3. **`retire_transient()` still fires on a queued submit.** `app.rs:4075` runs before both the `/` dispatch and the running guard, so a mid-run `Enter` retires the previous generation's `/`-command chatter. The same question applies to `history.push` (`app.rs:4089`): does a queued prompt enter ↑-history at enqueue time or at dequeue time? The brief does not say. **Decision: enqueue time** (the user typed it once; ↑ should recall it even if the queue is later dropped).

## 12. Locked amendments (human's sign-off on §11 and on the visual seam)

**A1 — History at enqueue time.** `history.push(text)` happens in the enqueue branch, next to the `queue.push_back`, not at dequeue.

**A2 — The queued state gets BOTH a transient confirmation and the count.** §4.2's "the transient notice is removed" is superseded. A silent `⇥ N queued` footer alone is too quiet for a keystroke the user expects to have done something.

- **On enqueue:** `push_on(idx, Block::Transient(format!("queued · {n} waiting")))` — one line, retired by the next submit exactly like every other `/`-command chatter (D018). A second enqueue retires the first notice and lands `queued · 2 waiting`, so there is never a stack of them.
- **On dequeue:** `retire_transient()` on that surface **before** the `Block::User` push — the same rule `submit`'s idle path follows (`app.rs:4075`), so the "queued · 1 waiting" line does not linger under a prompt that has already started. `dequeue_next` must therefore mirror `submit`'s ordering, not just its body (§4.1).
- **Footer count stays** (`⇥ N queued`): the transient is the momentary acknowledgement, the count is the persistent state. They answer different questions and are not redundant.

**A3 — The glyph and the two strings are the designer's to bless.** `⇥`, `queued · N waiting` and the footer segment all enter `docs/tui-design.md` §2/§4, so the design doc is amended by the designer before the developer writes the render code (the `design-taste` skill's rule: never invent a glyph without a call site — here the call site exists, the spec entry does not). If the designer rejects `⇥`, only the glyph changes; the seam does not.

**A4 — `push_on`'s doc is part of the change** (see §11.2), and §6's "regression tests kept green" is void: `a_second_submit_while_running_is_refused` is rewritten by step 1.
