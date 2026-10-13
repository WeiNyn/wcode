# D024 — Orchestrator team control at runtime

- **Status:** accepted (human sign-off — W020; the brief's §0 amendments)
- **Date:** this session
- **Relates to:** D017 (named project teams — `./.wcode/teams/<name>.toml`),
  D003 (the `[[team]] file =` routing hint), D014 (a loud tool allow-list,
  no silent drop), D018 (duplicate `[team]` names rejected, not overwritten),
  the A2A `Request`/`Registry` seam (`docs/interface-protocol-brainstorm.md`),
  the `Task.owner` DAG (`docs/task-dag-plan.md`)
- **Work item:** [`work/W020-orchestrator-team-control.md`](../work/W020-orchestrator-team-control.md)
  · **Tracker:** [`next-steps.md`](../next-steps.md) item 82

## Context

The root orchestrator could `spawn` a teammate and `message` it, but had no
runtime control over the team it already had: no way to change a teammate's
**model** or **effort** post-spawn, to **retire** one, to **compact** a running
peer's history, to spawn from a discovered `.wcode/agents/<name>.md` role *by
name*, or to **apply** a named `./.wcode/teams/<name>.toml` to a session that did
not start with one. W020 adds all six as **model-facing tools only** — no
TUI/REPL `/`-command, no VS Code command (the brief's §0 amendment 3).

Two of the six needed a decision because the honest behaviour is narrower than
"the obvious thing":

1. **Retire** — what happens to the member's persisted transcript, and does it
   come back on `--resume`?
2. **Apply-team** — the root's `# Your team` **system-prompt block** is composed
   at startup (`repl::system_prompt`); `Agent` exposes no system-swap API (only
   `set_plan_mode` recomposes `base_system`). Can a live apply update the prompt?

## Decision

1. **Retire is live *and* durable — the file survives.**
   `member { op: "retire" }` cancels the member if running
   (`member.rs`, the `cancel_if_running` poll loop), then
   `Registry::remove` (`registry.rs` — drops `peers`/`owners`/`models`/`states`/
   `efforts`/`windows` and re-publishes the shrunk roster) and
   `Phonebook::remove` (`agents.rs`). Then it sets a **`retired: bool` flag** on
   the member's `MemberRecord` in the group manifest
   (`session_groups::retire_member`). The member's `.jsonl` stays on disk as an
   **audit trail**; the flag is what suppresses the respawn — `rebuild_team`
   reports a retired member as `MemberResume::Skipped { reason: "retired" }` and
   does not rebuild it (no new enum variant: the `--resume` summary loop matches
   exhaustively). A **recordless** member (the phantom-member window — a header
   file written before its manifest record) still resumes, so retirement only
   ever suppresses a member *with* a `retired` record. The flag mirrors
   `read_only`'s serde shape (`#[serde(default, skip_serializing_if = "is_false")]`),
   so an older manifest reads absent-as-false.
   A live task node that **owns** the member (`Task.owner`) **blocks** retire
   unless `force: true`, so the DAG never loses a worker mid-task.

2. **A live apply joins members only — the prompt is NOT recomposed.**
   `team { name }` (`tools/team.rs`) parses the team file's `[[team]]` **directly**
   (a team-only deserialize — routing the file through `Config::load_with` would
   demand a full config with a `model`), resolves any `[[team]] file =` member via
   `agent_files::resolve_member_files`, and spawns each through
   `Orchestrator::spawn_worker` (the loud allow-list + duplicate-name gates apply).
   The root's `# Your team` block is **unchanged** — the same v1 boundary stated
   in the brief's §2. The prompt recompose is a **named follow-up**, not part of
   W020.

## Consequences

- **Partial apply is non-transactional.** There is no un-spawn, so if a later
  member fails (e.g. a duplicate name), the members that already joined **stay**.
  The tool's output names **how many** joined before the failure, and the session
  is marked "team applied" once **≥1** joined — so a retry is **refused** rather
  than colliding on the duplicate name it already hit.
- **A re-apply is refused for *any* name.** The guard is
  `applied_team().is_some()` — a `[team]`-at-startup session (seeded with the
  marker `"[config]"`) or a prior live apply (the team's name). The refusal never
  names a file that was never applied.
- **The apply-team path duplicates `main.rs::resolve_team_files`.** The runtime
  twin lives in `agent_files::resolve_member_files` (returns `String`, skips the
  final-uniqueness check — the `team` tool's duplicate check is `spawn_worker`'s).
  The two share *intent*, not *code*; hoisting `main.rs`'s body onto the runtime
  one is a **follow-up**. Until then, `agent_files` tests pin the **same override
  rules** (name agreement; a sibling inline key wins; `read_only` `true` wins),
  so the two cannot silently diverge.
- **`Registry::remove` leaves no stale `peers` entry.** The per-session
  state-watcher `register` spawned can re-insert a `states` entry after a
  `remove` returns, but **never** a `peers` entry — so `locals()`/`contains`/
  `resolve` report the id gone at once. No `Request` ends a *served* peer's
  session over a socket: retire is root-local (the brief's §2 non-scope).
- **Which member op ACKs mid-run.** `SetModel`/`SetEffort`/`SetPlanMode` are
  re-queued and **ACKed immediately** even mid-run (`actor.rs`), so `model`/
  `effort` never hang; `Compact` is **not** in that list, so it defers to the
  next turn boundary and can time out — `member`'s `compact` waits
  `COMPACT_TIMEOUT` (120 s) and, on timeout, says "busy — its run must end
  first" rather than reporting a blind send.
- **No human surface.** All six capabilities are tools; there is no TUI/REPL
  `/`-command and no VS Code command (amendment 3), so no new visual element and
  `docs/tui-design.md` is untouched.
- **Spawn-from-md keeps arg-over-pool.** `SpawnArgs::to_worker_spec` stays a pure
  1:1 map; `Spawn::spec_for` layers the resolved md on top, so an explicit arg
  always wins and only the fields the args left `None` come from the md. An
  unknown `agent` name errors, listing the available roles.
