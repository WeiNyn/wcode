# W017 — docs-refresh-0.6.0 — brief

- **Status:** **SHIPPED** — approved; implemented across 10 commits (`a4720bd`…`fbe5994`); second-layer review APPROVED every slice. See §10 (Log).
- **Work item:** W017 (slug `docs-refresh-0.6.0`)
- **Tracker:** [`docs/next-steps.md`](../next-steps.md) item 63
- **Decisions:** [D022](../decisions/D022-eink-landing-page-theme.md) (the e-ink
  landing-page theme); D007 (content-addressed editing) is the anchor the
  hash-language fixes defer to
- **Author:** agent:brainstormer (worker, read-only)
- **Recon:** agent:recon-readme, agent:recon-pages, agent:recon-meta
- **Recorded by:** orchestrator
- **Date:** 2026-10-11

## 1. The ask

> "recently we update many things in wcode but document, description and github
> pages, information not updated, let's scan and brainstorm updates needed"

A **documentation-only refresh**: bring the public surfaces — `README.md`,
`AGENTS.md`, the GitHub-Pages landing page (`index.html`/`style.css`) — plus the
secondary descriptions (the VS Code extension's `package.json`/`README`, the
Homebrew `Formula`) back in line with the shipped 0.6.0 code. The scope is
reconciliation, not rewriting: fix wrong facts, add the surfaces that shipped
since the last sweep, and refresh metadata/versions. **No behavior changes and
no code changes.**

## 2. Scope / Non-scope

**In scope.**

- **README.md / AGENTS.md** — fix false facts (a `replace` tool that does not
  exist; "Absent by design: … subagents"; a stale line count; a wrong
  default-theme label; a mislabeled `wcode` line) and add missing coverage
  (VS Code extension, named project teams, `member`, in-repo `.wcode/*.toml`).
- **index.html / style.css** — fix the retired hash-address language, the tool
  count/`replace` row, the `○ queued` mockup state, the `peers` description;
  add head metadata + sections for shipped surfaces.
- **Secondary descriptions** — `editors/vscode/package.json` + `README.md`,
  `docs/vscode-extension-plan.md` status line, `editors/vscode/package-lock.json`
  version, `crates/wcode-tui/Cargo.toml` category nit.
- **Homebrew Formula** — reconcile `Formula/wcode.rb` to v0.6.0 **if it is
  actually stale** (on `origin/main` it is not; see §4/§5 — flag, do not blindly
  re-run).

**Non-scope.**

- **Any code / behavior change.** This brief is docs + metadata; every "the code
  is wrong" item below is a *pointer to a separate code change*, not this one.
- **Moving `docs/*.md` into `docs/{plans,designs,analysis}/`** — tracked item 58
  with its own link sweep; touching it here would balloon the diff
  (`≈144 references across ≈56 files`, `docs/next-steps.md:67`).
- **Adding a GitHub-Pages Actions workflow** — the page is served from `main`
  (repo root) with `.nojekyll` by design (`docs/next-steps.md:54`); introducing
  an Actions build is a new deployment mechanism, not a doc refresh.
- **Rewriting the "Philosophy" section wholesale** — the human signs off on how
  far to go (§3, judgment call). MCP and permission prompts remain genuinely
  absent; only the "subagents" clause is false.
- **The VS Code extension's own source behavior** (dead commands,
  activationEvents) — `editors/vscode/src/*.ts` changes are code; only the
  `.md`/`package.json` *descriptions* are in scope. (The duplicated `src/teams.ts`
  layout row is doc-only and thus in scope.)
- **Regenerating the Formula now** — the release job already did on
  `origin/main`; re-running the script by hand risks a diff from nothing.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| WS1 | README/AGENTS **correctness** (false `replace` tool, "Absent by design … subagents", line count, theme label, REPL→TUI) | medium | low | low | Wrong facts on the front door mislead every reader; worst case a wrong sentence. |
| WS2 | **index.html correctness** (retire hash-anchor language, 19→18 tools, drop `replace`, `○ queued`, `peers`) | medium | low | low | The page contradicts the shipped D007 content-addressed editing — a visitor-visible error. |
| WS3 | README/AGENTS **additions** (VS Code extension, named teams, `member`, `.wcode/*.toml`, kernel method/file lists) | medium | medium | low | Closes whole-surface gaps; medium because the additions must be worded to match `--dump-config` reality, not a guess. |
| WS4 | index.html **additions** (head `og:`/`twitter:`/`canonical`, VS Code + headless workflow + socket sections) | low | low | low | Metadata + prose; no behavior, and the human decides whether head metadata is wanted. |
| WS5 | **secondary metadata** (VS Code `package.json`/`README`/plan, lockfile, tui category) | low | low | low | Polish on a second client; a wrong description is the worst case. |

## 4. Interface & structure

**Added.** None (documentation-only; no new files).

**Changed** (docs + metadata; every anchor re-verified, quoted — the number
locates, the snippet pins):

- `README.md:5` — `edit/edits/replace, write, and ast-grep search/rewrite` — drop
  `replace` from the tool list; align with the real set (`tools/mod.rs:39-74`
  registers `edit`/`edits`/`write`, no `Replace`; the only "replace" is edit's
  `replace_all` arg, `README.md:419`).
- `README.md:525-527` — `about 10k lines and the whole workspace about 52k …
  Absent by design: MCP, subagents, permission prompts` — measured `≈11k`
  (harness) and `≈58k` src / `≈64k` incl. tests; and drop `subagents` (a full
  team/agent system exists: `agents.rs` spawn/member/message, `config.rs:193
  TeamMember`, `:235 Workflow`).
- `README.md:118` — `name = "gruvbox-dark"      # optional; a catalog preset
  (default: palette B)` — the default is the `default` catalog preset
  (`theme.rs:378 ("default", DEFAULT)`); "palette B" is an internal name the
  reader cannot resolve.
- `README.md:310` — `wcode                        # REPL` — bare `wcode` is the
  **TUI** on a TTY (`README.md:324 --tui # default on a TTY`); only `--no-tui`
  is the line REPL.
- `README.md:361` — `[--no-session]` (rebuild + re-exec into the current
  session …` — the command name `/reload` is missing from the sentence (the TUI
  table row is `/reload [--no-session]`, `app.rs:3720 usage: /reload
  [--no-session]`).
- `README.md:370` — `prints the roster.` — preceded by `/team`
  (`README.md:369`); bare `/team` opens the **picker**, `/team roster` prints the
  listing (`app.rs:461`).
- `README.md` tool table (`:411-427`) — add a `member` row (real tool:
  `member.rs:164 fn name "member"`), next to `spawn`/`message`/`peers`.
- `README.md` (~`:256-300`, teams) — document `--team <name>` +
  `./.wcode/teams/*.toml` (undocumented; verified `README.md` contains no
  `--team` and no `.wcode/teams`, while `.wcode/teams/{recon,pair,full-stack}.toml`
  ship).
- `README.md:326` `wcode serve --stdio …` — name its consumer (the VS Code
  extension; `editors/vscode/README.md:1 # wcode — VS Code extension`).
- `AGENTS.md:42` — `  edit, edits, replace, write.` — same `replace` fix.
- `AGENTS.md:29` — `` `wcode-protocol` client: `run(surfaces, options,
  new_surfaces)`, a pure reducer `` — the TUI `run` takes a 4th arg
  (`crates/wcode-tui/src/lib.rs:358`); add `new_tasks`.
- `AGENTS.md:12-22` — the kernel method/file lists understate reality (missing
  `set_plan_mode`/`notify`/`side_ask`, `before_inbound`/`after_run`,
  `actor.rs`/`protocol.rs`/`stats.rs`) — reconcile to the crate.
- `index.html:48` — `<div><b>19</b><span>tools</span></div>` — real count is
  **18** (drop `replace`).
- `index.html:326` — `<h2>19 tools, one small kernel</h2>` — → 18.
- `index.html:332-336` — the tool table: `read` row says `file contents as
  <code>ANCHOR│line</code> — the 5-char anchor is a line's address`, plus a
  `replace` row and `grep … hits carry anchors`. Retire the entire anchor scheme
  per D007 — outputs are `<line number>\t<content>` with literal-text
  `old_string` matching (`README.md:441-452`); drop the `replace` row.
- `index.html:288` — `5-char anchor is its address, so old code is never
  re-typed.` — same D007 retirement.
- `index.html:255` — `⚙ message(reviewer) → "#3 review the diff"          ○
  queued` — there is no `Queued` state (states are Idle/Running/Done/Failed,
  `app.rs:1568-1577`); reword or drop the glyph.
- `index.html:150` — `<code>peers</code> lists the roster with live state.` —
  `peers` lists **messageable peers** (`README.md:426`); "the roster" reads as
  the team.
- `index.html:270` — `a 7-theme catalog` — the static catalog is 7 presets but
  `omarchy` is a live 8th source (`theme.rs:378-384` + `README.md:120`).
  Judgment call (§5).
- `index.html:3-13` (`<head>`) — only `title`/`description`; add
  `og:`/`twitter:`/`canonical` (and `og:image` if one exists) — **judgment call**.
- `editors/vscode/package.json:4` — `"description": "wcode team surface: chat
  panel, native diffs, and a live member sidebar"` — the sidebar is now a team
  caption (`editors/vscode/README.md` reducer row).
- `editors/vscode/package-lock.json:3` — `"version": "0.0.6"` vs
  `package.json:5 "version": "0.0.7"`.
- `editors/vscode/README.md` — the layout table lists `src/teams.ts` twice and
  omits `src/highlight.ts`.
- `docs/vscode-extension-plan.md:3` — `**Status:** design locked; P0 not
  started.` — shipped (W010 / tracker items 53-56).
- `crates/wcode-tui/Cargo.toml:11` — `categories = ["command-line-interface",
  "development-tools"]` vs cli `Cargo.toml:12 ["command-line-utilities", …]` —
  align the category spelling.

**Integration points** (the facts this brief relies on; re-verify before coding —
a line moves if the line above it changes):

- `crates/wcode-cli/src/tools/mod.rs:39-74` — the registered tool set: `read,
  webfetch, bash, bg, edit, edits, write, todo, session_search` (always) +
  `grep, find, ast_search, ast_edit` (conditional). No `replace`.
- `crates/wcode-cli/src/tools/*.rs` — the 18 tool names via `fn name`: `ast_edit,
  ast_search, bg, bash, edit, edits, find, grep, member, message, peers, read,
  session_search, spawn, task, todo, webfetch, write`.
- `crates/wcode-cli/src/tools/member.rs:164` — `fn name(&self) -> &str { "member" }`.
- `crates/wcode-cli/src/agents.rs:548-575` — the root's A2A tools — spawn,
  message, peers, member, task — the surface "Absent by design" denies.
- `crates/wcode-cli/src/config.rs:193,235,246` — `TeamMember`, `Workflow`,
  `WorkflowNode`.
- `crates/wcode-tui/src/theme.rs:378-384` — the 7-preset catalog; `README.md:120`
  adds `omarchy`.
- `crates/wcode-tui/src/app.rs:461,1568-1577` — `/team` picker + the four team
  states.
- `editors/vscode/README.md:1`, `package.json:2-5` — a real second client at
  v0.0.7.
- `.github/workflows/release.yml:99,112` — `gen-homebrew-formula.sh
  "${GITHUB_REF_NAME}" Formula/wcode.rb` → `git commit -m "chore(homebrew):
  wcode ${GITHUB_REF_NAME} [skip ci]"` — **the Formula is regenerated by CI on
  tag.**

## 5. Plan

1. **WS1 — README/AGENTS correctness.** Deliverable: `README.md` + `AGENTS.md`
   fixed for the `replace` tool, "Absent by design … subagents", line count,
   theme default, REPL→TUI, the `/reload` name, the `/team` sentence, and
   `run()`'s 4th arg. Gate: `grep -n 'replace' README.md AGENTS.md` returns only
   `replace_all`/`replaced` prose; `grep -rn '"replace"' crates/wcode-cli/src/tools/`
   returns nothing.
2. **WS2 — index.html correctness.** Deliverable: `index.html` free of the
   retired anchor scheme, tool count at 18, `replace`/`○ queued`/`peers` fixed.
   Gate: `grep -in 'anchor\|5-char\|19 tools\|<b>19</b>\|○ queued' index.html` →
   no hits; tool-count grep matches 18.
3. **WS3 — README/AGENTS additions.** Deliverable: a VS Code extension
   paragraph, named-teams doc, a `member` table row, a `.wcode/*.toml` note, and
   reconciled kernel lists. Gate: each new claim re-checked against the cited
   code (`--dump-config`, `--help`, `--dump-system-prompt`).
4. **WS4 — index.html additions.** Deliverable: `<head>` metadata (if signed
   off) + sections for VS Code / headless `[workflow]` / socket peers. Gate: page
   opens locally, no console errors, `grep -i 'vscode\|stdio\|--task' index.html`
   hits.
5. **WS5 — secondary metadata.** Deliverable: VS Code `package.json`
   description, linted `README` layout table, `package-lock.json` version, the
   plan's status line, the tui category. Gate:
   `node -e "require('./editors/vscode/package-lock.json').version==='0.0.7'"`;
   `grep -c 'src/teams.ts' editors/vscode/README.md` is 1.
6. **Formula triage.** Deliverable: a recorded decision — `origin/main` already
   carries the CI-produced v0.6.0 formula (`8857667 chore(homebrew): wcode v0.6.0
   [skip ci]`), so **no manual edit**; the only gap is that the local checkout is
   1 commit behind. Gate: `git show origin/main:Formula/wcode.rb | grep 'version
   "'` → `0.6.0` (verified). **Cut the refresh branch from `origin/main`.**
7. **Consistency sweep + gates.** Deliverable: a clean `cargo test`/`clippy`
   (docs-only ⇒ trivially green) plus a final cross-file grep for stale strings
   (`replace` tool, `anchor`, `P0 not started`, `0.0.6`). Gate: the §6 command
   block, all green.

## 6. Quality gates

- `cargo test --workspace` → all pass (docs-only: a *no-regression* gate).
- `cargo clippy --workspace --all-targets` → clean.
- Docs-fact gates (the real ones here):
  - `grep -rn 'struct Replace\|"replace"' crates/` → only
    `crates/wcode-tui/src/ui.rs:1438,1616` (a known dead match-arm — out of
    scope, flagged), **never** a real tool.
  - `grep -in 'anchor\|5-char\|<b>19</b>\|19 tools\|○ queued\|P0 not started'
    README.md index.html docs/vscode-extension-plan.md` → no stale hits.
  - `git show origin/main:Formula/wcode.rb | grep -n 'version "'` → `version
    "0.6.0"`.
- Live check: `cargo run -p wcode-cli -- --dump-system-prompt` / `--dump-config`
  / `--list-themes` confirm the prompt/config/theme facts the docs now assert.

## 7. Testing

### 7.1 Automated

This change adds **no new tests** (docs-only). The automated gates are the two
`cargo` commands in §6 plus the grep gates, the mechanical form of "the doc no
longer states the false fact".

### 7.2 How a human verifies it

```sh
# 1. The tool set the docs now claim (no `replace`).
cargo run -p wcode-cli -- --dump-system-prompt

# 2. The landing page renders with the corrected tools/anchor language.
python3 -m http.server 8080   # open http://localhost:8080 — "18 tools", no "anchor"

# 3. The Formula (must show 0.6.0 at the branch base).
git show origin/main:Formula/wcode.rb | grep 'version "'

# 4. The default theme the README names.
cargo run -p wcode-cli -- --list-themes   # contains `default`, `omarchy`, …
```

Look for: 18 tool names and no `replace`; the page reads
`<line number>\t<content>` (or its prose), never "5-char anchor";
`version "0.6.0"`; `--list-themes` output matching the README's catalog claim.

## 8. Expected outcome

- `grep -e 'replace' README.md AGENTS.md index.html` matches only
  `replace_all`/prose — no tool by that name is claimed anywhere.
- `index.html` states **18** tools and describes literal-text editing, not the
  retired hash anchors.
- `README.md` and `AGENTS.md` name the VS Code extension,
  `--team`/`.wcode/teams/*.toml`, and the `member` tool; the "Absent by design"
  list no longer denies subagents.
- `--list-themes` output, the README's default-theme label, and the byte-exact
  editing section agree with the code.
- `editors/vscode`'s `package.json` description matches the shipped team-caption
  sidebar; `package-lock.json` says 0.0.7; `docs/vscode-extension-plan.md` no
  longer says "P0 not started".
- `Formula/wcode.rb` at the branch base is v0.6.0 (CI-produced; **no manual
  commit** in this item).
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean.

## 9. References

- `README.md`, `AGENTS.md` — the primary surfaces.
- `index.html`, `style.css`, `.nojekyll` — the Pages surface served from root.
- `docs/next-steps.md:54,58,62-65` — item 45 (Pages), item 58 (docs taxonomy,
  explicitly not this), items 53-56 (VS Code plan status).
- `docs/vscode-extension-plan.md` — the stale status line to fix.
- `editors/vscode/README.md`, `package.json` — the second client's descriptions.
- `crates/wcode-cli/src/{tools/mod.rs,tools/member.rs,config.rs,agents.rs}` — the
  shipped tool/team/workflow facts.
- `crates/wcode-tui/src/{theme.rs,app.rs}` — the theme catalog and `/team`
  states.
- `.github/workflows/release.yml:99-112` — the Formula regeneration.

---

## Facts NOT verified (named, per the skill)

1. **Every line number drifts** — snippet quoted, but numbers are as of
   `9dbfaf2`; the executor must re-grep.
2. **The Pages *live* rendering** — not opened; branch/root serving inferred
   from `.nojekyll` + root `index.html`.
3. **Whether `og:image` exists** to reference — assets not inventoried.
4. **The exact index.html additions content** (wording, head-meta values).
5. **The `editors/vscode` runtime truth** — the "caption vs sidebar" change is
   taken from the extension's own README note, not a live panel.
6. **`docs/vscode-extension-plan.md` completeness** — only its status line
   checked.
7. **`index.html:270` "7-theme" semantics** — whether `omarchy` is an 8th theme
   or a source is a naming call.
8. **Branch base** — RESOLVED: local is 1 behind `origin/main`; `origin/main`
   already fixes the Formula (verified by the orchestrator). Cut from
   `origin/main`.

## Judgment calls for the human to sign off on

- **"Philosophy" (README:526)** — how far to rewrite "Absent by design".
  Recommendation: *surgical* — drop `subagents` only; keep MCP/permission
  prompts/approval flows (still genuinely absent).
- **index.html head metadata** — add `og:`/`twitter:`/`canonical`/`og:image`, or
  leave the page metadata-minimal.
- **Formula** — record "non-issue" (release job already ran), versus a defensive
  manual edit.
- **index.html:270 "7-theme"** — leave as-is, or say "8 (incl. live `omarchy`)".
- **Automated grep gates** — a one-off reviewer command, or a committed
  `scripts/` snippet (arguably new scope).
- **Stale code pointer** `crates/wcode-tui/src/ui.rs:1438,1616` (a dead
  `"replace"` match arm) — flag as a follow-up code item, or ignore.

## 10. Log (append-only)

- **Brief approved** (human sign-off — this session). The five judgment calls
  settled as recommended: Philosophy = **surgical** (drop `subagents` only); add
  `<head>` `og:`/`twitter:`/`canonical`; leave the "7-theme" wording; grep gates
  stay one-off commands; the dead `ui.rs` `"replace"` arm is a follow-up code
  item. **Plus one addition:** an **e-ink theme** for the landing page, shape
  **A** (opt-in toggle, dark stays the default) — recorded as
  [D022](../decisions/D022-eink-landing-page-theme.md).
- **Shipped — 10 commits**, each slice second-layer reviewed (reviewer re-ran
  `cargo build` · `cargo test --workspace` 1187 passed/4 ignored ·
  `cargo clippy --workspace --all-targets` clean each time):
  1. `a4720bd` `docs:` README/AGENTS correctness (WS1)
  2. `20dd1c6` `pages:` retire the hash-anchor language; 19→18 tools (WS2)
  3. `301197a` `docs:` drop the stale `subagents` claim from AGENTS.md (cleared
     the reviewer's one blocker)
  4. `22ac3cc` `pages:` align the landing-page line counts with README
  5. `fac7846` `pages:` an e-ink theme for the landing page (WS6 → D022)
  6. `64c80ea` `docs:` document the 0.6.0 teams, `member` tool, VS Code surface (WS3)
  7. `6493e0d` `pages:` note the headless runs and the VS Code client (WS4)
  8. `372a100` `vscode:` refresh the extension descriptions for 0.6.0 (WS5)
  9. `3578860` `docs:` mark the VS Code extension plan as shipped (WS5)
  10. `fbe5994` `tui:` align the crate category with the cli crate (WS5)
- **Review verdicts:** Task 1 BLOCKED (AGENTS.md still denied `subagents`) →
  cleared; Task 2 APPROVE; Task 3a APPROVE; Task 3b/3c APPROVE. Reviewer confirmed
  each corrected fact against code and found zero `SKETCH` markers.
- **Formula — non-issue (resolved):** `Formula/wcode.rb` reads `0.5.0` only in a
  checkout one commit behind `origin/main`, which already carries the
  CI-generated `version "0.6.0"` (`8857667`). No manual edit; cut from
  `origin/main`.
- **Follow-ups (non-blocking, not in scope here):**
  - **code** — the dead `"replace"` match arm at `crates/wcode-tui/src/ui.rs:1438,1616`.
  - **code** — the extension's redundant `activationEvents` + the unreachable
    `wcode.focusPanel`/`wcode.showLog` commands.
  - **doc nits** — README:299 could say `--config`/`WCODE_CONFIG`; `index.html`'s
    `<meta name="color-scheme" content="dark">` now disagrees with the CSS
    `color-scheme: light` in e-ink; the toggle label could show its state.
