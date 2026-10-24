# D041 — `/resume` marks workflow runs, and lists only sessions a human touched

- **Status:** accepted — first layer by the orchestrator (brief + amendments A1–A5),
  second layer **APPROVED at `afa52d8`** (no blockers; two design judgements both
  cleared: the `#[cfg(test)]` wrapper and the second `messages()` scan per row)
- **Date:** this session
- **Relates to:** [`main.rs`](../../crates/wcode-cli/src/main.rs) (`session_items`,
  `session_items_in`, `session_label_of`, `is_real_user_message`, `first_user_line_of`),
  [`session_groups.rs`](../../crates/wcode-cli/src/session_groups.rs) (`RunRecord`,
  `load_run`), [`app.rs`](../../crates/wcode-tui/src/app.rs) (`SessionItem`,
  `open_session_picker`), [`W035`](../work/W035-resume-rows.md)
- **Extends:** [`D035`](D035-session-titles.md) — the picker row is still a
  REPLACEMENT (`<title> · <age>`), not a prefix; D041 adds two optional suffixes and a
  listing rule, and does not touch the replacement contract
- **Distinct from:** [`D037`](D037-toml-only-team-membership.md) — D037 decides who a
  team **is**; D041 only reads the artifact that says a run **was** a workflow

## Context

The `/resume` picker's rows were `<title> · <age>` or `<id> · <age> · <first user
line>` (D035), plus a ` · team` suffix for a group that carries members. Two gaps:

1. **A headless workflow run is indistinguishable from an ordinary session.** Every
   fresh root is a session group — "Every fresh root is a session group (Q(G))"
   (`main.rs:1347`) — so the directory shape says nothing, and the transcript carries
   no mode at all: `SessionEntry::Header` is `{ version, id, cwd, created }`
   (`session.rs:16`), and `SessionInfo` carries model/effort/window/state
   (`protocol.rs:175`).
2. **A brand-new session listed itself.** `load_session` (`main.rs:2307`) creates this
   run's group dir before `dispatch` builds the picker list (`main.rs:2007`), so the
   picker offered the session you were sitting in as `<id> · 0s` — no title, no first
   line, nothing to recognise.

## Decision

1. **A workflow is read from `run.json`, not from the transcript.** `RunRecord` is
   written ONCE by `run_workflow` after the empty-plan guard (`main.rs:2658`), only
   when a group dir exists — so its presence means exactly "launched headless with
   `--task`". `session_groups::load_run` (`session_groups.rs:491`) loses the
   `#[allow(dead_code)]` its docstring had been reserving for this consumer.
2. **The row is `<label>[ · team][ · workflow]`** — team first, workflow second, in the
   one `GroupEntry::Group` arm that already appends ` · team`. Because the picker
   fuzz-matches the label, the suffixes are also filter keys (`/resume workflow`).
3. **A row is listed iff its transcript opens and holds a REAL user message.**
   `is_real_user_message` (`User`, and not `COMPACTION_SUMMARY_PREFIX`) is shared by the
   filter and by `first_user_line_of`, so the two readers cannot disagree about "did a
   human touch this session".
4. **An unopenable transcript is still listed**, degraded to `<id> · <age>` — the
   previous behaviour. Nothing is hidden because parsing failed.

## Alternatives rejected

- **A mode field on `SessionEntry::Header` (or a `Run` entry).** Labels only sessions
  created after it lands, adds a forward-compat surface to the transcript, duplicates a
  fact `run.json` already holds, and teaches the kernel a CLI concept — `run.json` is a
  sidecar precisely so the kernel never learns about `--task`.
- **`plan.ndjson` (does this group have nodes?).** Every group is handed a journal path
  (`main.rs:1487` `TaskList::with_journal_at(path, cap)`), so its presence proves "a
  `task` op happened", not "a workflow launched". Reading it per row would also mean a
  replay-only parse inside a list loop.
- **Group-dir vs flat file.** Dead as a signal since every fresh root became a group;
  `GroupEntry::Legacy` now only marks pre-item-16 files.
- **Filtering inside the TUI.** The TUI owns no disk access and receives
  `SessionItem { label, path }`; the composition root is the only place with the truth.
- **Hiding unopenable transcripts.** An over-broad filter is this change's rated risk:
  an unopenable file is exactly what a human may need to resume by hand.
- **Counting a whitespace-only first message as blank.** It would hide a session the
  human typed into; that row simply has no first line, today's shape.

## Consequences and accepted edges

- **A workflow whose `run.json` write failed is unmarked.** `write_run` is best-effort
  by design — `main.rs:2665` prints `warning: run record not written` and the run
  continues, because the printed `resume:` line is a convenience, not state. Owned: the
  rejected alternative (a transcript marker) trades a rare unmarked row for a permanent
  wire-compat surface.
- **The list is a startup snapshot** (`main.rs:2007`) — a session that gains its first
  user message mid-run stays absent until the next `/resume` re-exec. Consistent with
  the picker being injected rather than polled.
- **`Session::messages()` is called twice per row** (filter + label). Both are in-memory
  scans of an already-parsed transcript; the one-file-open-per-row property is unchanged
  and the picker lists tens of rows.
- **No TUI change.** An empty list already reads `no sessions to resume`
  (`app.rs:5346`), which is now the honest answer for a fresh session.

## Verification

- One commit `afa52d8` — `crates/wcode-cli/src/main.rs` (+202/−30),
  `crates/wcode-cli/src/session_groups.rs` (+2/−5). Eight new tests, each pinning a
  distinct behaviour, including the three negatives (unopenable listed, user-without-
  reply listed, whitespace-only listed) and the ordering case.
- `cargo test --workspace` → **1497 passed, 4 ignored**; `cargo clippy --workspace
  --all-targets` → clean. Mutation spot-checks: deleting the blank drop, the ` · workflow`
  push, or the unopenable exemption each fails its test.
- **Not verified:** the picker's rendering — no live pty/TUI driver exists (tracker row
  97). The row BUILDER is exercised end-to-end against real temp session dirs.
