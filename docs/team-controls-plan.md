# Team controls: enforced read-only + per-worker effort

**Status:** design locked; sketch → first-layer review → build → second-layer review.

## Why

A worker is customizable only partially today. `WorkerSpec`
(`crates/wcode-cli/src/agents.rs`) carries `name`, `model`, `system` (role),
`tools`, `base_url`, `api_key` — and the same six fields ride `TeamMember`
(`config.rs`), `SpawnArgs` (`tools/spawn.rs`), `Request::Define`
(`wcode-harness/src/protocol.rs`), `DefineArgs` (`wcode-protocol/src/server.rs`),
and `MemberRecord` (`session_groups.rs`). Three gaps:

1. **Read-only is soft.** There is no enforcement. A "read-only" worker is either
   a `tools` allow-list that omits the mutating tools — but `bash` is itself a
   full escape hatch (`rm`, a redirect, `python -c open(...,'w')`), so the
   allow-list only helps if it also omits `bash` — or a `role` string that merely
   *asks* the model not to write. The repo's own `explorer` member sets **no**
   `tools` list at all, so its read-only-ness rests entirely on prompt text.
2. **No per-worker effort.** `worker_config_with` clones the orchestrator's
   `LlmOpts` and overrides only `model`/`base_url`/`api_key`/`session_id`; the
   `effort` field is inherited wholesale and cannot be changed per worker.
3. **The shipped explorer is unconstrained** (same as gap 1, concretely).

## Ground truth (file:anchor)

- Worker build: `agents.rs` `worker_config_with` — system blurb + `# Role`
  (`R7hPb`), tool retain (`ot6H2`–`RPsbi`), `message` always pushed (`HdV3M`),
  `llm` clone + model/provider overrides (`shZkc`–`T1XyU`), hooks push
  `ReportBack` + `WorkspaceHooks` (`J30Et` region).
- Spec: `WorkerSpec` (`agents.rs` `ZV2bh`); `TeamMember` (`config.rs` `DDmnB`);
  `SpawnArgs` (`spawn.rs` `yMtEC`) + `to_worker_spec` (`clNZ` region).
- `[team]` → spec: `main.rs` `build_runtime` (`KQsCE` region, `905`).
- Protocol: `Request::Define` (`protocol.rs` `j5OKE`); `DefineArgs`
  (`server.rs` `UGBPY`); the destructure + handler call (`server.rs` `aHxX3`
  region); the CLI handler (`main.rs` `deyPq` region); the remote path
  (`spawn.rs` `spawn_remote`, `6Aggq` region).
- Persistence: `MemberRecord` (`session_groups.rs` `ABkh7`) +
  `from_worker_spec`/`to_worker_spec` (`z2Zi2`/`ULZdV`); manifest JSON doc
  (`session_groups.rs` `JYEDu` region).
- Existing policy hooks to mirror: `MUTATING_TOOLS` (`hooks.rs` `osi4C`),
  `PlanModeHooks::before_tool_call` (`hooks.rs` `ufmCT`), the private
  `looks_mutating` (`hooks.rs` `SsKd8`).
- The `explorer` member: `.wcode/team.toml` (`[[team]] name = "explorer"`).

## Locked decisions

**D1 — `ReadOnlyHooks`, a kernel policy.** New unit struct in
`crates/wcode-harness/src/hooks.rs`, beside `PlanModeHooks`/`BashRiskHooks`, with
`new()` + `Default` (matching `BashRiskHooks`). `before_tool_call` returns
`Some(reason)` — the loop prefixes `blocked: ` — for:
- any name in `MUTATING_TOOLS` (`edit`, `edits`, `write`, `replace`, `ast_edit`);
- `bg` (background `bash` can mutate);
- `bash` whose `command` satisfies the existing private `looks_mutating`
  (reused, not duplicated).

Everything else passes — `read`, `grep`, `find`, `ast_search`, `session_search`,
`webfetch`, `todo`, and the team tools `message`/`peers`/`task`. Reason text
starts with `read-only worker: `. Documented as a **guardrail, not a sandbox**
(the same FP/FN budget as plan mode's bash gate, `docs/plan-mode.md` §2.4): it
makes the file-mutating tools impossible and refuses the obvious `bash`
mutations, but does not stop an arbitrary interpreter.

**D2 — `read_only: bool`, default false.** Added to `WorkerSpec`, `TeamMember`
(`#[serde(default)]`), `SpawnArgs` (`#[serde(default)]`), `Request::Define`
(`#[serde(default)]`), `DefineArgs`, `MemberRecord`. `worker_config_with` pushes
`Arc::new(ReadOnlyHooks::new())` when true. **Orthogonal to `tools`**: `tools`
governs what is *offered*, `read_only` what is *permitted* — so a `read_only`
worker whose allow-list still names `edit` is offered it and then refused.

**D3 — `effort: Option<String>`, default None (inherit).** Added to the same
six. In `worker_config_with`, after the provider overrides:
`None` → inherit; `"-" | "none" | "off"` → clear (`llm.effort = None`);
otherwise `Some(level)` → `llm.effort = Some(level)`. Mirrors the CLI's
`--effort` clear synonyms (`main.rs` `JiYTF`).

**D4 — The shipped `explorer` becomes really read-only.** `.wcode/team.toml`'s
explorer entry gains `tools = ["read", "grep", "find"]` **and** `read_only =
true` (defense in depth; the allow-list already omits `bash`). `[tools] grep/find
= true` already makes those names valid allow-list entries.

**D5 — Protocol parity.** `Request::Define` and `DefineArgs` gain `effort` +
`read_only`; the `server.rs` destructure/`DefineArgs` construction, the CLI
`Define` handler (`main.rs`), and the remote path (`spawn.rs::spawn_remote`) all
thread them, so a worker defined on a served peer is as controllable as a local
one. Update the `protocol.rs` roundtrip test.

**D6 — Persistence.** `MemberRecord` gains `effort` + `read_only`;
`from_worker_spec`/`to_worker_spec` map them; the manifest JSON doc comment and
its roundtrip test are updated, so a resumed group restores both.

**D7 — Docs.** `README.md`'s `[team]` section documents `model`, `role`,
`tools`, `read_only`, `effort`, `base_url`, `api_key`.

## Non-goals (stated, not papered over)

- No OS sandbox, no permission prompts, no filesystem restrictions — D1 is a
  `Hooks` policy, exactly as doctrine requires.
- No per-worker `max_turns` / `parallel_tools` (stay fixed constants).
- `read_only` does not rewrite the tool set; combine it with `tools` for that.
- Effort clearing is only via the three synonyms; `None` always means *inherit*.

## Open questions (for the first-layer review)

- **Q1.** `read_only`'s bash rule: the `looks_mutating` heuristic (locked, mirrors
  plan mode) vs. a blanket `bash` block (harder, but kills `git status`/`ls` and
  contradicts the explorer's stated "non-mutating bash"). Confirm the heuristic.
- **Q2.** `MemberRecord.read_only` serialization: `#[serde(default,
  skip_serializing_if = "is_false")]` (tidy manifest) vs. always emit. Pick one.
- **Q3.** Should a `read_only` worker *also* have the mutating tools stripped from
  the offered set, or stay orthogonal (locked: orthogonal)?

## Integration points to touch (checklist)

- [ ] `crates/wcode-harness/src/hooks.rs` — `ReadOnlyHooks` (+ unit tests).
- [ ] `crates/wcode-cli/src/agents.rs` — `WorkerSpec` fields; `worker_config_with`
      applies effort + pushes the hook; fix literal `WorkerSpec { … }` test
      fixtures.
- [ ] `crates/wcode-cli/src/config.rs` — `TeamMember` fields; parse/merge tests.
- [ ] `crates/wcode-cli/src/tools/spawn.rs` — `SpawnArgs` fields, `to_worker_spec`,
      description, remote `Request::Define`, tests.
- [ ] `crates/wcode-cli/src/main.rs` — `[team]` → spec mapping; `Define` handler.
- [ ] `crates/wcode-harness/src/protocol.rs` — `Request::Define` fields + roundtrip.
- [ ] `crates/wcode-protocol/src/server.rs` — `DefineArgs` + destructure/construct.
- [ ] `crates/wcode-cli/src/session_groups.rs` — `MemberRecord` + JSON doc + tests.
- [ ] `.wcode/team.toml` — explorer `tools` + `read_only`.
- [ ] `README.md` — `[team]` fields.

## Acceptance criteria

1. `read_only = true` (spawn, `[team]`, or `Define`) makes `edit`/`write`/`replace`/
   `edits`/`ast_edit` and mutating `bash`/`bg` return `blocked: read-only worker: …`;
   read tools, `todo`, and `message` still work.
2. `effort = "high"` on a worker sets its `LlmOpts.effort`; absent inherits;
   `"-"`/`"none"`/`"off"` clears.
3. A worker defined on a served peer honors both (protocol parity).
4. A resumed group member restores both.
5. `explorer` in `.wcode/team.toml` is read-only by allow-list **and** flag.
6. `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean;
   tests assert behavior (the block fires, the effort lands), not merely compile.
