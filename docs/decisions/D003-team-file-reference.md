# D003 — `[team]` may reference an agent file by path

- **Status:** accepted
- **Date:** 2026-10-05
- **Supersedes / relates to:** `docs/project-team-loading.md` (D-C1, D-C2)

## Context

Markdown agent definitions already exist: `.wcode/agents/*.md` is discovered
(`crates/wcode-cli/src/agent_files.rs:9XTe5`) along two **fixed** roots — the
project `cwd/.wcode/agents/**/*.md` (`:CKCbg`) and the global
`<home>/agents/**/*.md` (`:jmdEk`). A preset therefore cannot point at a role
file outside those roots.

Worse, a same-named TOML `[[team]]` entry wins **wholesale** over an md member
(D-C2): the fold keeps the existing cfg member and drops the discovered one
rather than merging fields —

```rust
for member in agent_files::discover(&cwd, home.as_deref()) {
    if !cfg.team.iter().any(|t| t.name == member.name) {
        cfg.team.push(member);
    }
}
```

(`crates/wcode-cli/src/main.rs:YGsc5`, guard `:MC499`). A preset that names the
same member as a discovered file silently loses the whole md role.

## Decision

`[[team]]` gains `file = "<path>"`:

- The path is **cwd-relative** (an absolute path is also allowed).
- The named file supplies `name`/`role`/`tools`/`model`/`effort`/`read_only`/
  `base_url`/`api_key` (parsed by the existing `agent_files` reader,
  `crates/wcode-cli/src/agent_files.rs:XMcjX`).
- Sibling TOML keys override a field when set.
- A `name` mismatch between the TOML key and the file's `name` is **fatal**.
- A missing or malformed **explicitly named** file is **fatal** — unlike a
  scan-discovered file, which warns and is skipped
  (`crates/wcode-cli/src/agent_files.rs:tyJaU`).

TOML-file-relative resolution is **rejected**: `merge_values`
(`crates/wcode-cli/src/config.rs:HjeeZ`) folds files without source provenance,
so a member cannot know which file it came from.

## Consequences

- Enables relocatable-ish presets and kills the silent shadowing trap.
- One new fatal error class (missing/mismatched explicitly named file) next to
  the lenient scan.
