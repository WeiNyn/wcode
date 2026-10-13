# W020 — Orchestrator team control (runtime) — brief

- **Status:** **SHIPPED** — second-layer review APPROVED (`d3a4fcf`). Branch
  `w020-orchestrator-team-control`; commits `eefaea6` · `186626e` · `3ad8596` ·
  `c467250` · `d3a4fcf`. Gates: `cargo test --workspace` 1219 passed/0 failed ·
  `cargo clippy --workspace --all-targets` clean · zero `SKETCH` markers.
- **Decision record:** [D024](../decisions/D024-orchestrator-team-control.md) · tracker item 82.
- **Work item:** W020 (`docs/work/W020-orchestrator-team-control.md`)
- **Decisions:** proposes a new **D0NN** (retire semantics + the live apply-team / prompt gap)
- **Author:** brainstormer (read-only brief; recorded by the orchestrator)
- **Revision read:** `d53e260` (`chore: release 0.7.0`)

## §0 Human sign-off amendments (this session)

1. **Op 6 = option (a).** Applying a team to a live session **joins members only**;
   the root's `# Your team` system-prompt block stays as composed at startup. The
   live-prompt recompose is a named follow-up, not part of W020.
2. **Retire is durable.** Add a `retired` flag to the group manifest
   (`MemberRecord`) so a retired member **does not respawn on `--resume`**. The
   member's `.jsonl` file stays on disk; the flag is what suppresses the
   respawn.
3. **No human surface.** All six capabilities are **model-facing tools only**.
   No TUI/REPL `/`-command, no VS Code command (drops the brief's W6 and plan
   step 7).

## §1 The ask

Give the **root orchestrator** first-class runtime control over its team, driven by
model-facing tools:

1. change a teammate's **model** post-spawn;
2. change its **effort**;
3. **retire** a teammate (remove from roster + phonebook + registry; decide the
   session/group file's fate);
4. **compact** a running peer's conversation;
5. **spawn** a teammate from a markdown agent file (`.wcode/agents/<name>.md`) by
   *name*, not pasted `role` text;
6. **apply** a config team file (`.wcode/teams/<name>.toml`) to the current session
   when it is not already a team session.

Each needs a tool path; the TUI/REPL `/`-command and VS Code surface are decided,
not assumed.

## §2 Scope / non-scope

**In scope**
- New `member` tool ops `model`, `effort`, `compact`, `retire` (ops 1–4) with args
  + tests.
- `Registry::remove` and `Phonebook::remove` (+ roster re-publish).
- A `spawn` arg to resolve a `.wcode/agents/<name>.md` role by name (op 5).
- A root-local `team` tool to apply a `.wcode/teams/<name>.toml`'s members to a
  *live* session (op 6), **members join only**.
- A `retired` flag on the group manifest (`MemberRecord`) so a retired member
  does **not** respawn on `--resume` (amendment 2).
- One decision record (D0NN) for retire semantics + the apply-team/prompt gap; a
  tracker item.

**Non-scope (with why)**
- **Recomposing the root's system prompt at runtime.** `system_prompt` composes the
  `# Your team` block at startup from `team: &[TeamMember]`
  (`crates/wcode-cli/src/repl.rs:44`, `:71`), and `Agent` exposes no system-swap API
  — only `set_plan_mode` recomposes `base_system` (`crates/wcode-harness/src/agent.rs:132`).
  Op 6 therefore *joins members without touching the prompt*; the prompt block is a
  named follow-up. — *Not papering over: stated as a v1 boundary.*
- **Durable retirement across restart.** The live retire mutates the
  registry/phonebook/roster **and** sets the manifest `retired` flag so the member
  does not respawn (amendment 2). Deleting the member's `.jsonl` (and rewriting
  the group) is NOT done — the file stays as an audit trail.
- **A human surface.** No TUI/REPL `/`-command and no VS Code command for any of
  the six — tools only (amendment 3).
- **Killing a peer's session over a socket.** No `Request` exists to end a served
  peer's session; retire is root-local.
- **Per-member context-menu UI / new TUI panels.** No new visual element; commands
  and tool ops only (`docs/tui-design.md` untouched).
- **MCP / permission prompts / approval** — absent by design (`AGENTS.md`).
- **A second kernel path** — all reuse `Request`/`Registry` seams.

## §3 Estimates

| # | workstream | value | complexity | risk | why |
|---|-----------|-------|-----------|------|-----|
| W1 | `member` ops `model`/`effort` (1,2) | high | low | low | kernel seam exists (`Request::SetModel/SetEffort`); two `ask_within` arms + roster mirror. |
| W2 | `retire` (3) | high | med | med | needs new `Registry::remove`/`Phonebook::remove` + roster re-publish; touches the live multi-surface roster. |
| W3 | `compact` a peer (4) | med | low | med | `Request::Compact` exists, but mid-run it is *deferred* (`actor.rs:621`) — timeout/first-cancel discipline needed. |
| W4 | spawn-from-md (5) | high | med | low | discovery exists (`agent_files.rs:119`); the gap is plumbing the pool into the `spawn` tool. |
| W5 | apply-team live (6) | med | high | high | members can join, but the prompt cannot recompose — the honest scope is narrow. |
| W6 | docs + decision record | low | low | low | retire semantics + apply-team gap warrant a D0NN. |

## §4 Interface & structure

### 4.1 New units

**`member` tool ops** — `crates/wcode-cli/src/tools/member.rs`. Today:
```
30  #[derive(Deserialize, schemars::JsonSchema)]
31  pub struct MemberArgs {
32      /// The member's name or address (`w1` or `agent:w1`).
33      to: String,
37      op: String,
40      text: Option<String>,
```
Add fields `model: Option<String>`, `effort: Option<String>`,
`instructions: Option<String>`, `force: bool`; new arms in `execute` (member.rs:177):
```
177  match args.op.as_str() {
178      "status" => self.status(&to).await,
```
Add `"model" | "effort" | "compact" | "retire"`. `target()` (member.rs:60) already
resolves phonebook→address; reuse it. Model/effort go through `Backend::ask_within`
(mirroring `status` at member.rs:70):
- `Request::SetModel { model }` → `AgentEvent::Ack` (`crates/wcode-harness/src/actor.rs:397`).
- `Request::SetEffort { effort }` → `Ack` (`actor.rs:403`); `-`/`none`/`off` → `None`.
- After an `Ack`, mirror into the roster: `registry.set_model(id, m)` /
  `set_effort` (`crates/wcode-protocol/src/registry.rs:209`, `:242`) — **the socket
  server already does this for remote peers** (`crates/wcode-protocol/src/server.rs:345`,
  `:359`), so the local path must match.
- `Request::Compact { instructions }` → `AgentEvent::Compaction { summarized, kept }`
  (`actor.rs:413`), `Ack` on `NothingToDo`, or `Error`.

Timeouts: reuse the module consts (member.rs:22 `STATUS_TIMEOUT`, `:24` `ASK_TIMEOUT`,
`:26` `CANCEL_TIMEOUT`); add `COMPACT_TIMEOUT: Duration = from_secs(120)`.

**`Registry::remove`** — `crates/wcode-protocol/src/registry.rs`. Confirmed absent
(`grep 'fn remove|unregister|deregister' crates/wcode-protocol/src` → no matches).
Add, beside `register` (`:108`) / `locals` (`:160`):
- drop `peers`, `owners`, `models`, `states`, `efforts`, `windows` entries for `id`;
- call `refresh_roster()` (`:190`) so `subscribe()` (`:181`) re-publishes and the
  socket server pushes the shrunk roster (`server.rs:213`).
- Note: the per-session state-watcher task `register` spawned (`registry.rs:121`)
  ends only when the session drops; a lingering watcher can re-insert a `states`
  entry but never `peers`, so `locals()` stays clean — document it.

**`Phonebook::remove`** — `crates/wcode-cli/src/agents.rs:52` (`insert`), `:57`
(`get`), `:62` (`entries`); no `remove`. Add one beside them.

**`retire` op** — cancel-if-running (reuse the poll loop at member.rs:131), then
`registry.remove(id)` + `phonebook.remove(short_name)`. Refuse (or warn) when a task
node names the member: `Task.owner: Option<SessionId>`
(`crates/wcode-cli/src/tasks.rs:113`). Inspect the shared `TaskList`; the `member`
tool does not hold it today (`agents.rs:566` builds `Member` with registry/me/phonebook
only) — **either** pass `TaskList` in **or** keep the DAG guard in the
`Orchestrator::tools` wiring.

**`spawn` from a md role** — `crates/wcode-cli/src/tools/spawn.rs:16` `SpawnArgs`. Add
`agent: Option<String>`; resolve it against the discovered pool → fill `WorkerSpec`
fields not otherwise set, default `name` to the md `name`. Discovery already exists:
`crates/wcode-cli/src/agent_files.rs:119` `pub fn discover(cwd, home)`, `:81`
`parse_agent_md` → `TeamMember` (`{name, model, role, tools, base_url, api_key,
read_only, effort}`). Unknown name → error **listing available names** (the
`load_config_raw` precedent: `main.rs:563`, `"available: {…}"`).

**Pool plumbing (the real op-5 gap).** The md set is folded into `cfg.team` at startup
(`main.rs:748` `fold_discovered_members`), consumed in the `[team]` spawn loop
(`main.rs:1489`), and **not** carried into `Runtime` (`main.rs:1540`
`Runtime { instructions, skills, hooks, orchestrator }`). So the pool must be threaded
to the `spawn` tool — a new field on `Orchestrator`/`Spawn`, populated in
`build_runtime` from `cfg.team` (the resolved `Vec<TeamMember>`).

**`team` tool (op 6, live apply)** — new tool in `Orchestrator::tools` (`agents.rs:549`).
Resolve `.wcode/teams/<name>.toml` via `crates/wcode-cli/src/teams.rs:67`
`overlay_path(name)`, parse+merge it as a config overlay, and for each `[[team]]`
member call `Orchestrator::spawn_worker` (`agents.rs:590`) — validate tools loud (D14),
reject duplicate names (existing). Guard: refuse when the session is already a team
session. **The prompt is not recomposed** (see §2). The alternate path — the TUI's
re-exec — stays: `Outcome::Team(String)` (`crates/wcode-tui/src/lib.rs:326`) handled at
`main.rs:1994`.

### 4.2 Changed units

- `crates/wcode-cli/src/tools/member.rs` — `MemberArgs`, `Member::execute`,
  `description()` (`:168`).
- `crates/wcode-cli/src/agents.rs` — `Phonebook` (+`remove`), `Orchestrator::tools`
  (wire `Member` with `TaskList`/pool; add `team` tool).
- `crates/wcode-protocol/src/registry.rs` — add `remove`.
- `crates/wcode-cli/src/tools/spawn.rs` — `SpawnArgs` (+`agent`), `to_worker_spec` (`:56`).
- `crates/wcode-cli/src/main.rs` — thread the resolved team pool into
  `build_runtime`/`Orchestrator`.
- `crates/wcode-tui/src/app.rs` — `COMMANDS` (`:573`) + `run_command` (`:3552`) new arms.
- `crates/wcode-cli/src/repl.rs` — `Command` enum (`:104`) + parser (`:149`) + dispatch.
- `editors/vscode/src/commands.ts` — `SLASH_COMMANDS` (`:37`) (decide; see §7.2 note).

## §5 The plan

1. **`Registry::remove` + `Phonebook::remove`.** Deliverable: both APIs + roster
   re-publish. Gate: new unit tests — `remove(id)` drops `contains`/`states`/`models`,
   `subscribe()` wakes, `locals()` excludes it; `Phonebook::remove` drops `get`.
2. **`member` ops `model`/`effort`.** Deliverable: two arms + args + description.
   Gate: unit tests on the `member.rs` fake-stream harness (`text_stream`,
   `hanging_stream`, member.rs:260–276) asserting `Ack`→output and roster mirror; a
   refused `ask_within` timeout message mirrors `status`.
3. **`member` op `compact`.** Deliverable: arm + `COMPACT_TIMEOUT`. Gate: a
   `Compaction` reply is surfaced; a **Running** member is handled (cancel-first or a
   clear "busy, run ends first" message — decided in W7's record).
4. **`member` op `retire`.** Deliverable: cancel-then-remove arm, phonebook drop,
   DAG-dependent guard. Gate: after retire `registry.contains(id) == false`,
   `phonebook.get(name) == None`, `subscribe()` reflects it; a task-owned member is
   refused/warned.
5. **`spawn` `agent` (spawn-from-md).** Deliverable: pool plumbing + resolution +
   unknown-name listing. Gate: `spawn { agent: "judge", task }` yields a worker whose
   spec carries the md's `model`/`role`/`tools`/`read_only`/`effort`; unknown name
   errors with the available list.
6. **`team` tool (live apply).** Deliverable: resolve `.wcode/teams/<name>.toml` →
   `spawn_worker` each member; refuse an already-team session. Gate: applying a temp
   team file adds its members to the roster/phonebook; a re-apply of the same name is
   refused.
7. **Manifest `retired` flag (amendment 2).** Deliverable: `MemberRecord.retired:
   bool` (`#[serde(default, skip_serializing_if = "is_false")]`); a
   `session_groups::retire_member(group, name)` that upserts the flag; and
   `rebuild_team` **skips** a member whose record is `retired`. Gate: unit test —
   record a member, retire it, reopen the group → the member is not restored, and a
   member with no manifest record still resumes (the phantom-member rule is intact).
8. **Docs + decision record.** Deliverable: D0NN (retire = live + manifest flag;
   apply-team joins without prompt recompose), a `docs/next-steps.md` row. Gate:
   `--dump-system-prompt` unchanged for a teamless root; the record's claims match the
   code.

## §6 Quality gates

- `cargo test --workspace` — all pass.
- `cargo clippy --workspace --all-targets` — clean.
- Live (`live-verification` skill), keyless local endpoint `--base-url http://localhost:11434/v1`:
  `wcode --agents -p 'spawn a worker w1; then member {to:"w1", op:"model", model:"…"}; then member {to:"w1", op:"retire"}; then peers'`
  → the roster shows the change and then the removal.
  Refused-port path: `WCODE_RETRY_MAX=2 wcode --base-url http://127.0.0.1:9/v1 …`.
- Regression tests that must stay green (by name): `member.rs::status_reports_state_and_last_work`,
  `::cancel_stops_a_running_member`, `::unknown_op_is_an_error`;
  `agents.rs::spawn_registers_and_owns_the_worker`,
  `::spawn_emits_a_surface_spec_on_success_only`;
  `registry.rs::set_model_and_set_effort_republish_the_roster`,
  `::subscribe_yields_the_new_roster_after_register`;
  `spawn.rs::spawn_args_map_onto_a_worker_spec`,
  `::a_spawn_to_a_remote_peer_defines_and_starts_the_worker`;
  TUI `app.rs::add_surface_adds_a_member_row_and_leaves_focus`.

## §7 Testing

### 7.1 Automated
- **`member` ops (unit, fake stream).** `op:"model"` → `Ack` → output names the model
  and `registry.model_of` reflects it; `op:"effort"` with `"-"` clears;
  `op:"compact"` surfaces `summarized/kept`; `op:"retire"` → `contains == false`,
  phonebook empty, roster shrinks.
- **`Registry::remove` (unit).** `contains`/`state_of`/`model_of` cleared and a
  `subscribe()` receiver wakes with the smaller list.
- **`spawn` from md (seam).** A temp `.wcode/agents/judge.md` → `spawn { agent: "judge" }`
  builds a spec equal to the md; an unknown name errors listing names.
- **`team` tool (seam).** A temp `.wcode/teams/scout.toml` → members appear in the
  roster/phonebook; a second apply is refused.
- **Named regression pair** (the kernel/CLI seam): `member` ops against a live
  `SessionActor` registered in a `Registry` — proving the `Request` round-trips a reply.

### 7.2 Human verification
```sh
cargo run -p wcode-cli -- --agents --base-url http://localhost:11434/v1 \
  -p 'spawn {task:"say hi", name:"w1"}; then call member {to:"w1", op:"effort", effort:"low"}; then member {to:"w1", op:"retire"}; then peers'
```
Look for: the effort op's `Ack`, then `peers` no longer listing `w1`, and (over
`--socket`) the sidebar losing the row. Served-peer check: `wcode serve --agents --socket
/tmp/a.sock` in one shell, drive `SetModel` to a member — the peer's roster push
(`server.rs:220`) must show the new model. The VS Code surface
(`editors/vscode/src/commands.ts`) is **client-local**: a `/member`-style command needs
a host handler (likely W6b, decide in the record).

## §8 Expected outcome
- `member` accepts `model`/`effort`/`compact`/`retire`; the root can retune and drop a
  teammate mid-run by tool call.
- `spawn { agent: "<role>" }` spawns from `.wcode/agents/<role>.md` without pasting
  text; an unknown role lists what is available.
- A `team` tool applies a `.wcode/teams/<name>.toml`'s members to a live non-team
  session; the root's `# Your team` prompt is **unchanged** (documented).
- A retired member is gone from `peers`, the registry, and the roster, and does
  **not** respawn on `--resume` (manifest `retired` flag); its `.jsonl` remains on
  disk as an audit trail.
- `Registry::remove` / `Phonebook::remove` exist and are unit-tested; a decision record
  explains retire and the prompt gap.

## §9 References
- Sibling skill: `brainstorm-brief`; conventions: `wcode-conventions`, `live-verification`,
  `traceability-log`.
- Closest prior art: tracker item 40 (`team-controls-plan.md`), item 74 (D017 named
  teams), item 57 (`work/W001-…`).
- `docs/interface-protocol-brainstorm.md` (the `Request`/`Backend` seam),
  `docs/team-and-tui-plan.md` (F1–F4), `docs/task-dag-plan.md` (the `Task.owner` DAG).

## §10 Facts NOT verified
- **`Orchestrator::tools` can reach the shared `TaskList`** for the retire DAG-guard —
  it holds `self.tasks` (`agents.rs:525`) so this is *likely* trivial, but the `Member`
  wiring path is not yet exercised.
- **The exact `Ack` vs deferred behaviour of `Ask{Compact}` mid-run** — `actor.rs:610-621`
  defers everything but `Cancel`/`Notify`/`Interrupt`/`Wake`/`GetHistory`/`Status`/`SideAsk`;
  `Compact` is read as deferred, not run live.
- **Whether removing a member leaves a stale entry** via the per-session watcher
  (`registry.rs:121`) — reasoned from `locals()` reading `peers`, not empirically.
- **The VS Code host's request routing for a member op** — `commands.ts` is the pure
  table; the host handler was not read.
- **Line numbers** drift; every citation pairs the number with a quoted snippet.

## §11 Review log

| gate | outcome |
|---|---|
| First layer (sketch) | **BLOCKED** — 4 blockers (out-of-scope dirty files; two missed `Member::new`/`Spawn::new` call sites; the team-file parse could not route through `Config::load_with`), settled as amendments A1–A13. |
| Implementation | `eefaea6` (registry remove) · `186626e` (member ops) · `3ad8596` (pool + team tool) · `c467250` (docs). One turn-limit stop mid-flight (red build) — resumed and finished. |
| Second layer | **BLOCKED** once (B1: the `team` partial-apply branch was untested) → `d3a4fcf`. Then **APPROVED**. |

Carried-forward, non-gating: `Orchestrator::pool()` `#[allow(dead_code)]` (tests-only);
`#[derive(Default)]` on `SpawnArgs`/`MemberArgs` (test convenience); the retire DAG guard does
not filter by task state (a `Done` node still blocks without `force`); `spawn_remote` + `agent`
untested (only the in-process path exercises `spec_for`); `resolve_member_files` duplicates
`main.rs::resolve_team_files` (hoist deferred, D024).

**Not verified:** no live model-driven E2E and no served-peer socket check (no endpoint was
listening) — the six capabilities rest on kernel/CLI `Request`/`SessionActor` seam tests plus a
refused-port boot run that exercised the startup team.
