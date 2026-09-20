# wcode — team: define, declare, see (plan)

Status: **F1–F4b landed — item 10 complete.** Companion to [`next-steps.md`](next-steps.md) (tracker),
[`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md) (the
protocol this builds on, §10.1 A2A) and [`tui-plan.md`](tui-plan.md) (the TUI
project). This is the **team** workstream: how an agent is *defined*, how a team
is *declared*, and how the TUI *shows* it — plus the command surface that drives
all of it.

## Problem

The A2A capability (S4) exists but is a stub in three directions, and the TUI's
command surface has no completion. Four gaps, one spine.

1. **Agents are not customizable.** A spawned worker inherits its parent
   wholesale. `WorkerSpec` carries **only `name`** (`agents.rs`); `SessionFactory::build`
   hardcodes the `# You are a worker` blurb, clones the parent's `llm`, and takes
   `default_tools(&t.tools)`. So the orchestrator's model can `spawn` a worker but
   cannot say *which model*, *what role*, or *which tools*. The struct is
   doc-stamped as "the seam for per-worker customization … left at their
   defaults" — the seam is unused.
2. **The team forms by prompting.** There is no preset. `--agents` merely gives
   the root a `spawn` tool (`main.rs`); workers exist only after the *model*
   decides to call `spawn`. Every session that wants a team must be told so, in
   prose, again.
3. **The team is invisible.** The TUI holds **one** session (`App` is flat and
   single-session; one `backend.subscribe()` stream). There is no notion of peers,
   workers, or a team anywhere in `wcode-tui`; `event.rs` marks panels as future
   intent. Spawn a team and you cannot see who exists, what they are doing, or
   what they returned without reading the transcript.
4. **No command completion.** `/`-commands are a scattered `match` in
   `App::command`, duplicated as a prose string in `/help`. Typing `/ses` suggests
   nothing. Worse, the TUI and the REPL keep **two independent parsers**
   (`App::command` vs `repl::parse_command`) whose command sets have already
   diverged.

## Design

### The dependency spine

These are two capabilities sharing one chain:

```
  DEFINE AN AGENT (F2) ──▶ DECLARE A TEAM (F3) ──▶ SEE THE TEAM (F4)
   WorkerSpec fields         [team] table +          status sidebar +
                             startup spawn           multi-surface
                                          ▲
  DRIVE IT (F1): TUI command table ────────┘  (F3/F4 add /team, /switch)
```

**F2 is the hinge.** A team preset *is* a named `WorkerSpec`, so F3 depends on F2
(a preset without per-agent fields is a list of identical names). F4 needs F3's
data to have a team to show. **F1 is orthogonal** — TUI-local, no dependency — but
it builds the **command table** that F3/F4 extend, and its `Picker` machinery is
exactly the "surface switcher" F4 will want. So F1 lands first (cheap, useful),
then the F2 → F3 → F4 chain.

The whole chain was **in-process first.** Agent definitions now cross the wire:
`Request::Define` lets a client ask a served peer to build a worker, answered with
`AgentEvent::Spawned` via a server-injected handler (`49b227e`), and `spawn { to }`
defines one on a served peer (`7627b9e`). The socket layer multiplexes many sessions
with a live roster (`c52df8c`, `6f3ef23`), and the per-agent provider landed too
(`9c02e96`).

### F1 — TUI slash-command completion

**Shape.** One `Command` table is the single source of truth: `{ name, aliases,
args: Option<&str>, summary }`. `App::command` dispatches *through* the table,
and `/help` is **generated** from it — killing the duplicated prose string. The
table is TUI-local; unifying with the REPL's `parse_command` is deferred (the two
sets legitimately differ: TUI has `/changes`/`/copy`, the REPL has
`/new`/`/skills`/…). Build it as data so a shared table is a later option, not a
rewrite.

**Completion.** A **non-modal inline popup** — not the modal `Overlay`. An
`Overlay` captures every key (its `Char` arm edits the picker's `query`), which
would make it impossible to keep typing `/model gpt-…` args. Instead:

- App gains `completion: Option<Completion>` (`matches: Vec<usize>`, `selected`).
- It is shown when the **first token** of the buffer starts with `/` and contains
  no space yet — i.e. the user is still choosing the *command name*.
- While open it owns **only** `Up`/`Down` (move), `Tab` (accept), `Esc` (dismiss).
  `Char`/`Backspace` **fall through** to the input and recompute the matches;
  `Enter` accepts only when the token is **not** an exact name/alias (a bare `/`
  never accepts; an exact name/alias submits).
- Accept inserts `/<name> ` (with trailing space) and closes the popup.
- Rendering reuses the picker's *render/style* (`highlight`) over its own prefix
  matcher (`prefix_indices`, D11), drawn just above the input band — not the
  overlay's modal gate.

**Prerequisite.** `Key` has no `Tab` variant and `event.rs` maps no Tab; add one.

### F2 — customizable spawned agents

**Shape.** Extend `WorkerSpec` and `SpawnArgs` with the same optional fields.
`SessionFactory::build` is a thin `SessionActor::spawn(worker_config(…))`
wrapper over a **pure** `worker_config(&self, id, owner, spec) -> AgentConfig`
(the unit the tests exercise) that applies them:

| field | effect | notes |
|---|---|---|
| `model: Option<String>` | overrides `llm.model` for the worker | the parent's `stream_fn` is shared; a per-agent `base_url`/`api_key` also landed, a worker on its own provider (`9c02e96`) |
| `system: Option<String>` | **appended** after the worker identity blurb as `\n\n# Role\n<text>` | the blurb (who it is, who owns it, auto-report) always survives |
| `tools: Option<Vec<String>>` | allow-list filter over `default_tools(&t.tools)`: `None` = all, `Some(vec![])` = only `message` | `message` is **always** kept and always valid in the list; `spawn` is never present; any other unavailable name **fails loudly** via `validate_tools` (D14), never silently dropped |

`spawn`'s args stay additive: `{ task, to?, name?, model?, role?, tools?,
base_url?, api_key? }` (the `role` arg maps to `WorkerSpec.system`; `to` names a
served peer, so the worker is defined there). A worker never gains `spawn` — bounded
fan-out by construction stays intact. A worker is its own conversation, so
`worker_config` gives it its own `llm.session_id` (its address) rather than
inheriting the root's `x-opencode-session` (D15).

**Scope.** In-process by default; a *remote* worker is defined on a served peer via
`Request::Define` / `spawn { to }` (`49b227e`, `7627b9e`).

### F3 — team preset

**Shape.** A `[team]` config table: a list of members, each reusing F2's fields
plus the local ones a worker needs.

```toml
[[team]]
name   = "explorer"
model  = "claude-haiku-4-5"          # optional; default = the root model
role   = "recon only; never edit; cite file:line"
tools  = ["read", "grep", "find", "bash"]   # optional; default = all

[[team]]
name   = "reviewer"
role   = "verify diffs; PASS / NITS / FAIL"
```

At startup — after `Orchestrator::new`, mirroring the existing `[peers]`
registration loop — the composition root spawns each member through the *same*
`SessionFactory` and records its name in the phonebook. Because it calls the
factory directly (bypassing the `spawn` tool), it must first call
`SessionFactory::validate_tools` per member so a bad `tools` entry **fails
loudly** at startup rather than being silently dropped (D14). The orchestrator
then begins with the team already present: no prompt required. `Config`/`FileConfig`
gain a `team: Vec<TeamMember>` field with the usual env-is-not-involved
pass-through in `merge` (a team is data, not a scalar knob).

**Decisions.** `[team]` is **local** spawned workers; `[peers]` stays the
**remote** map (socket/address). They compose — a `[peers]` entry can name one of
the team's workers if it is served — but they are not merged in v1. A `[team]`
requires `--agents` (D16); the block enters the root prompt only when the team is
non-empty (D17); duplicate member names fail loudly at load (D18); members spawn
via `Orchestrator::spawn_worker` — `validate_tools` (D14) then the phonebook (D19).
An optional `[orchestrator] guidelines` string (D20) is the root's workflow: it
renders as a `# Orchestrator workflow` section **after** the roster, root-only
(requires `--agents`; a served `--owner` worker and workers never see it).

**`/new` semantics.** The team lives in-process, so it persists across `/new`
(same process, same `Orchestrator`). A `/reload` re-exec re-reads `[team]`.

**Overlay config (F3c).** A team belongs in a per-repo file, not the provider
config. `--config <path>` / `WCODE_CONFIG` (flag beats env) names an **overlay**
that is deep-merged over the global `config.toml` at the TOML-value level — tables
recurse per key, a scalar/array is replaced wholesale — so an overlay's
`[tools] grep = true` adds without clobbering the global `[tools]`. A missing or
unparseable overlay is a hard error; `MissingModel` carries the post-overlay file
so `/`-rescue still works. The repo's own `.wcode/team.toml` is this overlay: the
dev team + workflow + `grep/find`, leaving provider/model global.

### F4 — TUI multi-surface + team status sidebar

**Two tiers, landed in order.** Tier 1 is UI-only and cheap; tier 2 is the
separable socket work.

**Tier 1a — status sidebar (F4a).** Split the `body` band horizontally in `ui.rs`
`draw` — only when the injected roster is non-empty **and** the terminal is ≥ 60
cols, so a teamless/narrow frame is byte-identical to before — and render a
`draw_sidebar` pane: one line per teammate, `name · model · state` (idle /
running / done, the row styled by state). The TUI learns the team from the
composition root, which **injects** a `Vec<Teammate>` (same pattern as
`models`/`sessions`) plus, for live state, an `mpsc::UnboundedReceiver<TeamUpdate>`
fed by one forwarder per member (`Orchestrator::subscribe_worker` → `AgentStart` =
running, `AgentEnd` = done). `/team` lists the same roster. (The optional "last
result" line is not built yet.)

**Tier 1b — multi-surface.** Split `App`'s single-session state into a `Surface`
(`status`, `transcript`, `live`, `changes`, `running`, `cancelled`,
`context_used`, `scroll`, `max_scroll`, `viewport`, `last_total`, `last_width`)
and make `App` hold `surfaces: Vec<Surface>` + `focus: usize` + the **shared**
composer (`input`, `cursor`, `history`, `overlay`, `completion`). Lands in two
steps: **F4b-1** is the pure refactor (exactly one surface, byte-identical
frames); **F4b-2** is the feature (multiple surfaces, `SessionId` on events). The
reducer becomes `(SessionId, AgentEvent)`; the TUI subscribes to N `Backend`s —
the in-process `Registry` already maps `SessionId → Backend`, so no wire change is
needed. Focus switches via a picker — **reusing F1's `Picker`/selection
machinery** (a new `PickerKind::Surface`) — or a cycle key; the sidebar highlights
the focused surface. Per the locked decisions (D23–D28): surfaces are the root plus
one per member; `Submit`/`Cancel`/`Ask` target the focused surface; each surface
keeps its **own** prompt history (a change from the shared-composer sketch above);
**F4b-2 subsumes F4a's `TeamUpdate` feed** (a surface's own `AgentStart`/`AgentEnd`
*is* the sidebar state, so the forwarder channel goes away); and the sidebar shows
each teammate's **model + state only** — the root keeps the full status line.

**Landed (F4b-1 + F4b-2).** `Surface { id, label, model, is_root, status,
transcript, live, changes, running, finished, cancelled, context_used, scroll,
max_scroll, viewport, last_total, last_width, history, history_index, draft }`;
`App { surfaces, focus, …shared composer… }` with `focused()`/`focused_mut()`
(total — the list is never empty). `AppEvent::Agent(SessionId, AgentEvent)` routes
by id (an unknown id is ignored). `run(Vec<SurfaceSpec>, Options)` merges every
surface's `Backend` subscription into one channel; `Submit`/`Cancel`/`Ask` hit the
focused backend. `/surface` (`PickerKind::Surface`) + `Ctrl-N` switch focus; the
sidebar bolds the focused member and derives `label · model · state` from the
member surfaces (`Surface::state()` = running/finished/idle). The F4a
`TeamUpdate` / `Options.teammates` / `spawn_team_feed` machinery is gone;
`Orchestrator::worker_backend(name)` builds a member's backend. (Note: with the
shared composer + per-surface history, recalling on surface A, switching to B, then
recalling saves A's recalled buffer into B's `draft` — a benign consequence of the
D25 split.)

**Tier 2 — landed.** One socket carries many sessions: `serve` demuxes on
`Frame.session`, a `Client` routes inbound frames by that field, and
`Client::with_session` narrows a connection to one session (`c52df8c`, `218aa8c`,
`d632d0b`). The roster is now **live**: `serve` takes a `Registry::subscribe()` watch,
a per-connection task fans each newly-seen session and pushes an unsolicited
`AgentEvent::Sessions` when the roster grows, and the `--socket` TUI feeds
`new_surfaces` from `Client::subscribe_roster()` (`6f3ef23`, `c3eb261`). A
runtime-spawned worker is thus visible to a `--socket` client.

## Decisions (locked)

| # | question | decision |
|---|----------|----------|
| D1 | F2 fields | `model` + appended `system` role + `tools` allow-list |
| D2 | F2 scope | per-agent provider (`9c02e96`) and remote-agent definition (`Request::Define`, `49b227e`/`7627b9e`) both **landed** — originally deferred |
| D3 | worker `message` tool | **always** kept regardless of the `tools` allow-list (the report path) |
| D4 | F3 shape | `[team]` = **local** spawned workers; `[peers]` stays the **remote** map |
| D5 | F3 startup | spawn the team after `Orchestrator::new`, mirroring the `[peers]` loop; `/new` keeps it |
| D6 | F4 order | **tier 1 first** — sidebar (1a) then in-process multi-surface (1b); socket multiplexing (tier 2) landed (`6f3ef23`, `c3eb261`) |
| D7 | F1 popup | **non-modal inline**, reusing the picker's render/style — its own prefix matcher (D11) — **not** the modal `Overlay` |
| D8 | F1 table | TUI-local `Command` table; `/help` generated from it; REPL unification deferred. Item 21 adds the TUI `/reload [--no-session]` (mirrors the REPL: rebuild + re-exec into the current session, surfaced as `Outcome::Reload`; refused over a socket) |
| D9 | F1 keys | popup owns `Up`/`Down`/`Tab`/`Esc`; `Char`/`Backspace`/`Enter` fall through. While the popup is open `Up`/`Down` select rows **even for an exact token** (e.g. `/model`), so history recall is suppressed in that state — a deliberate change from before |
| D10 | sequence | F1 → F2 → F3 → F4 |
| D11 | F1 matching | completion matches case-insensitive **prefix** (canonical names + aliases); the **picker keeps substring** filtering — a separate matcher, not the shared one |
| D12 | F1 Enter | accept only when the token is not an exact name/alias; a **bare `/` never accepts** (no `/exit` trap) |
| D13 | F1 alias | `/sessions` aliases `/resume`, so `/ses` completes to it (aliases are dispatchable + suggestable, never shown in `/help`) |
| D14 | F2 unknown tool | `message` is **always valid** in an allow-list (a no-op — it is kept regardless); any other unavailable name — including `spawn` — **fails loudly** via the reusable `SessionFactory::validate_tools` (used by `spawn` and, in F3, the startup loop), never silently dropped |
| D15 | F2 session id | a worker gets its **own** `llm.session_id` (its address), not the root's `x-opencode-session` |
| D16 | F3 gate | a non-empty `[team]` requires `--agents` — else `error: [team] requires --agents`, exit 2 |
| D17 | F3 prompt | the `# Your team` block renders **only** when the root has a non-empty team; a `--owner` served worker never sees it |
| D18 | F3 names | duplicate `[team]` member names are a **config error** at load (names are unique phonebook keys) |
| D19 | F3 spawn | the startup loop spawns via `Orchestrator::spawn_worker`, which runs `validate_tools` (D14) and registers the name in the phonebook |
| D20 | F3b guidelines | an `[orchestrator] guidelines` string renders as `# Orchestrator workflow` **after** the team block, root-only (requires `--agents`); empty/absent adds nothing |
| D21 | F3c overlay | `--config <path>` / `WCODE_CONFIG` (flag > env) deep-merges a TOML overlay over the global config — tables recurse, scalars/arrays replace; a missing/unparseable overlay is a hard error; `.wcode/team.toml` is the repo's overlay |
| D22 | F4a sidebar | the team sidebar splits `body` only when the roster is non-empty **and** the terminal is ≥ 60 cols; rows are `name · model · state` (idle/running/done), fed live by per-member `AgentStart`/`AgentEnd`; `/team` prints the roster |
| D23 | F4b surfaces | surfaces = the **root + one per team member** |
| D24 | F4b routing | `Submit`/`Cancel`/`Ask` route to the **focused** surface |
| D25 | F4b history | **per-surface** prompt history (a change from the shared-composer sketch in §F4) |
| D26 | F4b focus | a `/surface` picker (F1 machinery, `PickerKind::Surface`) **plus** a cycle key; the sidebar highlights the focused surface |
| D27 | F4b feed | F4b-2 **subsumes** F4a's `TeamUpdate` feed — a surface's own `AgentStart`/`AgentEnd` is the sidebar state, so the forwarder channel goes away |
| D28 | F4b rows | teammates show **model + state only**; the root keeps the full status line. **Revised** by [`tui-sidebar-plan.md`](tui-sidebar-plan.md): the row is now status glyph + name + live action, and the model is dropped from the sidebar |

## Phased tasks

**Phase 1 — F1: command table + completion.** ☑ landed — reviewed
- [x] `Key::Tab` in `event.rs` (variant + mapping; `BackTab` stays unmapped).
- [x] `Command` table (`{ name, aliases, args, summary }`) as the single source;
      `App::command` dispatches through it and `/help` is generated from it
      (byte-identical to the old string).
- [x] `Completion` state + recompute on every buffer edit; shown for a lone `/`
      first token.
- [x] `on_key`: popup owns `Up`/`Down`/`Tab`/`Esc`; `Char`/`Backspace` fall
      through; `Enter` accepts only a partial (a bare `/` or an exact name/alias
      submits). While the popup is open `Up`/`Down` select rows even for an exact
      token (e.g. `/model`), so history recall is suppressed in that state — a
      deliberate change from before.
- [x] `draw_completion` above the input band (picker-style rows, prefix highlight,
      alias hint).
- [x] Tests: table drives dispatch + `/help`; **prefix** filters (names + aliases);
      accept inserts `/<name> `; `Esc` dismisses (buffer unchanged); history and
      picker-substring unaffected. Verified via `TestBackend` (no TTY here).

**Phase 2 — F2: worker customization.** ☑ landed — reviewed
- [x] `WorkerSpec` + `SpawnArgs`: `model`, `system` (role), `tools`.
- [x] `SessionFactory::build`: apply model override; append the role block; filter
      tools by allow-list, always keeping `message`.
- [x] Tests: model override reaches the built `Agent`'s `llm`; role appended and
      the identity blurb retained; allow-list filters but `message` survives;
      `spawn` absent from a worker; empty/absent spec = today's behavior.

**Phase 3 — F3: team preset.** ☑ landed — reviewed
- [x] `TeamMember` in `config.rs` (`[team]`), pass-through in `merge`.
- [x] Start-up spawn loop in `main.rs` after `Orchestrator::new` (mirror
      `[peers]`), registering each name in the phonebook.
- [x] Tests: a `[team]` parses; N members spawn and are addressable by name; an
      absent table leaves today's behavior; a bad member fails loudly.

**Phase 4 — F4: sidebar + multi-surface.** ☑ landed — reviewed
- [x] **1a** `draw_sidebar` (horizontal split of `body`); `Vec<Teammate>` injected
      by the composition root; `name · model · state` lines.
- [x] **1a** feed per-teammate state from their event streams (idle/running/done).
- [x] **1b** split `App` → `Surface` + focused map; reducer keyed by `SessionId`.
- [x] **1b** subscribe to N in-process `Backend`s (via `Registry`); focus switch
      (`PickerKind::Surface`, reusing F1) or a key.
- [x] Tests: sidebar lists the injected team; two surfaces keep independent
      transcripts; focus switch routes input to the focused surface.

## Open questions

- **Per-agent provider** (a worker on a different `base_url`/key) — **landed**
  (`9c02e96`): `WorkerSpec`/`TeamMember`/`SpawnArgs` carry `base_url`/`api_key` and
  `worker_config` applies them. `StreamFn` stays shared — rig rekeys per
  `ClientKey(base_url, api_key, session_id)`.
- **Remote agent definition** — **landed** (`49b227e`, `7627b9e`):
  `Request::Define`, answered by a served peer with `AgentEvent::Spawned` through a
  server-injected handler, and `spawn { to }`, which defines a worker on a served
  peer.
- **Role as a preset vs free text**: `role` is free text appended to the prompt in
  v1; a named-role registry (skills-like) is a possible later layer.
- **Team in the TUI vs the CLI**: the sidebar is fed by injection today; surfaces
  are built **once at startup** from `cfg.team` (`crates/wcode-cli/src/main.rs`,
  the `for member in &cfg.team` loop before `wcode_tui::run`). Tier 2 landed
  (`6f3ef23`/`c3eb261`): a `--socket` TUI now adds runtime sessions from the roster
  push, so a local TUI could likewise enumerate the `Registry` — decide then.
  - **Fixed (2026-09-18, `5d80432`) — a runtime `spawn` now appears in the TUI.**
    `SessionFactory` gained an optional `SurfaceSpec` sink (`set_spawn_sink`),
    emitted on a successful spawn only; `main.rs` installs it *after* both startup
    loops and passes the receiver to `wcode-tui::run`, whose event loop adds a
    `Surface` at runtime (`App::add_surface`). Seam-tested; mutation-verified.
    - **Resolved (2026-09-18, `c3eb261`) — a served/socket root is now visible.** The
      server serves a **live** roster and pushes growth, and a `--socket` client feeds
      `new_surfaces` from the client's roster push (`Client::subscribe_roster()`), so
      runtime-spawned workers reach a socket TUI. The earlier gap (the server installed
      no sink and the `--socket` client passed `None`) is closed.
    - **Latent footgun (unreachable today):** the runtime arm pushes to `backends`
      unconditionally while `add_surface` may no-op; unique ids make it
      unreachable, but a guard/comment would harden it.
    - **Minor no-replay race:** `subscribe()` is a live broadcast with no history,
      so a worker's first events can precede the TUI's subscription (the sidebar
      may skip `running`). The surface itself is still added.
  - **Known bug (2026-09-18) — a runtime `spawn` can silently collide on name.**
    `spawn { name: "explorer" }` when a preset worker named `explorer` exists
    reuses `SessionId::agent("explorer")`; `Registry::register` and
    `Phonebook::insert` both overwrite, orphaning the original actor (its handle
    is replaced). D18 guards only `[team]` **load**, never a runtime spawn.
    **Fixed** (`201b9f1`) — `SessionFactory::spawn` now rejects an explicit
    duplicate name loudly (and auto-bumps generated `w{n}` names); it is the
    single chokepoint both `spawn_worker` and the `spawn` tool call.
- **`[peers]` address-alias shadowing (low severity, follow-up).** `spawn`'s
  duplicate guard checks the `Registry` *ids*, but an address-valued `[peers]`
  entry is only a `Phonebook` alias (`Orchestrator::alias`), so
  `spawn { name: "bob" }` can shadow a `[peers]` alias named `bob` (no actor is
  orphaned — only the name mapping is shadowed). Distinct namespace from the
  worker-id collision the guard fixes; revisit if name shadowing bites.
- **Tool allow-list granularity**: names only in v1; per-tool argument restrictions
  (path scopes) would be a `Hooks` impl, not config (stays true to the stance).
- **`/team` command**: the *list* form landed (F4a — `/team` prints the roster).
  A *re-spawn* / edit-members form is still open — cheap, rides on F1's table.
- **`reload_args` and `--owner`**: **resolved** (`7cd385d`) — the re-exec now
  forwards `--owner`/`--name`, so a served worker resumed interactively keeps
  its address and ownership edge.
- **Tier 2 — socket multiplexing**: one socket serving many sessions — **landed**
  (`c52df8c`, `218aa8c`, `d632d0b`; then the live roster + push `6f3ef23`,
  `c3eb261`). See §F4 tier 2.

## Progress

| phase | scope | status |
|-------|-------|--------|
| 1 | F1 — TUI command table + inline completion (+ `Tab`) | ☑ done (reviewed) |
| 2 | F2 — customizable workers (`model`/`role`/`tools`) | ☑ done (reviewed) |
| 3 | F3 — `[team]` preset + startup spawn | ☑ done (reviewed) |
| 4a | F4 — team status sidebar | ☑ done (reviewed) |
| 4b | F4 — in-process multi-surface | ☑ done (reviewed) |
| 5 | F4 tier 2 — socket multiplexing | ☑ done (`c52df8c`, `218aa8c`, `d632d0b`, `6f3ef23`, `c3eb261`) |

Legend: ☑ done · ◐ in progress · ☐ todo.
