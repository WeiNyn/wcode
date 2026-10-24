---
name: agent-roles
description: "Use when authoring or editing an agent role file under .wcode/agents/ — the frontmatter fields, what each one does, how the body becomes the worker's `# Role`, the tools allow-list form, and how to DECLARE the role as a team member (`[[team]] file =`). Not for authoring a SKILL.md and not for the [workflow] plan template."
---

# Agent roles

An **agent role** is a markdown file under `.wcode/agents/`: YAML frontmatter
(the member's fields) plus a markdown body that becomes the worker's `role` —
the text appended to its system prompt as `# Role`.

A team is assembled from roles; the roles themselves are reusable across
sessions and presets. Authoring a role is how a member's identity, tools, model,
and read-only posture are defined **once**.

## §0 — Where roles live (a convention, not a scan)

- **project:** `./.wcode/agents/<role>.md` — the convention this repo uses.
- **global:** `~/.config/wcode/agents/<role>.md` — the same convention, under
  the config dir.

Neither path is a mechanism. **A role file is a team member only when a
`[[team]]` entry declares it** (W033/D037 — membership is declared, never
discovered):

```toml
[[team]]
file = ".wcode/agents/<role>.md"   # relative to the working dir
```

`[[team]] file =` is resolved at boot by `main.rs::resolve_team_files` (and in
the live `team` tool by `agent_files::resolve_member_files`). A `.md` that no
entry points at is **inert**, however it is named and wherever it sits — a
directory listing creates nothing. Two consequences worth knowing:

- An unparseable file (no `name`, bad YAML) that IS declared is a **hard boot
  error** (`ConfigError::TeamMemberFile`), not a warning.
- A repo whose roles are all `.md` and whose `[[team]]` array is empty has **no
  team**, and does not auto-enable agents mode.

A declared file keeps its own `name`; give the `[[team]]` entry a `name` only
when you want to assert it (a mismatch is fatal). Names must be unique across
the whole final team (`ConfigError::DuplicateTeamMember`).

## §1 — The frontmatter

```markdown
---
name: reviewer                       # required, unique, the phonebook key
description: Two gates; no rubber-stamping.   # optional, informational
tools: [read, grep, find]            # optional; CSV string OR YAML list
model: gpt-5                         # optional; inherit the root's if absent
effort: high                         # optional; '-'/'none'/'off' clears
read_only: true                      # optional; wcode-native
base_url: https://api.example.com/v1 # optional; per-agent provider
api_key: sk-...                      # optional; per-agent provider
---

You orchestrate two review gates. This body becomes the member's `role`.
```

### Field-by-field

| field | required | what it does |
|-------|----------|--------------|
| `name` | **yes** | the member's address — the phonebook key and the `message` `to` value. A file without one is skipped. |
| `description` | no | informational in v1; used as the `role` fallback **only when the body is empty**. |
| `tools` | no | an allow-list over the default tools. Absent = inherit all; present-but-empty = only `message`. |
| `model` | no | overrides the worker's model id. Absent = the orchestrator's model. |
| `effort` | no | reasoning-effort override. `-`/`none`/`off` clears to send-nothing. |
| `read_only` | no | enforces read-only: mutating tools and mutating `bash`/`bg` are refused. |
| `base_url` | no | per-agent provider base URL (default: the orchestrator's). |
| `api_key` | no | per-agent provider API key (default: the orchestrator's). |

Unknown keys (`license`, `allowed-tools`, …) are **ignored** (serde default),
matching `SKILL.md`.

## §2 — The body is the role

The **markdown body** becomes the member's `role`, appended to the worker's
system prompt as `# Role` after the identity blurb (who it is, who owns it, the
auto-report). The body is **free text**: write the worker's job, its output
contract, and its boundaries.

If the body is empty, the `description` is used as the role fallback
(`crates/wcode-cli/src/agent_files.rs:XMcjX`). Do not rely on it — write the
body.

Write the body for an agent: what it does, what it never does, what it returns,
and the format of that return. Concrete over prose.

## §3 — The `tools` allow-list

- **CSV string or YAML list.** `tools: read, grep, find` and
  `tools: [read, grep, find]` are equivalent (`de_tools`,
  `crates/wcode-cli/src/agent_files.rs:iRidm`).
- **Lowercase wcode tool names.** `read`, `grep`, `find`, `bash`, `edit`,
  `write`, `webfetch`, `ast_search`, `ast_edit`, `message`, `bg`, `todo`, and so
  on. No Claude-style case remapping.
- **`message` is always kept** regardless of the list (the report path).
- **An unavailable name fails loudly** at spawn via `validate_tools` — never
  silently dropped (team plan D14). So a typo'd tool is a boot error, not a
  worker missing a capability.

Watch the two failure modes:

- A tool named in the list but **off by default** (`grep`, `find`) must also be
  enabled in `[tools] grep/find = true`, or the worker will not have it.
- **A description that promises a tool the `tools` list omits** is a trap: the
  worker is told it can do a thing it cannot. Keep the body and the list in sync
  — if the body says "non-mutating bash", `bash` must be in `tools` **and**
  enabled.

## §4 — Declaring the role (and what an undeclared file does)

Writing the `.md` is half the job: the other half is the **declaration**. Add
one entry per role — a project file in `.wcode/team.toml`, a global one in
`~/.config/wcode/config.toml`:

```toml
[[team]]
file = ".wcode/agents/reviewer.md"
```

Two facts that used to be traps and are now structural (W033/D037):

- **A `.md` that no `[[team]]` entry references is inert.** Dropping a file into
  `.wcode/agents/` used to add a member on every boot; it no longer does
  anything at all. There is no directory scan left to shadow, so no shadowing
  trap: a `[[team]]` entry either points at a file or it does not.
- **A `[[team]]` array is REPLACED wholesale at merge time**, not merged. An
  overlay that declares members therefore *is* the team — a global `[[team]]`
  array contributes nothing once a project overlay declares one. Declare the
  complete set where you want it to apply.

## §5 — Checklist

- [ ] The file lives under `.wcode/agents/`.
- [ ] A `[[team]] file = "…"` entry declares it — the ONLY thing that makes it a member.
- [ ] `name` is present and unique across the final team.
- [ ] The body states the job, the boundaries, and the output format.
- [ ] `tools`, if given, lists **lowercase** wcode names; `message` is assumed.
- [ ] Any `tools` name that is off by default is enabled in `[tools]`.
- [ ] The body does not promise a tool the `tools` list omits.
- [ ] `read_only: true` for a recon/review role that must not mutate.

## References

- `docs/project-team-loading.md` (Part C) — the frontmatter/parse design.
- `docs/team-and-tui-plan.md` (F2/F3) — what each field maps to.
- `crates/wcode-cli/src/agent_files.rs` — the parser (`parse_agent_md`) and the
  declared-file resolver (`resolve_member_files`).
- `README.md` §"Configure" — the `[team]` field table.
- `docs/decisions/D003-team-file-reference.md` — `[[team]] file = "<path>"`.
- `docs/decisions/D037-toml-only-team-membership.md` — why a `.md` alone is not
  a member (W033).

## Editing this file

Keep the frontmatter **quoted** and on one line. `description` ≤ 1024 chars,
clipped to 200 in the prompt — the "Use when … Not for …" trigger must sit in
the first 200. Validation: `crates/wcode-cli/src/skills.rs:efuUM`, `:Vlnli`,
`:ftQCJ`.
