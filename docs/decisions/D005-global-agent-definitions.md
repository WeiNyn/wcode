# D005 — global agent definitions live at `~/.config/wcode/agents/`

- **Status:** accepted
- **Date:** 2026-10-05
- **Supersedes / relates to:** `docs/project-team-loading.md` (D-A5)

## Context

`config_dir()` is `~/.config/wcode` (`crates/wcode-cli/src/config.rs:IbFj3`,
body `DMdtQ`), and `agent_files::discover_global` already scans
`<home>/agents/**/*.md` (`crates/wcode-cli/src/agent_files.rs:jmdEk`) — i.e.
`~/.config/wcode/agents/`. But `README.md` documents only the project form,
`./.wcode/agents/**/*.md` (`README.md:U8wUO`).

The `--help` text says `--no-project-config`/`WCODE_PROJECT_CONFIG=off` disables
the global scan too: "disable ALL auto-discovery: ./.wcode/{config,team}.toml,
./.wcode/agents/, and ~/.config/wcode/agents/"
(`crates/wcode-cli/src/main.rs:3vCf7`, `:Lqk9P`). That contradicts
`docs/project-team-loading.md:zQnrT` (D-A5), which frames only "layers 1 and 4"
as surviving — the global config plus an explicit `--config` — not the global
agents scan.

## Decision

Global agent definitions live at `~/.config/wcode/agents/**/*.md`.
`--no-project-config` / `WCODE_PROJECT_CONFIG=off` **no longer** disables that
global scan; the switch governs **project** config only (the `./.wcode/`
overlays and the project `./.wcode/agents/` scan). The `--help` text and
`README.md` are corrected to match.

## Consequences

- `~` is not governed by a project-scoped switch — the global scan always runs.
- One behaviour changes: an opt-out that used to suppress the global agents scan
  no longer does.
