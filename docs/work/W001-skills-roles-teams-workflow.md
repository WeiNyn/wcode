# W001 — Skills, roles, teams & the brainstorm-first workflow

- **Status:** shipped — P0–P6 + P9 landed; P8 (node artefacts as files) deferred; P7 Phase 2 (the docs link sweep) deferred by decision
- **Decisions:** [D001](../decisions/D001-workflow-task-fenced-instantiation.md),
  [D002](../decisions/D002-gates-are-structural-not-verification.md),
  [D003](../decisions/D003-team-file-reference.md),
  [D004](../decisions/D004-docs-taxonomy.md),
  [D005](../decisions/D005-global-agent-definitions.md)
- **Tracker:** [next-steps.md](../next-steps.md) item 57

## 1. The ask

Deliver, for the wcode repository:

1. **Pre-defined agent roles** as `.wcode/agents/*.md` files, so the dev team is
   authored as markdown (frontmatter + body) instead of long TOML strings.
2. A **suite of agent skills** written to the repo's skill standard
   (`.wcode/skills/<name>/SKILL.md`), so an agent can load a capability on
   demand.
3. **Team presets** — a standard team and an alternative — declarable without
   re-typing roles each session.
4. A **brainstorm-first, team-based workflow**: recon → brief → sketch → review →
   implement → verify, with a **decision log** (ADRs) and **work items** for
   traceability.

## 2. Scope / Non-scope

**In scope.**

- The categorised docs skeleton (`docs/{decisions,work,plans,designs,analysis}/`)
  and the five ADRs + this work item.
- The eight-skill suite and the `skill-authoring` validator script.
- Fencing `[workflow]` to `--task` (D001) and workflow validation after the
  `.md` fold.
- Roles to `.wcode/agents/*.md`; delete the duplicated `[[team]]` blocks.
- A prompt-layer brainstorm → brief → verify guide + a brief template.
- A `[[team]] file = "<path>"` resolver (D003).
- README/`--help` corrections for the global agent path and opt-out semantics
  (D005).
- Automated checks over the skills and roles.

**Non-scope.**

- Moving or renaming any **existing** `docs/*.md` (Phase 2, D004); this change
  only adds buckets and new documents.
- Any `crates/` logic change beyond what a workstream above names — a code change
  discovered mid-flight is reported, not silently folded in.
- Node artefacts as files (P8) and the automated role/skill checks (P9) are
  planned but optional.
- New gate *types*, permission prompts, MCP, or behaviour config (the standing
  `AGENTS.md` stance).

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | docs skeleton + ADRs + W001 | high | low | low | prose only; unlocks all traceability |
| P1 | `--task`-fenced instantiation + post-fold workflow validation | high | medium | medium | changes boot behaviour (D001) |
| P2 | roles → `.wcode/agents/*.md`; drop duplicated `[[team]]` | high | low | low | file moves; no logic |
| P3 | skills suite + `skill-authoring` validator | high | medium | low | many small files; one shell script |
| P4 | brainstorm→brief→verify prompt guide + brief template | high | low | low | prompt text folded at startup |
| P5 | `[[team]] file = "<path>"` resolver (D003) | medium | medium | medium | new fatal error paths |
| P6 | README/`--help` corrections (D005) | medium | low | low | one behaviour changes; docs catch up |
| P7 | docs Phase 2 link sweep | low | medium | medium | 112 refs across 45 files (D004) |
| P8 | optional: node artefacts as files | low | medium | medium | speculative; not required |
| P9 | automated checks over skills and roles | medium | low | low | rides the validator |

## 4. Interface & structure

**Added / changed.**

- `docs/decisions/D001–D005*.md`, `docs/work/W001-*.md`; empty
  `docs/{plans,designs,analysis}/` with `.gitkeep` (D004).
- `.wcode/skills/<name>/{SKILL.md,references/…,scripts/…,assets/…}` — eight
  skills (P3).
- `.wcode/agents/{explorer,sketcher,developer,reviewer}.md` (P2) — the members
  today duplicated across `.wcode/team.toml` and `.wcode/workflow.toml`.
- `[[team]] file = "<path>"` (D003) — parsed in `crates/wcode-cli/src/agent_files.rs`
  and resolved where the fold happens.

**Integration points (verify before coding; lines drift).**

- Workflow boot: `crates/wcode-cli/src/main.rs:3Tsl0` (predicate), call site
  `:GbCBX`, boot guards `crates/wcode-cli/src/main.rs:IAMr0`.
- The `.md` fold that must run **before** workflow validation currently sits at
  `crates/wcode-cli/src/main.rs:DVO1u` (comment), discover call `:YGsc5`, keep
  guard `:MC499` — inside `load_config_raw` (`crates/wcode-cli/src/main.rs:vHrWt`).
- Agent-file discovery: `crates/wcode-cli/src/agent_files.rs:9XTe5`
  (`discover`), project root `:CKCbg`, global root `:jmdEk`,
  `parse_agent_md` `:XMcjX`, lenient skip `:tyJaU`.
- Config fold: `crates/wcode-cli/src/config.rs:HjeeZ` (`merge_values`).
- Skills loader + validation: `crates/wcode-cli/src/skills.rs:efuUM`
  (name rule), `:Vlnli` (description cap), `:ftQCJ` (prompt clip 200),
  `:7ib2K` (only `name`+`description` folded).

## 5. Plan

Each step states its **deliverable** and its **gate**.

1. **P0 — skeleton.** Deliverable: `docs/decisions/D001–D005*.md`,
   `docs/work/W001-*.md`, the three `.gitkeep` buckets. Gate: the files exist
   under `docs/decisions/` and `docs/work/`; `docs/plans|designs|analysis/`
   are present.
2. **P1 — fence `[workflow]`.** Deliverable: `should_instantiate_workflow`
   consults the task (D001); a `[workflow]` without a task prints a one-line
   notice and starts the interactive team; workflow validation runs **after**
   the `.md` fold. Gate: `cargo test --workspace` + `clippy --all-targets`
   clean, plus the two named regression tests below.
3. **P2 — roles to markdown.** Deliverable: four `.wcode/agents/*.md` files;
   the duplicated `[[team]]` blocks removed from both TOMLs. Gate: `wcode`
   boots the standard team from the md files; no `[[team]]` left in either TOML.
4. **P3 — skills.** Deliverable: eight skills + `scripts/validate_skill.sh`.
   Gate: the validator exits 0 over `.wcode/skills/**`.
5. **P4 — prompt guide.** Deliverable: the brainstorm → brief → verify guide +
   brief template folded into the system prompt. Gate: `--dump-system-prompt`
   shows it.
6. **P5 — `file =` resolver.** Deliverable: the D003 resolver. Gate: unit tests
   for resolve / override / name-mismatch / missing-file.
7. **P6 — README/help.** Deliverable: the global agent path + opt-out
   corrections (D005). Gate: `--help` and `README.md` agree with the code.
8. **P7 — docs Phase 2.** Deliverable: the flat → bucketed link sweep. Gate:
   link check clean (deferred).
9. **P8 — node artefacts as files.** Deliverable: optional. Gate: TBD.
10. **P9 — automated skill/role checks.** Deliverable: checks over the skills
    and roles. Gate: the check exits 0.

## 6. Quality gates

Exact commands and expected output:

- `cargo test --workspace` — all tests pass (exit 0, no failures).
- `cargo clippy --workspace --all-targets` — clean (exit 0, no warnings).
- `sh .wcode/skills/skill-authoring/scripts/validate_skill.sh` — exit 0, no
  `file: reason` lines.
- `cargo run -p wcode-cli -- --dump-system-prompt | sed -n '/Available skills/,/^$/p'`
  — lists every discovered skill.
- P1 regression tests (named): `should_instantiate_workflow` gated on the task,
  and workflow validation running after the `.md` fold.

## 7. Testing

### 7.1 Automated

- `cargo test --workspace` — unit + seam tests, including the P1 regression
  pair and the P5 resolver tests.
- `cargo clippy --workspace --all-targets` — lint.
- `sh .wcode/skills/skill-authoring/scripts/validate_skill.sh` — the skill
  standard, over `.wcode/skills/**`.

### 7.2 How a human verifies it

```sh
# 1. The skills fold into the prompt (no network).
cargo run -p wcode-cli -- --dump-system-prompt | sed -n '/Available skills/,/^$/p'

# 2. The validator passes over the suite.
sh .wcode/skills/skill-authoring/scripts/validate_skill.sh ; echo "exit=$?"

# 3. The standard team starts from the markdown roles (no [[team]] in either TOML).
grep -c '\[\[team\]\]' .wcode/team.toml .wcode/workflow.toml   # both 0
cargo run -p wcode-cli -- --agents            # prints the team roster

# 4. A [workflow] without --task is inert (D001) and prints a notice.
cargo run -p wcode-cli -- --config .wcode/workflow.toml --agents   # notice, no DAG
```

Look for: every skill name in the `# Available skills` section; `exit=0` from
the validator; a roster naming the four members; the one-line notice instead of
an unseeded DAG run.

## 8. Expected outcome

- The team is authored as `.wcode/agents/*.md`; `.wcode/team.toml` and
  `.wcode/workflow.toml` each keep a single, non-duplicated team.
- `[workflow]` only drives the scheduler when a task is present (D001); gates
  are described as shape, never verification (D002).
- Eight skills load on demand; one validator enforces the skill standard.
- New docs are categorised; the decision log and work items make the effort
  traceable.

## 9. Log

- **2026-10-05** — Effort opened. Five decisions locked: **D001** (`[workflow]`
  fenced to `--task`), **D002** (a gate is structural, not verification),
  **D003** (`[[team]] file = "<path>"`), **D004** (the docs taxonomy),
  **D005** (global agent definitions at `~/.config/wcode/agents/`). Evidence
  base: a **static read of the code** — `main.rs` (workflow predicate + boot
  guards + the `.md` fold), `verify_gate.rs`, `tasks.rs`, `agent_files.rs`,
  `skills.rs`, `config.rs` — and the docs (`next-steps.md`,
  `project-team-loading.md`, `skills-references-plan.md`,
  `team-and-tui-plan.md`). **No build or test had been run at that point.**
  Known defects recorded for P2: the explorer role text promises "non-mutating
  bash" while its `tools` list omits `bash` (a worker told it has a tool it
  cannot use); and the four `[[team]]` role blocks are duplicated verbatim
  between `.wcode/team.toml` and `.wcode/workflow.toml`.
- **2026-10-05** — **Outcome.** Seven commits, each gated on
  `cargo test --workspace` + `cargo clippy --workspace --all-targets` clean:
  `a399d94` (F1: the `[workflow]` fence + the doc edits), `2d273d8` (F2: the
  validation split), `2739b94` (`[[team]] file =` + the fold extraction),
  `ee71bb5` (the roles moved into `.wcode/agents/*.md`, the duplicated `[[team]]`
  blocks deleted, the loop encoded in the guidelines), `9d10eb0` (the skills
  suite + the validator), `09457d0` (the decisions, this work item, the docs
  buckets), `967857b` (D005: the opt-out governs project discovery only).
  Final gates: **1113 passed, 4 ignored; clippy clean.**
- **2026-10-05** — **Live verification** (a real binary, not `cargo build`):
  a workflow naming members supplied only by `.wcode/agents/*.md` **loads** (the
  F2 regression); a loaded `[workflow]` with no `--task` prints
  `workflow: present but inert (no --task)` and boots, with the old
  `uses {{task}} but no --task` exit-2 gone; the same with `--task "hi"`
  reports `workflow: 2 nodes` and dispatches `#1 [doing] recon`; the repo team
  forms as six members from the role files with no `[[team]]` anywhere; and a
  probe in `~/.config/wcode/agents/` survives `--no-project-config` while the
  project members do not.
- **2026-10-05** — **Two defects found on the way, both fixed:** the explorer's
  role text promised "non-mutating bash" while its `tools` list refused `bash`
  (the role files now list `bash` + `read_only: true`, which `ReadOnlyHooks`
  makes true — it blocks mutating `bash` and `bg` but permits `cargo test` and
  `cargo clippy`); and the four `[[team]]` role blocks were duplicated verbatim
  across the two TOMLs. The duplication turned out to be a **workaround** for the
  validation-order bug that F2 fixed, which is why the role move had to wait for
  it.
- **2026-10-05** — **Deferred, with reasons.** *P8 (node artefacts as files)*:
  it changes `tasks.rs::complete`'s contract and the DAG's invariants, and the
  plan required a sketch + first-layer review before such a change — no reviewer
  was available at that point, so it is tracked here rather than landed
  unreviewed. *P7 Phase 2 (the docs link sweep)*: deferred by lock L3 — moving
  the ~36 top-level files touches 112 `docs/*.md` references across 45 files
  (including `README.md`, `AGENTS.md` and Rust doc comments) and deserves its own
  change with a link check.
