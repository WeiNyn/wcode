# W035 — Resume rows: mark workflow runs, drop blank sessions

- **Status:** shipped — second layer **APPROVED at `afa52d8`** (no blockers; both design
  judgements cleared: the `#[cfg(test)]` wrapper and the second `messages()` scan per row)
- **Work item:** W035 · tracker row 103 · decision [D041](../decisions/D041-resume-workflow-mark.md)
- **Decisions:** extends [D035](../decisions/D035-session-titles.md) (does not amend its
  replacement rule; adds an optional suffix and a blank-row filter)
- **Author:** agent:brainstormer (orchestrator-recorded)
- **Date:** 2026-04-28

## 1. The ask

Two small, composable improvements to the `/resume` picker, both in the composition
root's row builder:

1. **Mark** a session that was launched as a headless workflow run, so it is
   distinguishable from an ordinary session (` · workflow`, alongside the existing
   ` · team`).
2. **Drop** sessions that have no user message yet (a freshly created, still-blank
   session), so the list is cleaner — including the current run's own freshly created
   group, which today shows up as a blank row.

The kernel and the TUI stay untouched: the TUI owns no disk access, and the kernel
stays presentation-free.

## 2. Scope / Non-scope

**In scope.**

- `crates/wcode-cli/src/main.rs::session_items()` — filter blank rows and append
  ` · workflow`.
- `crates/wcode-cli/src/main.rs::session_label` — refactor to accept an already-open
  `Option<&Session>` so the blank check and the label share ONE open per row.
- `crates/wcode-cli/src/session_groups.rs::load_run` — drop the now-unused
  `#[allow(dead_code)]` (it becomes a live reader).
- Tests in `crates/wcode-cli/src/main.rs` (tests module) pinning the new behaviours.
- Tracker row + README note.

**Non-scope.**

- The REPL `/sessions` inventory (`repl.rs::sessions_row` / `sessions_lines`) — stays
  filename-led and pasteable per D035; no marker, no filter.
- The VS Code client picker — it reads the transcript head itself; tracked separately
  (W031 follow-up).
- A transcript/kernel mode marker — `Header`/`SessionEntry`/`SessionInfo` get no new
  field; the marker stays a filesystem sidecar (`run.json`).
- `plan.ndjson` as a discriminator — it exists for any session that ran a `task` op.
- Group-dir vs flat-file as a discriminator — every fresh root is a group now.
- Auto-titling, compaction, or any kernel change.

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| W035a | Mark workflow runs in the picker | medium | low | low | Reuses `load_run`, an existing public reader whose doc already promises this consumer; one `is_some()` per row. |
| W035b | Drop blank sessions from the picker | medium | low | medium | The filter hides rows, so an over-broad predicate silently hides real data; mitigated by A4. |
| W035c | Refactor `session_label` to take an open session | low | low | low | Pure plumbing to keep "one open per row" true; no behaviour change on its own. |

## 4. Interface & structure

**Changed.**

- `crates/wcode-cli/src/main.rs::session_items()` (~:2420) — becomes a `filter_map`
  that opens the session once, drops the row when the transcript opened and
  `first_user_line_of(..).is_none()`, computes the label from that open session, and
  appends ` · team` then ` · workflow`.
- `crates/wcode-cli/src/main.rs::session_label(path)` (~:2455) — becomes a thin
  wrapper: `session_label(path)` → `session_label_of(path, row_session(path).as_ref())`,
  with the existing body moved into `session_label_of`, reading `session` instead of
  calling `row_session` itself. The "ONE open for both halves" invariant is preserved
  and extended to cover the blank check.
- `crates/wcode-cli/src/session_groups.rs::load_run` (~:491) — remove
  `#[allow(dead_code)]`.

**Integration points (verified by the brief's author).**

- `crates/wcode-cli/src/main.rs:2420` `fn session_items()` — the single builder for the
  picker rows.
- `crates/wcode-cli/src/main.rs:2455` `fn session_label(path)` and `:2482`
  `fn row_session(path)` — one open per row, deliberate.
- `crates/wcode-cli/src/main.rs:2518` `fn first_user_line_of(session)` — re-used as the
  blank predicate (skips the compaction summary, requires a non-empty first line).
- `crates/wcode-cli/src/session_groups.rs:472` `RunRecord` and `:491` `load_run` — the
  workflow predicate.
- `crates/wcode-cli/src/main.rs:2658` `write_run` after the empty-plan guard, only when a
  group dir exists — confirms `run.json` marks real workflow runs.
- `crates/wcode-cli/src/main.rs:1347` "Every fresh root is a session group" — confirms
  group dir ≠ workflow.
- `crates/wcode-cli/src/main.rs:2007` `sessions: session_items()` is called **after**
  `load_session()` (~:2307) creates this run's fresh group — so the current fresh
  session DOES appear today as a blank row.
- `crates/wcode-tui/src/app.rs:1229` `SessionItem { label, path }` and `:5346`
  `open_session_picker` — the label is the fuzz key, so a suffix is also a filter key.

## 5. Plan

1. **Refactor `session_label`.** Deliverable: `session_label_of(path, session)` holds the
   body; `session_label(path)` delegates via `row_session`. Gate: the existing
   `session_label_*` tests pass unchanged.
2. **Filter blank rows + add ` · workflow`.** Deliverable: `session_items()` is a
   `filter_map` with one `row_session` open per row, blank-drop per A4, suffix order
   ` · team` then ` · workflow`. Gate: `cargo test --workspace` green; the new tests pass.
3. **Add the test seam (A1).** `session_items()` → `session_items_in(&session_dir())`.
4. **Drop the dead-code allow.** Gate: `cargo clippy --workspace --all-targets` clean.
5. **Tests.** Deliverable: §7.1. Gate: each new test fails if its corresponding code is
   reverted.
6. **Tracker/docs.** `docs/next-steps.md` row 103, README's picker sentence, ADR D041.

## 6. Quality gates

- `cargo test --workspace` → all pass (baseline 1478 passed / 4 ignored).
- `cargo clippy --workspace --all-targets` → clean.
- Regression tests kept green: `session_label_prefers_the_title`,
  `session_label_titled_without_an_age_is_the_title_alone`,
  `session_label_falls_back_to_id_age_first_line`,
  `first_user_line_ignores_a_compaction_summary`,
  `session_label_reads_the_first_user_line_and_age`.
- Live check: a scratch `HOME` session dir — a fresh blank group is absent from the row
  list; a group with `run.json` carries ` · workflow`; an ordinary group carries neither.

## 7. Testing

### 7.1 Automated

- `session_label_of` equivalence with `session_label` for titled, untitled and blank
  transcripts (no drift from the refactor).
- A blank group (header only) is absent from `session_items_in`.
- A group with a user message but no assistant reply is still listed.
- An unopenable transcript file is still listed (A4 — no silent hiding).
- A whitespace-only user message is still listed (a human touched that session; the
  row simply has no first line, today's shape) — see A2.
- ` · workflow` present for a group with `run.json` (written via `write_run`).
- ` · workflow` absent for an ordinary group.
- Ordering: members + `run.json` ⇒ label ends ` · team · workflow`.

### 7.2 Human verification

```sh
cargo build --bin wcode
# fresh blank session absent from its own picker: open the TUI, /resume
# workflow marked: a headless `--task` run's row ends " · workflow"
# ordinary session: neither suffix
```

## 8. Expected outcome

- `/resume` no longer lists the session you just started (no blank rows).
- A headless workflow run is visually distinguishable (` · workflow`), composing with
  ` · team` as ` · team · workflow`.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` green.

## 9. Orchestrator amendments (locked at dispatch)

- **A1 — a test seam is required.** `session_items()` hardcodes `session_dir()`, so
  every new test needs a directory parameter: `fn session_items() ->
  Vec<wcode_tui::SessionItem> { session_items_in(&session_dir()) }`. The repo's
  `run_status_in` / `load_session` precedent. Without it the filter is untestable and
  the gate is unmeetable.
- **A2 — the predicate is `has any real user message`, not `has a first line`, and no
  `is_blank_session(Option<&Session>)` helper.** Extract the shared test —
  `fn is_real_user_message(message: &AgentMessage) -> bool` (`User` and not
  `COMPACTION_SUMMARY_PREFIX`) — and let BOTH readers use it: `first_user_line_of`
  (which additionally requires a non-empty first line) and the filter
  (`session.messages().iter().any(is_real_user_message)`). One source of truth, no
  `Option<&Session>` helper, and A4 stops being a lie. Consequence: a session whose
  only user text is whitespace KEEPS its row — hiding a session the human typed into is
  exactly the silent-data-loss risk W035b is rated on; the row just has no first line,
  today's shape.
- **A3 — suffix order is ` · team` then ` · workflow`**, appended after the label, in
  the same `if let GroupEntry::Group(group)` block that exists today.
- **A4 — an unopenable transcript stays listed** (today's degraded `<id> · <age>` row).
  Only an OPENED session with no user message is dropped. No silent hiding of data.
- **A5 — the REPL `/sessions` row is untouched** (it is an inventory, not a choice
  surface). If the README's `/resume` sentence lists the row shapes, extend that
  sentence only.

## 10. References

- `crates/wcode-cli/src/main.rs` — `session_items` (~:2420), `session_label` (~:2455),
  `row_session` (~:2482), `first_user_line_of` (~:2518).
- `crates/wcode-cli/src/session_groups.rs` — `RunRecord` (~:472), `load_run` (~:491),
  `write_run`, `scan_members`, `GroupEntry`.
- `crates/wcode-tui/src/app.rs` — `SessionItem` (~:1229), `open_session_picker` (~:5346).
- `docs/decisions/D035-session-titles.md` — the row contract this extends.
- `docs/work/W031-session-titles.md` §5.6 — the row-shape precedent.

## 11. Facts the brief's author did not verify

- The test suite and clippy were not run; anchors were verified by reading only.
- `run_workflow`'s `group` plumbing beyond the `write_run` call site — assumed `group` is
  `Some` for any resumable workflow run.
- Whether a VS Code spawn path calls `session_items` (grep showed one caller).
- The exact test names in §7.1 are proposals; the implementer may rename them, but each
  must pin the stated behaviour.
