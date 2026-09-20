# Session groups — team session persistence & resume

> Status: **design locked (D1–D5); sketch APPROVED with 3 amendments
> ([`session-groups-implementation.md`](session-groups-implementation.md) §1b);
> implementation in progress.** Companion to
> [`next-steps.md`](next-steps.md) (item 16). Kernel-free: this is a CLI-layer
> change (`wcode-cli`); `Session`/`Registry`/`WorkerSpec` already provide every
> primitive needed.

## 1. Problem

Team work today is not safely resumable. Two structural weaknesses:

1. **Workers are in-memory.** `SessionFactory::spawn` builds a worker with
   `session: None` (`crates/wcode-cli/src/agents.rs` — "v1 workers are
   in-memory (no session file)"). A crash, a `/new`, or a quit loses every
   worker's transcript and the team topology (`Registry` `peers`/`owners`/
   `models` live in RAM only, `crates/wcode-protocol/src/registry.rs:63`).
2. **Resume knows nothing about teams.** `--resume <path>` opens *one* JSONL
   and rebuilds *one* `Agent` from `s.messages()` (`main.rs:550`). There is no
   on-disk record of who the workers were, their `WorkerSpec`s, or the
   report-back/owner edges — so even a surviving root session cannot
   reconstruct its team.

And if we *added* naive per-worker persistence, `list_sessions` (every `.jsonl`
in the flat `sessions/` dir, newest first — `repl.rs:333`) would flood the
`/resume` picker with indistinguishable worker rows.

## 2. Design: one directory per root session (a "session group")

```
~/.local/share/wcode/sessions/
  <millis>_<id8>/              # one group dir per ROOT session
    root.jsonl                 # the root's transcript (today's flat file, moved in)
    manifest.json              # team topology — fast path only (see D5)
    members/
      w1.jsonl                 # each worker's own transcript
      w2.jsonl
  <old flat files…>            # legacy: still listed, treated as bare roots
```

- Group dir name = the root file's existing name minus `.jsonl`, so
  chronological listing stays lexicographic — identical to today.
- `manifest.json` = `{ group: <root id>, cwd, members: [WorkerSpec …] }`.
  A `WorkerSpec` already carries `name`/`model`/`role`/`tools`/provider
  (`base_url`/`api_key`) — **the spec that spawned a worker *is* its resume
  record**. Nothing new to invent.
- Member transcripts reuse `SessionEntry` unchanged — the kernel's `Session`
  (append, torn-write tolerance, compaction boundary) is untouched.

## 3. Write path (no behavior change while running)

- `SessionFactory::spawn` still builds + registers the worker exactly as today;
  additionally it now gets a `session: Some(Session::create(group_members_dir))`
  so the worker appends its JSONL live (workers become durable with zero extra
  machinery — the actor already persists via `Session::append`).
- When the root session is created, create the group dir + `root.jsonl`;
  workers create `members/<name>.jsonl` under it.

## 4. Resume path — the whole team comes back

`--resume <group>` (group id = dir name; the picker hands it back verbatim):

1. Open `root.jsonl` → build the root `Agent` from `s.messages()` (existing
   path, `main.rs:550`), restoring `model`/`effort` from the header as today.
2. Load group members **by scanning `members/`** (D5) → for each
   `members/<id>.jsonl`, open it, seed the worker's context with its **full
   transcript** (D1), build via the same `worker_config`-style path as
   `SessionFactory::spawn`, then `registry.register` + `set_owner` +
   report-back edge + names map. The team is live again, each member resuming
   exactly where it was — a crashed worker re-reads its own history, not a
   blank slate.
3. Workers whose file parses (torn-tail tolerant `Session::open`) join; a
   corrupted/missing member is reported, not fatal — the rest of the team
   resumes.

## 5. Locked decisions

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | Worker resume depth | **Full transcript** (compaction allowed, same knob as root) | A resumed worker keeps its whole thread; compaction is the existing escape hatch for token cost. |
| D2 | Stale-state on resume | **Let the digest-CAS catch it** | Resume stays dumb: no digest bookkeeping at load; the first `edit` against a moved base refuses (refuse-and-ask) and the tool layer self-validates. No extra state. |
| D3 | Claims across resume | **Start clean** | Claims are advisory and RAM/Registry-scoped; resuming is a fresh session, stale claims are noise. Freed on session death as today. |
| D4 | Remote members (`spawn { to }`) | **Document as out of scope** | Group resume rebuilds *local* members. A served peer that defined workers remotely persists its own side; this doc notes it, no code. |
| D5 | Manifest vs directory | **Scan `members/` as source of truth**; manifest = fast path only | The dir listing cannot be half-written (a file either exists or not); a crash between file and manifest entry re-syncs by scan. |

## 6. Out of scope (now)

- Team-session *grouping* in the TUI beyond the picker (a team strip for a
  resumed group already works — members are ordinary sessions).
- Persisted claims/leases, remote-member resurrection (D4), session *forking*/
  branching (the kernel's "reserved for future tree/fork sessions" note stays).

## 7. Implementation sketch (what the developer fills in)

All in `wcode-cli`; kernel and protocol untouched.

- `src/session_groups.rs` (new): `SessionGroup` (dir, root path, members),
  `open_group(dir) -> io::Result<SessionGroup>`, `list_groups()` (dirs newest
  first + legacy flat files), `load_member_specs(group)` (scan `members/`,
  read each `WorkerSpec`), and the resume assembly
  `rebuild_team(group, registry, factory) -> Vec<SpawnedWorker>`.
- `src/repl.rs`: `session_dir()` unchanged; `list_sessions` extended to return
  **group dirs** (and legacy files) — member files never listed.
- `src/main.rs`: `session_items`/`session_label` label groups by root
  transcript; the `--resume` arm (the `Some(path)` branch at `main.rs:550`)
  calls `open_group` + `rebuild_team` instead of only `Session::open`.
- `src/agents.rs`: `SessionFactory::spawn` gains group-member persistence
  (`session: Some(...)`); `worker_config` unchanged otherwise.
- Tests: pure unit tests for `list_groups` (groups + legacy mixed), torn-tail
  member open, `rebuild_team` closure (registry has all members, owners set,
  names map populated), and a resume round-trip (spawn team → drop registry →
  rebuild → members report to the root).

## 8. Sequencing

1. `session_groups.rs` sketch (interfaces + skeletons — [`session-groups-implementation.md`](session-groups-implementation.md)).
2. Developer fills in layout + picker + resume.
3. Live end-to-end check: spawn a team, kill the process, `--resume` the group,
   confirm each member replies with prior context intact.