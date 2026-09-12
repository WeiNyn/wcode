# wcode — skills & references plan

Status: **planning**. Companion to [`next-steps.md`](next-steps.md) (item 6).
Covers **skills** (on-demand capability packages) and **references** (the
context/reference material folded into the prompt). **Agents/subagents are out
of scope** — deferred until we have an agent-to-agent communication protocol
worth designing against.

| phase | scope | status |
|-------|-------|--------|
| 1 | References as a *set*: global + ancestor context files (AGENTS.md/CLAUDE.md) | ☐ todo |
| 2 | Skills: discovery, frontmatter, prompt section, load-on-demand | ☐ todo |
| 3 | Skills polish: `/skills`, `/skill`, extra roots, docs | ☐ todo |

---

## 1. Scope

**In:**

- **References — context files.** Generalize today's *single* project-instruction
  file into a discovered **set**: an optional global file plus the ancestor
  chain from the working dir up to the repo root, with a candidate-name list
  (`AGENTS.md`, `CLAUDE.md`, …), merged into the system prompt.
- **References — skill assets.** The `references/` (plus `scripts/`, `assets/`)
  convention inside a skill — served by the existing `read`/`bash` tools, no new
  mechanism. Mostly documentation + making sure relative paths resolve.
- **Skills.** `SKILL.md` packages: discover them, put `name` + `description` in
  the system prompt, load the body on demand. Reuses the instruction-file
  machinery already shipped (`load_instructions`).

**Out (deliberately):**

- **Agents / subagents / swarm** — needs the inter-agent protocol first.
- **MCP** — not in wcode's philosophy.
- **`@`-file mentions and prompt templates** — editor features; they belong to
  [`tui-plan.md`](tui-plan.md).
- **Per-skill tool policy** (`allowed-tools`) — that is a `Hooks` variant, not a
  config surface. Parsed and ignored in v1 (documented).

## 2. Reference: how pi & jcode do it

**pi** (`../wi/pi/packages/coding-agent/docs/skills.md`, `src/core/resource-loader.ts`):

- Agent Skills spec, *lenient* (warns, still loads; deliberately does **not**
  require `name` to match the parent dir, for shared skill dirs).
- Roots: `~/.pi/agent/skills/`, `~/.agents/skills/` (global); `.pi/skills/`,
  `.agents/skills/` (project, only when trusted); packages/settings/`--skill`.
- Prompt gets an **XML block of name+description only**; the body loads via the
  plain **`read`** tool, or is forced with `/skill:name`.
- Context files: candidate order `AGENTS.override.md` → `AGENTS.md` →
  `AGENTS.MD` → `CLAUDE.md` → `CLAUDE.MD`, discovered **per directory up the
  tree**, plus a global `~/.pi/agent/AGENTS.md`; `SYSTEM.md` replaces the prompt.

**jcode** (`../jcode/crates/jcode-base/src/skill.rs`, `src/prompt.rs`):

- Same frontmatter (`name`, `description`, `allowed-tools`); a process-wide
  **global registry** (own dir + `~/.agents/skills` + Claude Code plugin
  installs, manifest-aware) plus a **per-session project overlay** (`.jcode`,
  `.agents`, `.claude/skills`) that wins by name and is read fresh from disk.
- Prompt gets a `# Available Skills` list; it lives in the **static, cacheable**
  half of a `SplitSystemPrompt` so it doesn't bust the prompt cache. Descriptions
  clipped to 120 chars.
- Tool `skill_manage` (`load|list|reload|read`); slash `/skillname`.

**Takeaways for us:** both lean on the same Agent Skills frontmatter and the
same `.agents/skills` shared convention; pi proves **no new tool is needed**
(`read` loads the body); jcode's static/dynamic prompt split is a good habit to
keep in mind even without an explicit cache mechanism.

## 3. Design for wcode

### 3.1 References — context files as a set

Today `repl.rs::load_instructions` returns **one** file (first match walking up,
stop at the first dir containing `.git`), capped at 32 KiB. Target shape:

- **Global file** (optional): `~/.config/wcode/AGENTS.md` (config dir, beside
  `config.toml`). Loaded first.
- **Ancestor chain**: from the repo root down to the working dir, so the
  **nearest** file appears **last** (most specific wins by recency). Same
  stop-at-`.git` rule.
- **Candidate names** per directory, first hit wins:
- **Candidate names** per directory, first hit wins: `AGENTS.override.md`,
  `AGENTS.md`, `CLAUDE.md` (default list; configurable) — `CLAUDE.md` included
  for portability, as pi/jcode do.
- **Merge**: each file rendered as `# Project instructions (<path>)` + body,
  deduped by canonical path, each capped at 32 KiB, the whole block at 64 KiB.
- Controls: keep `--no-instructions`; `WCODE_INSTRUCTIONS` and
  `[instructions] file = …` keep working as an **explicit override** (a path or
  name → that file only, no discovery — today's behavior).

This is a small, low-risk generalization of code we already have and test.

### 3.2 References — skill assets

A skill directory may ship `references/*.md`, `scripts/`, `assets/`. Nothing to
build: the injected skill entry includes the skill's path, so the model can
`read <skill-dir>/references/x.md` or `bash <skill-dir>/scripts/y.sh`. The work
is (a) the prompt section tells the model this, and (b) `/skill <name>` prints
the skill's absolute dir so relative paths are unambiguous.

### 3.3 Skills

**Frontmatter** (Agent Skills standard, parsed minimally):

```
---
name: pdf-tools
description: Extract text/tables from PDFs and fill forms. Use for PDF documents.
---
```

- Required: `name` (lowercase `a-z0-9-`, ≤64), `description` (non-empty, ≤1024).
- Parsed with **`serde` + a YAML frontmatter deserializer** — not hand-rolled
  (block scalars and quoted values containing `:` otherwise bite). Crate:
  **`serde_yaml_ng`**, the maintained fork — upstream `serde_yaml` was archived
  by dtolnay. It pulls `unsafe-libyaml` (a pure-Rust libyaml port, no system C
  library). Deserialize into a struct of optional fields; unknown keys
  (`license`, `allowed-tools`, …) are ignored.
- Malformed frontmatter or a missing `name`/`description` → **warn to stderr and
  skip** (lenient, like pi); never fatal.

**Roots** (first `name` wins, so the scan runs highest-priority first):

- Project, nearest first — for each dir from the working dir up to the repo
  root: `.wcode/skills/` (always), then the shared pair — `.agents/skills/`
  preferred, `.claude/skills/` only as the fallback where `.agents/skills/` is
  absent in that directory. The working dir is the most specific, so it wins.
- Global (lowest priority) — `~/.local/share/wcode/skills/`, then the shared
  pair (`~/.agents/skills/` if it exists, otherwise `~/.claude/skills/`).
- Explicit extra roots (`[skills] dirs = [...]`, `WCODE_SKILLS`) are scanned
  first, ahead of both project and global.
- Within a root: any directory containing `SKILL.md`, recursive, bounded depth
  (4). Directory name need not match `name` (the standard's rule is bad for
  shared dirs — pi's reasoning).

**Prompt section**. Appended to the system prompt after the project
instructions:

```
# Available skills

Load a skill with `read` when a task matches; it may reference files
(scripts/, references/, assets/) relative to its own directory.

- pdf-tools — Extract text/tables from PDFs… (read .wcode/skills/pdf-tools/SKILL.md)
- code-review — …
```

Paths are **relative to the working dir** when under it (compact + stable), else
absolute. Descriptions clipped (200 chars); the whole section capped (16 KiB),
overflow noted. Kept **stable for the session** (no per-turn churn).

**Loading the body**: pi-style — **no new tool**. The model calls the existing
`read` on the path in the list. A `/skill <name>` REPL command force-loads a
skill (injects its body as a user turn) for when the model doesn't bite, and
`/skills` lists what was discovered.

**Controls**: `--no-skills`; `[skills] enabled = false`, `dirs = [...]`,
`disabled = ["name", …]`; env `WCODE_SKILLS`.

## 4. Where the code goes

- **New module `crates/wcode-cli/src/skills.rs`** — `Skill`, frontmatter parse,
  discovery, `prompt_section(&[Skill])`. Pure, unit-testable.
- **New dep (`wcode-cli`)**: the YAML frontmatter parser (`serde_yaml_ng`) — the
  only new dependency this plan introduces.
- **New module `crates/wcode-cli/src/instructions.rs`** — move the existing
  `Instructions` / `load_instructions` / `truncate_instructions` out of `repl.rs`
  and generalize to the set (a plain move + extend; keeps `repl.rs` about the
  loop). `repl.rs::system_prompt` composes instructions + skills.
- **`config.rs`** — `SkillsConfig`, extend `InstructionsConfig` (candidate
  names, global on/off), env plumbing.
- **`main.rs`** — discover once at startup, pass both into `build_agent`; add a
  **`--dump-system-prompt`** flag (print the composed prompt and exit) so the
  section is verifiable from a real binary without a model.

## 5. Phased tasks

**Phase 1 — References as a set (generalize).**
- [ ] Move instruction loading to `instructions.rs` (pure move, tests pass).
- [ ] Global file + ancestor-chain set; candidate names incl. `CLAUDE.md`.
- [ ] Merge with headers, dedup by canonical path, per-file + total caps.
- [ ] `--dump-system-prompt`; extend the `[instructions]` config + tests.

**Phase 2 — Skills core.**
- [ ] `skills.rs`: frontmatter parser + validation (warn/skip), unit tests.
- [ ] Discovery: global + project roots (`.wcode`; `.agents` preferred,
      `.claude` fallback), depth bound, collision = first wins.
- [ ] Prompt section rendering with relative paths, clipping, cap.
- [ ] `[skills]` config + `WCODE_SKILLS` + `--no-skills`; wire into startup.
- [ ] Live check: temp repo skill, `--dump-system-prompt` shows it; a real
      endpoint actually loads the body via `read`.

**Phase 3 — Polish.**
- [ ] `/skills` (list) and `/skill <name>` (force-load) REPL commands.
- [ ] Extra roots (`[skills] dirs`) and `disabled`.
- [ ] Document the skill-asset (`references/`) convention + README + AGENTS.md.
- [ ] Optional: accept `<root>/<name>.md` skills (pi's root-file rule).

## 6. Testing & verification

- **Unit**: frontmatter (valid, missing description, bad name chars, quoted,
  multi-line, unknown fields), discovery (precedence, collision, depth bound,
  project vs global), context-file set (global + ancestors, dedup, caps),
  section rendering (relative paths, clipping, overflow).
- **Live**: a temp repo with `.wcode/skills/demo/SKILL.md`; `--dump-system-prompt`
  shows the section; a real local endpoint (`--base-url http://localhost:11434/v1`)
  has the agent actually load the body when asked — the end-to-end proof that
  matters (`cargo build` proves nothing).

## 7. Decisions & open questions

**Decided.**

- **Frontmatter parsing** — a real YAML deserializer, not hand-rolled. Use
  `serde_yaml_ng` (upstream `serde_yaml` is archived; this is the maintained
  fork).
- **Skill roots** — the shared `.agents/skills` convention takes **priority**;
  `.claude/skills` is a **fallback**, read only where `.agents/skills` is absent.
- **Global context file** — `~/.config/wcode/AGENTS.md`, beside `config.toml`.
- **Context-file candidates** — `AGENTS.override.md` → `AGENTS.md` → `CLAUDE.md`
  (first hit per directory); `CLAUDE.md` stays in the default list for portability.

- **Collision order** — **project over global**; within the project chain the
  nearest directory (the working dir) wins. First `name` found wins, so the roots
  are scanned project-first (see §3.3).
- **Skills: prompt, not message** — the list lives in the system prompt (stable
  per session); the body still loads on demand via `read`.
- **`name` vs directory** — follow pi: the directory name need not match
  `name` (better for shared skill dirs).

**Deferred.**

- **Per-skill tool policy** (`allowed-tools`) — parsed and ignored in v1; a
  `Hooks` variant owns it later.

## 8. Progress

- [x] Settle parsing / roots / global file / `CLAUDE.md` / collision order /
      prompt / `name`-vs-dir (see §7).
- [ ] Phase 1 (references as a set) — code + tests + `--dump-system-prompt`.
- [ ] Phase 2 (skills core) — code + tests + live check.
- [ ] Phase 3 (polish + docs).
