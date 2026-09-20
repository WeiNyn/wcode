# Session groups — implementation sketch map (Item 16)

Status: design locked (`docs/session-groups.md`, D1–D5). This is the **sketch
step** (design §8 step 1): the interface surfaces a developer fills in. The
sketched module is `crates/wcode-cli/src/session_groups.rs` (new); every
integration point below is grounded in the current code with `file:anchor`
evidence. **No existing file has been modified.**

The other two outputs of this step: the sketch itself
(`crates/wcode-cli/src/session_groups.rs`) and the first-layer review sheet
(`docs/session-groups-review.md`).

## 0. Ground truth read (what the sketch integrates with)

| Point | Where it lives today |
|---|---|
| `SessionFactory::spawn(owner, spec) -> Result<SpawnedWorker, String>` | `crates/wcode-cli/src/agents.rs` (`jFyJ5`) |
| `SpawnedWorker { id }` | `agents.rs` (`4Waak`) |
| `worker_config(id, owner, spec) -> AgentConfig`, private, `session: None` / `context: Vec::new()` | `agents.rs` (`XtJQp`, `Sfc3L`, `7MXo4`) |
| v1 in-memory contract ("workers are in-memory (no session file)") | `agents.rs` module doc (`yUykl`) |
| `WorkerSpec` — `#[derive(Clone, Default)]` only (NOT serde) | `agents.rs` (`ZV2bh`) |
| `Phonebook` insert/get/entries; `short_name()` | `agents.rs` (`f9yvO`, `qMdzx`) |
| `Orchestrator::new` (registry + factory, id `agent:orchestrator`), `spawn_worker` inserts phonebook | `agents.rs` (`wjKJg`, `yM6Dm`) |
| `Session::create` / `create_with_cwd` / `open` (torn-tail) / `in_memory` / `append` / `messages` / `model` / `effort` | `crates/wcode-harness/src/session.rs` (`bP9l7`, `MZV2E` incl. `{millis}_{uuid8}.jsonl` naming at `0oxzb`, `ilfeh`, `qreTu`, `EcNck`, `pUlH8`, `HyYmb`, `UsWF4`) |
| `SessionEntry` (Header/Message/ModelChange/EffortChange/Compaction/Unknown) | `session.rs` (`YK8j1`) |
| `session_dir()` (`~/.local/share/wcode/sessions`), `list_sessions` (`.jsonl` only, newest first), `resolve_session_path` (existing path verbatim) | `crates/wcode-cli/src/repl.rs` (`XSulR`, `xdqdj`, `lUzPx`) |
| REPL `/resume` arm — opens + rebuilds in place | `repl.rs` (`jzVeZ` → `3qfu3`) |
| `--resume` arm in `main` — `Some(path)` branch, `Session::open`, model/effort restore | `crates/wcode-cli/src/main.rs` (`YLlQv`, `SU1ct`, `MKBuB`/`Mu9Z9`) |
| orchestrator/factory construction (AFTER the resume arm) | `main.rs` (`sY76a`) |
| `[team]` startup spawn loop (skipped for `--owner`) | `main.rs` (`jwG8v`) |
| `session_items()` / `session_label()` / `age_millis()` / `first_user_line()` | `main.rs` (`LfAZa`, `PUpZr`, `QEuhm`, `MW2yE`) |
| TUI `/resume` handoff — re-exec `--resume <SessionItem.path>` | `main.rs` (`KeoMe`); `SessionItem { label, path }` at `crates/wcode-tui/src/app.rs` (`BLVVX`) |
| `Registry::register` / `set_owner` / `set_model` / `contains` / `permitted` / `resolve` / `deliver` / `subscribe` | `crates/wcode-protocol/src/registry.rs` (`xJEuJ`, `XCmse`, `Oxg6W`, `s4TFm`, `AMC5l`, `Tq0bK`, `TobHl`, `iPJPJ`) |
| `SessionId::agent(name)` / `new` / `as_str` | `crates/wcode-harness/src/protocol.rs` (`8NEdF`, `cfjEj`, `wczz0`) |
| remote `spawn { to }` (D4: documented out of scope) | `crates/wcode-cli/src/tools/spawn.rs` (`9Wwgg`) |

## 1. The new module — `crates/wcode-cli/src/session_groups.rs`

Full signatures, contracts and `todo!()` bodies are in the module; this map
only summarizes the shape and the design decisions it encodes:

- `SessionGroup { dir, root_path, members_dir }` + derived accessors
  (`id()`, `root()`, `members()`, `manifest_path()`, `member_path(name)`).
- `GroupEntry::{Group(SessionGroup), Legacy{path}}` with `path()` / `id()` /
  `is_group()` — what the picker lists and what `--resume` consumes.
- Write path: `create_group(base)`, `create_root_session(dir, cwd)`,
  `create_member_session(members_dir, name)`, `validate_member_name(name)`,
  `record_member(group, spec)` (manifest upsert by `SessionFactory::spawn`).
- Discovery: `list_groups(dir)` (newest first, dirs + legacy files, members
  never listed), `is_group_dir_name(name)` (pure), `open_group(dir)`.
- Manifest (D5 fast path): `Manifest { group, cwd, members: Vec<MemberRecord> }`
  with `load_manifest` / `write_manifest` / `member_spec(group, name)`
  (manifest record → `WorkerSpec`, else default) / `MemberRecord
  {from,to}_worker_spec`.
- Members: `scan_members(group)` (source of truth), `member_id(path)`.
- Resume: `load_root(group)`, `rebuild_team(group, factory, phonebook, root)
  -> Vec<MemberResume>`, `MemberResume::{Restored{id, messages}, Skipped{name,
  reason}}`, `SpawnedMember { id }` (design-name parity).

Deviation from the design doc's sketched signature, with reason: the doc's
`rebuild_team(group, registry, factory)` passes a `registry` the `SessionFactory`
already owns (`factory.registry()`, `agents.rs:yQDCK`) and omits the names
map, which lives in the `Phonebook` the factory never sees. The sketch takes
`(group, &SessionFactory, &Phonebook, &SessionId)` instead, and returns
`Vec<MemberResume>` (corrupt members are non-fatal §4.3) rather than
`Result<…>` (hard failures belong to `open_group`/`load_root`, which the `main`
arm already treats as fatal).

## 1b. Review verdict (first-layer gate, APPROVE + 3 mandatory amendments)

Reviewer: **APPROVE** — sketch faithful to D1–D5, all six friction findings
verified true; fillable without redesign. Two new frictions found at review
(6a, 6b below) are local fills gating the *implementation*, not the module
contract. All review-sheet questions A–H answered **Option 1** (see
`session-groups-review.md` decision table): WorkerSpec derives serde (A);
manifest = spec source, scan = membership source (B); collision → prefer
group, hide legacy (C); lexicographic-desc sort (D); additive
`Session::create_named*` (E); REPL `/resume` arm extended (F); every root a
group, no migration (G); `WorkerTemplate.members_dir: Option<PathBuf>` (H).

**Mandatory punch-list amendments (must land before step 3 — Resume):**
- **6a — Q-reachability.** `rebuild_team` cannot receive `&SessionFactory` +
  `&Phonebook` as sketched: `Orchestrator`'s fields are private
  (`agents.rs:IueNb`) with accessors only for `registry()`/`id()`. Add
  `pub fn factory(&self)` + `pub fn phonebook(&self)` to `Orchestrator`, or
  reshape `rebuild_team(group, &Orchestrator, root)` mirroring `spawn_worker`'s
  encapsulation (preferred — a second factory/phonebook would diverge the
  registry edges/name map from the live tools).
- **6b — file-shaped re-entry.** After group resume, `session_path()` =
  `<dir>/root.jsonl`, so `/reload` (`repl.rs:653` → `reload_args` `JOsBk`)
  re-execs `--resume <dir>/root.jsonl`, which the arm treats as a flat file →
  team silently dropped. Fix: resume arm maps `<dir>/root.jsonl` up to its
  group (or `/reload` carries the dir). Must be a listed touch-point.
- **6c — `Manifest.cwd` is dead.** Nothing sets it; it duplicates
  `root.jsonl`'s header `cwd`. Give it a writer or drop the field (drop
  preferred — one source of truth).

## 2. Planned changes to existing files (the developer's punch list)

Each row is the exact touch-point; new signatures replace or extend what is
anchored.

1. **`crates/wcode-harness/src/session.rs`** — additive (pending review Q(E)):
   - `pub fn create_named(dir: &Path, name: &str) -> io::Result<Session>` and
     `pub fn create_named_with_cwd(dir: &Path, name: &str, cwd: &Path) ->
     io::Result<Session>` — like `create`/`create_with_cwd` (`MZV2E`) but the
     file is exactly `<dir>/<name>.jsonl` (header content identical). Needed
     because the design's tree requires `root.jsonl` and `members/w1.jsonl`
     (deterministic names) while `create_with_cwd` hardcodes
     `{millis}_{uuid8}.jsonl` (`0oxzb`). No change to append/torn-tail/
     compaction/`messages()` — the design's "kernel untouched" promise holds
     for every behavior, only this constructor is new.
   - Kernel-free fallback (if reviewers insist on zero kernel diff): write the
     `SessionEntry::Header` line by hand with `serde_json` then `Session::open`
     the file. Both options are stubbed out in the module doc.

2. **`crates/wcode-cli/src/agents.rs`**:
   - `WorkerTemplate` (`BHn04`) gains `pub members_dir: Option<PathBuf>`
     (default `None` = today's in-memory workers, preserving the `yUykl`
     contract; set by `main` when a group exists).
   - `SessionFactory::spawn` (`jFyJ5`): when `template.members_dir` is `Some`,
     build the worker with `session: Some(create_member_session(dir, &name))`
     (name already computed at `4R9DN`) and `record_member` the spec
     (manifest fast path).
   - New `pub fn resume_worker(&self, owner: &SessionId, spec: WorkerSpec,
     session: Option<Session>, context: Vec<AgentMessage>) ->
     Result<SpawnedWorker, String>` — mirrors `spawn`'s name allocation +
     `register`/`set_owner`/`set_model`/surface-sink steps (`GneBa`…`nEOdh`)
     but builds via a new private `worker_config_with(id, owner, spec, session,
     context)` that fills the fields `worker_config` today hardcodes to
     `session: None` / `context: Vec::new()` (`Sfc3L`, `7MXo4`).
   - Optional (review Q(A)): `#[derive(Serialize, Deserialize)]` on
     `WorkerSpec` (`ZV2bh`) so the manifest embeds the spec as-is.

3. **`crates/wcode-cli/src/repl.rs`**:
   - `list_sessions` (`xdqdj`) becomes a thin wrapper over
     `session_groups::list_groups` returning the flat `Vec<PathBuf>` of paths
     (group dirs + legacy files), so every existing caller (`main.rs:X0BJB`,
     `session_items`, the REPL `/resume` default at `cZrg8`, `/sessions`) and
     the existing test (`list_sessions_newest_first`) keep compiling and now
     see groups. The `.jsonl`-only filter moves into `list_groups`.
   - REPL `/resume` arm (`jzVeZ`) gets the same group branch (see Q(F)).

4. **`crates/wcode-cli/src/main.rs`**:
   - `mod session_groups;` alongside `bBVWC`.
   - Fresh-session arm (`9WX8b`): `Session::create(&session_dir())` →
     `let group = session_groups::create_group(&session_dir())?` +
     `session_groups::create_root_session(&group.dir, &cwd)`; thread
     `group.members_dir` into the `WorkerTemplate` at `sY76a`.
   - Resume arm (`YLlQv`/`SU1ct`): after `resolve_session_path` (`553`), branch
     on `p.is_dir()` — a group calls `open_group` + `load_root` instead of
     `Session::open`, and defers `rebuild_team` to *after* the orchestrator is
     built at `sY76a` (this ordering fix is friction #3); a flat file resumes
     exactly as today.
   - `[team]` loop (`jwG8v`): skip when resuming a group (members come from
     the files, not config — else duplicate-name spawn errors); print the
     restored team instead of `team: …`(`WAzGM`).
   - `session_items` (`LfAZa`) → `list_groups`; `session_label` (`PUpZr`) and
     `first_user_line` (`MW2yE`) resolve a group entry to its `root.jsonl`
     (friction #4); `age_millis` already works on the dir name.
   - TUI handoff (`KeoMe`) is unchanged in shape: `SessionItem.path` is now a
     group dir for groups and `reload_args(…, Some(&path), …)` +
     `resolve_session_path` (existing paths verbatim, `lUzPx`) already carry a
     dir through to the resume arm's `is_dir()` branch.

5. **Remote members (D4)** — no code: `spawn { to }` (`spawn.rs:9Wwgg`)
   defines workers on a *served peer*; their transcripts live in the peer's own
   session store, never in this process's group. `manifest.json` does not claim
   them; the module's write path only handles local `spawn`/`[team]` members.

## 3. Integration order (design §8)

**1 — Layout (write path).** `Session::create_named*` (or CLI fallback) →
`session_groups` create/list/open/scan → `agents.rs` durable spawn +
`record_member` → `main.rs` fresh-session arm + `WorkerTemplate.members_dir`.
Exit criterion: spawn a team, kill the process, and the files exist at
`<session_dir>/<millis>_<id8>/{root.jsonl, manifest.json, members/w1.jsonl,
…}`; `list_groups` shows exactly one group row.

**2 — Picker.** `repl::list_sessions` → `list_groups` wrapper →
`session_items`/`session_label`/`first_user_line` group-aware. Exit criterion:
the TUI `/resume` picker shows one row per group (and legacy files),
labeled from `root.jsonl`, and handing back a group re-execs
`--resume <dir>`.

**3 — Resume.** `main.rs` arm group branch + `rebuild_team` + `[team]`
suppression, then the REPL `/resume` arm parity. Exit criterion: the design's
§8.3 end-to-end — spawn a team, kill, `--resume <group>`, each member replies
with prior context intact.

## 4. Friction between the design doc and the real code

Ordered by how much they shape the implementation.

1. **Deterministic file names need a constructor the kernel lacks (design
   §2 vs §7).** The tree requires `root.jsonl` / `members/<name>.jsonl`, and
   D5's scan only works because the file stem is the member's address —
   but `Session::create_with_cwd` hardcodes `{millis}_{uuid8}.jsonl`
   (`session.rs:0oxzb`) and `worker_config` can only set `session: None`
   (`agents.rs:Sfc3L`). §7's "kernel untouched" and the §2 tree are mutually
   inconsistent unless we add `Session::create_named*` (recommended) or
   hand-written headers + `Session::open` in the CLI. **Open question (E).**

2. **`SessionFactory` must learn a resume-shaped build AND a members dir.**
   `worker_config` is private, hardcodes `session: None`/`context: Vec::new()`,
   and nothing in the factory knows a session dir today. The design says
   "spawn grows `session: Some(Session::create(group_members_dir))`" without
   saying how the factory sees `group_members_dir` — it must be plumbed via
   `WorkerTemplate.members_dir` (planned change) or a `spawn` parameter, and
   the resume path needs the new `resume_worker`. **Open question (H).**

3. **Order of construction in `main.rs`.** The `--resume` arm (`YLlQv`) runs
   ~80 lines before the orchestrator/factory are built (`sY76a`), but
   `rebuild_team` needs the factory, registry, phonebook, and the root's
   `agent:orchestrator` id. The arm must be split: resolve + `load_root` early
   (as today), `rebuild_team` after the orchestrator exists. Also, resuming a
   group must suppress the `[team]` startup loop (`jwG8v`) or same-named
   members trip `spawn`'s duplicate-name check (`agents.rs:SbRUR`).

4. **The picker/label path assumes a single `.jsonl` file.** `session_label`
   (`PUpZr`) strips `.jsonl` and calls `first_user_line(path)` (`MW2yE`),
   which does `Session::open(path)` — opening a *directory* fails, so a group
   row would lose its "first user line" unless the label path resolves
   `dir/root.jsonl`. `age_millis` is fine (group names keep the millis prefix).
   `SessionItem.path` (`app.rs:BLVVX`) doubles as the re-exec arg
   (`main.rs:KeoMe`) — for groups it must carry the dir; that works end-to-end
   only once the resume arm branches on `is_dir()`.

5. **`WorkerSpec` is not serializable and the transcript carries no spec.**
   The manifest claims "the spec that spawned a worker is its resume record"
   — but `WorkerSpec` derives only `Clone, Default` (`ZV2bh`), and the
   transcript (`SessionEntry`, `session.rs:YK8j1`) records Header/Message/
   ModelChange/EffortChange/Compaction — no model/role/tools/provider. A
   scan-only resume (D5) therefore cannot recover a worker's launch spec; the
   manifest (fast path) is the only source, else a default that inherits the
   *current* template. **Open questions (A), (B).**

6. **REPL `/resume` duplicates the arm (design §7 omission).** `main`'s arm is
   only half the story: the line REPL rebuilds in place
   (`repl.rs:jzVeZ` → `3qfu3`) and needs the same group branch. The design
   lists only `main.rs`; flag for the reviewer (Q(F)).

## 5. Compile status

`cargo build` currently fails (expected): `mod session_groups;` is not declared
in `main.rs`, and the module references planned symbols
(`Session::create_named`, `SessionFactory::resume_worker`,
`WorkerTemplate::members_dir`). `cargo check` may be used freely; nothing here
modifies build state. The module's type shapes are "conceptually compilable" —
a developer applying §2 in order 1→3 gets a green build without redesign.

## 6. Test plan (skeletons live in the module's `#[cfg(test)]`)

Pure/unit: `create_group_lays_out_dir_and_members`, `is_group_dir_name_…`,
`list_groups_mixes_groups_and_legacy_newest_first`,
`list_groups_survives_a_missing_session_dir`, `open_group_requires_root_jsonl_…`,
`create_root_and_member_sessions_roundtrip_through_open`,
`validate_member_name_…`, `scan_members_lists_only_jsonl_sorted`,
`member_id_derives_session_id_from_file_stem`, `manifest_roundtrips_member_specs`,
`member_spec_falls_back_to_default_when_manifest_entry_missing`,
`record_member_upserts_instead_of_duplicating`, `load_root_extracts_model_and_effort_…`.

Behavioural: `torn_tail_member_transcript_is_tolerated_on_resume`,
`corrupt_mid_file_member_is_skipped_not_fatal`,
`rebuild_team_closes_registry_owners_and_names` (registry has all members,
`permitted(member, root)` both ways, `set_model` restored, phonebook resolves),
`resume_roundtrip_members_report_to_the_root` (spawn → drop → rebuild → the
restored member reports to the root; mirrors `agents.rs:qHDUv`). Final
gate: the design §8.3 live check against a real endpoint.