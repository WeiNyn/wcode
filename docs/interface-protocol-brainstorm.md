# Interface & protocol — brainstorm

Status: **landed through S2; S3 (TUI) P0–P3d shipped; S4-1/2/3 landed; S4-4
transport + outbound landed, reply path next** (see §14).
Companion to [`tui-plan.md`](tui-plan.md) and the deferred "inter-agent
communication protocol" note in
[`skills-references-plan.md`](skills-references-plan.md).
This doc asks one question and follows it where it goes:

> Can the TUI's client/kernel seam and the multi-agent communication seam be
> **one protocol**?

Read it as a design discussion to review, not a spec to implement. §13 is the
decision list; §14 is a staged path; §15 is the recommendation.

---

## 0. The prompt, restated

- We are reworking the interface toward a full-screen TUI, in the spirit of
  jcode (`../jcode`).
- We are *also* thinking about multi-agent communication (agents talking to
  agents).
- We want **one implementation** for both: "the same protocol be used for
  those."
- "Communication is, at the end of the day, an interface."

The last line is the whole thesis. A TUI talks to the agent through an
interface; an agent talks to another agent through an interface; if those are
the *same* interface, we build it once.

---

## 1. The thesis (one sentence)

**A session is a peer with a mailbox.** The TUI, another agent, and a script
are all just peers holding addresses. So the client protocol and the
inter-agent protocol are the *same* protocol — a request in, an event out — and
the only optional layers are **transport** (in-process channel → socket) and
**routing** (direct → bus).

If that holds, "TUI rework" and "multi-agent comms" stop being two projects.
They become one project with two front doors.

---

## 2. What we actually have today

wcode already has most of the *parts* of a protocol — they are just asymmetric
and unnamed.

| Today | File | Shape |
|-------|------|-------|
| **Outbound facts** | `wcode-harness/src/event.rs` | `AgentEvent` — a typed, already-`Serialize`/`Deserialize` enum streamed over a `tokio::mpsc::UnboundedSender`. The doc literally calls it "the UI seam". |
| **Inbound messages** | `agent.rs` | `run(prompt, sink)` + `steer(AgentMessage)` + `follow_up(AgentMessage)` — three ad-hoc entry points. |
| **Inbound commands** | `wcode-cli/src/repl.rs` | A separate `Command` enum (`/model`, `/effort`, `/compact`, `/resume`, …) parsed from strings, handled inline in the REPL loop. |
| **Durable history** | `wcode-harness/src/session.rs` | `Session` = append-only JSONL of `SessionEntry` (`Header`, `Message`, `ModelChange`, `EffortChange`, `Compaction`). |
| **The client** | `wcode-cli/src/repl.rs` | `print_events(rx)` drains the sink; the loop reads stdin lines. |

Observations:

- The **outbound** half is already a protocol: typed, serializable, versioned by
  serde tags. It is *exactly* what a wire protocol is made of.
- The **inbound** half is not: it is a method (`run`), two channels
  (`steer`/`follow_up`), and a UI-only command enum. There is no single inbound
  type.
- There is **no addressing** (a client cannot name a session), **no correlation**
  (nothing pairs a request with its reply), and **no framing** (the seam is an
  in-process channel; nothing survives a process boundary).
- `Session` is *already an event-sourced log*. The live event stream and the
  persisted history are two views of the same thing.

So the unification work is mostly: **give the inbound half the same shape the
outbound half already has**, then decide where the seam is allowed to cross a
process boundary.

---

## 3. What jcode teaches (findings, by name)

jcode is the mature reference. The relevant lessons, not the whole system:

1. **One framing, two directions.** `jcode-protocol` defines `Request`
   (client→server, `wire.rs`) and `ServerEvent` (server→client) as two
   internally-tagged serde enums, newline-delimited JSON over a Unix socket.
   Everything — chat, model switching, cancellation, *and* agent comms — rides
   those two enums. (Crate doc: "Main socket: TUI/client…; Agent socket:
   Inter-agent communication (AI-to-AI)".)

2. **The client transport is an abstraction, not a wire.** In
   `jcode-tui/src/tui/backend.rs` the TUI speaks `Request`/`ServerEvent` to a
   **backend** that may be a *local harness* or a *remote connection*
   (`RemoteConnection`). The same message-processing path runs in-process or over
   a socket. The TUI never special-cases local vs remote. This is the single most
   important idea to steal.

3. **A curated, versioned public API on top of the big internal one.**
   `jcode-harness-api` is "the *public* boundary between the harness and any UI
   (TUI, desktop, web, scripts)". Every frame is NDJSON; every frame carries
   `v` (major version); `ClientFrame { v, id, request }` /
   `ServerFrame { v, reply_to, event }`; every request/event carries a
   `session_id`; both enums have an `Unknown` catch-all so old clients survive
   new server events; additive changes bump minor, breaking changes bump major
   and are negotiated in a `Hello` handshake. It ships with schema-snapshot and
   capability-coverage tests. It even ships a `harness_repl` example — a second
   client, beyond the TUI.

4. **Addressing by `session_id` is the whole addressing scheme.** A client
   attaches to a session (`AttachSession { session_id }`) and then every
   streaming event carries the `session_id` back. Multi-session is "one
   connection, many `session_id`s" (`MULTI_SESSION_CLIENT_ARCHITECTURE.md`,
   phase 2 "multiplexed client protocol").

5. **Sessions are server-owned runtimes; clients are attachments.** A session is
   not a window or a process — it is history + model state + tool state +
   persistence + background tasks. A *surface* is a client-side view of a
   session. → `session = server-owned runtime`, `surface = attachment`,
   `client = container of surfaces`.

6. **Soft interrupts are the A2A delivery primitive.** `SoftInterrupt`,
   `CommDeliveryMode { Notify, Interrupt, Wake }`, and
   "notifications are queued as soft interrupts and injected into running agents
   at safe points" (`SWARM_ARCHITECTURE.md`). That is *exactly* wcode's `steer`
   (inject at turn start) and `follow_up` (run after stop). Our kernel already
   speaks the vocabulary; it just does not name it.

7. **The A2A model, minimized.** From `SWARM_ARCHITECTURE.md` +
   `SWARM_TASK_GRAPH.md`: only the root spawns (bounded fan-out); the
   spawn/parent edge is encoded by `report_back_to_session_id` and a tree is
   reconstructed from it; ownership = "you may stop/expand only your subtree";
   broadcast is subtree-scoped, DMs are the preferred exception channel; there
   are no locks — conflicts are resolved by DM; the newer direction demotes
   agents to **fungible workers over a task DAG**. Also present: plan/task ops,
   channels, shared context keys — the parts we would *not* take.

The takeaway: jcode did not invent a separate "communication protocol". It made
the session's own request/event interface the communication protocol and pointed
other sessions at it. That is the design to copy.

---

## 4. The model: peer + mailbox

Reframe the kernel around two nouns.

- **Peer** — anything with an address that can send and receive. The human
  client, a session, a future subagent, a script.
- **Mailbox** — a peer's interface: an **inbox** that accepts *requests*, and an
  **outbox** (a subscribable stream) that emits *events*.

```
        ┌───────────────── Session (peer) ─────────────────┐
        │  inbox:   Request stream                          │
        │     Submit · Steer · FollowUp · Cancel · Command  │
        │  outbox:  Event stream  (AgentEvent today)        │
        │     MessageStart/Update/End · Tool* · Turn* · …   │
        └───────────────────────────────────────────────────┘
                             ▲            │
              Request ───────┘            └─────── Event
                             │            │
        ┌────────────┐   ┌───────────┐   ┌───────────┐
        │ TUI client │   │ agent B   │   │ script    │
        │ (peer)     │   │ (peer)    │   │ (peer)    │
        └────────────┘   └───────────┘   └───────────┘
```

The TUI is peer #1. Agent B is peer #2. The difference between them is only
*which address they hold* and *how they render the event stream* (a terminal vs
another model's context). Nothing in the protocol distinguishes "a human told me
X" from "another agent told me X" — and that is the point.

Concretely, this means turning `Agent` into a long-lived **session actor**:

```
Agent (owns ctx/model/session)  ──►  SessionActor::spawn(agent) -> Handle
Handle::send(Request)                 // inbox
Handle::subscribe() -> EventStream    // outbox, N subscribers
```

`Agent::run()` becomes "handle `Request::Submit`, then drive the loop"; it stays
as a thin convenience for one-shot and tests. `steer`/`follow_up` become
`Request::{Steer, FollowUp}`. The REPL's `Command` enum becomes `Request`
variants. The loop already *is* a reducer over this mailbox; today it just has
four doors instead of one.

---

## 5. The protocol: one shape

Design one wire shape that works in-process *and* across a socket, so nothing
has to change when the transport does.

### 5.1 Two enums, one framing

- `Request` — inbound intent (the missing half).
- `Event` — outbound fact. `AgentEvent` **is** this type; keep the existing
  name/shape and treat it as the canonical event enum.

Both are internally tagged (`#[serde(tag = "type", rename_all = "snake_case")]`),
which is already how `AgentEvent` serializes. NDJSON framing is then trivial:
one JSON object per line, `serde_json` in, `serde_json` out.

### 5.2 The envelope

A thin envelope carries the four things a bare payload cannot:

```rust
struct Frame<P> {          // P = Request or Event
    v: u32,                // protocol major version
    id: u64,               // sender-assigned, monotonic per connection
    reply_to: Option<u64>, // correlation: which request this answers
    session: SessionId,    // addressing: which session this concerns
    #[serde(flatten)]
    body: P,
}
```

- **version** — so a newer server can talk to an older client (`Unknown`
  catch-all on both enums, like jcode's).
- **id / reply_to** — so A2A request/response works, and so a client can pair
  answers with asks.
- **session** — the entire addressing scheme. Reuse the id already written in
  `SessionEntry::Header`. The human is a well-known peer (`"user"`); a client is
  `"client:<n>"`; peers are `"agent:<session_id>"`.

Keep separate `Request`/`Event` enums rather than one mega-enum: the direction is
a type-level invariant, and it is exactly jcode's split. (The envelope is generic
over direction.)

### 5.3 The request surface (generalizes today's API)

| Today (wcode) | Unified `Request` | Notes |
|---|---|---|
| `Agent::run(prompt, sink)` | `Submit { text, images? }` | starts a turn |
| `Agent::steer(msg)` | `Steer { content, urgent? }` | soft interrupt at next turn start |
| `Agent::follow_up(msg)` | `FollowUp { content }` | runs after the loop would stop |
| `Agent::cancel()` | `Cancel` | maps to the cancel token |
| REPL `Command::{Model,Effort,Compact,Resume,…}` | `SetModel`, `SetEffort`, `Compact`, `Resume`, … | the command enum lifted into the protocol |
| `Agent::messages()` | `GetHistory` | reply: `Event::History` |
| `Session` open/list | `ListSessions`, `AttachSession`, `ForkSession` | multi-session TUI |
| (nothing) | `Notify`/`Interrupt`/`Wake`, a peer addressed by the router | A2A — delivered to a peer |

The `Event` side is what `AgentEvent` already carries, plus request replies
(`History`, `Ack`, `Error { reply_to }`) and, for A2A, `MessageReceived { from }`.

### 5.4 The seat we would grow into A2A

A2A is then *not a new protocol*: it is `Request::Steer`/`Ask` addressed at
another session's mailbox, delivered as a `Message` at a safe point, answered by
that session's `Event` stream. Delivery modes map cleanly:

- `Notify` → append to context, no turn (a `Submit { no_reply: true }`).
- `Interrupt` → `Steer` at the next turn boundary.
- `Wake` → `FollowUp` (run even if idle).

We already own all three semantics in the kernel; only the naming and the
addressing are new.

---

## 6. Where the unification is literal

The sentence "the same protocol for TUI and A2A" becomes mechanically true:

| Concern | TUI | Agent-to-agent |
|---|---|---|
| Encode | `Frame<Request>` | `Frame<Request>` |
| Decode | `Frame<Event>` | `Frame<Event>` |
| Address target | `session` = the session it views | `session` = the peer it talks to |
| Framing | NDJSON / in-proc channel | NDJSON / in-proc channel |
| Reconnect/replay | subscribe to the event log | subscribe to the peer's event log |
| Auth/scope | (none, local) | ownership tree from `report_back_to` |

The TUI's *backend abstraction* (jcode lesson #2) is therefore also the A2A
transport. Build the local backend and the remote backend once; both the TUI and
agents use them.

---

## 7. Where the code lives

Keep the kernel's *interface types* in the kernel; keep *transport* above it.
This preserves "the kernel owns the loop; presentation is a client" and keeps
the kernel dependency-light.

- **`wcode-harness`** — owns `Request` and `Event`/`AgentEvent` (the interface is
  the kernel's contract), plus the optional `SessionActor`/`Handle`. No sockets,
  no framing assumptions beyond serde.
- **`wcode-protocol`** (new, thin) — NDJSON framing, the generic `Frame`, a
  `Client`/`Server` helper, socket paths. Depends on `wcode-harness`. This is the
  reusable transport both the TUI and A2A sit on. Mirrors `jcode-transport` +
  the framing half of `jcode-harness-api`.
- **`wcode-tui`** (new, per `tui-plan.md`) — the peer that renders Events and
  sends Requests. Depends on `wcode-protocol`, **not** on `Agent` directly. This
  is what makes it "just another client."
- **`wcode-cli`** — composition root: builds the agent, decides local vs socket,
  and runs either the REPL or the TUI.

Do we need the *curated stable API* split (jcode's `jcode-harness-api`)? Only
when we have an external client to keep stable. Recommendation: **don't fork a
second enum now.** One `Request`/`Event` pair with an `Unknown` catch-all is
enough; add a curated facade later if a third-party client appears. (jcode needed
it because they ship an SDK and an iOS app; we do not.)

---

## 8. Transport: three shapes, one type

The envelope is designed so these are interchangeable behind the backend:

1. **In-process channel** (today). `Handle::send(Request)` / `subscribe()`.
   No serialization required; the actor can even pass typed values directly.
   This is "transport #0" and proves the seam with zero network.
2. **Per-session Unix socket** (jcode's model). `wcode serve` owns sessions; a
   client connects, speaks NDJSON `Frame`s, reconnects on drop, resumes by
   `session` id. This unlocks multi-session TUI and remote/`--remote-working-dir`.
3. **Bus / router** (A2A's model). A registry maps addresses → mailboxes;
   routing, fan-out, subtree scope, and presence live here. A session is just
   another address. Only build this when there is a second *agent*.

Key point: **#2 and #3 share everything below the router.** A session reachable
by a TUI over a socket is reachable by another session over the same socket. If
we build #2 for the TUI, A2A is "add addressing + a registry", not "add a
protocol".

Shadowing/backpressure: the outbox wants N subscribers. `tokio::sync::broadcast`
is enough in-process (slowest subscriber lags/drops). Over a socket, give each
subscriber its own channel and **replay from the session log on connect** — which
is free, because `Session` is already the log.

---

## 9. Event-sourcing closes the loop

`Session` (JSONL `SessionEntry`) is an append-only log of typed facts. The live
`Event` stream is derivable from the same facts. So:

- **Replay = attach.** A TUI attached to a finished session replays `Event`s
  from the log; the renderer has no separate "history mode".
- **A2A transcript = the log.** Add `SessionEntry::{Sent, Received}` carrying a
  `Frame`; then a multi-agent run is one auditable, replayable transcript, and
  the same viewer shows it.
- **Golden tests.** Because request/event are serde, a test can feed `Request`s
  and assert on `Event`s — no terminal, no socket. This is the "pure reducers"
  idea from `tui-plan.md` §7 applied one layer down: the actor is the reducer.

One concrete wrinkle to fix while here: **`AgentEvent::MessageUpdate` carries the
whole accumulated `AgentMessage`, not a delta.** In-process that is fine; over a
socket (or in a broadcast to N peers) it is O(n²) bytes for a long message.
jcode's wire carries `TextDelta`/`ToolInputDelta`. If Events are to cross a
socket, either add delta-shaped variants or make the wire event a projection of
`AgentEvent` (deltas) while the in-proc channel keeps whole messages. Flag this
now; decide when the socket lands. (The `MessagePrinter` in `repl.rs` already
computes exactly these deltas — the diffing logic exists.)

---

## 10. Multi-agent semantics worth taking (minimized)

If/when we point sessions at each other, take the small, load-bearing subset of
jcode's design and leave the framework behind (wcode's minimalism):

**Take**
- **Bounded fan-out by construction.** Only a root session spawns; recursion is
  opt-in (`swarm-deep` equivalent). Cheapest possible guardrail.
- **Ownership = the `report_back_to` edge.** A message carries who it reports to;
  ancestry is a walk of that edge. You may address/stop only your subtree.
  Scope by construction, not by an ACL — which fits "no permission prompts".
- **Soft-interrupt delivery at safe points.** Already `steer`; name it.
- **Completion report auto-forwarded** to the owner on turn end — no extra DM.
- **DMs first; no locks.** Optimistic; conflicts resolved by a direct message.
- **Events are the UI.** A swarm/session graph is just a subscriber that reduces
  `Event` + lifecycle facts. No separate telemetry channel.

**Leave**
- Plan / task-DAG objects, channels, shared context keys — the DAG reframe is
  elegant but it is a *product*, and it needs a scheduler. Note it as the north
  star if deep swarms ever become a goal; do not build it for v1.
- A coordinator role as a user-facing concept (it is scheduler *policy*).
- Worktree managers (git integration is orthogonal).

And re-cast it in wcode's idiom: **who-may-message-whom and message
transformation are `Hooks`.** A `before_inbound` hook (the mirror of
`before_tool_call`) returns a reason to drop a message, or rewrites it. That is
the wcode-native way to express policy *without* config — exactly the philosophy
in `README.md` §Philosophy.

### 10.1 Communication rules: orchestrator (v1) → group (later)

Two modes, one protocol. **v1 is the orchestrator:** one agent initiates, a star of
workers answers. A later **group** mode (mesh) is reserved, not built.

| | **orchestrator** (v1) | **group** (later) |
|---|---|---|
| Who initiates | the orchestrator only | any member |
| Topology | star (one hub, N workers) | mesh / graph |
| A worker may reach | its orchestrator only | any member |
| Extra capability | — | shared context ("all can *know*") |

**The rule — request-down / report-up.** An edge is one-directional *per role*: the
orchestrator **requests down**, a worker **reports up**. No lateral (worker → worker)
edges, no worker → a foreign orchestrator. Ownership is the `report_back_to` edge
collapsed to one level — only the root spawns, so no recursion → a star.

**Permissions = `to`-resolution scope.** No new protocol: the rule only bounds which
address a `to` may resolve to (the registry enforces it, §14 S4-2):

- an **orchestrator** may resolve its **workers** (its children);
- a **worker** may resolve its **orchestrator** only (`report_back_to`).

**Async reports, not blocking asks.** The orchestrator `message`s a worker and moves
on; the worker's **report arrives later as an inbound message** (`MessageReceived`,
injected into the orchestrator's context tagged with its sender, and it **wakes
the orchestrator if it is idle** — a report is not left sitting). It never blocks on
a worker. `Request::Ask` (a correlated reply) stays for a *synchronous* caller — a
tool that wants a result now — but the default orchestration loop is
request → report-as-inbound-message. This answers §14 S4-2's open question: the reply
is **not** a subscription to the peer's stream; it is a message back.

**One tool, no new verb.** `message { to?, content, mode? }`, with `to` *defaulting to
the sender's `report_back_to`*: the orchestrator passes `to` (many workers), a worker
omits it (one place to go). The default *is* the topology. A *report* is just
`message` to the owner — no `report` tool; and no `Ack` verb (§13.13/§13.14). Note
"tell" collides with `Message::Tell` (the fire-and-forget mailbox item in the code),
so call the answer a **report**.

**Enforcement, three layers, cheapest first.** (1) *Construction* — a worker is
handed only its orchestrator's handle (ideal, but the model addresses by name, so it
cannot hold alone). (2) *Resolution* — the registry refuses a `to` outside the
sender's permitted set (the real gate for v1). (3) *`before_inbound`* — the
receiver-side backstop and the seam a user extends. No ACL, no config.

**Reserved for the group mode.** Only the permitted set changes (any member, not
owner/children), plus a **shared context** — which the *Leave* list above already
defers — and optionally a broadcast. None of the v1 sender plumbing (`from`,
`MessageReceived`, `send_from`) is discarded; the scope check just widens.

---

## 11. Tensions & risks (to decide against, explicitly)

- **Premature generality.** A bus and addressing are over-engineering *today* —
  there is one peer. Mitigation: ship the *types* first (cheap, no behavior
  change), the *actor* when the TUI needs multiple subscribers, the *socket* when
  remote/multi-session is real, the *bus* only when a second agent exists. Each
  stage must stand alone.
- **Kernel purity vs the actor.** A `SessionActor` in the harness is a runtime
  concern, not presentation — acceptable — but it does grow the kernel. Keep it
  a thin driver over the existing loop, not a second loop.
- **Wire efficiency** (see §9). Decide before the socket, not after.
- **Interleaving semantics are subtle.** Inbound messages are delivered at turn
  boundaries, not instantly. Callers (and the TUI's status line) must not assume
  synchronous delivery. Name `Notify`/`Interrupt`/`Wake` so intent is explicit.
- **A2A scope creep.** "Agents can talk" invites orchestration, DAGs, schedulers.
  Hold the line at DM + report-back until a concrete need appears.
- **Two protocol layers.** jcode ended up with an internal wire *and* a curated
  API. We should consciously stay at one until a second *client ecosystem*
  exists.

---

## 12. What this changes in `tui-plan.md`

- The TUI is described there as a client of the `AgentEvent` channel. That stays
  true and gets *stronger*: it should be a client of **`Request`/`Event` over a
  backend**, never of `Agent` directly. Update §3 "seams" to add the inbound
  `Request` type and the backend abstraction (local vs socket).
- The "client/server split" listed as a v1 non-goal in §1 is still a non-goal for
  the *first* TUI slice — but the seam should be shaped now so the split is a
  transport swap later, not a rewrite. The event loop, state reducer, and
  rendering are unaffected.
- Multi-session/swarm moves from "out of scope" to "same protocol, later
  transport" — a pointer, not a plan.

---

## 13. Decision list (review these)

1. **Unify inbound+outbound into `Request`/`Event`?** Recommended: yes. Extend
   `event.rs` with `Request`; keep `AgentEvent` as the `Event` type.
2. **Envelope now or later?** Recommended: define the fields (`v`, `id`,
   `reply_to`, `session`) now; use them in-process where free; make framing real
   with the socket.
3. **One `Request` enum, or a curated facade over it?** Recommended: one enum;
   no facade until an external client exists.
4. **Where do the types live?** Recommended: interface types in `wcode-harness`;
   framing/transport in a new `wcode-protocol`; TUI in `wcode-tui`.
5. **Session actor in the kernel?** Recommended: yes — a thin `SessionActor`
   owning the existing `Agent`/loop, exposing inbox + subscribable outbox.
   Keep `Agent::run` as a convenience wrapper.
6. **Addressing scheme?** Recommended: `SessionId` (reuse `Header.id`) +
   well-known `"user"`; peers as `"agent:<id>"`.
7. **Transport order?** Recommended: in-proc → per-session socket → bus. Build
   #2 only when the TUI needs it; #3 only when a second agent exists.
8. **Wire event shape?** Recommended: decide before the socket — deltas vs whole
   messages (the O(n²) issue in §9).
9. **A2A delivery vocabulary?** Decided: the kernel's target-side verbs are
   `Notify`/`Interrupt`/`Wake`; `Steer`/`FollowUp` are serde aliases
   (`steer`/`follow_up`). Mapping: `Interrupt`→steer, `Wake`→follow_up,
   `Notify`→append-no-turn. `ask` is a *tool mode* (deliver + expect a report),
   not a kernel variant; addressing is the router's (`to`, §10.1).
10. **A2A policy?** Decided: `before_inbound(from, request)` — a hook that may
    drop or rewrite by *who* sent it; ownership by `report_back_to`; no ACL, no
    config. A local handle leaves `from` unset ("the human").
11. **Do we take the task-DAG reframe?** Recommended: no for v1; note as the
    direction if deep swarms become a goal.
12. **Does the line REPL change?** Recommended: no behavior change; it becomes a
    second client of the same protocol (as jcode's `harness_repl` example is).
13. **The model's A2A surface?** Decided: **one** `message { to, content, mode }`
    tool, `mode` ∈ `wake` (default) / `notify` / `ask` / `interrupt`; an inbound
    message is rendered in the model's context tagged with its sender. `wake` is
    the default so a report wakes an idle recipient. No per-mode tools.
    rendered in the model's context tagged with its sender. No per-mode tools.
14. **Is there an `Ack` verb?** Decided: no. Receiving a message *is* the ack (it
    lands in the context, tagged); a reply is just `message` to the `from`. No
    handshake, no delivery receipt.
15. **Address book?** Later (S4-5): a `phonebook` (name → address) above the
    registry, so a sender can say `to: "reviewer"` instead of `agent:<id>`.
16. **Communication rule (topology)?** Decided (§10.1): **orchestrator** v1 — one
    agent requests down, workers report up, no lateral edges, a star (only the
    root spawns). A **group** (mesh + shared context) mode is reserved for later.
    The registry enforces the permitted set; the default loop is **async** (a
    report arrives as an inbound message, not a blocking reply).

---

## 14. Staged roadmap (each stage shippable; tests + clippy + a live check)

- ☑ **S0 — Types.** `Request` + `Frame`/`SessionId`/`PROTOCOL_VERSION` in
  `wcode-harness/src/protocol.rs`, serde round-trips. *No behavior change.*
- ☑ **S1 — Actor.** `SessionActor`/`SessionHandle` in
  `wcode-harness/src/actor.rs`: the `Agent` moves onto a task, an inbox of
  `Request`, a `broadcast` outbox; `Submit`/`Steer`/`FollowUp`/`Cancel`/`SetModel`/
  `SetEffort`/`Compact` are serviced (a biased select keeps `Steer`/`Cancel`
  timely during a run; other requests arriving mid-run are deferred in order).
  **Landed: the actor core, exercised by in-module integration tests.** Deferred
  to **S1b-1/S1b-2** (below): read-back requests and their reply events — they
  need the request/reply correlation the envelope defines — and pointing the CLI
  at the actor. `Agent::run` stays as the low-level method the actor calls.
- ☑ **S1b-1 — Replies.** `Request::GetHistory` and the reply variants
  (`AgentEvent::{History, Ack, Stopped}`, reusing `Error`/`Compaction`);
  `SessionHandle::ask(request) -> AgentEvent`, correlated in-process by a
  one-shot channel (the wire will use the envelope's `reply_to`). The actor's
  `dispatch` answers; `run` now returns its `StopReason` so `Submit` replies
  `Stopped`. Replies are never streamed.
- ☑ **S1b-2 — Repoint the CLI.** The REPL and `-p` hold a `SessionHandle`:
  `run_turn` subscribes and `ask(Submit)`s (the printer stops at `AgentEnd`);
  `/usage` → `ask(GetHistory)`, `/model`/`/effort`/`/compact` → asks; `/new` and
  `/resume` spawn a new actor and swap the handle; Ctrl-C sends `Cancel` to the
  current handle. `/reload`'s build is *not* specially cancellable (a developer
  path — Ctrl-C there is a non-case). Verified live end-to-end against a local
  model.
- ☑ **S2 — Transport.** New `wcode-protocol` crate: NDJSON `read_frame`/
  `write_frame`, `bind`/`connect` (Unix), a `serve` that bridges a session
  `SessionHandle` to sockets (a writer + an event fan + a per-request reply
  task, correlated by `reply_to`), a `Client` that mirrors the handle
  (`send`/`ask`/`subscribe`), and a `Backend` enum that makes local and remote
  interchangeable.
  - ☑ **S2-1** — the crate, with a real-socket round-trip test (a client
    `ask`s, streams the run, reads history back; two clients share one session).
  - ☑ **S2-2a** — `wcode serve [--socket P]` owns and serves the session;
    `wcode --socket P -p "..."` runs one shot through a remote `Backend`.
  - ☑ **S2-2b** — the *interactive* remote REPL: `repl::run` takes a
    `SessionSource` and drives a `Backend`; `--socket` attaches the REPL to a
    served session; `/new`/`/resume`/`/reload` are gated (the server owns the
    session); attaching replays the transcript (a `GetHistory` read-back); the
    `Client` reconnects on a drop (backoff, requests queued across the gap),
    though an in-flight `ask` whose reply was lost fails with `Closed`.
  *(This is the jcode TUI architecture, minimal.)*
- ◐ **S3 — TUI.** Landed as a `wcode-protocol` client (per `tui-plan.md`):
  P0–P3d — three bands, streaming, scrollback, `/`-commands, prompt history,
  multiline, markdown, context bar, resize, `/copy`; then the overlay layer (the
  model, `/changes`, and `/resume` pickers), tool-diff rendering, a per-run
  changeset, and a session picker that hands off by re-exec. Single surface;
  multi-surface on the same connection (`session` in the frame) is not started.
- ◐ **S4 — A2A.** Route `Notify`/`Interrupt`/`Wake` to a peer (the router
  resolves `to`); a registry for addresses; ownership from `report_back_to`;
  a `before_inbound` hook for policy. Orchestrator v1 (§10.1); DM + report-back
  only. Too large for one step, so it splits like S1 did
  (`S1 → S1b-1 → S1b-2`); each slice stands alone.
  - ☑ **S4-1 — Delivery vocabulary + policy seam (in-process, self-addressed).**
    The A2A verbs and the policy hook, exercised by a session sending to its own
    handle — no registry, no second session, no socket yet.
    - `Request::{Notify { content }, Interrupt { content }, Wake { content }}`
      (§13.9): `Interrupt` → `Steer`, `Wake` → `FollowUp`, `Notify` → *append to
      context, no turn* (the one genuinely new semantic). `is_inbound`
      names the three.
    - `Hooks::before_inbound(&mut Request) -> Option<String>` (§13.10) — the
      mirror of `before_tool_call`: `Some(reason)` drops the message (an `ask` is
      answered `Error`); the hook may rewrite the request in place. No config.
    - `AgentEvent::MessageReceived { from, content }` so the recipient (and the
      TUI) surfaces an arriving message.
    - Decision taken: `Notify` *idle* appends to `ctx` + session directly
      (`Agent::notify`, no turn); `Notify`/`Interrupt` *mid-run* ride the steering
      channel). A `Wake` **starts a turn even when idle** (otherwise a worker
      handed a task would only queue and never act); mid-run it rides the
      follow-up channel. The actor picks by run state, and
      every accepted verb emits `MessageReceived`.
    - `SessionEntry::{Sent, Received}` deferred to S4-2, where frames exist to
      carry (§9).
    - **Landed.** Verified by the in-module actor tests and a socket round-trip
      (`a_notify_crosses_the_socket`); the CLI is unchanged (no surface yet).
  - ◐ **S4-2 — Registry, addressing, and the sender.** The mailbox carries the
    **sender** (landed), and a `Registry` (`SessionId` → `SessionHandle`) lets
    an addressed `Request::Ask { to, content }` reach a peer (next). Concretely:
    - ☑ Canonical verbs: `Notify`/`Interrupt`/`Wake`/`Ask`. `Steer`/`FollowUp`
      became serde **aliases** (`#[serde(alias = "steer")]` / `"follow_up"`) so
      old frames still parse; the kernel methods keep their names (§13.9).
    - ☑ The actor's inbox item carries a sender: `Message::{Tell, Ask}` hold
      `from: Option<SessionId>`, set through `SessionHandle::{send_from,
      ask_from}` (`None` = the local human, emitted as `SessionId::user()`).
      `MessageReceived.from` is no longer a constant.
    - ☑ `Hooks::before_inbound(from: Option<&SessionId>, request: &mut Request)`
      — policy can decide by *who* sent it (ownership from `report_back_to`).
    - ☑ Registry — the in-process bus (`wcode_protocol::Registry`): an address
      book (`SessionId` → `SessionHandle`) plus the ownership map
      (`report_back_to`), with `permitted`/`resolve`/`deliver` enforcing the
      **permitted set** (§10.1): an orchestrator resolves its workers, a worker
      its orchestrator, nothing lateral.
    - **Addressing lives at the router, not in `Request`.** A `Request` is
      target-side and names no peer; the router turns a `to` into a mailbox. This
      supersedes the earlier `Request::Ask { to, content }` sketch — a `to` in the
      kernel was redundant with the frame's `session` on the wire and with the
      held handle in-process.
    - ☐ Wire a spawn to `register`/`set_owner` (S4-3) and the `message` tool to
      `deliver`; auto-forward the worker's completion report on turn end along
      `report_back_to`.
    - Resolved (§10.1): the answer is a **message back**, delivered as an inbound
      event — not a subscription to the peer's stream, and not a blocking reply.
    - Resolved (§10.1): the answer is a **message back**, delivered as an inbound
      event — not a subscription to the peer's stream, and not a blocking reply.
  - ☑ **S4-3 — Spawn + the `message` tool.** Only the root spawns; a worker cannot
    (bounded fan-out by construction — the root's tool set has `spawn`, a worker's
    does not). A `SessionFactory` builds a worker `Agent` from the parent's
    template, spawns its actor, and `register`s + `set_owner`s it in the
    `Registry`. Decisions:
    - **Opt-in** (`--agents`), so the default single-agent behaviour is unchanged.
    - **`spawn { task, name? }`** — combined: create the worker and deliver `task`
      as a `Wake` (so it starts working), returning `"agent:<name>"`.
    - **In-memory workers** (no session file) for v1; per-worker files later.
    - **Model-driven report** for v1 (the worker's prompt tells it to `message`
      its owner). Auto-forward (a "run-ended" hook) is a follow-up.
    - Future customization (`spawn { system?, tools?, write?, read? }`, a
      `WorkerSpec`): v1 fills defaults (inherit the parent) and uses only `name`;
      the tool/path restrictions become a child `Hooks` impl — no config.
    - ☑ **S4-3a** — `SessionFactory` + `spawn { task, name? }` + registry wiring,
      opt-in `--agents`. Landed; verified live.
    - ☑ **S4-3b** — the `message` tool (`to` defaults to the owner) + sender
      tagging (`[message from <addr>]`), threaded through `repl` so `/new` keeps
      `spawn`+`message`. Landed; the report round-trip is model-driven.
    - ☑ **S4-3c** — completion auto-forward: a `Hooks::after_run` point + a
      `ReportBack` hook forwards a worker's final message to its orchestrator as
      a `Wake`. Landed; the worker prompt no longer asks it to report by hand.
    - `message { to?, content, mode? }`, `mode` ∈ `wake` (**default**) / `notify` /
      `interrupt` / `ask`, mapping 1:1 onto the wire verbs. The default is
      `wake`, so a message (a task, or a worker's report) **runs a turn even if
      the recipient is idle** — otherwise a report would sit unread until the
      next human prompt. `to` **defaults to the sender's `report_back_to`** — a
      worker has exactly one place to send. One tool, one concept: the mode is
      data (§13.13).
    - An inbound message is injected into the model's context **tagged with its
      sender** (`[message from agent:abc] …`), so the model knows the address to
      reply to. **`Ack` is not a verb**: reply = `message` to the `from` you
      just saw. No handshake (§13.14).
  - ◐ **S4-4 — Socket peers.** Point the registry at served sessions, so a
    message reaches across the socket. Landed: `Frame.sender`, `Client::send_from`
    /`Backend::send_from`, the server honoring `sender` (via `ask_from`), a
    `Registry` that holds a `Backend` (`register_remote`), and
    `wcode --agents --peer <name>=<socket>`. Delivers *out* to a served peer with
    the sender attributed. **Follow-up:** the *reply* direction needs the served
    peer to be a worker with an owner (a `--owner` on `serve`, giving it a
    `ReportBack`) — or the orchestrator served too; not yet wired.
  - **S4-5 — Phonebook (later).** A **name → address** map above the registry,
    so the model (and the human) can address `to: "reviewer"` instead of a raw
    `agent:<id>`. The registry stays the transport-level address book; the
    phonebook is the human/agent-facing alias layer. Discovery and persistence
    of names are its own small design (§13.15).
  - *(future surface)* **Worker visibility / perspective switch.** A worker
    emits to its *own* event stream and nothing renders it; letting the TUI
    switch to a worker's view (and/or forwarding its activity into the root's
    transcript) is a later surface — out of scope for S4.
- **S5 — (optional, far)** Task-DAG / deep swarm, only if wanted.

The dependency is linear and each stage is independently useful: S0 unblocks S1
unblocks both S3 (TUI) and S4 (A2A), which then share S2's transport.

---

## 15. Recommendation

Do the unification, but let it be **one small thing**: make `AgentEvent` the
`Event` half of a request/event protocol, add the `Request` half to match it, and
wrap both in a versioned, addressed envelope. Put the interface types in the
kernel and the framing in a thin `wcode-protocol` crate. Turn `Agent` into a
session actor so a session is a peer with a mailbox. Then **the TUI is peer #1
and agent-to-agent is peer #2, on the same wire** — which is exactly the property
we set out to get.

Ship it in the order S0 → S1 → S2, because that is the order in which each step
stops being speculative: types cost nothing, the actor is what the TUI needs
anyway, the socket is what multi-session needs, and the bus only earns its keep
when a second agent actually exists. Resist forking a second protocol, and resist
the DAG until it is the only thing that can work.

---

### Appendix — file map for the reader

- kernel interface: `crates/wcode-harness/src/event.rs`,
  `agent.rs`, `loop_.rs`, `session.rs`, `hooks.rs`.
- the client: `crates/wcode-cli/src/repl.rs` (event printing, commands),
  `main.rs` (composition root).
- jcode references: `../jcode/crates/jcode-protocol` (`Request`/`ServerEvent`),
  `../jcode/crates/jcode-harness-api` (versioned NDJSON `ClientFrame`/
  `ServerFrame`, `Hello`, `SessionInfo`, `AttachSession`),
  `../jcode/crates/jcode-tui/src/tui/backend.rs` (local-vs-remote backend),
  `../jcode/docs/SERVER_ARCHITECTURE.md`, `MULTI_SESSION_CLIENT_ARCHITECTURE.md`,
  `SWARM_ARCHITECTURE.md`, `SWARM_TASK_GRAPH.md`.
