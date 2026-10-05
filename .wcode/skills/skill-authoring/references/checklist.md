# Skill authoring checklist

Run this alongside `sh scripts/validate_skill.sh` after every skill edit. The
script checks the mechanical rules; this list checks the ones a human must
judge.

## Frontmatter

- [ ] `---` is the **first line**, and there is a closing `---`.
- [ ] `name` equals the **directory** name.
- [ ] `name` is `[a-z0-9-]`, 1–64 chars, no leading/trailing/`--` hyphen.
- [ ] `description` is **quoted** and on **one line** (an unquoted `: ` is
      invalid YAML and silently drops the skill).
- [ ] `description` ≤ 1024 chars.
- [ ] `description` starts with the use case ("Use when …").
- [ ] `description` ends with an explicit **negative trigger** ("Not for X").
- [ ] The trigger sits inside the **first 200 characters** (the prompt clip).
- [ ] Unknown keys (if any) are deliberate — the loader ignores them.

## Body

- [ ] Under **500 lines**.
- [ ] Third-person imperative ("the reviewer emits …", not "you should …").
- [ ] One consistent term per concept across the whole file.
- [ ] Concrete templates / skeletons, not prose about templates.
- [ ] A checklist section a reader can run.
- [ ] A **References** section naming the real sources.
- [ ] An **"Editing this file"** note (the quoting/clip traps).

## Layout

- [ ] `references/`, `scripts/`, `assets/` are the only subdirectories used.
- [ ] Every file is **one level deep** (`references/x.md`, never
      `references/a/b.md`).
- [ ] No `README.md` inside the skill.
- [ ] Relative paths from the body to each asset are correct.

## Discovery

- [ ] The body names the artefacts the model needs to act (paths, tools, docs).
- [ ] The description distinguishes this skill from its nearest neighbour with
      the negative trigger.
- [ ] `sh .wcode/skills/skill-authoring/scripts/validate_skill.sh` exits 0.
- [ ] `--dump-system-prompt` lists the skill (run
      `cargo run -p wcode-cli -- --dump-system-prompt | sed -n '/Available skills/,/^$/p'`).
