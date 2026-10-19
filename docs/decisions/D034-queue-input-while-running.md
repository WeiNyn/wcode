# D034 — Input typed mid-run is queued and run at `AgentEnd`, not discarded

- **Status:** accepted (human-locked for W029; shipped in `bcb1297`,
  follow-ups `f3ddef9`)
- **Date:** this session
- **Relates to:** [`crates/wcode-tui/src/app.rs`](../../crates/wcode-tui/src/app.rs)
  (`Surface.queue`, `submit`, `dequeue_next`, `begin_run`, `Action::SubmitTo`),
  [`crates/wcode-tui/src/lib.rs`](../../crates/wcode-tui/src/lib.rs) (the action
  dispatch), [`crates/wcode-tui/src/ui.rs`](../../crates/wcode-tui/src/ui.rs)
  (`corner_titles`, the drop ladder)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §2 (glyph table) and §4
  (foot grammar, drop ladder, notices). Supersedes the
  `a turn is already running` guard recorded in
  [`D018`](D018-tui-live-thinking-annotation.md) and [`D026`](D026-tui-info-modals.md).
- **Work item:** [`work/W029-queue-input-while-running.md`](../work/W029-queue-input-while-running.md)

## Context

A prompt submitted while a turn was in flight was **destroyed**.
`Surface::submit` took the text off the buffer at `app.rs:4057`
(`std::mem::take(&mut self.input)`) *before* the running guard, and the guard
then returned without using it (`app.rs:4081-4088`). The user lost the message
**and** the buffer — the worst of both.

The kernel was already correct: the actor defers a second `Submit` "becomes the
next run" (`actor.rs:79`). The REPL is serial by construction and never loses
input. Only the TUI discarded.

## Decision

**Queue it, TUI-locally.** `Surface.queue: VecDeque<String>` (per surface, beside
`running`). A mid-run `Enter` enqueues and acknowledges; the run starts at
`AgentEnd`.

**TUI-local, not the kernel's deferral.** The TUI's `running`/`live`/`open_turn`
bookkeeping assumes a *single* in-flight run; a second `AgentStart` before the
matching `AgentEnd` would clear `finished`/`failed` and flip `running` mid-run.
The queue starts exactly one run at a time, so that invariant holds.

**The start names the queue's OWN surface.** `Action::SubmitTo(SessionId, String)`
— because `Action::Submit` targets the *focused* surface, and a member's turn can
end while the root is focused.

**Acknowledgement is both momentary and persistent.** A transient
`queued · N waiting` (ordinary `/`-command chatter, retired at the next submit)
*and* a persistent footer count `⇥ N queued`. Neither alone suffices: the footer
is too quiet for a key the user pressed expecting an effect, and the transient
vanishes leaving no trace of a pending queue.

**Esc keeps the queue** — cancelling a run must not silently eat what was typed.
A second Esc (abandon) drops it with the process; an idle Esc with a pending
queue drops it with a notice. `/`-commands stay immediate (the `starts_with('/')`
branch precedes the guard).

**Two ordering rules.** `history.push` happens at **enqueue** time (the user
typed it once; `↑` must recall it even if the queue is later dropped), and
`dequeue_next` calls `retire_transient()` **before** its `Block::User` — the same
order `submit`'s idle path uses, so the acknowledgement cannot linger under a
prompt that has already started.

**Glyph `⇥` (U+21E5)** — East Asian Width *Neutral → deterministically one
cell*, so it cannot perturb the foot budget the way an emoji-presentation or
CJK-ambiguous glyph would. `dim`, not `accent`: the run's state chip owns the
line's single accent, and queued must read quieter than running. It is dropped
**last** among the droppable foot segments.

## Consequences

- The footer count is dropped only after every mode token (`↑ N` → gauge →
  `plan`/`browse` → `queued`), and a shed count was already announced by its
  transient — so it never vanishes silently.
- `queued_count()` reads the focused surface while the state chip reads the
  chrome surface (root, in All mode) — the pre-existing majority convention
  (`plan`/`browse`/gauge/`↑ N` are all focused). Not introduced here.
- `Action::SubmitTo`'s unknown-target arm drops silently. It is dead code today
  (`backends` is append-only; the App removes no surface), but a dropped
  `SubmitTo` would leave the App **armed** — `begin_run` already set
  `running = true` — with no `AgentEnd` coming. Named in the `lib.rs` comment so a
  future retirement path does not rediscover it as a hang.

## Known gaps (reviewer-identified, non-blocking)

- A dequeued `Block::User`'s **position** (below the sealed turn and any
  `⏹ aborted`/changes notices) is asserted only by the dump frame, not by a test.
- The plural drop notice (`n > 1`) and a dequeue following a **cancelled** run's
  `AgentEnd` are untested.
- The socket-client path with a mid-run queue was not exercised (the deferral
  lives in the actor, so it should behave identically).
