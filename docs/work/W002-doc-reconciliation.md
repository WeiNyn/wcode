# W002 — Docs reconciliation (v0.3.x → README, VS Code README, tracker) — brief

- **Status:** shipped — P0–P3 landed (4 docs-only commits); `cargo test`/`clippy` clean
- **Work item:** W002
- **Tracker:** [next-steps.md](../next-steps.md) item 59
- **Decisions:** [D004](../decisions/D004-docs-taxonomy.md) (docs taxonomy; Phase 2 link sweep anticipated)
- **Author:** brainstormer (session `agent:brainstormer`), recorded by the orchestrator
- **Date:** 2026-10-06

## 1. The ask

Bring wcode's user-facing docs back in line with the shipped code. Three read-only
recons (core / TUI / VS Code) plus the orchestrator's own verification found that
the docs still describe v0.3.0 while the repo ships v0.3.4. The drift is of two
kinds: **missing** (tools, REPL commands, flags, config, session groups that exist
but are undocumented) and **wrong** (the README describes a right-hand ≥60-col
sidebar; the VS Code README documents a native `Members` `TreeView` and a
`memberIconSpec` symbol that do not exist, and presents palette-only `member.*`
commands as live context-menu actions). This is a **docs-only** change — no
behavior change, no code. It is the reconciliation pass, so every edit is a
correction *toward the code*, not a feature.

## 2. Scope / Non-scope

**In scope.**

- `README.md` — the **core/tools/flags/config** surface: the tools table, the REPL
  command table, the `## Use` flag block, the `[workspace]` config + digest-CAS
  guard, session groups.
- `README.md` — the **TUI** surface: correct the wrong sidebar description; add the
  working-team band, input-box corner chrome, browse keys, mouse gestures, and the
  missing TUI `/`-commands.
- `editors/vscode/README.md` — a rewrite of the stale status, layout, and click-path
  sections to match the shipped `webviewView` surface and the real symbol/command set.
- `docs/vscode-ui-v3-plan.md` — flip V13r and V14 from *proposed* to *shipped* in the
  status line and the progress table (the tracker's shipped-commit list is the
  authoritative source; see §4 open item).
- `docs/next-steps.md` — extend the **Sequencing** section past item 23, and correct
  rows 43/44 from "v0.3.0" to the current release(s), plus the §44 "v0.2.0" detail.

**Non-scope** (each names something a reader might assume is in scope):

- **The Homebrew `Formula/wcode.rb` 0.3.4 bug and the missing v0.3.4 release-workflow
  commit** — a separate `ci:`/`chore:` item with a code/build gate; not a docs edit.
- **Any code or behavior change — including *wiring* the VS Code `member.*` menus.**
  The VS Code README documents the *reality* (they are palette-only and inert over a
  webview view) and flags the wiring as a follow-up. Editing `package.json`/
  `extension.ts` would be a `feat:` with its own review.
- **The `--help` / `USAGE` config dump.** The code's `USAGE` string may itself omit
  `[workspace]` — that is a `main.rs` edit, i.e. code, so out of scope.
- **The D004 Phase-2 docs move** (relocating the flat `docs/*.md`) — item 58 stays
  open (`docs/next-steps.md:YiDNs`); this pass does not move files, so it needs no
  link sweep of its own.
- **Adding a committed automated drift test** (e.g. a Rust test asserting the README
  tool table matches `default_tools`) — valuable, but it is code; proposed as a
  follow-up in §6/§7.
- **`AGENTS.md`** — it reads correctly today; no edit.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | README core/tools/flags/config | high | low | low | adds the shipped `todo`/`session_search`/A2A tools, `/plan`/`/verify`, four flags, `[workspace]`, session groups; pure prose |
| P1 | README TUI correction | high | low | low | fixes a description that is *wrong* (sidebar side/width), not merely absent; readers act on it today |
| P2 | VS Code README rewrite | high | medium | low | a whole stale section (a non-existent file and symbol, an inert menu presented as live); needs careful grounding in `reducer.ts`/`package.json` |
| P3 | tracker + v3-plan reconciliation | medium | low | low | bookkeeping only, but it is the index of record; a wrong status misleads the next planner |

Value = does the doc tell the truth about the shipped product. Complexity = how
much prose/logic. Risk = what breaks if the doc is wrong (low here — no runtime
impact, but a wrong TUI/VS Code description wastes a user's time).

## 4. Interface & structure

**Added.** None. This is an in-place edit of existing files (no new docs beyond this
work-item record).

**Changed.**

*README.md — core/tools/flags:*

- `README.md::## Configure` (`README.md:NkRTf`) — add a `[workspace]` table (digest
  CAS) after `[retry]`. Truth: `config.rs:jSE4x` (`WorkspaceConfig`),
  `config.rs:XO39Q` (`[workspace]` field), `workspace.rs:Ou96m`
  (`EXPECTED_DIGEST_KEY`).
- `README.md::## Use` flag block (`README.md:2xWUd`…`:UHQtU`) — add `--dump-config`,
  `--detect-endpoint`, `--peer`, `--name`, `--owner`. Truth: `main.rs:F2R77`
  (`--dump-config`), `main.rs:13Aq2` (`--detect-endpoint`), `main.rs:DvxOk`/`qegKu`/
  `yU0NF` (`--peer`/`--name`/`--owner`).
- `README.md::REPL command table` (`README.md:kW6Xi`) — add rows for `/plan [on|off]`
  and `/verify`. Truth: `repl.rs:1171` (`Plan` handler), `repl.rs:1186` (`Verify`
  handler), `repl.rs:162`/`:168` (parse). Also document **plan mode** itself
  (`PlanModeHooks`, `PLAN_SECTION`): `hooks.rs:7GyLO`, `agent.rs:zGWHg`.
- `README.md::## Tools` table (`README.md:mz1I4`) — add `todo`, `session_search`, and
  the root A2A set `spawn`/`message`/`peers`/`task` (keep the existing `member` prose
  at `README.md:YhdWK`). Truth: `tools/mod.rs:0Ikg4` (`todo`), `:VajFP`
  (`session_search`); `agents.rs:yalsd` (`Orchestrator::tools` →
  `spawn`,`message`,`peers`,`member`,`task`).
- `README.md::## Sessions` (`README.md:bq3gR`) — add session groups
  (`session_groups.rs:7XRo7` `rebuild_team`).

*README.md — TUI:*

- `README.md::sidebar paragraph` (`README.md:bUKqZ`, `README.md:K890I`) — **correct**:
  it is a **left**-docked sidebar, `SIDEBAR_WIDTH=30`, shown at **≥80** columns, with
  **Team / Todos / Changes** sections and a `*` on the focused row — not a right-hand
  `label · model · state` list at ≥60 cols. Truth: `ui.rs:qEHff` (=30), `ui.rs:Bd6nu`
  (=80), `ui.rs:4S7x3` (left split), `ui.rs:OVnem` (`draw_sidebar`), `ui.rs:PXysK`
  (`*` marker).
- `README.md` — add the **working-team band** (`ui.rs:3yhde`), the **input-box corner
  chrome**, the **browse** keys `Home`/`End`/`{`/`}`/`/`/`?` (keymap `app.rs:Spmdk`),
  the **mouse** gestures (click block / click member / drag-select), and the TUI
  commands `/btw` `/plan` `/verify` `/tasks` (`app.rs:np3od` `COMMANDS`).

*editors/vscode/README.md:*

- `editors/vscode/README.md::Status` (`editors/vscode/README.md:mGoM9`) — replace
  "Status: P3 … a live member sidebar that is a native control surface". Reality: one
  **webview view** `wcode.surface` (`package.json:JZ8w4`) with an in-webview rail
  (`webview/chat.ts`), not a native `TreeView`.
- `editors/vscode/README.md::Layout` table (`editors/vscode/README.md:kLxli`) — drop
  the `src/roster.ts` row; the real files are `surface.ts`, `commands.ts`,
  `sessions.ts`, `startup.ts`, `review.ts`, `webviewView.ts` (confirmed: **no
  `roster.ts` exists**).
- `editors/vscode/README.md::What is verified` (`editors/vscode/README.md:kgY8k`) —
  `memberIconSpec` does **not** exist; the real symbols are `memberGlyph`
  (`reducer.ts:RNWdV`) and `memberViews` (`reducer.ts:cHTAz`).
- `editors/vscode/README.md::click-path step 8` (`editors/vscode/README.md:raCXd`) —
  "Right-click a member → `wcode: Peek Member`" is **not live**: the only contributed
  menu is `view/title` → `wcode.openInEditor` (`package.json:bPdPH`); `wcode.member.*`
  are palette-only and receive `undefined`, so they are inert (`extension.ts` ~line
  114). Document the reality; flag wiring as a follow-up.

*docs/vscode-ui-v3-plan.md:*

- `docs/vscode-ui-v3-plan.md` status line and the progress table rows for V13r/V14 →
  mark shipped. (Commits exist: `910c9f5` V13r, `6d4f6f8`/`0abc18c` V14; the tracker
  `docs/next-steps.md:Xmf4n` already lists them shipped.)

*docs/next-steps.md:*

- `docs/next-steps.md::## Sequencing` (`docs/next-steps.md:02Mhj`) — currently stops at
  item 23; extend through item 58 (or state explicitly that sequencing is retired).
- `docs/next-steps.md:EZykg` (row 43) and `:nw8HN` (row 44) — "v0.3.0" → the current
  release(s); `docs/next-steps.md:kiBJi` (§44 detail) — "for v0.2.0" → the current
  version.

**Integration points** (verified before writing; anchors drift only if the line itself
changes):

- `crates/wcode-cli/src/tools/mod.rs:0Ikg4` / `:VajFP` — `todo` and `session_search`
  are always registered.
- `crates/wcode-cli/src/agents.rs:yalsd` — `Orchestrator::tools` returns
  `spawn`,`message`,`peers`,`member`,`task`.
- `crates/wcode-cli/src/repl.rs:1171` / `:1186` — `/plan` and `/verify` handlers exist.
- `crates/wcode-harness/src/hooks.rs:7GyLO`, `crates/wcode-harness/src/agent.rs:zGWHg`
  — `PlanModeHooks`, `PLAN_SECTION`.
- `crates/wcode-cli/src/main.rs:F2R77` / `:13Aq2` — `--dump-config`, `--detect-endpoint`.
- `crates/wcode-cli/src/config.rs:jSE4x`, `crates/wcode-cli/src/workspace.rs:Ou96m` —
  `[workspace]`/digest-CAS exist.
- `crates/wcode-tui/src/ui.rs:qEHff` / `:Bd6nu` / `:4S7x3` / `:OVnem` / `:3yhde` —
  sidebar is left, width 30, min 80; `draw_sidebar`, `draw_working_team`.
- `editors/vscode/package.json:JZ8w4` / `:bPdPH` — one webview view; no `member.*` menu;
  `view/title` → `openInEditor` only.
- `editors/vscode/src/reducer.ts:RNWdV` / `:cHTAz` — `memberGlyph`, `memberViews`.
- `editors/vscode/src/extension.ts` (~line 114) — `member.*` are inert.
- `docs/decisions/D004-docs-taxonomy.md:GVEFs` — Phase 2 carries its own link check.

**One flag (do not silently resolve):** the tracker `docs/next-steps.md` row 56
(`Xmf4n`) lists V13r as shipped, while `docs/vscode-ui-v3-plan.md` lists V13r/V14 as
*proposed*. Two docs disagree; the reconciliation must pick the authoritative source
(the tracker's shipped-commit list) and, if the plan is stale, update it — **not** the
reverse.

## 5. Plan

Each step states its **deliverable** and its **gate**.

1. **P0 — README core/tools/flags/config.** Deliverable: `README.md` documents
   `todo`/`session_search`/A2A tools, `/plan`+`/verify`+plan mode,
   `--dump-config`/`--detect-endpoint`/`--peer`/`--name`/`--owner`, `[workspace]`
   digest CAS, session groups. Gate: a consistency grep shows every
   `erased(`-registered tool named in the tools table, and `wcode --help` output is a
   subset of the flag block.
2. **P1 — README TUI correction.** Deliverable: the sidebar paragraph reads
   *left-docked, 30 wide, ≥80 cols, Team/Todos/Changes, `*` focus*; the working-team
   band, input-box chrome, browse keys, mouse gestures and TUI commands are listed.
   Gate: `git grep` finds no "right-hand sidebar"/"60 columns" in `README.md`; the
   browse keys match the `app.rs` keymap.
3. **P2 — VS Code README rewrite.** Deliverable: `editors/vscode/README.md` has no
   `roster.ts`, no `memberIconSpec`, an accurate status/surface, and a click-path that
   marks `member.*` as palette-only/inert. Gate:
   `git grep -nE "roster\.ts|memberIconSpec|native .*TreeView|Right-click a member"
   editors/vscode/README.md` → no matches; every symbol named exists in `src/`.
4. **P3 — tracker + v3-plan reconciliation.** Deliverable: `docs/next-steps.md`
   Sequencing runs to the last item and rows 43/44 (§44) name the current release;
   `docs/vscode-ui-v3-plan.md` V13r/V14 marked shipped. Gate: manual read of the two
   files against `git tag` (`v0.3.4`) and the row-56 commit list.

**Proposed commit split (4 commits).** AGENTS.md mandates **one logical change per
commit** with an area prefix and a second-layer review per change. The four
workstreams map onto **distinct truth sources**, which is what makes them
independently reviewable:

- **`docs: README — core, tools, flags, workspace, sessions`** (P0).
- **`docs: README — correct the TUI description`** (P1). Kept separate because it is a
  **correction of a wrong claim**, not an addition — a focused diff helps the
  reviewer verify against `ui.rs`.
- **`docs: VS Code README — match the shipped webview surface`** (P2).
- **`docs: reconcile the tracker and the v3 UI plan`** (P3).

## 6. Quality gates

Docs-only, so the Rust gates prove *absence of accidental code change*, and the real
gate is a docs check.

- `cargo test --workspace` → all pass (unchanged — proves the diff touched only
  Markdown).
- `cargo clippy --workspace --all-targets` → clean.
- **Docs consistency (the real gate), run manually** (no in-repo tool exists today):
  - `git grep -nE "right-hand sidebar|60 columns|roster\.ts|memberIconSpec" -- README.md editors/vscode/README.md` → **no matches**.
  - Tool-table drift: `git grep -h "erased(" crates/wcode-cli/src/tools/mod.rs` and
    compare each `erased(<Type>)` against a name in the `README.md` tools table; every
    one present.
  - Flag drift: every flag in the README `## Use` block is a real flag.
- **Proposed docs gate as a committed follow-up (not this diff):** a `docs/*.md`
  **relative-link check** — the sweep D004 anticipates (`D004:GVEFs`); today no such
  script/tool is present in-repo. A second candidate is a Rust **doc-drift test**
  asserting the README tool table matches `default_tools`; both are code, so they ride
  a separate `test:`/`chore:` commit.

## 7. Testing

### 7.1 Automated

- `cargo test --workspace` — asserts nothing else changed (the diff is Markdown; a red
  suite means a stray file was touched).
- `cargo clippy --workspace --all-targets` — clean.
- The `git grep` consistency checks in §6 — assert the *wrong* strings are gone and the
  *missing* names are now present.

### 7.2 How a human verifies it

```sh
cargo run -p wcode-cli -- --help            # every flag in the README ## Use block appears
cargo run -p wcode-cli -- --dump-config     # the newly documented flag actually runs
cargo run -p wcode-cli -- --tui             # Ctrl-B toggles a LEFT sidebar; F1 lists browse keys
git grep -n "roster.ts\|memberIconSpec" editors/vscode/README.md   # expect: nothing
git tag | tail -1                           # v0.3.4 — matches the tracker rows
```

## 8. Expected outcome

- `README.md` tool table names **every** tool registered by `default_tools`
  (`tools/mod.rs:QuVka`) **and** the root A2A set (`agents.rs:yalsd`) — including
  `todo` and `session_search`.
- `README.md` REPL command table names `/plan [on|off]` and `/verify`, and plan mode +
  `PLAN_SECTION` are described.
- `README.md` `## Use` block names `--dump-config`, `--detect-endpoint`, `--peer`,
  `--name`, `--owner`; the `## Configure` section contains a `[workspace]` table.
- `README.md` describes the TUI sidebar as **left-docked, width 30, ≥80 cols,
  Team/Todos/Changes, `*` focus**, and lists the working-team band, browse keys, mouse
  gestures, and `/btw` `/plan` `/verify` `/tasks`.
- `editors/vscode/README.md` contains **no** `roster.ts`, **no** `memberIconSpec`,
  describes the single **webview view** surface, and marks `wcode.member.*` as
  palette-only/inert.
- `docs/vscode-ui-v3-plan.md` shows V13r and V14 as **shipped** (consistent with
  `next-steps.md` row 56).
- `docs/next-steps.md` Sequencing reaches the last item, and rows 43/44 (§44) name the
  current release.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` are still green;
  `git diff --stat` touches **only** `.md` files.

## 9. Facts NOT verified

- **No binary was built or run.** Every claim is from source reading; the §7.2 commands
  were not executed.
- **No docs link check was run** — current link breakage in `docs/**` was not measured.
- **`.wcode/team.toml` / `.wcode/agents/*.md`** contents were not read; the README team
  section may have its own drift not assessed.
- **`docs/*-plan.md` other than v3** were not read; other plan docs may carry the same
  "proposed-but-shipped" staleness.

## 10. Outcome — shipped

- **2026-10-06** — **Landed.** Four docs-only commits, one per workstream:
  `e0ef078` (P0 — README core/tools/flags/workspace/sessions), `a9772a3` (P1 —
  README TUI correction), `7c166f0` (P2 — VS Code README), `f0ca9d4` (P3 —
  tracker + v3 plan). `git diff --stat` over the four touches **only** `.md`
  files: `README.md`, `editors/vscode/README.md`, `docs/next-steps.md`,
  `docs/vscode-ui-v3-plan.md`.
- **2026-10-06** — **Gates.** `cargo test --workspace` → **1113 passed, 4
  ignored**; `cargo clippy --workspace --all-targets` → clean. Both unchanged
  from the pre-change baseline, which is the proof this diff added no code.
- **2026-10-06** — **Consistency checks (the real gate).** `git grep -nE
  "right-hand sidebar|60 columns|roster\.ts|memberIconSpec" README.md
  editors/vscode/README.md` → no matches. Every `erased(…)` tool in
  `crates/wcode-cli/src/tools/mod.rs` is named in the README tools table
  (`member` stays prose — it is root-only, `README.md` §Tools). Every README
  `## Use` flag exists in `main.rs`: `--dump-config`, `--detect-endpoint`,
  `--peer`, `--name`, `--owner`.
- **2026-10-06** — **Corrections verified against the code** (the point of the
  pass): `SIDEBAR_WIDTH = 30` / `SIDEBAR_MIN_WIDTH = 80` (`ui.rs`), so the TUI
  sidebar is left-docked and needs ≥ 80 columns; the `wcode.member.*` commands
  are `onCommand`-activated only and the sole `view/title` menu is
  `wcode.openInEditor` (`package.json`), so the README's "Not yet wired" note is
  accurate; there is no `src/roster.ts`; `memberGlyph`/`memberViews` are the real
  symbols (`reducer.ts`).
- **2026-10-06** — **Deferred (non-scope, unchanged):** the Formula v0.3.4 bug
  and the missing release-workflow commit (a `ci:`/`chore:` change); wiring the
  VS Code `member.*` menus (a `feat:`); a committed docs link-check / doc-drift
  test (a `test:`/`chore:`). Item 58 (the D004 Phase-2 file move) stays open.
