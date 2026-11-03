# W049 — A tab that arrives replays its history — brief

- **Status:** **SHIPPED** — three commits, second layer APPROVED twice (see §10, which
  includes the one check still open). Shape: **option B, off-loop reply-driven** (§4),
  approved by the human in session ("ok please fix this" after the size estimate).
- **Item:** W049 (W046, W047, D047 and D048 are closed; W041–W045 reserved per
  `docs/work/W040-multi-root-conversations.md` §14).
- **Predecessor / origin:** **W046** — commit `091b28a` *"tui: /resume opens the picked
  session as a conversation"* deleted `pending_resume` (the quit-then-`--resume`-re-exec
  hand-off) and routed the picker into `Action::NewConversation(RootSpec::Resume { dir })`.
  Re-exec used to land on the startup replay; a tab does not. The gap is named in writing
  by **W047** (`docs/work/W047-build-notices-belong-to-the-tab.md:184`, "The honest limit
  the review found (a W046-era gap, NOT a W047 defect)") and in `docs/next-steps.md`
  row 114's *Stated limit* — but it has no row of its own until this one.
- **Decisions:** proposes **D049** — *a conversation that arrives at runtime replays its
  history on the same seam the startup root uses, off the loop, and only while the tab is
  still clean.*

---

## 1. The ask

The human's ask, verbatim: *"is it true that recent improvement on wcode multi
conversation on TUI make resume no longer render old messages of that session"* — then
*"oke please fix this"*.

The answer was **yes, for the in-process `/resume` only**: `wcode --resume <path>` at
launch still replays (`crates/wcode-tui/src/lib.rs:460`), while a `/resume` pick inside a
running TUI opens a tab whose transcript is blank even though its backend holds the
transcript and continues from it correctly.

---

## 2. Scope / Non-scope

### 2.1 In scope

1. **A runtime arrival asks its backend for history.** `ConversationEvent::Added`
   (`lib.rs:613`) seeds every surface it brought — root **and** members — from
   `Request::GetHistory`, mirroring the startup seam's D1 lesson
   (`lib.rs:483`-`:496`).
2. **Off the loop.** The ask rides the existing `spawn_ask` machinery
   (`lib.rs:910`/`:922`) rather than an inline `await` in the select arm: a wedged
   backend must not freeze input and draws for `ASK_TIMEOUT` (10s, `lib.rs:894`). This is
   the repo's stated rule — `lib.rs:933`, *"Bound the ask so a hung-but-open backend cannot
   wedge the spinner"*.
3. **A reply that seeds, not one that prints usage.** `AgentEvent::History` reaching the
   reducer renders a `/usage` one-liner (`app.rs:2178`-`:2181`); a *replay* reply must call
   `App::seed_history` (`app.rs:6861`) instead. So the replay gets its own app event rather
   than sharing `AgentEvent::History`'s arm.
4. **The clean-tab guard.** A late reply must not interleave old blocks under a live turn.
5. **Notices stay above the divider.** W047's clause becomes true for a runtime tab:
   `add_conversation` pushes notices at accept time (`app.rs:3118`-`:3121`) and the replay
   lands after, so `Surface::seed`'s `insert_block(start, …)` (`app.rs:2450`, `:2503`)
   puts the `⋯ N earlier message(s)` divider below them — the startup ordering
   (`set_conversations` → notices, `app.rs:3035`-`:3048`; then `replay_history`) is
   reproduced exactly.
6. **Tests** (§7) and one ADR (D049).

### 2.2 Non-scope (each names something a reasonable reader would assume is in scope)

- **`/new` / `/tab new`.** A `Fresh` root has no history; nothing to replay. Unchanged.
- **The startup replay.** `replay_history` keeps its inline `await` — it runs *before* the
  loop, so there is nothing to freeze. Only the *runtime* path moves off-loop. Sharing the
  `spawn_replay` emitter with startup is explicitly **not** required.
- **Prompt-history sharing across tabs** (W046 §14 item 4: `Options.history` is one path
  per process). A different gap, deliberately untouched.
- **`--resume`'s path canonicalization** (W046 §14 residue). Untouched.
- **The kernel, the protocol, the CLI.** No `GetHistory` change (it already answers for a
  runtime root — `crates/wcode-cli/src/roots.rs:1540`), no new `Request`, no
  `PROTOCOL_VERSION` bump, no `ConversationSpec` field. `RootSpec::Resume` already loads
  the session's messages into the agent (`roots.rs:855`-`:897`), so the backend's answer is
  the transcript.
- **A visible failure for a failed replay.** `replay_history` ignores a backend that
  errors or answers something else (`lib.rs:490`, *"leaves its surface empty, as before"*).
  The runtime path matches it: no `Block::Error`, no transient. A replay is not a run.
- **Seeding the socket client.** `/resume` is refused over a socket (`app.rs:6464`), and a
  socket client's `feeds.incoming` is `None` (`main.rs:1223`), so the arm cannot fire.
- **Re-ordering or re-rendering the transcript** (scroll position, browse mode, the
  expand/collapse state of replayed tools). `seed` already handles all of it
  (`app.rs:2443`-`:2507`) and is used unchanged.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|---|---|---|---|---|
| W1 | `AppEvent::Replayed` + its reducer arm (`app.rs`) | high | low | low | The seam (`seed_history`) exists; the arm is 3 lines and the guard is a predicate over `Block`. |
| W2 | `spawn_replay` (`lib.rs`) | high | low | low | A 15-line sibling of `spawn_ask_within`, which already owns the timeout + error mapping. |
| W3 | Wiring `reply_tx` into `accept_conversation` | medium | low | low | One parameter, one call site (`lib.rs:614`), and `reply_tx` is already in scope at `lib.rs:564`. |
| W4 | Tests (§7) | high | medium | medium | The load-bearing one must redden when the replay loop is deleted; the guard test must redden when the predicate is removed. |
| W5 | Live pty check | high | low | low | `/resume` a real multi-turn session: old turns render, notice above the divider. |

Net expected diff: **~40 lines of production code across two files, ~110 test lines**, plus
docs. No kernel/CLI/protocol change.

---

## 4. The interface, and where it plugs into real code

### 4.1 A new app event — `crates/wcode-tui/src/app.rs`

Beside `Agent` (`app.rs:94`-`:109`), whose doc says a fact from a session is tagged with the
surface it belongs to. `AppEvent` derives `Clone, Debug` (`app.rs:93`) and nothing else, so
the payload may be owned.

```rust
/// A **replay** reply: the transcript a surface's backend already holds, sent
/// when its conversation arrives (`spawn_replay`). Distinct from
/// `Agent(_, AgentEvent::History)`, whose arm renders a `/usage` one-liner
/// (`app.rs:2178`) — the same wire event, two jobs, so the caller decides.
Replayed(SessionId, Vec<AgentMessage>),
```

Reducer arm in `App::handle` (`app.rs:4546`):

```rust
AppEvent::Replayed(id, messages) => self.seed_replay(&id, &messages),
```

### 4.2 The guard — a new `App` method, beside `seed_history` (`app.rs:6861`)

```rust
/// Seed a **runtime** replay, but only while the tab is still clean: a reply
/// that arrives after the user has already submitted would otherwise splice
/// old blocks under a live turn and insert the `⋯ N earlier message(s)`
/// divider mid-transcript. "Clean" = no `Block::User` and no `Block::Turn`
/// (notices, transients and the D5 hint are all fine — they are what a fresh
/// tab legitimately holds).
fn seed_replay(&mut self, id: &SessionId, messages: &[AgentMessage]) { … }
```

A reply for a surface that has since been closed is already safe: `seed_history` looks the
id up and no-ops when it is gone (`app.rs:6862`).

### 4.3 The emitter — `crates/wcode-tui/src/lib.rs`

A sibling of `spawn_ask_within` (`lib.rs:922`), same timeout, same error mapping, but it
sends `AppEvent::Replayed` and **stays silent** on anything that is not
`Ok(AgentEvent::History { messages })` (matching `replay_history`'s
*"leaves its surface empty, as before"*, `lib.rs:487`).

### 4.4 The wiring — `accept_conversation` (`lib.rs:786`)

Gains `reply_tx: &mpsc::UnboundedSender<AppEvent>`; after the existing body/backend loop
(`lib.rs:800`-`:803`) and **only when `add_conversation` returned `true`** (a refused
conversation has no bodies to seed), it asks every wired surface:

```rust
for (id, backend) in &backends_just_wired { spawn_replay(id, backend, reply_tx); }
```

Call site: the `ConversationEvent::Added` arm, `lib.rs:613`-`:615`. `reply_tx` is bound at
`lib.rs:564` and drained by the `replies.recv()` arm (`lib.rs:624`) — no new channel.

**Friction worth stating:** the ask is fire-and-forget, so the tab draws blank for the
duration of one local `GetHistory` (sub-millisecond in practice, but not zero). The
alternative — inline `await` — is one line shorter and freezes the loop for up to 10s; the
repo has already ruled against that shape once (`lib.rs:933`).

---

## 5. Plan

1. **`tui:` `AppEvent::Replayed` + `App::seed_replay` + the reducer arm** (`app.rs`).
   Gate: `a_replayed_history_seeds_the_tab_it_names`, `a_replay_after_a_live_turn_is_dropped`.
2. **`tui:` `spawn_replay` + `accept_conversation` asks every arriving surface** (`lib.rs`).
   Gate: `an_arriving_conversation_replays_every_surface` (mutation: replay only the root →
   reddens), `an_arriving_conversation_that_was_refused_asks_nothing`.
3. **`tui:` the ordering assertion** — a notice pushed at accept time sits above the
   replayed divider. Gate: `a_tab_notice_sits_above_the_replayed_divider`.
4. **`docs:` D049 + the W049 record + row 117.** Gate: `docs/next-steps.md` row 117 points
   at this file; zero `SKETCH` markers in `crates/` (this item adds none).

One commit per step, area-prefixed. The human's sign-off on the shape is recorded in the
status line above.

---

## 6. Quality gates

```
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets
grep -rn "==== SKETCH" crates/      # must not grow (currently 2, both labelled)
```

Plus the live check in §7.2 — `cargo build` proves nothing about behaviour.

---

## 7. Testing

### 7.1 Automated

| test | file | asserts | reddens if |
|---|---|---|---|
| `an_arriving_conversation_replays_every_surface` | `lib.rs` | `accept_conversation` over a 2-surface spec yields an `AppEvent::Replayed` for **both** ids, carrying each backend's own messages | the loop replays only `spec.surfaces.first()` (the exact D1 bug `lib.rs:1093` guards at startup) |
| `an_arriving_conversation_that_was_refused_asks_nothing` | `lib.rs` | a spec for a non-live conversation emits no `Replayed` and pushes no backend | the ask is hoisted above the `add_conversation` check |
| `a_replayed_history_seeds_the_tab_it_names` | `app.rs` | `AppEvent::Replayed(id, …)` puts the `Block::User`s on **that** surface and no other | the arm seeds the focused surface instead of the named one |
| `a_replay_after_a_live_turn_is_dropped` | `app.rs` | a `Block::User` already present ⇒ transcript unchanged, no divider | the guard is deleted |
| `a_tab_notice_sits_above_the_replayed_divider` | `app.rs` | `add_conversation`'s notice precedes the `⋯ N earlier message(s)` divider in `transcript()` | the replay is moved before `push_build_notices` (W047's clause) |

Each is a behaviour assertion on `transcript()`, never on a counter or a presence flag.

### 7.2 Human / live

Against a real binary on a pty, with a session that has ≥3 turns:

1. `wcode` → `/resume` → pick that session → **the old turns render**, the divider reads
   `⋯ N earlier message(s)`, and any `team: …` notice sits **above** it.
2. The previous tabs are still in the band and still show their own transcripts.
3. A follow-up prompt on the resumed tab continues from the old context (the model does not
   restart) — proving the display and the backend agree.
4. `wcode --resume <path>` at launch is byte-identical to before.

---

## 8. Expected outcome

A `/resume` picked inside a running TUI shows the session it resumed, exactly as
`--resume` at launch does; the tab's build notices sit above the replayed prefix; and the
one place W047 had to qualify its own ruling ("true for tab 0 only") becomes true
unconditionally. W046's re-exec removal stops costing the user their transcript.

---

## 9. Facts I did not verify

- **The fix was not compiled or run** when this brief was written; every line estimate is a
  reading of the code, not a measurement.
- **`ASK_TIMEOUT` in practice** — I did not measure how long a local `GetHistory` on a large
  session takes; "sub-millisecond" is an inference from the local-actor path, not a
  measurement.
- **Whether an inline `await` compiles in the `select!` branch body** (`lib.rs:612`) — I
  believe it does, but it is moot under the chosen shape.
- **How many review rounds this takes.** W046 and W047 each took two blocked rounds; the
  line count is not the cost.

---

## 10. Outcome

**Shipped:** `073d15c` (`AppEvent::Replayed` + `App::seed_replay` + its guard,
`spawn_replay`, and `accept_conversation` asking every surface it wired), with
`6d92d6e` (the twins' assertions: the replayed block itself in the ordering test,
`an_empty_replay_is_a_no_op`) and `f95eb04` (the resumed root really carries its
session — `a_resumed_root_answers_get_history_with_its_session`).

### §7.2, item by item

| # | Claim | Status |
|---|---|---|
| 1 | `/resume` → pick → the old turns render, the divider reads `⋯ N earlier message(s)`, the tab's notice sits above it | **verified** — live pty against a real binary |
| 2 | the other tabs are still in the band with their own transcripts | **verified** — live pty |
| 3a | the backend half: a resumed root answers `GetHistory` with the session's own turns | **verified** — automated, `crates/wcode-cli/src/roots.rs`'s `a_resumed_root_answers_get_history_with_its_session` |
| 3b | the round trip: a follow-up prompt on a resumed tab **continues from the old context** | **NOT verified** — see below |
| 4 | `wcode --resume <path>` at launch is unchanged | **verified** — live pty |

### The one thing still unverified (§7.2 item 3b)

**No prompt was ever sent to a model from a resumed tab, so it is unproven in
practice that the reply continues from the old context rather than restarting.**
No paid API call was made and there is no keyless endpoint on this machine; every
statement below is about the code path, not a measured round trip.

What *is* proven is the seam both halves draw from: the resume loads the session's
messages into the agent (`build_root`, `roots.rs:884` — pinned by the test above),
the replay asks a **clone of the same `Backend` handle** that the loop later
dispatches `Action::Submit` to, and `replay_history` seeds the display from that
one answer. Display and backend therefore cannot disagree beyond the accepted
drop-on-submit window — a *structural* argument, not an observation.

The residual risk is also not new to W049: `wcode --resume` at launch has loaded
its context through that same line all along, and this item changes only *when*
the client asks for the history, not what the agent was built with. A defect here
would be a pre-existing resume defect, not a regression from this change.

**To close it:** one paid run — `wcode`, `/resume` a session with ≥3 turns, then a
follow-up that can only be answered from the resumed context (e.g. *"restate my
first question verbatim"*), with the reply quoting it. Until that is recorded, this
document does not claim the round trip works.

### 10.1 Two further limits the second layer named (recorded, not waived)

1. **3a is proven for the flat/legacy resume only.** `a_resumed_root_answers_get_history_with_its_session`
   pins `roots.rs:884` (`let context = session.messages();`, the flat-file branch). The
   **group** branch takes its context from `session_groups::load_root` (`roots.rs:863`-`:864`)
   and is covered only by `a_resumed_team_says_it_restored` (`roots.rs:2046`), which asserts
   the `restored` notice — not the loaded messages. So "the backend half is proven" is exactly
   true of a legacy resume and *unproven* for a team resume; the live pty check (§7.2 item 1)
   did exercise a group (269 restored member messages rendered), which is evidence, not a test.
2. **The group branch is the one place a future regression would hide silently** — a
   `load_root` that returned an empty context would blank a resumed team's transcript with
   every gate still green. A test asserting the group root's `GetHistory` is the obvious
   follow-up; it was out of this item's scope.

### Residue (non-blocking, from the delta review)

- `roots_in(tx.clone(), 220)` in the new test reuses the id block of
  `a_resume_of_a_path_that_is_neither` (`roots.rs:2077`), against `roots_in`'s own "each test
  owns a disjoint id block" contract. Harmless today (the neighbour builds nothing) and the
  file already tolerates two duplicates at base 200; a free block would make the convention
  honest.
- `idle_backend()` (`app.rs:13172`) re-derives `backend_streaming` (`lib.rs:1108`); a
  `#[cfg(test)] pub(crate) mod test_support` would stop the two `AgentConfig` literals drifting.
