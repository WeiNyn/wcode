# Project team loading — auto-discovered `.wcode/` config + markdown agent files

Status: **design locked (orchestrator)** — open to first-layer review before any
code is written. Tracker: [`next-steps.md`](next-steps.md) item 52.

## Motivation

Today a team is only used when you pass it explicitly:

```
wcode --agents --config .wcode/team.toml
```

1. The project's `.wcode/team.toml` is invisible unless named on the command line.
2. A team member's `role` is a single TOML string — long prompts inside TOML are
   awkward, and every other harness (Claude Code subagents, etc.) already lets you
   author an agent as a **markdown file with YAML frontmatter**.

This lands two things: (A) auto-discovered project config, and (B) a markdown
agent-definition file that maps onto the existing `TeamMember`.

## Part A — project config auto-load

### Full precedence (low → high)

Extends today's single-overlay merge (`Config::load_with` →
`load_paths` → `merge_values`) from one overlay to an **ordered list**, folded
left-to-right (last writer wins for scalars and arrays; tables recurse — the
existing `merge_values` semantics are unchanged):

| # | layer | notes |
|---|-------|-------|
| 1 | `~/.config/wcode/config.toml` | the global file, always (today) |
| 2 | `./.wcode/config.toml` | **new** — auto-discovered, working dir only |
| 3 | `./.wcode/team.toml` | **new** — auto-discovered; overwrites `config.toml` (D-A2) |
| 4 | `--config <path>` / `WCODE_CONFIG` | explicit; layers **on top** (D-A3), still the top |

- **D-A1 — scope:** working dir only (`./.wcode/…`); no ancestor walk. Matches
  the user's `./.wcode/[config\|team].toml` framing.
- **D-A2 — order:** `config.toml` then `team.toml`, so **team overwrites config**.
- **D-A3 — explicit overlay still wins** and does *not* disable discovery.
- **D-A4 — missing auto files are skipped silently**; a missing *explicit*
  `--config` file stays a hard error (unchanged). A *present-but-malformed* auto
  file is NOT skipped: a read/parse error surfaces as a hard error — only
  *missing* auto files are silent.
- **D-A5 — opt-out:** `--no-project-config` flag (+ `WCODE_PROJECT_CONFIG=off`
  env). When set, only layers 1 and 4 load (today's behavior). Forwarded across
  the REPL re-exec exactly like `--agents`.
  **Amended by [D005](decisions/D005-global-agent-definitions.md):** the opt-out
  governs *project* discovery only. The global agent scan
  (`~/.config/wcode/agents/**/*.md`, Part C) is not project config, so it
  survives the opt-out — as the global `config.toml` (layer 1) always does. The
  shipped `--help` text said otherwise and was corrected.

## Part B — a team auto-enables agents mode

- **D-B1:** agents mode is ON (`args.agents = true`) when the resolved config has
  a non-empty `[team]` (TOML) **or** ≥ 1 discovered agent `.md` member — no
  `--agents` required. This makes `wcode` (and `wcode --agents`) "just work" in a
  repo that ships `.wcode/team.toml`.
- The `[team] requires --agents` guard stays as a safety net; with D-B1 it is
  normally already satisfied. `[workflow]` / `[orchestrator]` guards are unchanged
  (a workflow/orchestrator-only config still needs an explicit `--agents`).
- Enabling agents also enables what keys off `orchestrator.is_some()` today:
  the `spawn` tool, the `[team]` startup seed, verify-gate hooks, `[peers]`
  registration, workflow instantiation, and the scheduler.
- **Open (D-B2):** whether a *global* `[[team]]` (layer 1) should also auto-enable
  agents, or only project-discovered ones. v1: any non-empty folded `[team]`.

## Part C — markdown agent files (`.wcode/agents/*.md`)

### Location & discovery

- **project:** `./.wcode/agents/**/*.md` (recursive)
- **global:** `~/.config/wcode/agents/**/*.md` (recursive)
- **D-C1 — depth:** working dir only (consistent with D-A1). Walk-up to the repo
  root (the `skills.rs::project_dirs` pattern) is a possible follow-up.
- **D-C2 — collision:** a member is keyed by its `name`. Lowest→highest:
  global md < project md < the folded TOML `[team]` (layer 2–4 of Part A). So the
  explicit `[[team]]` TOML wins over a same-named `.md`; a project `.md` beats a
  global one. Same-name files *within* one dir: first in read order wins (log a
  warning). Reuses the existing duplicate-name rule (`ConfigError::DuplicateTeamMember`).

### Frontmatter → `TeamMember`

YAML frontmatter between `---` lines, then a markdown **body = the member's
`role`** (the worker's system prompt). Reuses `serde_yaml_ng` (already a
dependency via `skills.rs`). Unknown keys are ignored (serde default), matching
`SKILL.md`.

```markdown
---
name: reviewer                       # required, unique, the phonebook key
description: Two gates; no rubber-stamping.   # optional (see D-C4)
tools: [read, grep, find]            # optional; CSV string OR YAML list
model: gpt-5                         # optional; inherit if absent
effort: high                         # optional
read_only: true                      # optional (wcode-native)
# base_url / api_key                 # optional (per-agent provider)
---

You orchestrate two review gates. Body text becomes the member's `role`.
```

Mapping (1:1 onto `TeamMember` → `WorkerSpec`):

| md field | `TeamMember` | consumption today |
|----------|--------------|-------------------|
| `name` | `name` | phonebook key / `message` `to` |
| body | `role` | `WorkerSpec.system` → `# Role`; also the roster line |
| `tools` | `tools` | `default_tools` + allow-list; `message` always appended |
| `model` | `model` | worker `llm.model` |
| `base_url`/`api_key` | same | per-agent provider override |
| `effort` | `effort` | `-`/`none`/`off` clears, else sets |
| `read_only` | `read_only` | `ReadOnlyHooks` |
| `description` | — | **D-C4** |

- **D-C3 — `tools`:** accept a comma-separated **string** (`Read, Grep`) *or* a
  YAML **list**; normalized to `Vec<String>`. Absent = inherit all (`None`);
  empty = none. Names are wcode's **lowercase** tool names, validated by the
  existing `validate_tools` (no Claude-style case remapping in v1).
- **D-C4 — `description`:** accepted but informational in v1. `role` = the body,
  or the `description` when the body is empty. (Future: feed the roster blurb.)
- Files with no `name`, or unparseable YAML, are **skipped with a warning** (the
  skills lenient-warn/skip style), never fatal.

### Parser

The existing `skills.rs::parse_frontmatter` is private and returns **only** the
deserialized struct — no body. Extract a small shared module
(`crates/wcode-cli/src/frontmatter.rs`) exposing the delimiter split
(`---\n…\n---\n` → `(yaml, body)`), generic over the target type; refactor
`skills.rs` onto it and add the agent-`.md` parser beside the team logic.

## Where the code changes

Reference by function (lines drift):

- `config.rs` — `Config::load_with` / `load_paths`: take an ordered overlay
  **slice** and fold `merge_values` left-to-right; keep the `MissingModel` rescue
  carrying the post-fold file. `merge`'s `[team]`/`[workflow]` validation runs on
  the folded file (unchanged).
- `main.rs` — `load_config_raw`: assemble the ordered list (global is inside
  `Config::load*`; append `./.wcode/config.toml`, `./.wcode/team.toml`, then the
  explicit overlay); skip missing auto files; honor `--no-project-config`. After
  folding, discover agent `.md`s, merge them under the TOML `[team]` (D-C2), and
  set `args.agents |= team present` (D-B1) before `build_runtime`.
- new `crates/wcode-cli/src/agent_files.rs` — discover + parse `.wcode/agents/*.md`
  → `Vec<TeamMember>` (uses `frontmatter.rs` + `TeamMember`).
- new `crates/wcode-cli/src/frontmatter.rs` — shared frontmatter split.
- `skills.rs` — `parse_frontmatter` / `load_skill` refactored onto `frontmatter.rs`
  (behavior unchanged).
- `repl.rs` — `push_launch_args`: also forward `--no-project-config` when set.
  Auto-discovery needs no forwarding (re-exec inherits cwd).
- `main.rs` — `config_dump` (`--dump-config`): optionally print the overlay sources
  it folded, so a discovered project overlay is visible.

## Tests

- `config.rs`: fold N overlays in order (scalar last-wins; `[[team]]`/`[[workflow.node]]`
  arrays replace wholesale — assert the repo's `team.toml` overrides `config.toml`).
- Discovery: point cwd at a temp dir with `.wcode/config.toml` + `.wcode/team.toml`;
  assert they load; `--no-project-config` disables both.
- Auto-agents: a folded config with a team ⇒ agents mode on (assert `spawn` is
  registered / the `[team] requires --agents` guard does not fire).
- `agent_files.rs`: parse `name`/`tools` (string and list)/`model`/`effort`/
  `read_only`; body → `role`; empty body → `description`; unknown keys ignored;
  missing `name` → skipped with a warning; unparseable YAML → skipped.
- Collision: TOML `[[team]]` beats a same-named `.md`; project `.md` beats global.
- Explicit `--config` still layers on top of the discovered pair.
- Live check: in this repo, `wcode --agents` (no `--config`) starts the four-member
  team from `.wcode/team.toml`; `wcode -p "hi"` (no `--agents`) is unaffected apart
  from the auto-enabled team — **note the interaction** (a plain `-p` in this repo
  now auto-enables agents because `team.toml` ships a team; confirm that is wanted).

## Decisions (locked)

- D-A1 working-dir scope · D-A2 team over config · D-A3 explicit on top ·
  D-A4 silent skip of missing auto files · D-A5 `--no-project-config` opt-out.
- D-B1 team ⇒ agents on.
- D-C1 cwd-only agents dir · D-C2 TOML `[team]` > project `.md` > global `.md` ·
  D-C3 CSV-or-list tools, lowercase names · D-C4 body→role, description fallback.

## Open for first-layer review

- **D-B2** — should a layer-1 *global* `[[team]]` also auto-enable agents?
- **D-A6** — with D-B1, a plain `wcode -p "…"` in this repo now auto-enables
  agents (its `.wcode/team.toml` has a team). Acceptable, or should auto-load be
  agents-mode-gated (my original option)? The user chose auto-enable; confirming.
- **D-C5** — `description` handling (ignore vs fallback vs a new roster field).
- **D-C6** — cwd-only vs repo-root walk-up for `.wcode/agents/`.
