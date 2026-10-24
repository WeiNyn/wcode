# D037 — Team membership is declared, not discovered

- **Status:** **implemented (W033 shipped)** — human sign-off on the W033 brief
  (answers #1 `A`, #2 yes, #3 ok); second-layer review APPROVED at `6e2d660`, after
  one BLOCK (two prose items) and a first-layer APPROVE with amendments A1–A12
- **Date:** this session
- **Relates to:** [`main.rs`](../../crates/wcode-cli/src/main.rs)
  (`load_config_raw`, `resolve_team_files`, the removed `fold_discovered_members`),
  [`agent_files.rs`](../../crates/wcode-cli/src/agent_files.rs),
  [`agents.rs`](../../crates/wcode-cli/src/agents.rs) (`AgentPool`, `with_team`)
- **Supersedes:** `project-team-loading.md` **D-C2**'s discovery half and
  **D-B1**'s "or ≥ 1 discovered agent `.md` member" clause; **D005**'s
  global-scan carve-out
- **Makes moot:** **D017**'s fold-suppression amendment
- **Work item:** [`work/W033-toml-only-team-membership.md`](../work/W033-toml-only-team-membership.md)

## Context

W033 was opened on a human observation: *"we glob all agents in `.wcode/agents`
to the team, without definition in team config."* The code agreed. `<TEAM>`
membership was the folded union of two sources — the TOML `[[team]]` array and
every `.md` under `./.wcode/agents/**` and `~/.config/wcode/agents/**`
(`main.rs:864` `let discovered = if project && !team_overlay {` →
`main.rs:871` `team.push(member);`). `.wcode/team.toml:8` said so in prose:
`# The members are NOT declared here.`

That made the filesystem the startup roster. Dropping a `.md` into the directory
silently gained a worker on every boot (`main.rs:1681` `for member in &cfg.team {`
→ `spawn_worker` → `exit(2)` on a bad allow-list), with no line of team config to
review, and it also *turned agents mode on* by itself (D-B1,
`main.rs:839` `args.agents |= !cfg.team.is_empty();`). The team config was not the
authority for the team.

The same `Vec` also served `spawn { agent }`'s lookup pool
(`main.rs:1550` `.with_team(Arc::new(cfg.team.clone()), …)` →
`tools/spawn.rs:115` `agent_files::resolve_role(&self.pool, name)?`), so one
vector carried two meanings: *who boots* and *who can be named*.

## Decision

**Boot membership is a declaration.** A team member exists only as a `[[team]]`
entry — inline, or pointing at a role file with `[[team]] file = "<path>"`. The
`[[team]]` array is the roster, the spawn pool, the `# Your team` block, the
`--dump-config` roster and the `[workflow]`-membership set: one vector, one
source, no second path into it.

Chosen shape (**A**, strict): **the pool collapses onto the roster.** The
discovery functions (`discover`, `discover_project`, `discover_global`,
`load_members`, `collect_agent_files(_at)`, `MAX_SCAN_DEPTH`) are **deleted**, not
retained: `wcode-cli` is a binary crate, so keeping a `pub` function that nothing
calls would be rot, not an API. `spawn { agent }` can name only declared roles;
an ad-hoc worker remains expressible through `spawn`'s existing `role` / `model` /
`tools` arguments (`tools/spawn.rs:26-51`).

**The global scan dies too.** `~/.config/wcode/agents/**` no longer creates
members. A global role is declared in `~/.config/wcode/config.toml` as
`[[team]] file = "/abs/path/role.md"` — absolute paths are already supported
(`main.rs:899-903`), and `resolve_team_files` runs *outside* the
`--no-project-config` guard (`main.rs:795-801`), so nothing becomes unreachable.

**The directory remains a convention.** `.wcode/agents/` is where this repo keeps
role text — the body of a `file =`-referenced `.md` still becomes the worker's
`role` (D-C3's mapping is untouched). It is no longer a mechanism: a `.md` nobody
references does nothing.

## Consequences

- One membership rule replaces two. `fold_discovered_members` is removed, along
  with `team_overlay` (`main.rs:661`), whose only consumer it was.
- **This repo must declare its own team.** `.wcode/team.toml` gains six
  `[[team]] file = ".wcode/agents/<role>.md"` entries — the shape
  `.wcode/teams/full-stack.toml:10-26` already used. Without it this repo boots
  no team *and* `.wcode/workflow.toml`'s `member = "explorer"` nodes fail at
  `config.rs:1250` (`"node `{}`: member `{m}` is not a [team] member"`).
- **A visible break, owned deliberately:** a repo with agent `.md`s and no
  `[[team]]` no longer auto-enables agents mode — D-B1 now keys off declarations
  only (`main.rs:839` unchanged, its input narrowed). It gets its own test.
- **D017 becomes structurally moot.** `--team <name>` used to suppress the
  project scan (`team_overlay`); with no scan there is nothing to suppress. The
  `/new`-after-`--team` asymmetry (the resolved path is stashed into `args.config`
  at `main.rs:678` while `team_overlay` keys off `args.team` at `:661`, and
  `repl::push_launch_args` forwards only `--config`) can no longer manifest —
  because no fold exists to re-run. *(Claim to confirm live before any ADR cites
  it as a fix; the code path is read, not observed.)*
- **D-C2's precedence ladder collapses** (`folded TOML > project .md > global
  .md`). There is one tier. Its collision rules — project `.md` beats global,
  first-wins within a directory, the `warning:` on a same-name duplicate — go with
  the code. The `[[team]]`-name uniqueness check (`main.rs:962-967`) becomes the
  only duplicate guard, and its comment at `:958-961` ("handled by the fold's own
  dedup") must be reworded.
- **Docs-of-record move with it:** `README.md` (the "two roots" section), the
  `--help` USAGE lines, `docs/project-team-loading.md` (D-B1/D-C2/the locked
  list), `.wcode/team.toml`'s header, `.wcode/workflow.toml`'s prose, and
  `.wcode/skills/agent-roles/SKILL.md` — the last is *prompt input*, so a stale
  one misinstructs every future worker about a trap that no longer exists.
- **Non-scope:** the `file =` format and its override/error rules; the live
  `team` tool and the `/team` pickers (a named team file already *is*
  declaration); session-group resume; and unifying `resolve_team_files` with
  `agent_files::resolve_member_files` (the W020 A9 friction) — adjacent, not
  included.
