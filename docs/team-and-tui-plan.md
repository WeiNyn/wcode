# wcode — team: define, declare, see (plan)

Status: **planned.** Companion to [`next-steps.md`](next-steps.md) (tracker),
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

The whole chain is **in-process first.** No agent-definition crosses the wire:
`Request` has no define/spawn variant, and `Client`/`serve` are one-session-each.
Remote agents are a separate, deferrable follow-up (they need a new `Request` and
socket multiplexing).

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

**Shape.** Extend `WorkerSpec` and `SpawnArgs` with the same three optional
fields. `SessionFactory::build` is a thin `SessionActor::spawn(worker_config(…))`
wrapper over a **pure** `worker_config(&self, id, owner, spec) -> AgentConfig`
(the unit the tests exercise) that applies them:

| field | effect | notes |
|---|---|---|
| `model: Option<String>` | overrides `llm.model` for the worker | parent's provider (`base_url`/`api_key`/`stream_fn`) is shared — per-agent provider is deferred |
| `system: Option<String>` | **appended** after the worker identity blurb as `\n\n# Role\n<text>` | the blurb (who it is, who owns it, auto-report) always survives |
| `tools: Option<Vec<String>>` | allow-list filter over `default_tools(&t.tools)`: `None` = all, `Some(vec![])` = only `message` | `message` is **always** kept and always valid in the list; `spawn` is never present; any other unavailable name **fails loudly** via `validate_tools` (D14), never silently dropped |

`spawn`'s args stay additive: `{ task, name?, model?, role?, tools? }` (the
`role` arg maps to `WorkerSpec.system`). A worker never gains `spawn` — bounded
fan-out by construction stays intact. A worker is its own conversation, so
`worker_config` gives it its own `llm.session_id` (its address) rather than
inheriting the root's `x-opencode-session` (D15).

**Scope.** In-process only. Customizing a *remote* worker needs a new `Request`
variant; deferred.

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
the team's workers if it is served — but they are not merged in v1.

**`/new` semantics.** The team lives in-process, so it persists across `/new`
(same process, same `Orchestrator`). A `/reload` re-exec re-reads `[team]`.

### F4 — TUI multi-surface + team status sidebar

**Two tiers, landed in order.** Tier 1 is UI-only and cheap; tier 2 is the
separable socket work.

**Tier 1a — status sidebar.** Split the `body` band horizontally in `ui.rs`
`draw` and render a `draw_sidebar` pane: one line per teammate — `name · model ·
state` (idle / running / done) and, optionally, the first line of its last
result. The TUI learns the team from the composition root, which **injects** a
`Vec<Teammate>` (the same pattern as `models`/`sessions` today), since the TUI
owns no `Registry` and cannot enumerate one.

**Tier 1b — multi-surface.** Split `App`'s single-session state into a
`Surface { transcript, live, changes, status, running, cancelled, context_used,
scroll }` and make `App` hold `surfaces: Vec<Surface>` + `focus: usize` + the
**shared** composer (`input`, `cursor`, `history`, `overlay`, `completion`). The
reducer becomes `(SessionId, AgentEvent)`; the TUI subscribes to N `Backend`s —
the in-process `Registry` already maps `SessionId → Backend`, so no wire change is
needed. Focus switches via a picker — **reusing F1's `Picker`/selection
machinery** (a new `PickerKind::Surface`) or a key.

**Tier 2 (deferred).** One socket carrying many sessions: a `serve` that accepts a
set of handles and demuxes on `Frame.session`, and a `Client` that routes inbound
frames by that field (today the field is inert — "Single-session server (S2)").
Needed only once surfaces cross process boundaries.

## Decisions (locked)

| # | question | decision |
|---|----------|----------|
| D1 | F2 fields | `model` + appended `system` role + `tools` allow-list |
| D2 | F2 scope | **in-process only**; per-agent provider and remote-agent definition deferred (need new wire) |
| D3 | worker `message` tool | **always** kept regardless of the `tools` allow-list (the report path) |
| D4 | F3 shape | `[team]` = **local** spawned workers; `[peers]` stays the **remote** map |
| D5 | F3 startup | spawn the team after `Orchestrator::new`, mirroring the `[peers]` loop; `/new` keeps it |
| D6 | F4 order | **tier 1 first** — sidebar (1a) then in-process multi-surface (1b); socket multiplexing (tier 2) deferred |
| D7 | F1 popup | **non-modal inline**, reusing the picker's render/style — its own prefix matcher (D11) — **not** the modal `Overlay` |
| D8 | F1 table | TUI-local `Command` table; `/help` generated from it; REPL unification deferred |
| D9 | F1 keys | popup owns `Up`/`Down`/`Tab`/`Esc`; `Char`/`Backspace`/`Enter` fall through. While the popup is open `Up`/`Down` select rows **even for an exact token** (e.g. `/model`), so history recall is suppressed in that state — a deliberate change from before |
| D10 | sequence | F1 → F2 → F3 → F4 |
| D11 | F1 matching | completion matches case-insensitive **prefix** (canonical names + aliases); the **picker keeps substring** filtering — a separate matcher, not the shared one |
| D12 | F1 Enter | accept only when the token is not an exact name/alias; a **bare `/` never accepts** (no `/exit` trap) |
| D13 | F1 alias | `/sessions` aliases `/resume`, so `/ses` completes to it (aliases are dispatchable + suggestable, never shown in `/help`) |
| D14 | F2 unknown tool | `message` is **always valid** in an allow-list (a no-op — it is kept regardless); any other unavailable name — including `spawn` — **fails loudly** via the reusable `SessionFactory::validate_tools` (used by `spawn` and, in F3, the startup loop), never silently dropped |
| D15 | F2 session id | a worker gets its **own** `llm.session_id` (its address), not the root's `x-opencode-session` |

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

**Phase 3 — F3: team preset.** ☐
- [ ] `TeamMember` in `config.rs` (`[team]`), pass-through in `merge`.
- [ ] Start-up spawn loop in `main.rs` after `Orchestrator::new` (mirror
      `[peers]`), registering each name in the phonebook.
- [ ] Tests: a `[team]` parses; N members spawn and are addressable by name; an
      absent table leaves today's behavior; a bad member fails loudly.

**Phase 4 — F4: sidebar + multi-surface.** ☐
- [ ] **1a** `draw_sidebar` (horizontal split of `body`); `Vec<Teammate>` injected
      by the composition root; `name · model · state` lines.
- [ ] **1a** feed per-teammate state from their event streams (idle/running/done).
- [ ] **1b** split `App` → `Surface` + focused map; reducer keyed by `SessionId`.
- [ ] **1b** subscribe to N in-process `Backend`s (via `Registry`); focus switch
      (`PickerKind::Surface`, reusing F1) or a key.
- [ ] Tests: sidebar lists the injected team; two surfaces keep independent
      transcripts; focus switch routes input to the focused surface.

## Open questions

- **Per-agent provider** (a worker on a different `base_url`/key): a real need
  (cheap model for exploration, strong model for review), but bigger — `StreamFn`
  is built once. Deferred; revisit if the single-provider assumption bites.
- **Remote agent definition**: a new `Request` variant (e.g. `Define`) so a served
  peer can be customized. Blocked on the same wire work as tier-2 multiplexing.
- **Role as a preset vs free text**: `role` is free text appended to the prompt in
  v1; a named-role registry (skills-like) is a possible later layer.
- **Team in the TUI vs the CLI**: the sidebar is fed by injection today. Once
  tier 2 lands, the TUI could enumerate the `Registry` itself — decide then.
- **Tool allow-list granularity**: names only in v1; per-tool argument restrictions
  (path scopes) would be a `Hooks` impl, not config (stays true to the stance).
- **`/team` command**: once F3 lands, a `/team` to list/re-spawn members is
  natural — rides on F1's table, so it is cheap; scope with F4.

## Progress

| phase | scope | status |
|-------|-------|--------|
| 1 | F1 — TUI command table + inline completion (+ `Tab`) | ☑ done (reviewed) |
| 2 | F2 — customizable workers (`model`/`role`/`tools`) | ☑ done (reviewed) |
| 3 | F3 — `[team]` preset + startup spawn | ☐ todo |
| 4a | F4 — team status sidebar | ☐ todo |
| 4b | F4 — in-process multi-surface | ☐ todo |
| 5 | F4 tier 2 — socket multiplexing (deferred) | ☐ todo |

Legend: ☑ done · ◐ in progress · ☐ todo.
