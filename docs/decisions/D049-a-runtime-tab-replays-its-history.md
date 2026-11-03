# D049 — A tab that arrives replays its history, off the loop, and only while it is clean

- **Status:** accepted — the human's ask (*"is it true that … resume no longer render old
  messages of that session"* → *"oke please fix this"*), briefed and approved in session as
  [`W049`](../work/W049-resumed-tab-replay.md), implemented in `073d15c` and approved at
  second layer.
- **Date:** this session
- **Relates to:** [`app.rs`](../../crates/wcode-tui/src/app.rs) (`AppEvent::Replayed`,
  `App::seed_replay`, `App::seed_history`, `App::handle`),
  [`lib.rs`](../../crates/wcode-tui/src/lib.rs) (`spawn_replay`, `accept_conversation`,
  `replay_history`)
- **Closes:** the gap **W046** opened (`091b28a`, *"tui: /resume opens the picked session as
  a conversation"*) and **W047** recorded in writing at
  [`work/W047-build-notices-belong-to-the-tab.md:184`](../work/W047-build-notices-belong-to-the-tab.md)
  — *"`ConversationEvent::Added` never asks for history and nothing else does"*
- **Nothing on the wire:** no new `Request`, no `ConversationSpec` field,
  `PROTOCOL_VERSION` stays 1; the CLI already answers `GetHistory` for a live root
  (`roots.rs:1538`-region)

## Context

W046 removed the last two re-execs from the TUI: `/new` and `/resume` now open a
**conversation tab** instead of quitting so the composition root can rebuild the process.
That is what let three tabs survive three `/new`s. It also quietly deleted a behaviour
nobody named at the time: a `/resume` pick used to end the process and come back as
`wcode --resume <path>`, whose startup path seeds every surface from `GetHistory`
(`lib.rs:488`). A tab does not: `ConversationEvent::Added` → `accept_conversation` →
`App::add_conversation` registered bodies and build notices and asked for nothing.

The result was a tab that was *blank but not empty*: the backend held the resumed
transcript and the next prompt continued from it correctly — only the rendering was
missing. W047 found this while shipping build notices, because its own ruling placed a
notice "above the replayed history", and for a runtime tab there was no history to be above.

## Decision

**A conversation that arrives at runtime replays its history on the same seam the startup
root uses — every surface it brought, off the loop, and only while the tab is still clean.**

Four clauses, each of which was a fork in the road:

1. **Every surface, not just the root.** The startup seam's D1 lesson (`lib.rs:483`,
   *"Seeding only the root left a member surface blank after a team resume"*) applies
   unchanged: a resumed group's members each hold their own transcript. `accept_conversation`
   asks inside the loop that already walks `spec.surfaces`, so root and members are covered
   by construction.
2. **Off the loop.** The ask rides a `tokio::spawn` sibling of `spawn_ask_within`, bounded by
   `ASK_TIMEOUT`, instead of an inline `await` in the select arm. An inline await is one line
   shorter and freezes input, draws and every other tab for up to 10 seconds on a
   hung-but-open backend — the exact shape `lib.rs:949` already refuses for `/usage`
   (*"Bound the ask so a hung-but-open backend cannot wedge the spinner"*). The startup seam
   keeps its inline await: it runs *before* the loop, so there is nothing to freeze.
3. **A reply that seeds, not one that prints.** `AgentEvent::History`'s reducer arm renders a
   `/usage` one-liner (`app.rs:2183`) — it is the answer to `Action::Ask(Request::GetHistory)`,
   a *different job* for the same wire event. A replay therefore gets its own app event,
   `AppEvent::Replayed(SessionId, Vec<AgentMessage>)`, and the caller decides which job it is.
   Reusing the `History` arm would have printed a usage line over a resumed transcript.
4. **Only while the tab is clean.** Because the ask is fire-and-forget, a reply can land
   after the user has already submitted on the tab. `App::seed_replay` refuses when the
   surface already holds a `Block::User` or `Block::Turn`, then delegates to the unchanged
   `seed_history`. Without the guard, `Surface::seed` would append old blocks *after* the
   live turn and insert the `⋯ N earlier message(s)` divider above the message it claims is
   earlier — a transcript that lies about its own order. Notices, transients, tool notes and
   the D5 empty-state hint are explicitly not "occupied": they are what a fresh tab holds.

A failed or unexpected reply stays **silent** — no `Block::Error`, no transient — matching
the startup seam (`lib.rs:487`, *"leaves its surface empty, as before"*). A replay is not a
run, and `error` red is reserved for a run that failed.

## Alternatives rejected

- **Inline `await` in the select arm.** Rejected on clause 2; would have been ~20 lines and
  one file.
- **Reusing `AgentEvent::History`.** Rejected on clause 3; it would need a side-channel to
  say which reply is which — more machinery than a variant.
- **Carrying the messages in `ConversationSpec`.** Rejected: it would put a transcript on
  the CLI→TUI wire type that already has a `Backend` handle able to answer, and would have to
  grow for a group's members. The backend is the owner of the transcript; `GetHistory` is
  the seam.
- **Restoring a re-exec for `/resume`.** Rejected: that is the behaviour W046 deleted to keep
  the tab set alive, and it would destroy every other tab.
- **Replaying `accept_member` too** (a worker spawned *during* a run). Unnecessary, and
  provably a no-op: a runtime worker is a fresh `Agent` with an empty context, so `GetHistory`
  answers `[]` and `Surface::seed` returns `false` at its first line (`app.rs:2449`) before it
  even retires the hint. The member that *does* have a transcript arrives through
  `ConversationEvent::Added`, which is covered.

## Consequences

- **W047's clause is now unconditional.** "A build notice sits above the replayed history"
  was true for tab 0 only; `push_build_notices` runs at accept time and the replay lands
  after, so the divider goes below the notice on a runtime tab too — asserted, not assumed
  (`a_tab_notice_sits_above_the_replayed_divider`).
- **W046's re-exec removal stops costing the user their transcript.** `/resume` shows the
  session it resumed, exactly as `--resume` at launch does.
- **Two seams, one owner.** `seed_history` remains the only writer of a replayed transcript;
  `replay_history` (startup) and `seed_replay` (runtime) differ only in when they run and in
  the clean-tab guard.
- **A future `RootSpec` variant inherits this silently.** The replay hangs off
  `ConversationEvent::Added`, so a *new* way to open a conversation gets a replay for free —
  correct for anything that carries prior history, and a no-op for anything that does not
  (clause 4's `vec![]` case is pinned by a test). Worth remembering when a third variant
  arrives.
- **A very large resumed session may show a one-frame blank tab** while its local
  `GetHistory` is in flight. Unmeasured; the ask is off the loop, so the frame draws and
  fills in.
