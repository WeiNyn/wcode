# W033 — Team membership is declared, not discovered (`[[team]]` only)

**Status:** **shipped** — signed off (§0), sketched, implemented, **second-layer APPROVED** at `6e2d660` after one BLOCK (two prose items). Commits `25a2967` → `3031ba8` → `7d6e4d2` → `6e2d660`.
**Supersedes:** D-C2's discovery half, D-B1's "or ≥ 1 discovered agent `.md` member" clause, D005's global-scan carve-out, and D017's fold-suppression amendment (which becomes structurally moot).
**Decision record:** [`decisions/D037-toml-only-team-membership.md`](../decisions/D037-toml-only-team-membership.md).
**Read-only author note:** the brief was authored by a read-only worker — `cargo test`, `cargo clippy`, `--dump-config` and `--dump-system-prompt` could not be run. Every live check in §8/§10 is *proposed*, not run. Line numbers are from reading the tree; §12 lists what that leaves unverified.

---

## 0. Human sign-off (recorded)

Asked in §5, answered by the human:

1. **A vs B → (A), strict.** Pool == boot roster == `[[team]]`. `agent_files::discover`
   and its five companions are **deleted**, with their three module tests.
   `spawn { agent }` names declared roles only.
2. **Global scan → dies.** `~/.config/wcode/agents/**` no longer creates members;
   a global role is declared in `~/.config/wcode/config.toml` as
   `[[team]] file = "/abs/path/role.md"`.
3. **The agents-mode break → accepted.** A repo with agent `.md`s and no
   `[[team]]` no longer auto-enables agents mode; it gets its own test.

So: §5.1 = A, §5.2 = no (unchanged), §5.3 = scan dead + declared in the global
config. Step 1 of §7 is satisfied; the sketch (Step 2) targets option A only.

## 0b. First-layer verdict — APPROVE, amendments A1–A12 (binding)

Shape approved: inert (0 deletions, 0 added code lines, 18 markers paired), faithful to A
(nothing hedges toward B), both deletion boundaries complete, no `#[allow(dead_code)]`
retained, no resolver unification. Gates at review time: `cargo build` / `cargo test
--workspace` (1462 passed, 4 ignored) / `cargo clippy --workspace --all-targets` clean.

The amendments are the implementer's contract:

- **A1 (teeth, `cli_flags.rs`).** `project_with_agent_md` must also write a uniquely
  named `.wcode/agents/hidden.md` (`name: hidden`, body `hidden body`), and
  `an_unreferenced_same_named_md_is_not_a_member` + `a_declared_file_member_boots_the_role`
  must assert `config.matches("team.").count() == 1` **and** `!config.contains("team.hidden:")`.
  Without it the fold's own name-dedup makes every "not a member" assertion hold on the
  pre-change binary — unfalsifiable. Keep `a_named_team_still_resolves_a_file_member`
  verbatim (`hidden`, not `scout`, keeps its `!stdout.contains("- scout")` honest).
- **A2 (honest scope, `main.rs`).** `a_workflow_member_needs_a_declaration` must NOT claim
  to prove an `.md`-only repo fails membership; its doc says it guards the
  `resolve_team_files` → `validate_workflow_members` seam and the `config.rs:1250` text.
- **A3 (teeth, `main.rs` unit test).** `a_file_member_resolves_and_a_same_named_md_is_irrelevant`
  needs a distinctly named on-disk role (`scout.md`, `name: scout`) so a scan re-introduced
  inside `resolve_team_files` breaks `assert_eq!(team.len(), 1)`.
- **A4 (anchor corrections — the number locates, the snippet pins).** `repl.rs:79`
  (not `:84`) is `for member in team`; `write_md` has four surviving call sites
  (`:488`, `:508`, `:525`, `:539`), not five; `dump_prompt`'s signature is `cli_flags.rs:175`
  (not `:176`); the `/team` re-exec citation is `main.rs:2251-2252` (`repl::new_session_args`),
  not `push_launch_args`; `Args::no_project_config`'s doc is `main.rs:190-194`.
- **A5 (missed stale comment, Step 4).** `main.rs:639-642` still describes `--no-project-config`
  as gating "the `./.wcode/agents/` scan" and the global scan — reword: `project` gates the
  `[.wcode/config.toml, .wcode/team.toml]` overlays (`:707`); there is no scan to gate.
- **A6 (missed stale comment, Step 4).** `main.rs:1727-1729` "resolved from the folded `[team]`"
  → "the declared `[team]`". (`:786`'s "folded here" is the theme/view fold — leave it.)
- **A7 (amnesty → steps, decisive).** Step 4: `tools/spawn.rs:53-58`/`:87-89`/`:313-314`,
  `agents.rs:93-96`/`:600-602`/`:652-665` (keep `pool()`'s existing `#[allow(dead_code)]`),
  A5, A6, `config.rs:1234-1241`. Step 3: `.wcode/team.toml` header + six entries. Step 6:
  `main.rs:69-72`/`:94-96`/`:135`/`:190-194`, `.wcode/workflow.toml:55-57`, `README.md`,
  `docs/project-team-loading.md`, D003/D005/D017 amendment notes,
  `.wcode/skills/agent-roles/SKILL.md`, **plus** `docs/book/{02-install-and-configure,
  09-instructions-and-skills,11-teams-and-roles}.html` (the last carries the two-root
  precedence table at `:90-91`). Step 8: D037 status + tracker row 98. Decided-stale, not
  forgotten: `docs/sketches/agent_files.md`, `docs/work/W001|W002|W007`; if `docs/book/**`
  is generated, name the source that regenerates it.
- **A8 (Step 6 gate broader).** The narrow `grep` misses the HTML pages; triage all hits of
  `grep -rln 'agents/\*\*\|agents/\*\.md\|folded' README.md docs .wcode/skills` (17 files).
- **A9 (§8 R1 compares nothing).** `--dump-system-prompt` prints `# Your team` then a blank
  line; the roster is under `Members:`. Use `sed -n '/^Members:/,/^$/p'` or `grep '^- '`.
- **A10 (`.wcode/team.toml` header).** The new header must say the array is REPLACED wholesale
  (`config.rs:1344-1346`), and must stop claiming the file "adds the team … on top of your
  global config" — true for tables, false for `[[team]]`.
- **A11 (no retyping).** Keep `main.rs:795-801` byte-identical; the sketch's inlined copy is
  illustration only.
- **A12 (confirmed, do not "improve").** `mod tests`' `use std::path::PathBuf;`,
  `parse_agent_md`'s wording fix, and `git log` as the archive for the deleted fns.

---

## 1. The ask

Boot membership of a team becomes a **declaration**, never a side effect of the filesystem. Today `cfg.team` is the folded union of (a) the TOML `[[team]]` array and (b) every `.md` under `./.wcode/agents/` and `~/.config/wcode/agents/`. The human has LOCKED: a `.md` under `.wcode/agents/` enters only when a `[[team]]` entry names it, `file = ".wcode/agents/<role>.md"`.

One decision is still open (§5): whether `spawn { agent }`'s lookup pool stays discovery-fed (**B**) or collapses onto the declared roster (**A**).

---

## 2. Recon verification (what I re-read, and the corrections)

Confirmed as stated in the recon, now with the anchors I will cite:

**The fold** — `crates/wcode-cli/src/main.rs:857` `fn fold_discovered_members(`, body:
- `:864` `let discovered = if project && !team_overlay {`
- `:865` `agent_files::discover(cwd, home)` (project scan)
- `:867` `home.map(agent_files::discover_global).unwrap_or_default()` (the global scan — the branch `--no-project-config` *and* `--team` fall into)
- `:869-871` `for member in discovered { if !team.iter().any(|t| t.name == member.name) { team.push(member);`

Call site `:815-824` writes into `&mut cfg.team`.

**The file mechanism already exists and runs before the fold** — `:797` `if let Err(e) = resolve_team_files(&mut cfg.team, &cwd) {` inside the unconditional block `:795-801`; `:894` `fn resolve_team_files(team: &mut [TeamMember], cwd: &Path) -> Result<(), ConfigError>`. It already enforces "neither name nor file" (`:952-956`) and final-name uniqueness (`:958-967`). Blessed precedent: `.wcode/teams/full-stack.toml:10-26`, six `[[team]] file = ".wcode/agents/<role>.md"` entries.

**D-B1 auto-enable** — `:839` `args.agents |= !cfg.team.is_empty();`. It reads the *folded* vector today, so it keeps working unchanged once the fold is gone (declared-only).

**Boot / pool / roster all read `cfg.team`** (this is the two-meanings fact, corrected line numbers):
- boot spawn loop `:1677-1696`, the loop itself `:1681` `for member in &cfg.team {` → `:1692` `o.spawn_worker(spec)` → `:1694` `exit(2)`.
- the "team: names" notice `:1698-1700`.
- spawn pool `:1550` `.with_team(std::sync::Arc::new(cfg.team.clone()), !cfg.team.is_empty())`, consumed at `crates/wcode-cli/src/agents.rs:693` `erased(Spawn::new(self.factory.clone(), self.id.clone(), self.phonebook.clone(), self.pool.clone()))` → `crates/wcode-cli/src/tools/spawn.rs:115` `let member = agent_files::resolve_role(&self.pool, name)?;`.
- system-prompt roster `:984-988` `repl::system_prompt(&cfg.tools, &instructions, &skills, &cfg.team, …)` → `crates/wcode-cli/src/repl.rs:71` `if !team.is_empty() { … "# Your team"`.
- `--dump-config` roster `:593-601` `"team.{}: model {} · base_url {} · api_key {}"`.
- guards keyed on the same emptiness: `:1598` `if !cfg.team.is_empty() && orchestrator.is_none() {`.
- workers get `&[]` (`:1512`), so the roster is root-only — unchanged by W033.

**Workflow coupling (hard failure if missed)** — `:831-836` `if let Some(w) = &cfg.workflow && let Err(e) = crate::config::validate_workflow_members(w, &cfg.team)` → exit 1; the message at `crates/wcode-cli/src/config.rs:1250` `"node `{}`: member `{m}` is not a [team] member"`. `.wcode/workflow.toml` names `explorer` `:162`, `sketcher` `:168`, `reviewer` `:173`+`:186`, `developer` `:180`+`:193`+`:199`. **Today those four exist only because of the fold** — so `.wcode/team.toml` must declare them or this repo's own `--config .wcode/workflow.toml` invocation fails at boot.

**This repo declares zero members** — `.wcode/team.toml:8` `# The members are NOT declared here.` (through `:13`), while six role files exist (`.wcode/agents/{brainstormer,designer,developer,explorer,reviewer,sketcher}.md`, every frontmatter `name` matching its filename — checked). So the data change is mandatory in the same effort.

**The dead-code cascade under (A)** — `agent_files::discover` has exactly one production caller (`main.rs:865`) plus one test caller (`main.rs:4178`). `discover_project` (`agent_files.rs:221`) has one caller (`:120`); `discover_global` (`:232`) has two (`:123`, `main.rs:867`); `load_members` (`:241`) and `collect_agent_files(_at)` (`:277`, `:281`) have none outside; `MAX_SCAN_DEPTH` (`:19`) belongs to them. Surviving users: `parse_agent_md` (`:81`, via `main.rs:906` and `tools/team.rs`), `resolve_role` (`:139`, via `tools/spawn.rs:115`), `resolve_member_files` (`:156`, via `tools/team.rs:96`).

**Corrected/added vs the recon:**
1. The boot loop is at `:1681` (the recon's `1687` is inside the `WorkerSpec` literal).
2. `agent_files.rs` has a **second** file resolver, `resolve_member_files` (`:156`), used by the live `team` tool (`crates/wcode-cli/src/tools/team.rs:96`) — it already carries a FRICTION note (`:152-155`) about duplicating `main.rs::resolve_team_files`. Under (A) it becomes the *only* live resolver path besides `main.rs`, so the hoist noted there gets more attractive, not less.
3. **Two docs the recon missed, both load-bearing:** `.wcode/skills/agent-roles/SKILL.md` — `:3` (description promises "the TOML `[[team]]` shadowing trap"), `:16-26` ("§0 — Where roles live (discovery)", precedence "folded TOML `[[team]]` > project `.md` > global `.md`"), `:97-114` ("§4 — The TOML `[[team]]` shadowing trap", quoting `fold_discovered_members`' guard), `:125` (checklist: "No same-named TOML `[[team]]` block shadows the file"). This skill is *loaded by agents*, so leaving it stale teaches every future worker the wrong rule. And `docs/decisions/D024-orchestrator-team-control.md:18` describes `spawn { agent }` as "spawn from a discovered `.wcode/agents/<name>.md` role" — the ADR that (A) must amend.
4. `docs/next-steps.md` numbering is messy (rows run to `97` at `:120`, with `96` at `:122` out of order). Highest existing row = **97**; the new row is **98**.
5. Next free ADR number = **D037** (`docs/decisions/` ends at `D036-tui-settings-panel.md`).

---

## 3. Scope and non-scope

**In scope**
1. Boot membership = the `[[team]]` array only (`main.rs:857-874` removed or re-targeted per §5).
2. The global `~/.config/wcode/agents/` scan stops creating members (its fate is explicit: §5.3).
3. `.wcode/team.toml` declares its six roles via `[[team]] file =`, so this repo still boots its team and `.wcode/workflow.toml` still validates.
4. `--no-project-config` docs/semantics reworded: it no longer governs an agents *scan* (there is none); it governs the project overlays — and therefore any `file =` role they declare. The global `config.toml`'s `[[team]] file =` entries keep resolving, because `resolve_team_files` sits outside the project guard (`main.rs:795-801`) — D005's carve-out survives in a *narrower, cleaner* form.
5. The `--help` truth: `main.rs:69-72`, `:93-96` (`./.wcode/team.toml, then agent .md files under ./.wcode/agents/;`), `:135`.
6. Test rewrites that assert the NEW rule positively (pair: `.md` alone boots nothing; the same file via `file =` boots it), and deletion of tests whose subject no longer exists.
7. Docs-of-record: `README.md:325-330`, `:345-348`, `:356-358`; `docs/project-team-loading.md:57-59`, `:78-82`, `:180-182` (and the Part C header `:70-75`); `.wcode/team.toml:1-13`; `.wcode/workflow.toml:55-57`; `.wcode/skills/agent-roles/SKILL.md`; D005/D017/D024 amendment notes; a new D037; `docs/next-steps.md` row 98.
8. One user-visible behaviour change to own loudly: a repo with `.wcode/agents/*.md` and no `[[team]]` **no longer auto-enables agents mode** (since `:839` now sees an empty team). That is the direct consequence of "declared, not discovered" and needs its own test.

**Non-scope**
1. The `[[team]] file =` format, its override rules (`main.rs:925-946`), or its error types. Untouched.
2. The live `team` tool (`tools/team.rs`) and `/team`/`--team` pickers — `[[team]]` from a named team file already is declaration. Untouched except for the docs that describe them.
3. The `# Your team` block's shape, ordering, or content; `[workflow]` structure validation; the scheduler.
4. `spawn`'s other args (`role`/`model`/`tools`/`read_only`/`effort`/`to`) and the remote `Request::Define` path.
5. Unifying `main.rs::resolve_team_files` with `agent_files::resolve_member_files` (the W020 A9 hoist). Tempting because W033 touches the neighbourhood — split it out.
6. Session-group resume/rebuild (`session_groups::rebuild_team`) — it does not read `cfg.team` (it restores from member files), so it is unaffected; verifying that is a non-goal beyond not breaking its tests.
7. Skills discovery, instructions files, plan mode, the TUI/VS Code rendering of rosters.

---

## 4. Value / complexity / risk per workstream

| # | workstream | value | complexity | risk | one-line why |
|---|---|---|---|---|---|
| W1 | Remove the fold (`main.rs:815-824`, `:843-874`) and re-point the roster/pool per §5 | **high** | **low** (~40 lines net) | **medium** — this repo's own `wcode` and `--config .wcode/workflow.toml` break the instant the fold goes without W2 | one membership rule replaces two; nothing else in the kernel moves |
| W2 | Declare six `[[team]] file =` in `.wcode/team.toml:8-13` (copy `full-stack.toml:10-26`) | **high** | **trivial** | **low** — and provably a no-op *before* W1, because the fold dedups by name (`main.rs:869-871`) | without it this repo boots no team and its workflow fails validation |
| W3 | (A only) delete `discover`/`discover_project`/`discover_global`/`collect_agent_files(_at)`/`load_members`/`MAX_SCAN_DEPTH` + 3 tests | **medium** | **low** | **low** — `wcode-cli` is a bin crate, so `pub` is not an API; `git log` is the archive | minimalism is the stated point; dead code + a doc link to a deleted fn is the drift this repo fights |
| W4 | Test rewrites: 4 deleted, 1 kept verbatim, 1 rewritten into a positive/negative pair, 2 new | **high** | **low-med** | **medium** — a rewrite that only asserts absence is worse than no test | the behaviour change *is* the test suite's subject |
| W5 | Docs + ADR sweep (README, help, project-team-loading, D005/D017/D024, D037, tracker, agent-roles skill) | **high** | **low** | **low-med** — the agent-roles skill is *prompt input*, so a stale one misinstructs every worker | today's docs assert the exact rule being deleted |
| W6 | Pool/roster seam change (`AgentPool` doc `agents.rs:93-96`, `with_team` `:652-662`, `pool()` `:664-670`) | **low** (A) / **medium** (B) | **low** | **low** | comments and a doc contract, not code, under (A) |

Aggregate: **value high** (one rule, less code, a documented contradiction removed), **complexity low** (a removal plus config data), **risk medium** concentrated in one place — the repo's own boot and workflow, which is why W1+W2 are one commit's worth of intent even though W2 can land first.

---

## 5. The one open decision (plus two sub-decisions that ride with it)

### 5.1 Options A vs B

**(A) Strict — pool == boot roster == `[[team]]`.** Delete the fold; `cfg.team` stays the single vector it already is (no new invariant to maintain). `spawn { agent }` can name only declared roles. `agent_files::discover` & friends lose their last production caller (`main.rs:865`) and their three module tests (`agent_files.rs:402`, `:422`, `:435`) test nothing reachable. **Recommendation: delete, do not keep as public API** — `wcode-cli` is a binary crate, so `pub fn discover` is not a seam anyone can call; keeping it would need `#[allow(dead_code)]` + a "reserved" comment, which is exactly the rot `AGENTS.md` calls absent by design.

**(B) Split — boot = `[[team]]` (locked), pool = `[[team]]` ∪ discovered.** One-line seam at the fold: `let mut pool = cfg.team.clone();` then `fold_discovered_members(&mut pool, …)`; `with_team(Arc::new(pool), !cfg.team.is_empty())` at `:1550`. Keeps the documented capability: `crates/wcode-cli/src/tools/spawn.rs:53-58` (`agent` arg: "Spawn from a discovered agent `.md` … BY NAME") and `:313-314` (the tool description), plus W020 op 5 / `docs/next-steps.md:103`.

**The tradeoff, crisply.** (A) is one vector, ~90 fewer lines, no second source of truth, and it kills the D017 asymmetry for free. (B) keeps a shipped capability but institutionalises two membership concepts — the roster (prompt + `--dump-config` + boot) and the pool (`spawn`) — that can disagree with no test able to see both, and it keeps `discover`'s precedence rules (project `.md` > global `.md`) alive in a doc for a set nobody boots.

**Recommendation: (A).** Two reasons beyond minimalism. First, the capability is *documented but not used*: the shipped orchestrator guidelines say "prefer `message`ing an existing member over spawning a duplicate" and never mention `spawn { agent }` (`.wcode/team.toml:68-70`); its consumers are two doc-comments and a tracker row. Second, `spawn` already carries `role`/`model`/`tools` args (`tools/spawn.rs:26-51`), so an ad-hoc explorer remains expressible (`spawn { name: "x", role: "…", tools: ["read","grep"] }`) — the *discovery* is what (A) removes, not the *possibility*.

**If (A) is chosen, `spawn.rs:53-58` and `:313-314` are rewritten** to "spawns a **declared** team role by name (`[[team]]` entry, including a `file =` one); an unknown name errors listing the declared names" — which is accurate, since `resolve_role` (`agent_files.rs:139`) is unchanged and the pool is the roster.

### 5.2 Sub-decision: does the *pool* auto-enable agents mode?

**No, under either option.** `:839` stays byte-identical (`args.agents |= !cfg.team.is_empty()`). A pool that turns agents on but boots no members would give a session a `spawn` tool and an empty roster from a directory listing — the exact surprise W033 exists to remove.

### 5.3 Sub-decision: the global scan's fate (must be explicit)

Today `main.rs:864-868` folds `discover_global` (`agent_files.rs:232-236`) even under `--no-project-config` *and* under `--team` (D005's carve-out).

- **Recommended (works with A or B):** the global *scan* dies; global roles are **declared in the global `~/.config/wcode/config.toml`** — `[[team]] file = "/abs/path/role.md"` (absolute paths are already supported, `main.rs:899-903`). Nothing becomes unreachable, and D005 is superseded with a narrower true statement: "`--no-project-config` suppresses the project overlays; the global `config.toml` still loads, including any `[[team]]` it declares."
- **Rejected:** keeping the global scan for the pool only (it is (B) with a worse asymmetry: a role spawnable but never bootable), or keeping it for boot (violates the lock).

---

## 6. Interface and integration points

This is a **removal plus config data**, not a new module. The only "interface" is the meaning of one field and one builder argument.

**`AgentPool` / `with_team` — `crates/wcode-cli/src/agents.rs:93-96`**
```rust
/// The resolved agent pool — the FOLDED `[team]` + `.wcode/agents/*.md` set
/// (`cfg.team`), captured at startup. `spawn { agent }` (op 5) resolves against
/// it. `Arc`, so each tool's clone is cheap.
pub type AgentPool = std::sync::Arc<Vec<crate::config::TeamMember>>;
```
`:652-662` `pub fn with_team(mut self, pool: AgentPool, started_with_team: bool) -> Self` — under (A) the first argument becomes *the same* vector as the boot roster; the doc at `:600-602`, `:652-665` and the type doc change wording, not shape. `pool()` (`:664-670`) keeps its `#[allow(dead_code)]`. The test `with_team_records_the_pool_and_the_startup_marker` (`agents.rs:1937-1966`) is unaffected — it constructs a pool by hand.

**The removal site — `crates/wcode-cli/src/main.rs`**
```rust
// :817-823  (call site, inside the unconditional block :815-824)
        fold_discovered_members(
            &mut cfg.team,
            &cwd,
            config_dir().as_deref(),
            project,
            team_overlay,
        );
// :857-874  (the function itself)
fn fold_discovered_members(team: &mut Vec<TeamMember>, …) { … }
```
Under (A) both go, and `team_overlay` (`:661`) loses its only consumer → delete it and the comment at `:659-661` (it also holds the D017 explanation, which moves to the ADR). `config_dir()` may lose this call site — it is used elsewhere (`:820` here, `main.rs:980`, `repl::session_dir`), so no import churn beyond the immediate.

**Roster/pool/validation sites that must keep reading the SAME vector — `main.rs`**
- `:831-836` `validate_workflow_members(w, &cfg.team)` — unchanged, now sees declared-only. In-repo consequence: `.wcode/team.toml` must declare `explorer`, `sketcher`, `reviewer`, `developer`.
- `:839` `args.agents |= !cfg.team.is_empty();` — unchanged.
- `:1550` `.with_team(std::sync::Arc::new(cfg.team.clone()), !cfg.team.is_empty())` — unchanged under (A); becomes `pool` under (B).
- `:1598`, `:1677-1696`, `:1698-1700`, `:593-601`, `:984-988` — unchanged.
- `:958-961` comment ("A collision with a DISCOVERED `.md` is handled by the fold's own dedup, which runs after this") — must be amended: after W033 the uniqueness check at `:962-967` is the *only* duplicate guard.

**`agent_files.rs`** — module doc `:1-9` (the `Precedence (high → low, D-C2/A2)` block), `discover`'s doc `:113-118`, and `resolve_role`'s doc `:135-138` ("the pool is the folded `[team]` + `.wcode/agents/*.md` set already built by [`discover`]…") all describe W033's deleted mechanic; the `[`discover`]` intra-doc link dangles under (A).

**`tools/spawn.rs`** — `:87-89` pool field doc, `:53-58` `agent` arg doc, `:112-115` `spec_for` (code unchanged; `resolve_role` already reads whatever pool it is handed).

**`tools/team.rs:96` `agent_files::resolve_member_files(&mut team, cwd)?`** — the live `team` tool, untouched; it is the reason `agent_files.rs` survives at all under (A).

**Is a sketcher pass warranted?** Yes — a *small, in-place, comment-only* one, and the argument is not "the diff is big" (it isn't; ~40 lines out, ~10 in). It is that four things must be **agreed before** anyone edits: (i) which option's seam (§5.1) the fold site becomes; (ii) that the six `.wcode/team.toml` entries are copied from `full-stack.toml` rather than re-derived; (iii) that `AgentPool`/`with_team` change *docs only* under (A) — a reviewer must be able to see that no signature moved; (iv) the exact replacement test names (so the implementer cannot quietly keep a vacuous one). Those are all reviewable as inert `// ==== SKETCH … ====` blocks in four files. **Do not** open a new module — there is nothing new.

---

## 7. Numbered plan (deliverable · gate per step)

**Step 1 — settle the decision.**
Deliverable: §5 filled in with the human's choice (A/B, pool auto-enable = no, global scan = dead + declared-in-global-config) and `docs/decisions/D037-toml-only-team-membership.md` written with **Status: accepted (human sign-off)**, explicitly superseding D-C2's discovery half, D-B1's `.md` clause and D005's scan carve-out, and noting D017 is moot.
Gate: human sign-off recorded; no code touched.

**Step 2 — sketch (sketcher).**
Deliverable: in-place SKETCH blocks only, in (1) `.wcode/team.toml` (the six `[[team]]` blocks + the new header comment), (2) `main.rs` (the fold site's replacement/deletion marker + the `:958-961` comment rewording), (3) `agent_files.rs` (module doc + the deletion boundary, naming every symbol and test that goes), (4) the replacement tests as `#[test] fn …` skeletons with `todo!()` bodies in `main.rs`'s and `cli_flags.rs`' test modules.
Gate: first-layer review APPROVE; **zero real code lines modified**; both option shapes are not left ambiguous — the sketch is of the chosen one.

**Step 3 — data first (behaviour-neutral).**
Deliverable: `.wcode/team.toml` gains six `[[team]] file = ".wcode/agents/<role>.md"` entries (mirroring `.wcode/teams/full-stack.toml:10-26`) and its `:1-13` comment block is rewritten to "members are DECLARED here, pointing at the role files".
Gate: `cargo build` clean; `./target/debug/wcode --dump-config | grep -c '^team\.'` prints **6** (not 12 — the fold's name dedup at `main.rs:869-871` keeps it at six, which is what makes this step safe to land alone); `--dump-system-prompt` roster unchanged (identical six lines, roles from the `.md` bodies); `cargo test --workspace` green **without** touching any test.

**Step 4 — the removal.**
Deliverable: per the chosen option, `main.rs:815-824` + `:857-874` deleted (A) or re-pointed at a separate `pool` (B); `team_overlay` (`:661`) and its comment removed under (A); `.with_team(...)` at `:1550` fed the roster (A) / the pool (B); the `:958-961` comment amended; `AgentPool`/`with_team`/`pool()`/`spawn` docs updated. **Do not** mix in the dead-code deletion.
Gate: `cargo test --workspace` green (W4's rewrites land here or the diff does not compile — the old tests call a function that no longer exists); `cargo clippy --workspace --all-targets` clean; live dir-pair check (§10 A1/A2) shows the new rule.

> **Amendment (second-layer, follow-up 3):** Step 4's "**Do not** mix in the
> dead-code deletion" was wrong, and the implementer correctly overrode it.
> Removing the fold leaves `agent_files::{discover, discover_project,
> discover_global, load_members, collect_agent_files(_at)}` with no caller, and
> Step 5's own gate bans a new `#[allow(dead_code)]` while the clippy gate bans
> warnings — so **no** two-commit split can be clean in between. Steps 4 and 5
> therefore share commit `3031ba8` by force, not convenience.

**Step 5 — dead code (A only).**
Deliverable: delete `discover`, `discover_project`, `discover_global`, `load_members`, `collect_agent_files(_at)`, `MAX_SCAN_DEPTH`, the `HashSet` import they use, and the three tests at `agent_files.rs:402`, `:422`, `:435`; rewrite `agent_files.rs:1-9`, `:113-118`, `:135-143` and the `[`discover`]` link.
Gate: `cargo clippy --workspace --all-targets` clean with **no new `#[allow(dead_code)]`**; `cargo test --workspace` green; the commit message states the before/after test count (I measured neither — see §12).

**Step 6 — docs.**
Deliverable: `README.md:325-339` (rewrite the two roots into "one mechanism: `[[team]]`, inline or `file =`"), `:345-348` (`--no-project-config` no longer governs a scan), `:356-358` (this repo now declares its six); `main.rs:69-72`, `:93-96`, `:135` (`--help` + the env line); `docs/project-team-loading.md:46-53`, `:57-59`, `:70-82`, `:180-182`; `.wcode/workflow.toml:55-57`; `docs/decisions/D024:18`; amendment notes in D005/D017 pointing at D037; `docs/next-steps.md` new row **98** (`◐ shipped — see work/W033…`, do **not** edit row 52 at `:63`); `.wcode/skills/agent-roles/SKILL.md:3`, `:16-26`, `:97-114`, `:125`.
Gate: `grep -rn "auto-discovered and folded\|folded \*\*under\*\*\|shadowing trap" README.md docs .wcode/skills` returns nothing; `./target/debug/wcode --dump-system-prompt` still lists the seven skills (the edited SKILL.md still parses — `name`/`description` frontmatter quoted, one line).

**Step 7 — verification.**
Deliverable: second-layer review verdict + the live evidence block appended to the work item.
Gate: reviewer re-runs both cargo gates, confirms zero `SKETCH` markers, confirms each replacement test fails if the rule is inverted (the reviewer's teeth check), and confirms the §10 human checklist items that need a terminal were run.

**Step 8 — record.**
Deliverable: D037 status/flip + the work item's outcome section + row 98 ticked with commit hashes.
Gate: `docs/next-steps.md` remains a valid table (no reflow of historical rows).

---

## 8. Quality gates (exact commands)

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
```
Both cargo gates are mandatory and must be *clean*, not merely passing (`AGENTS.md`). Then, against a real binary (`cargo build --bin wcode`; the checks below use `./target/debug/wcode`):

```
# R1 roster facts (this repo, after step 3, and again after step 4 — identical output)
./target/debug/wcode --dump-config | grep '^team\.'
# The roster IS the `--dump-config` roster. `--dump-system-prompt` prints
# `# Your team` followed by a BLANK line; the members sit under `Members:`, whose
# roles carry their own `- ` bullets and blank lines — so neither
# `sed -n '/^Members:/,/^$/p'` nor `grep '^- '` is a clean oracle (A9; second-layer
# follow-up 1). Use the `--dump-config` list, plus `grep -c '^# Your team'` for
# presence only.

# R2 the workflow still validates on declared members
./target/debug/wcode --agents --config .wcode/workflow.toml --task "x" \
  --model m --base-url http://localhost:11434/v1 --no-session

# R3 negative: the same workflow against a team that does NOT declare its members
#     → exit 1, stderr "node `explore`: member `explorer` is not a [team] member"

# R4 error/retry path (no endpoint needed)
WCODE_RETRY_MAX=2 ./target/debug/wcode --no-session -p hi --model m \
  --base-url http://127.0.0.1:9/v1

# R5 the named-team shape is untouched
./target/debug/wcode --team full-stack --dump-system-prompt | sed -n '/^# Your team/,/^$/p'
```
`--dump-config`/`--dump-system-prompt` are the roster oracle because both are model-free and session-free (`main.rs:497-500` exits 0; `:973-990` diverges). R3's message text is `config.rs:1250` verbatim.

---

## 9. Automated tests — rewrite / delete / keep, per function

| test | anchor | today | W033 |
|---|---|---|---|
| `cli_flags::an_agent_md_is_discovered_into_the_team` | `tests/cli_flags.rs:190-197` (cwd has **no** team.toml, `:160-174`) | asserts `# Your team` + `- explorer — recon body` | **hard-fails** → split into the new pair: `an_agent_md_alone_boots_no_team` (same helper, asserts `!contains("# Your team")` **and** no `team.` line from `--dump-config`) and `a_declared_file_member_boots_the_role` (same helper + `[[team]] file = ".wcode/agents/explorer.md"`, asserts `# Your team` + `- explorer — recon body`) |
| `cli_flags::a_toml_team_member_beats_a_same_named_agent_md` | `:199-205` | TOML inline wins over same-named `.md` | **goes vacuous** (nothing to beat) → rewrite as `an_unreferenced_same_named_md_is_not_a_member`: same-named `.md` sits in the dir, the inline member renders `from-toml`, and the `.md`'s body appears **nowhere** in the prompt — a positive assertion of "only the declaration counts" |
| `cli_flags::an_explicit_team_replaces_the_project_agent_pool` | `:211-231` | D017: `--team recon` must not append the pool | **subject disappears under (A)** — delete, with the D017 note moved into D037 ("no fold ⇒ no re-append ⇒ the amendment is structural"). Under (B) it stays *but becomes vacuous for the roster* (the prompt is declared-only either way) → rewrite as a `main.rs` unit test on the pool, not a process test |
| `cli_flags::no_project_config_disables_agent_md_discovery` | `:256-261` | `--no-project-config` drops the discovered member | **subject disappears** → delete under (A); under (B) rewrite to assert the *pool* is project-gated (unit level). Its remaining truth (`--no-project-config` drops a project `[[team]]`) is already covered by `no_project_config_disables_discovery` (`:72-93`) and `wcode_project_config_env_off_disables_discovery` (`:95-112`) |
| `cli_flags::a_named_team_still_resolves_a_file_member` | `:235-254` | `[[team]] file =` in a named team resolves | **keep verbatim** — this *is* the new shape, and it is the pair-partner of the negative above |
| `cli_flags::a_project_team_auto_enables_agents_without_the_flag` | `:114-155` (helper writes an inline `[[team]]`, `:42-46`) | D-B1 off a declared member | **keep** (already TOML-only) |
| `cli_flags::no_project_config_keeps_the_global_team_and_auto_enables_agents` | `:277-329` (global **config.toml** `[[team]]`) | global *declaration* survives the opt-out | **keep** — it is exactly the D005 survivor statement §5.3 preserves |
| `main::fold_adds_a_discovered_member` | `src/main.rs:3583-3609` | the fold adds an unnamed member; opt-outs skip the scan | **delete under (A)**; under (B) rewrite to take `(boot, pool)` and assert boot unchanged **and** pool grown |
| `main::the_opt_out_keeps_the_global_agent_scan` | `:3614-3648` | global `.md` survives both opt-outs | **delete under (A)**; under (B) rewrite as `the_opt_out_keeps_the_global_pool` |
| `main::a_file_member_wins_over_a_discovered_md_of_the_same_name` | `:3555-3579` | ordering: resolve-before-fold | **survives but goes vacuous** — the second half (`fold_discovered_members`) no longer exists → reduce to `a_file_member_resolves_and_a_same_named_md_is_irrelevant` (call `resolve_team_files` only, assert one member, role from the file) |
| `main::f2_workflow_naming_an_md_only_member_loads` | `:4163-4197` (`:4178` `let team = agent_files::discover(dir.path(), None);`) | an md-only member satisfies workflow membership | **rewrite into the new pair**: `a_workflow_member_needs_a_declaration` (discovered-only team → `validate_workflow_members` errors with `not a [team] member`; a hand-built declared team with the same name → ok). Keeps F2's regression (membership runs on the final team) without discovery |
| `agent_files::project_md_beats_global_md_with_the_same_name`, `::same_name_within_one_dir_keeps_the_first`, `::global_scan_is_skipped_when_home_is_none` | `agent_files.rs:402`, `:422`, `:435` | discovery precedence | **delete under (A)** with the code they cover; keep under (B) |
| `agent_files::resolve_*` (`:326`–`:399` parser, `:462`–`:563` member-file/role) | — | parser + file resolution | **keep** — this is the surviving mechanism |
| `agents::with_team_records_the_pool_and_the_startup_marker` | `agents.rs:1937-1966` | pool + marker semantics | **keep** (constructs its pool directly) |
| **new** `a_team_of_md_files_without_a_team_toml_does_not_enable_agents` | — | — | assert exit ≠ 2 **and** no `team:` line, or better: `--dump-config` shows no `team.` line in that cwd — the D-B1 consequence of §3.8 |

The two replacement process tests must assert the rule **positively in both directions** (no `# Your team` *and* no `team.` line for the undeclared case; the roster line *present* for the declared case) — absence-of-a-string alone is what makes the current `:199-205`/`:256-261` rot.

---

## 10. Human testing (terminal, after step 4)

1. `cd <temp dir>; mkdir -p .wcode/agents; write .wcode/agents/x.md (name: x)` → `wcode --dump-config` prints no `team.` line; `wcode --dump-system-prompt` has no `# Your team`. Then add `.wcode/team.toml` with `[[team]] file = ".wcode/agents/x.md"` → both appear. **This is the whole change, in one breath.**
2. In the wcode repo: `wcode` (TUI) → the sidebar shows exactly six members, same identities as before W033 (compare names with the pre-change run).
3. In the wcode repo: `wcode --team full-stack` → six members; `wcode --team recon` → only recon's declared members (the D017 symptom must be *impossible*, not merely suppressed).
4. `wcode --agents --config .wcode/workflow.toml --task "noop" --status latest` after one refused-port boot → the plan prints; then confirm `explore`'s member resolves (no `not a [team] member`).
5. (A only) an interactive `--agents` session in a repo with only `.wcode/agents/*.md`: `spawn { agent: "explorer" }` must now error "no agent `explorer` (available: )" — the intended capability loss, seen once, deliberately.
6. (B only) the same session: `spawn { agent: "explorer" }` resolves from the pool while `# Your team` stays empty — the asymmetry (B) buys, seen once.

---

## 11. Expected outcome

After W033 there is exactly one way to be a team member: be a `[[team]]` entry (inline, or `file = "<any path>"`). `cfg.team` is the roster, the spawn pool, the prompt block, the `--dump-config` roster and the workflow-membership set — the same vector, with no second source anywhere. `.wcode/agents/` becomes a **convention** (where this repo keeps role files), not a mechanism; this repo declares its six in `.wcode/team.toml`, so its team and its `[workflow]` keep loading. `.wcode/agents/` may hold a file nobody references and nothing happens — which is the point. `agent_files.rs` shrinks to a parser plus two resolvers (`parse_agent_md`, `resolve_role`, `resolve_member_files`). The D017 fold/`--team`/`/new` asymmetry is structurally gone (verify: `/new` after `--team recon` still renders recon's declared members only — worth one live check, see §12). Docs, `--help`, the tracker, D037 and the `agent-roles` skill all state the declared rule; nothing in the tree still says a `.md` is discovered.

---

## 12. Facts NOT verified (read them before citing this brief)

1. **Nothing was executed.** `cargo build/test/clippy`, `--dump-config`, `--dump-system-prompt` and every live check in §8/§10 are *proposed*. Bash refused every command that runs a binary ("read-only worker"). Consequently: the recon's "`--dump-config` prints six today" is **unconfirmed by the brief's author** (it follows mechanically from `main.rs:593-601` + the six matching frontmatter `name`s read, but the output was not seen), and the current test count / clippy state is unknown.
   *Orchestrator note: the recon worker DID run `./target/debug/wcode --dump-config | grep 'team\.'` and reported six `team.<name>` lines, plus `--dump-system-prompt` rendering `# Your team`. That is the one live measurement behind §2.*
2. **D017's `/new` asymmetry** (`args.team` → `args.config` at `:678` vs `team_overlay` at `:661`; `repl.rs:508-519` forwards `--no-project-config`/`--config` only, never `--team`) — the *code path* was verified by reading, not the *symptom*. Under (A) no fold exists so the asymmetry cannot manifest, but **if an ADR asserts the bug was killed, verify it live first** (one `--team recon` + `/new` + `--dump-system-prompt`).
3. **Whether the TUI / VS Code surfaces read agent discovery at all.** `agent_files::*` callers were traced across `crates/` (cli-only), but `crates/wcode-tui`, `crates/wcode-protocol` and `editors/vscode/` were **not** reviewed for a team-membership or roster dependency; `/team roster` and the VS Code `/team` picker were not traced. The `serve --stdio` path reuses `load_config_raw` by inspection of the call, not by reading the serve entry.
4. **`session_groups` / resume**: assumed unaffected because `rebuild_team` restores from member files, not `cfg.team`. Not traced.
5. **Doc drift elsewhere**: `README.md`, `docs/**`, `.wcode/skills/**` were grepped for the discovery wording, but `docs/team-and-tui-plan.md`, `docs/sketches/agent_files.md` (a historical sketch — leaving it stale looks right, flagged as *decided* not *checked*), `docs/book/**` (19 pages, not searched) and `editors/vscode/README.md` were not read.
6. **`--list-teams`/`--list-themes` ordering** (`docs/next-steps.md:106`, row 85) — noted as an adjacent open item, not investigated.
7. **Cost of (B)**: "the pool and the roster can disagree with no test able to see both" — the plumbing claim is verified (`with_team`'s two arguments, `:1550`); the *untestability* claim is judgement, not a checked fact.

---

*No code was written and no file was edited. The human's sign-off on §5 (A vs B, pool auto-enable = no, global scan = dead + declared in the global config) is the gate before the sketcher starts.*

---

## 13. Second-layer follow-ups (recorded, non-blocking)

All five came out of the second-layer review; four are now fixed, one is filed.

1. **§8 R1's roster command was vacuous — FIXED in §8.** `sed -n '/^# Your team/,/^$/p'`
   prints only the header (the roster is under `Members:`, whose roles contain their
   own `- ` bullets and blank lines, so both `/^Members:/,/^$/p` and `grep '^- '` are
   also insufficient). The clean oracle is `--dump-config | grep '^team\.'`, with
   `grep -c '^# Your team'` for presence only.
2. **`tests/cli_flags.rs:116` said "a non-empty folded `[team]`" — FIXED** → "declared".
   Same class as amendment A6.
3. **§7's "Do not mix in the dead-code deletion" was wrong — FIXED in §7.** The merge
   into `3031ba8` is forced; the plan text now says so.
4. **`docs/book/11-teams-and-roles.html:98` figcaption pinned `agent_files.rs:23` —
   FIXED** → `:22` (the deletions moved `AgentFrontmatter`). `check-book.py` re-run green.
5. **`docs/book/appendix-decisions.html` lists D001–D034 — OPEN, deliberately out of
   scope.** D035, D036 and D037 are all absent, so the table was already three ADRs
   stale before W033; back-filling it is its own book task, not a W033 blocker.

Also fixed while closing the two blockers: **`docs/decisions/D024-orchestrator-team-control.md`**
gained a `## Amendment — D037 (W033)` section — its Context still taught `spawn { agent }`
as "a discovered `.wcode/agents/<name>.md` role", which the plan named three times
(`:124`, `:139`, `:263`) and the implementer skipped. Pattern matches D003/D005/D017.

6. **A pre-existing flake surfaced in this gate — FILED as tracker row 99.**
   `wcode-tui`'s `markdown.rs:1523` `install_syntax_theme_swaps_the_process_theme`
   swaps a **process-global** `LazyLock<RwLock<&'static Theme>>`; a parallel
   `markdown.rs:1225` `a_rust_fence_has_multiple_distinct_styles` renders while the
   swapped theme is live and sees < 2 distinct token styles. One of three
   consecutive `cargo test --workspace` runs on `7d6e4d2` reported `525 passed;
   2 failed`; both pass in isolation and the next two full runs were green
   (1457 passed). Unrelated to W033 — it makes "the gates are clean" a
   timing-dependent statement, which is why it is filed rather than shrugged at.

---

## 14. Outcome

**Shipped.** `25a2967` → `3031ba8` → `7d6e4d2` → `6e2d660`. Second-layer review
APPROVED at `6e2d660`, after one BLOCK consisting of two prose items (A6's
`main.rs:1683` "folded", and D024's Context) — no behavioural finding.

The end state, as measured rather than asserted:

- `--dump-config | grep -c '^team\.'` → **6**; `--dump-system-prompt | grep -c '^# Your team'`
  → **1**; `--team recon --dump-config | grep -c '^team\.'` → **2** (recon's declared
  pair only — the old D017 fold-symptom is unreachable, because no fold symbol
  exists in the binary).
- `agent_files::{discover, discover_project, discover_global, load_members,
  collect_agent_files(_at)}`, `MAX_SCAN_DEPTH`, the `HashSet` import and the three
  precedence tests are gone; `parse_agent_md`, `resolve_role`, `resolve_member_files`
  survive with live callers; **no new `#[allow(dead_code)]`**; `PathBuf` moved into
  `mod tests`.
- The two non-scope bodies are byte-identical across the diff — `resolve_team_files`
  `sha1 360891d2…`, `resolve_role` `698e7d45…` — so the W020 A9 resolver
  unification did not happen here (A11).
- Tests **1462 → 1457** (7 deleted, 2 added, 4 renamed). Every replacement test
  fails if the fold is restored; the one exception is A2's disclaimed limit,
  which its doc states outright.
- Gates: `cargo build --workspace` clean · `cargo test --workspace` 1457 passed,
  4 ignored · `cargo clippy --workspace --all-targets` No issues found ·
  `check-book.py docs/book` ok (19 pages) · `grep -rn SKETCH crates/` → 0.

**Deliberate capability loss, owned:** `spawn { agent }` names declared roles only.
An ad-hoc worker is still expressible through `spawn`'s own `role`/`model`/`tools`
arguments (D037).

**Filed, not fixed:** tracker row 99 (the pre-existing `wcode-tui` theme race that
flaked one of three gate runs) and the `docs/book/appendix-decisions.html` back-fill
(D035–D037, all missing before W033).
