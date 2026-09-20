# Session groups — first-layer review sheet (Item 16)

For the reviewer who decides the open questions **before** implementation.
Each question hangs on one or two `file:anchor` evidence points and lists
2–3 concrete options. A recommended default (the sketcher's choice, marked
**▶ default**) is given per question; the sketch was written to survive all
options, but **Q(A)/Q(B)/Q(E)** change the module's manifest types or the
kernel diff and should be settled first.

---

## Q(A) — Exact `manifest.json` schema: what is a "members" record?

Evidence: the design says `manifest.json` = `{ group, cwd, members:
[WorkerSpec …] }` — "the spec that spawned a worker *is* its resume record"
(`docs/session-groups.md` §2). But `WorkerSpec` is `#[derive(Clone, Default)]`
only — **not** `Serialize`/`Deserialize` (`crates/wcode-cli/src/agents.rs`,
`ZV2bh`) — so today it cannot be written as JSON. `WorkerSpec` fields:
`name`/`model`/`system`/`tools`/`base_url`/`api_key` (all `Option<…>`).

Options:
1. **▶ Derive `Serialize, Deserialize` on `WorkerSpec`** (one-word change to
   the derive at `ZV2bh`); the manifest embeds `WorkerSpec` literally, exactly
   as designed, and `SessionFactory::spawn` writes the same spec it used. The
   sketched `MemberRecord` collapses into an alias/no-op.
2. Keep `WorkerSpec` untouched; the module's serializable mirror
   `MemberRecord { name, model, role(=system), tools, base_url, api_key }`
   with explicit `from/to_worker_spec` conversions (what the sketch ships).
   Zero diff in `agents.rs` for serialization.
3. Manifest stores `{ name, file }` only and the spec is always inherited on
   resume (simplest; loses per-worker model/provider across resume — likely
   unacceptable given per-agent providers landed, `agents.rs:tkjnL`).

Beware: `WorkerSpec.system` is role text — naming it `role` in JSON (as the
sketch does) is friendlier but a schema choice; helper `to_worker_spec` must
map it back. Decide **before** writing `record_member`/`member_spec`.

## Q(B) — On resume, where do `model`/`role`/`tools`/provider come from?

Evidence: `SessionEntry` (`session.rs:YK8j1`) records Header(id/cwd/created),
Message, ModelChange, EffortChange, Compaction, Unknown. A worker's **launch
spec is not in its transcript** — the only per-worker metadata ever persisted
is a mid-run `ModelChange`, and `model()`/`effort()` (`session.rs:HyYmb`,
`UsWF4`) read those change entries. D5 says scan `members/` is the source of
truth, manifest a "fast path".

Options:
1. **▶ Manifest is the spec source; scan is the membership source.** Scan
   decides *which* `members/<name>.jsonl` exist; each file's spec comes from
   the manifest record (falling back to `WorkerSpec::default()` — inherit the
   *current* template — with a loud warning when a crash left a file without
   an entry). This is the only option that both honors D5 and restores
   per-worker model/provider. A default-inherited member may silently run on
   the wrong model — print `member <name>: no manifest record, inheriting
   template` so it is visible.
2. Spec must be fully scan-recoverable → encode it in `w1.jsonl` (a custom
   `{"type":"worker_spec",…}` line tolerated as `SessionEntry::Unknown`). But
   `Session::open` turns it into `Unknown` and never exposes the raw JSON —
   this fights the kernel and was explicitly not designed; reject.
3. Require the manifest on resume (a group missing it = fatal). Contradicts
   D5's "dir listing cannot be half-written" rationale.

## Q(C) — Group-dir vs flat-file name collision

Evidence: a group dir is `<millis>_<id8>` and its legacy flat form would be
`<millis>_<id8>.jsonl`. Nothing stops both from existing on disk — e.g. a
pre-group session left in place during a move, or a user copy. `list_groups`
(`session_groups.rs`) sorts lexicographically-descending, so on a tie the
flat file (longer name) lists **before** the dir, and the picker would show
two near-identical rows.

Options:
1. **▶ Prefer the group; hide the colliding legacy file.** In
   `list_groups`, drop a `Legacy{<millis>_<id8>.jsonl}` when a
   `<millis>_<id8>` dir exists (the dir is the newer, complete form; the flat
   file is the same root moved in). Deterministic, zero UI cost.
2. List both, disambiguate in `session_label` (`main.rs:PUpZr`) with a
   suffix (`" …(legacy)"`) — honest but noisy and leaves `--resume` ambiguous
   about which one a user meant.
3. List both and make `--resume` prefer the group when the bare id is given
   (accept ambiguity for explicit paths). Least deterministic.

Recommended: option 1 — the collision is definitionally the "same" session,
and two rows for one root defeats the whole point of the picker.

## Q(D) — `list_groups` ordering: name-lexicographic vs root-millis

Evidence: today `list_sessions` sorts file names lexicographically and
reverses (`repl.rs:xdqdj`), relying on the `{millis}_` name prefix making
name order == chronological order ("Names are `{millis}_{hex}.jsonl`, so
lexicographic order is chronological", `repl.rs:XqAaJ`). Group dirs keep the
same prefix (design §2: "the root file's existing name minus `.jsonl`").

Options:
1. **▶ Sort by name (lexicographic desc), exactly like today.** Zero
   behavioral surprise, groups interleave with legacy files as one sequence,
   `age_millis` (`main.rs:QEuhm`) keeps working off the name. A group created
   `now` sorts above a session created moments earlier only if its millis
   prefix is larger — same race as today, harmless.
2. Sort by `mtime` of `root.jsonl` — reflects last activity, but diverges
   from every existing listing, breaks the "paste-back" contract, and makes
   the pure function impure (I/O per entry). Reject.
3. Sort by parsed millis (numeric) — equivalent to option 1 for valid names;
   only differs for garbage names (which `is_group_dir_name` rejects anyway).

## Q(E) — Deterministic member/root file names: is a kernel touch acceptable?

Evidence: design §2 shows `root.jsonl` and `members/w1.jsonl` — names that
must be deterministic because D5's scan maps file stem → member address. But
`Session::create_with_cwd` hardcodes the file name as `{millis}_{uuid8}.jsonl`
(`session.rs:0oxzb`), and §7 says "kernel and protocol untouched". A rename
after `Session::create` breaks the `Session`'s internal `path` (`session.rs`
`uoZhf` struct: append reopens `self.path`, `EcNck`), so no CLI-only
workaround can keep a live, appending session at a renamed path.

Options:
1. **▶ Add thin, additive `Session::create_named(dir, name)` /
   `create_named_with_cwd(dir, name, cwd)`** next to `create_with_cwd`
   (`MZV2E`). Header content identical; only the file name is caller-supplied.
   No existing behavior, call site, or `SessionEntry` schema changes — the
   design's "append/torn-tail/compaction/messages untouched" promise holds
   verbatim.
2. CLI writes the `SessionEntry::Header` line with `serde_json` and
   `Session::open`s it (kernel 100% untouched). Works, but duplicates the
   header schema in `wcode-cli` and is easy to get subtly wrong (torn-tail
   logic, escaping) — and still needs `create_named`'s semantics once
   `Session` internals change.
3. Keep timestamped member files and record the name→file mapping only in the
   manifest. Violates D5 (a crash before the manifest entry loses the member
   from resume entirely). Reject.

## Q(F) — REPL `/resume` parity (design-doc gap)

Evidence: `main.rs`'s `--resume` arm (`YLlQv`) is not the only resume path —
the line REPL rebuilds **in place** at `repl.rs:jzVeZ` → `3qfu3`
(`Session::open` + `build_agent`). Design §7 lists only the `main.rs` arm;
a `/resume <group>` typed in the line REPL would hit `3qfu3` and fail on a
directory (or silently rebuild only the root if `list_sessions` starts
returning dirs via the §2 wrapper).

Options:
1. **▶ Extend the REPL arm with the same `is_dir()` branch + `rebuild_team`**
   (the REPL already holds the `orchestrator`, `hooks`, `tools`, etc., so the
   plumbing is shorter than `main`'s). Then group resume works identically in
   REPL, TUI (re-exec), and one-shot modes.
2. Make the REPL `/resume` re-exec itself (like the TUI handoff,
   `main.rs:KeoMe`) whenever the target is a group, so one implementation in
   `main` covers everything. Simpler but adds a process hop.
3. Reject groups in the line REPL (`/resume <group>` prints "use the picker /
   `--resume <group>`"). Consistent with the design's narrow scope but a
   surprising UX gap.

## Q(G) — When does a session become a group? (migration surface)

Evidence: today every root is a single flat file created at
`main.rs:9WX8b` (`Session::create(&session_dir())`); existing on-disk
sessions are all flat files `list_sessions` shows (`repl.rs:xdqdj`).

Options:
1. **▶ From now on, every root is a group** (a fresh `<millis>_<id8>/` dir
   even with zero workers — a group with an empty `members/` resumes exactly
   as a bare root, so nothing is lost). Legacy flat files stay listed and
   resumable forever; no migration step. One code path on write, one on
   resume.
2. Groups only when `--agents` (a team can exist); plain sessions stay flat.
   Saves the dir indirection for the common case, but "every root is a group"
   is what makes `list_groups` trivial and the resume arm uniform; and a
   later `spawn` mid-session would have to promote — pointless complexity.
   Reject.
3. A `--no-group` legacy escape hatch preserving today's exact layout for
   tests/scripts. Only if option 1 breaks a hard external dependency on the
   flat layout; the sketch assumes no such dependency.

## Q(H) — How does `SessionFactory::spawn` learn the members dir?

Evidence: the factory carries only `registry` + `template` + `seq` + `sink`
(`agents.rs:QJ4dm`); the design says spawn grows
`session: Some(Session::create(group_members_dir))` but nothing tells the
factory where the group is.

Options:
1. **▶ `WorkerTemplate.members_dir: Option<PathBuf>`** (None = v1 in-memory),
   set by `main` when a group exists. All spawn sites inherit it; `spawn`
   opts in with one `if let`. The `WorkerTemplate` is constructed in exactly
   two places (`main.rs:sY76a` and `agents.rs` tests) — both updated once.
2. Add `members_dir` to `SessionFactory::spawn`'s signature — every call site
   (`Orchestrator::spawn_worker`, `Spawn::spawn_local`, the `Define` handler,
   tests) changes. Noisier for zero benefit.
3. A factory-side `set_members_dir(PathBuf)` setter — mutable shared state
   alongside `sink`; a second writer already exists, but a setter opens an
   ordering trap (spawns before the setter still go in-memory). Prefer the
   immutable template field.

---

## Order of decision

Decide **E → A → H** first (they shape the kernel diff and the module's
write path), then **B** and **F** (resume semantics), then **C/G/D** (picker
polish). The sketch's `todo!()` bodies are written against the recommended
defaults; flipping an option leaves the signatures intact and only changes
bodies.

## What approving this sketch means

The reviewer signs off on: the module's types/signatures/contracts, the §2
punch list in `docs/session-groups-implementation.md` (file:anchor), the
integration order (1 layout → 2 picker → 3 resume), the six friction findings,
and the eight answers above. Blocking criteria: a signature that would fight
the real shapes cited, a decision that contradicts D1–D5, or a planned change
that touches existing behavior without a test skeleton covering it.