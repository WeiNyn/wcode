# D017 — Named project teams: `.wcode/teams/<name>.toml` + `--team` + `/team`

- **Status:** accepted (human sign-off 2026-10-10, this session)
- **Date:** 2026-10-10
- **Relates to:** the project-overlay discovery (`./.wcode/{config,team}.toml`,
  D005 / project-team-loading); the TUI's `/team` (D011-era command table);
  the VS Code surface's `/`-commands (W010 / D013)

## Context

A project team is `./.wcode/team.toml`, auto-discovered and folded over the
global config; `--config <path>` is an explicit overlay on top. That gives ONE
team per working dir — there was no way to keep **several named teams** and pick
one. The TUI's `/team` **already existed**, but it *listed the roster*
(`app.rs`, `summary: "list the team"`), so the name was taken.

## Decision

**Named teams live in `./.wcode/teams/<name>.toml`** (one file per team, name =
file stem), selected three ways:

| surface | command |
|---|---|
| CLI | `--team <name>` |
| TUI | `/team` (picker), `/team <name>`, `/team roster` (the old listing) |
| VS Code | `/team` (picker), `/team <name>` |

Sub-decisions (the brainstorm's five, all confirmed):

- **`--team <name>` is a flag** (not "callers pass `--config <path>`"), so the
  TUI and the extension pass a **name** and neither client knows the layout.
- **Precedence:** the team file is the *explicit* overlay, so it wins over
  `./.wcode/team.toml` and the global config. A `[[team]]` array is **replaced
  wholesale** by `merge_values` (`(_, over) => over`), which is exactly "use this
  team instead" — verified live, not assumed.
- **`--team` + `--config`/`WCODE_CONFIG` is refused loudly** — two explicit
  overlays are ambiguous.
- **A missing team is a loud error** naming the available teams.
- **`/team` in the TUI became the picker**; the roster listing moved to
  `/team roster` (keeping it was cheap).
- **Workspace-only** discovery (`.wcode/teams/`); a global
  `~/.config/wcode/teams/` is left as a hook.
- A `/team` session open in **both clients starts a NEW session** (a new tab in
  VS Code, a re-exec in the TUI) — a client cannot rebuild the agent.
- **An explicit `--team` replaces the team WHOLESALE, the project agent pool
  included** (amended 2026-10-11): the `[[team]]` array is replaced at merge
  time, and the `main.rs::fold_discovered_members` post-merge fold must not
  re-append `./.wcode/agents/*.md` underneath it — found live via
  `wcode --team recon` rendering 6 members (2 named + 4 discovered) instead of
  2. A named team that wants a project role writes
  `[[team]] file = ".wcode/agents/<role>.md"` (the `full-stack` shape); the
  GLOBAL agent scan (`~/.config/wcode/agents/`) still folds — it is not
  project config (D005). Regression tests:
  `an_explicit_team_replaces_the_project_agent_pool`,
  `a_named_team_still_resolves_a_file_member` (cli_flags.rs).

## Mechanism

- `wcode-cli/src/teams.rs` (new): `team_name` (pure), `discover(root)` (fs,
  files-only, sorted), `overlay_path(name)`, `TEAMS_DIR`.
- `main.rs`: `--team` parses into `Args.team`; `load_config_raw` refuses the
  `--config` combination, resolves `./.wcode/teams/<name>.toml` (error + the
  available list when absent), and **stashes the resolved path into `args.config`**
  so the existing overlay and re-exec plumbing forwards it with no signature change.
- `wcode-tui`: `Options.teams` (names, injected), `PickerKind::Team`,
  `App::set_teams` / `pending_team`, `open_team_picker` / `request_team`, and
  `Outcome::Team(String)`. `main.rs` re-execs with `--team <name>` for it; over a
  socket it is refused (there is no local binary to re-exec).
- `editors/vscode`: `src/teams.ts` (the twin discovery), a `team` slash command,
  `TabAction::team`, `SessionTabDeps.teamTab`, and `Manager.teamTab` opening a new
  tab with `["--team", name]` (label = the team name).

## Consequences

- A repo can ship several teams and pick one per session; the default
  `./.wcode/team.toml` still works untouched.
- Tests: `teams.rs` (name/discover/overlay), the TUI `/team` switch/picker/refuse,
  the command-table help text, `teams.ts` + the tab routing. `cargo test
  --workspace` (1144) + clippy, and `npm test` (243), all clean.
- Live-verified: `.wcode/team.toml` alone → `team.base`; `--team scout` → the
  team is REPLACED; `--team nope` → a loud error listing the available teams;
  `--team` + `--config` → refused.
