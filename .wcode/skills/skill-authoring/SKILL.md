---
name: skill-authoring
description: "Use when writing or editing a skill for the wcode repository — the SKILL.md frontmatter tuned for discovery, the under-500-line brain, progressive disclosure into references/scripts/assets at one level, and the validate_skill.sh gate. Not for authoring an agent role file (.wcode/agents/*.md) and not for a one-off prompt you will not keep."
---

# Skill authoring

A skill is a directory holding a `SKILL.md`: YAML frontmatter plus a body. Only
`name` + `description` reach the system prompt (as `# Available skills`); the
body loads on demand when the agent `read`s the path. A skill is written for an
**agent**, in third-person imperative, with concrete templates.

## §0 — The standard, at a glance

| rule | value | why |
|------|-------|-----|
| frontmatter | `---` on line 1, closing `---` | the loader splits on it |
| `name` | 1–64 chars, `[a-z0-9-]`, no leading/trailing/`--` hyphen | `valid_name` (`crates/wcode-cli/src/skills.rs:efuUM`) |
| directory name | **equals** `name` | the standard allows a mismatch; this repo matches anyway |
| `description` | non-empty, ≤ 1024 chars | `:Vlnli` |
| prompt clip | first 200 chars | `:ftQCJ` — the trigger must fit |
| body | **under 500 lines** | the brain stays small |
| assets | `references/`, `scripts/`, `assets/` — **one level deep** | `references/x.md`, never `references/a/b.md` |
| files inside a skill | no `README.md` | the `SKILL.md` is the entry point |
| quote the description | `description: "…"` on one line | an unquoted `: ` is invalid YAML; the skill is then dropped |

Discovery is recursive to depth 4 under `.wcode/skills/`
(`crates/wcode-cli/src/skills.rs:AdX6p`); unknown frontmatter keys are ignored.

## §1 — Frontmatter optimised for discovery

The description is the **only** text the model sees when deciding whether to
load the skill. Write it as a trigger:

1. **Start with the use case**: "Use when …", "Use to …".
2. **Name the artefacts** it acts on (paths, tools, doc names).
3. **End with an explicit negative trigger**: "Not for X." A negative trigger is
   what stops the model loading the wrong skill.
4. Keep the trigger inside the **first 200 characters** (the prompt clip).

```yaml
---
name: <kebab-case, equals the directory>
description: "Use when <situation> — <what it does, with the artefacts it names>. Not for <the nearest wrong use>."
---
```

### §1.1 The negative trigger is required

Every skill description carries an obvious `not for` (or `don't use`) marker.
`validate_skill.sh` warns when it is missing — a warning, not a failure. The
marker is what makes a suite of skills exclusive rather than a pile of
overlapping advice.

## §2 — The body is the brain, under 500 lines

The body is loaded when the skill is used, so it should be the **whole method**,
not a table of contents. Structure it the way this repo's TUI spec is written:

- a one-paragraph statement of what the skill is and is not;
- numbered sections (`## §1 — …`), third-person imperative;
- **concrete templates over description** — a fenced skeleton beats a paragraph
  about the skeleton;
- a **checklist** section a reader can run;
- a **References** section pointing at the real sources;
- an **"Editing this file"** note (see §6).

If the body is about to pass 500 lines, move material out (§3) rather than
growing the brain.

## §3 — Progressive disclosure: one level deep

| directory | holds | loaded |
|-----------|-------|--------|
| `references/` | bulky detail, long tables, rubrics | on demand via `read` |
| `scripts/` | deterministic operations | on demand via `bash` |
| `assets/` | output templates, skeletons | copied/filled by the agent |
| `SKILL.md` | the brain | on demand via `read` |

**Exactly one level deep.** `references/estimate-rubric.md` is fine;
`references/a/b.md` is not — the validator fails it. Refer to each file by its
**relative** path from the skill body (the prompt gives the skill's directory).

Never nest a `README.md` inside a skill — the `SKILL.md` is the entry point.

## §4 — Style rules

- **Third-person imperative.** "The reviewer emits one verdict", not "you
  should approve".
- **One consistent term per concept.** Pick "sketch" *or* "draft", "gate" *or*
  "check" — never both in one skill.
- **Concrete templates over prose.** A skeleton the reader fills in beats three
  paragraphs about what it should contain.
- **No filler.** Cut "it is important to note that". State the rule.
- **Numbers, not vibes.** "under 500 lines", "≤ 1024 chars" — measurable.

## §5 — The gate: `validate_skill.sh`

Run it from the repo root after every skill edit:

```sh
sh .wcode/skills/skill-authoring/scripts/validate_skill.sh ; echo "exit=$?"
```

It checks frontmatter delimiters, the `name` rule and directory match, the
`description` cap and the negative trigger (warn), the under-500-line body, the
absence of a `README.md`, and the one-level-depth rule. It prints `file: reason`
per failure and exits 1. Run
[`references/checklist.md`](references/checklist.md) alongside it.

## §6 — Documented in the body: quoting the description

Keep the frontmatter **quoted** and on one line. An unquoted `: ` is invalid
YAML and the loader drops the whole skill with only a stderr warning
(`crates/wcode-cli/src/skills.rs:9870X` → `parse_frontmatter`). The description
is capped at 1024 chars (`:Vlnli`) and clipped to 200 in the prompt (`:ftQCJ`),
so the trigger must sit inside the first 200. Only `name` + `description` reach
the prompt (`:7ib2K`); the body loads on demand via `read`.

## §7 — Checklist

- [ ] Directory name equals `name`.
- [ ] `name` is `[a-z0-9-]`, 1–64, no leading/trailing/`--` hyphen.
- [ ] `description` is quoted, on one line, ≤ 1024 chars, trigger in the first
      200, and ends with a negative trigger.
- [ ] Body is under 500 lines and third-person imperative.
- [ ] Bulk material is in `references/`/`scripts/`/`assets/`, one level deep.
- [ ] No `README.md` inside the skill.
- [ ] `sh .wcode/skills/skill-authoring/scripts/validate_skill.sh` exits 0.

## References

- `crates/wcode-cli/src/skills.rs` — the loader and its rules.
- `docs/skills-references-plan.md` — the discovery/roots design.
- `README.md` §"Skills" — the user-facing summary.
- [`references/checklist.md`](references/checklist.md) — the per-skill checklist.
- [`scripts/validate_skill.sh`](scripts/validate_skill.sh) — the gate.

## Editing this file

Keep the frontmatter **quoted** and on one line. `description` ≤ 1024 chars,
clipped to 200 in the prompt. Validation: `crates/wcode-cli/src/skills.rs:efuUM`,
`:Vlnli`, `:ftQCJ`.
