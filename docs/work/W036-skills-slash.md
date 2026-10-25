# W036 — Skills slash commands in the TUI

**Status:** brief — awaiting human sign-off. No code written.

The TUI is the default surface on a TTY but exposes none of the skill entry
points the REPL already has. `docs/next-steps.md:116` records this work as
parked on a branch `w036-skills-slash` that **does not exist in this clone**
(`git branch -a`, `git reflog --all`) and had no written brief (`docs/work/`
stops at W035). This item supersedes that note; the branch, if it exists
anywhere, should not be merged without this review.

## 1. The ask

In the **TUI**:

- **(a)** `/skills` — a popup listing the discovered skills;
- **(b)** `/skill <name> [args]` — force-load a skill's body as a turn;
- **(c)** `/<skill name>` — a bare slash-name that resolves to a skill.

Parity with the REPL, including the `# Skill: …` framing and the `## Task`
appendix, with **no protocol or kernel change**: the skill list is a
per-process constant discovered by the composition root, injected at startup
exactly like `models`, `sessions`, `teams`, `tasks`. A `--socket` client gets
an empty list — the same degradation `teams`/`sessions` already accept.

## 2. Scope

**In scope**

- `SkillItem { name, description, path }` + `Options.skills: Vec<SkillItem>`,
  injected by `main.rs` (local only; empty over `--socket`).
- `App.skills` + `App::set_skills`, mirroring `set_models`/`set_sessions`/`set_teams`.
- `skills` and `skill` rows in the TUI `COMMANDS` table (so `/help` and inline
  completion stay single-source).
- `/skills` opens a `Picker` (`PickerKind::Skill`); selecting a row reads the
  body and submits it as a turn.
- `/skill <name> [args]` loads the body and submits the framed turn; an unknown
  name is a transient `no such skill: X`.
- Bare `/<skill name>` resolves to the `/skill` path when **no command matches**;
  otherwise `unknown command:` is preserved.
- `Action::LoadSkill { name, path, args }` so the pure reducer never does IO; the
  event loop reads the file and submits `skill_turn(...)`.
- A TUI-local `skill_turn` formatter (the TUI cannot depend on `wcode-cli`),
  pinned by tests in both crates on the same literal.
- Empty / remote guards; `/help` gains two rows.

**Non-scope**

- No protocol change: no `Request::ListSkills` / `AgentEvent::Skills`. The list is
  a startup constant; it changes only on `/reload`, which re-execs.
- No dynamic completion of skill names: `COMMANDS` is `&'static`. Follow-up note.
- No change to the REPL's bare-slash fallthrough: an unknown `/foo` stays prompt
  text there (`repl.rs:183`, `:1396`). The asymmetry is deliberate and documented
  below (D3).
- No remote body loading: over `--socket` the path is server-side, so `/skill`
  and bare names refuse with a notice.
- No skill authoring, marketplace, per-skill tool policy, or `SKILL.md` editing.
- No new picker item type: `Picker.items: Vec<String>` is reused with
  `name — description` row text and `values` carrying the name.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| W1 | TUI injection + state | high | low | low | Mechanical mirror of `models`/`sessions`/`teams`; no new seam. |
| W2 | Commands + picker + bare resolution + `Action::LoadSkill` | high | medium | medium | Touches the single command source and the reducer; must not shadow a command or swallow typos. |
| W3 | CLI injection (both arms + the demo) | medium | low | low | The local arm already has `rt` in scope; remote stays empty by doctrine. |
| W4 | Tests + docs | medium | low | low | Headless app tests cover the reducer; `/help` goldens need a count bump. |

## 4. Interface and integration points

**Added**

- `crates/wcode-tui/src/lib.rs` — `pub struct SkillItem { pub name: String, pub
  description: String, pub path: PathBuf }`; `Options.skills: Vec<SkillItem>`;
  the `Action::LoadSkill` arm in the event loop.
- `crates/wcode-tui/src/app.rs` — `App.skills`; `PickerKind::Skill`;
  `Action::LoadSkill { name, path, args }`; `open_skills_picker()`;
  `load_skill(name, args)`; `skill_turn(name, body, args)`.

**Changed**

- `crates/wcode-tui/src/app.rs` — `COMMANDS` gains `skills`/`skill`;
  `command()`'s fallthrough resolves a bare skill name; `run_command` gains two
  arms; `accept_picker` gains a `PickerKind::Skill` arm; `App::new` initialises
  `skills`.
- `crates/wcode-tui/src/lib.rs` — `run()` destructures `skills` and calls
  `app.set_skills`.
- `crates/wcode-cli/src/main.rs` — the remote arm adds `skills: Vec::new()`; the
  local arm maps `rt.skills` → `Vec<SkillItem>`.
- `crates/wcode-tui/examples/demo.rs` — `skills: Vec::new()`.

**Verified anchors**

- `crates/wcode-cli/src/skills.rs:26` — `pub struct Skill {` with
  `pub name: String,` / `pub description: String,` / `pub path: PathBuf,` — the
  exact shape `SkillItem` mirrors.
- `crates/wcode-cli/src/skills.rs:41` — `self.skills.iter().find(|s| s.name == name)`
  (exact-name, case-sensitive lookup); `:45` `is_empty()`.
- `crates/wcode-tui/src/lib.rs:284` — `pub struct Options`, "Everything the
  composition root injects"; siblings `pub models: Vec<String>,` (`:288`),
  `pub sessions: Vec<SessionItem>,` (`:291`), `pub teams: Vec<String>,` (`:297`),
  `pub remote: bool,` (`:315`).
- `crates/wcode-tui/src/lib.rs:381` — `let Options { status, models, sessions,
  tasks, teams, … } = options;` then `:409` `app.set_models(models);`.
- `crates/wcode-tui/src/app.rs:2553` — `models: Vec<String>,` "injected by the
  composition root, since the TUI holds no `LlmOpts` and cannot list models
  itself." `:5905` `pub fn set_models`; `:5703` `pub fn set_sessions`;
  `:5716` `pub fn set_teams`.
- `crates/wcode-tui/src/app.rs:752` — `const COMMANDS: &[Command]` (single source
  for dispatch, `/help`, inline completion); `:910` `fn command_named`.
- `crates/wcode-tui/src/app.rs:4480` — `fn command(&mut self, line: &str)`; its
  fallthrough `:4487` `None => self.transient(format!("unknown command: {name}")),`.
- `crates/wcode-tui/src/app.rs:488` — `pub enum Overlay { Pick(Picker), Help,
  Info { .. } }` — ONE overlay slot.
- `crates/wcode-tui/src/app.rs:620` — `pub struct Picker { kind, title, items,
  values, query, selected }`; `:637` `PickerKind`; `:5399`
  `if self.models.is_empty() { self.transient("no models available to pick"); … }`
  then `:5403` `self.overlay = Some(Overlay::Pick(Picker::new(...)));`;
  `:5618` `match picker.kind` — the selection dispatch site.
- `crates/wcode-cli/src/repl.rs:1551` — `fn skill_turn(name, body, args)`, whose body
  is `format!("# Skill: {name}\n\n{}", body.trim_end())` plus
  `out.push_str("\n\n## Task\n")`.
- `crates/wcode-cli/src/repl.rs:1381-1394` — the REPL's `/skill` dispatch:
  `skills.find(name)`, then `std::fs::read_to_string(&skill.path)`.
- `crates/wcode-cli/src/repl.rs:183` — `_ => None,`; `:1396`
  `None => run_turn(&backend, line, &in_flight, &last_todos).await,`.
- `crates/wcode-cli/src/main.rs:1386` — `Runtime { instructions, skills, hooks,
  orchestrator }`; the local TUI arm reaches `rt.orchestrator` at `:2008`, so
  `rt.skills` is live at the `Options` literal (`:2002`) with no new plumbing.
- `crates/wcode-cli/src/main.rs:1087` — the remote arm's `Options` with
  `sessions: Vec::new()`, `tasks: Vec::new()`, `teams: Vec::new()`, `remote: true`.
- `crates/wcode-cli/src/main.rs:1220` — "The server owns the system prompt and
  skills, so a remote client discovers none of its own."
- `crates/wcode-tui/examples/demo.rs:145` — the second `Options` literal.
- Precedent, deliberately unused: `crates/wcode-harness/src/protocol.rs:345`/`:352`
  (`ListSessions`/`ListModels` — transport requests answered by the server, `Ack`
  in-process); `crates/wcode-harness/src/actor.rs:507`; the server intercepts at
  `crates/wcode-protocol/src/server.rs:243`/`:258`.

## 5. Decisions

- **D1 — inject at startup, do not add a Request.** The list is constant for the
  life of the process; `/reload` re-execs. A `ListSkills` request would touch the
  kernel enum, the actor catch-all, the registry and the server for a value the
  client could have been handed once. Consequence, accepted: `--socket` shows
  none, like `teams`/`sessions`.
- **D2 — `/skills` is a `Picker` (`PickerKind::Skill`), not an `Info` modal.**
  Every row is actionable. `docs/tui-design.md:470-508` governs the two families;
  an `Info` modal is "a report you read", a picker is "a row you select".
- **D3 — the REPL's bare-slash fallthrough stays as it is.** Unknown `/x` is
  prompt text there and `unknown command:` in the TUI; the TUI must not adopt
  prompt-text semantics for a slash line (it would silently send typos to the
  model). The asymmetry is now a documented difference, not an accident.
- **D4 — the formatter is duplicated, and pinned by a literal test in both
  crates.** Hoisting it into `wcode-protocol` would put prompt shape in the
  transport; hoisting it into the harness would put a CLI-only feature in the
  kernel (`AGENTS.md`: "Keep the kernel free of presentation concerns"). Six
  lines duplicated with a shared expected string is the cheaper, more honest
  seam.
- **D5 — the reducer stays pure.** Reading `SKILL.md` happens in the event loop
  on `Action::LoadSkill`, so `App` remains IO-free and every command is testable
  headlessly.
- **D6 — command names win over skill names.** Bare resolution runs only after
  `command_named` misses, so a skill named `help` can never shadow `/help`.

## 6. Plan

1. **TUI model.** `SkillItem`, `Options.skills`, `App.skills`, `set_skills`,
   `App::new` init. Gate: `cargo build -p wcode-tui`.
2. **TUI reducer.** `COMMANDS` rows, `PickerKind::Skill`, `open_skills_picker`,
   `load_skill` guards (empty / remote / unknown), bare-name fallthrough,
   `accept_picker` arm pushing `Action::LoadSkill`. Gate: the headless app tests
   in §8.1 pass.
3. **TUI side effect.** `Action::LoadSkill`; the event-loop arm reads the body and
   submits `skill_turn(...)`, transient on a read error; the local formatter.
   Gate: the literal test in both crates.
4. **CLI injection.** Remote arm + local arm + `demo.rs`. Gate:
   `cargo build --workspace`.
5. **Docs.** This file; `docs/next-steps.md` row; README command table note.
   Gate: clippy clean.

## 7. Quality gates

- `cargo test --workspace`
- `cargo clippy --workspace --all-targets`
- Live check: `cargo build --bin wcode`, then in a directory holding
  `.wcode/skills/*/SKILL.md`, run against a keyless endpoint
  (`--base-url http://localhost:11434/v1`): `/skills` lists names + descriptions;
  `Enter` submits the framed body as a turn; `/skill <name> extra` appends
  `## Task`; `/<name>` behaves like `/skill`. Confirm `/help` lists both rows.
  Confirm `--socket` shows no skills and refuses `/skill` with a notice. Error
  path: `--base-url http://127.0.0.1:9/v1` with `WCODE_RETRY_MAX=2` still starts.
- Keep green: the existing command/picker tests, `examples/demo.rs`, the REPL's
  `skill_turn` tests.

## 8. Testing

### 8.1 Automated

- `skills_picker_opens_on_slash_skills` — two injected skills; `/skills` yields
  `Some(Overlay::Pick)` with `kind == Skill` and both names.
- `skills_picker_empty_reports_no_skills` — no overlay; transient
  `"no skills discovered"`.
- `skill_command_loads_and_submits` — `/skill pdf-tools extract page 2` pushes one
  `Action::LoadSkill { name, args: Some("extract page 2"), .. }`.
- `skill_command_unknown_name_is_transient` — no action; `no such skill: nope`.
- `bare_slash_resolves_to_skill` — `/pdf-tools` → `LoadSkill`.
- `bare_slash_unknown_stays_unknown` — `/nope` → `unknown command: /nope`.
- `skill_named_like_a_command_does_not_shadow_it` — a skill named `help` still
  opens the help modal.
- `remote_skill_load_refused` — `remote == true`; a notice, no action.
- `skill_turn_frames_body_and_appends_args` — pins the exact literal, in the TUI
  **and** the CLI, so the two copies cannot drift.

### 8.2 Human

Selecting a skill mid-run behaves like any other `/` submit. A description
containing a newline renders as one clipped row, never a second line.

## 8a. First-layer review — APPROVED with amendments

Verdict: **APPROVE**, no blockers. The sketch is a comment-only diff (244
insertions, 0 deletions; every added line `//`), so the tree still compiles. The
implementer must apply all ten amendments below while filling the blocks.

- **A1 — `SkillItem` lives in `app.rs`, not `lib.rs`.** Put it beside
  `SessionItem` (`app.rs:1274`) and `TaskItem` (`app.rs:1283`), deriving
  `Clone, Debug, PartialEq`, and add it to the re-export at `lib.rs:37-40`
  (`pub use crate::app::{…, SessionItem, Status, TaskItem, …}`). `main.rs`'s
  `skill_items` keeps naming `wcode_tui::SkillItem`, which resolves via the
  re-export. This removes the sketch's `use crate::SkillItem` friction.
- **A2 — `/skills` on a socket is honest.** `open_skills_picker` checks
  `self.remote` (`app.rs:2634`) FIRST and pushes `skills unavailable over a
  socket`; only then does the empty-list guard push `no skills discovered`.
  Otherwise an empty list over `--socket` reports a falsehood
  (`main.rs:1220`: the server owns the skills).
- **A3 — `fn transient` becomes `pub(crate) fn transient`** (`app.rs:4942`), the
  minimal seam for the loop's read-error path. Rejected alternative: routing it
  as `AppEvent::Agent(id, AgentEvent::Error { .. })` — that would set
  `TeamState::Failed` and mislabel an IO error as a run failure.
- **A4 — `skill_turn` stays in `app.rs` but is `pub(crate) fn`**, not `pub`: only
  the same crate's loop and tests call it, and each crate pins its own copy (D4).
- **A5 — the bare-name strip is REQUIRED, not redundant.** `command()` splits the
  raw line, so `name` at the fallthrough is still `/pdf-tools`; `command_named`
  strips internally (`app.rs:953`), which is why the miss leaves the slash in
  place. Keep `name.strip_prefix('/').unwrap_or(name)`.
- **A6 — `PickerKind::Skill` args are correct as sketched.** `accept_picker`
  (`app.rs:5741`) sets `selected` from `picker.selected_value()`, which returns
  the `values` entry, and `Picker::with_values` puts the bare name there.
- **A7 — the loop's `backend` is the focused surface's** (`lib.rs:629-630`
  `let idx = app.focus(); let (id, backend) = &backends[idx];`), so a skill turn
  lands on the focused surface.
- **A8 — a FOURTH `Options` literal exists.** `lib.rs:1115`
  (`run_with_no_surfaces_quits_without_panicking` inside the test module) must
  gain `skills: Vec::new()` alongside `main.rs:1087`, `main.rs:2011`,
  `examples/demo.rs:145`. The `/help` name list that must gain `"skills"` and
  `"skill"` is `help_lists_every_command_and_the_keymap_from_the_tables`
  (`app.rs:9057`, list at `:9095`); the count assertions (`:9087`, `:9141`) are
  derived and bump on their own.
- **A9 — `Picker::with_values` is private (`app.rs:689`)** and `open_skills_picker`
  is an `App` method in the same module, so the call compiles. It is not callable
  from outside `app.rs`.
- **A10 — descriptions are collapsed:** `\n` → space so one skill never occupies
  two rows; an em dash inside a description cannot corrupt the selection because
  `selected_value()` resolves via `values`, not the label.

Structural facts the implementer must not miss: `Action` derives
`Clone, Debug, PartialEq` (`app.rs:121`) and its new fields satisfy it; the loop's
`match action` (`lib.rs:633`) has no catch-all, so the `Action::LoadSkill` arm is
mandatory; `match picker.kind` in `accept_picker` (`app.rs:5755`) is exhaustive;
the `PickerKind` match in `settings.rs:875` has a `_` arm and needs no change.

## 9. Expected outcome

`/skills`, `/skill <name> [args]` and `/<skill name>` work in the TUI exactly as
in the REPL, including the framing. `/help` gains two rows. Inline completion
still offers only static commands (documented follow-up: dynamic completion of
skill names). A local TUI lists the discovered skills; a `--socket` client shows
none and refuses a load with a notice. `cargo test --workspace` and
`cargo clippy --workspace --all-targets` are clean.

## 11. Follow-up: slash completion now suggests skill names (same item, later)

The user noticed that typing `/om` did not surface `omarchy` in the inline
completion popup — skill names were not in the candidate set at all (the
`COMMANDS` table is `&'static`, skills are runtime-injected). Fixed by
generalising the completion candidate from `usize` (index into `COMMANDS`)
to a `CompletionCandidate` enum (`Command(usize)` | `Skill(usize)`):

- `recompute_completion` appends skills after commands, in discovery order,
  skipping any whose name collides with a command or alias (command wins —
  the same rule as dispatch).
- `accept_completion` and `completion_rows` handle both variants; skill rows
  show `args: Some("[args]")` and `summary` = the (collapsed) description.
- `CompletionRow.summary` changed from `&'static str` to `String` so a skill
  description (runtime-owned) fits; the one `/help` site already called
  `.to_string()`.
- `is_exact_command` gained a `skills` parameter so typing a full skill name
  + Enter submits instead of being rewritten.

Verified live: `/om` now lists `/omarchy [args]`, and `/omarchy` + Enter arms
the run with the compact `# Skill: omarchy` block. 561 tui tests still pass;
clippy clean. No new tests added — the existing completion tests run with an
empty skill list, so they are unchanged; a skill-aware completion assertion
would be the natural next test but is not required for the gate.

Driven in a real pty (`tmux new-session -x 110 -y 60`) with a throwaway project
holding `.wcode/skills/live-check/SKILL.md`, against a scripted
OpenAI-compatible endpoint on `127.0.0.1:11434` (a 6-line Python server that
dumps the request body and answers one completion).

| check | observed |
|---|---|
| `/skills` | the picker opens with `❯ live-check — A live-check skill for W036` and the two global skills below it; each description occupies ONE clipped row, never two |
| `/skill live-check` | the transcript shows `YOU   # Skill: live-check` (compact — no body) and the surface enters `⠹ running` |
| the wire | the captured POST body carries `# Skill: live-check`, then the whole `SKILL.md` (frontmatter included), then `## Task\nextract the title` — the exact `skill_turn` framing |
| bare `/live-check` | identical to `/skill live-check`; no `unknown command:` |
| mid-run `/skill` | `cannot load a skill while a run is in flight`, and NO second `# Skill:` block — one run armed, not two |

One incidental finding, pre-existing and NOT W036: a **0×0 pty** panics at
`crates/wcode-tui/src/ui.rs:597` (`attempt to subtract with overflow`,
`x: area.x + (area.width - width) / 2`). That is the open row 96 in
`docs/next-steps.md`, reproduced here before the pane was given a size — a
second live sighting of it, not a regression from this item.

Cleanup: the tmux server, the listener and the temp project were removed; the
run used `--no-session`, so nothing was written into the repository.
