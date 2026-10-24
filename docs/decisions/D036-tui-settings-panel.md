# D036 — The settings panel edits SESSION state, and nothing persists

- **Status:** accepted (human sign-off 2026-10-24, session-scoped) and **shipped** — `8221ea9` … `b8cd0bc` + `d1b20d6` (code/docs), with the second-layer fixes in `8becfc2`; second layer **APPROVED**
- **Date:** this session
- **Relates to:**
  [`crates/wcode-tui/src/settings.rs`](../../crates/wcode-tui/src/settings.rs)
  (the taxonomy, `rows`, `on_key`, `SettingAction`),
  [`crates/wcode-tui/src/app.rs`](../../crates/wcode-tui/src/app.rs)
  (`App.settings`, `App.deferred_settings`, the precedence tier, `apply_setting`,
  `settings_snapshot`),
  [`crates/wcode-tui/src/ui.rs`](../../crates/wcode-tui/src/ui.rs)
  (`draw_settings`, the affordance column, the hint line),
  [`crates/wcode-harness/src/protocol.rs`](../../crates/wcode-harness/src/protocol.rs)
  (`Request::Settings`),
  [`crates/wcode-harness/src/event.rs`](../../crates/wcode-harness/src/event.rs)
  (`AgentEvent::Settings`),
  [`crates/wcode-harness/src/actor.rs`](../../crates/wcode-harness/src/actor.rs)
  (the idle and mid-run arms),
  [`crates/wcode-cli/src/main.rs`](../../crates/wcode-cli/src/main.rs)
  (`config_dump`'s new line, both `Options.config` injections)
- **Work item:** [`work/W032-tui-settings.md`](../work/W032-tui-settings.md)
- **Supersedes:** W032 v1's *read-only view* recommendation (withdrawn, see §1.1 of
  the work item)
- **Design:** [`tui-design.md`](../tui-design.md) §4 "Settings panel"

## Context

Seven of the TUI's knobs were chord-only or command-only (`Alt-[`, `Ctrl-G`,
`Ctrl-B`, `Ctrl-O`, `Ctrl-T`, `Alt-<digit>`, and `/effort`, `/plan`, `/theme`,
`/width`, `/view` behind words you had to know), so nothing listed what was in
effect nor let you change it in one gesture. Two on-screen values were provably
stale: `Status.effort` had no writer outside launch while the header printed
`model · effort`, and `Status.context_limit` was seeded once, so the gauge's
denominator ignored every later `/model` switch.

The TUI's shape constrained the design: `overlay` is a single slot with no stack
(every opener assigns unconditionally, every accept consumes it first), `Esc` is
pinned as the only close key for a modal, and there is no reusable single-line
text field (the composer's buffer is `App`-level and not a widget).

## The decision

A settings **panel**, hosted on `App.settings: Option<SettingsState>` — a second
App-level lifetime beside `overlay`, not an `Overlay` variant. That is what lets
`Enter` on the `model` row open the ordinary picker **on top of** the panel:
accepting it removes the picker, not the editor. Precedence is `overlay` →
`settings` → `completion` → `Mode::Browse` → composer, and the panel bypasses
`on_overlay_key` (which is keyed on `Overlay` and would swallow a non-variant's
keys through its `_ => {}`).

A closed five-kind row taxonomy (`Toggle`, `Cycle`, `Nudge`, `Pick`, `ReadOnly`)
over a pure builder, `rows(snapshot)`. Every act applies **immediately** through
a setter that already exists; there is no dirty or commit state, so `Esc` still
closes and is still the only close key. `Request::Settings` →
`AgentEvent::Settings { model, effort, plan, context_limit }` supplies the live
truth and settles the optimistic values: a change the harness accepted but has not
applied yet shows a dim `· pending` mark beside its value.

The read-only `config` section is the finished `--dump-config` string, injected by
the CLI, so the flag and the panel are one builder and cannot disagree. Over
`--socket` the heading reads `config (this client)` — a socket client can vouch
for its own config and nothing else.

## Consequences

- **No runtime `config.toml` writer — refused, not deferred.** The editable set
  and the config-file set barely overlap, and seven of the rows already have a
  runtime override path. Making a setting stick for the *next* launch is a
  different item: it would be this repository's first writer, it needs its own
  record, and over `--socket` it would write the client's config while the agent
  lives on the server. D016's split stands: config at launch, session override at
  runtime.
- **Costs, stated:** a fourth tier in `on_key`; `Tab`/`BackTab` no longer switch
  surfaces while the panel is open (`Esc` restores the cycle); the effort
  vocabulary is duplicated in `settings.rs` and `editors/vscode/src/tab.ts`; the
  panel shows `--dump-config`'s scope, not the nine config tables it does not
  represent.
- **The two staleness defects are fixed by reading, not by echoing.** `Surface`
  writes the `Settings` reply into `Status` (the header and the gauge follow it),
  and a mutation's `Ack` triggers ONE read rather than a guessed value. Mid-run the
  ack means "applied at the run boundary", so the intent — not the echo — is what
  the row shows, with `· pending` saying so.
- **`PROTOCOL_VERSION` stays 1.** Both wire members are additive: an older peer
  catches `Request::Settings` as `Unknown` and answers `Ack`, which the panel reads
  as "unavailable", never as a value; an older client drops `AgentEvent::Settings`
  the way it drops any unknown frame.
- **Forecloses:** the writer (above), editable-`config` rows, and a typed numeric
  or text field (a named follow-up — `/width <cols>` and `/effort <level>` remain
  the escape hatches). No confirmations, no approval flow, no per-row "apply?".

## What the review added to the decision

Two pieces of behaviour shipped that the record above does not derive, both
ratified by the second layer as *requirements*, not extras:

- **`restore_pending_intents`.** A `Settings` reply is written into `Status`
  before the settle block runs, so a *stale* mid-run reply would otherwise
  overwrite the row's echo with a value the user never asked for while the mark
  still said "pending". The unsettled intents are restored after each reply, so
  the row shows the **intent plus the mark** — which is what "the intended value
  is written optimistically" has to mean across a stale reply.
- **The mark is drawn only while a run is in flight** (`pending` is empty at
  idle). At idle the harness applies the mutation in the same `dispatch` that
  answers `Ack`, so a mark there would claim a deferral that never happened. The
  *intent* is still stored at idle — it is what triggers the one read — so state
  and display are deliberately different facts.

And one thing the review could not close, recorded so it is not overstated: the
VS Code `tsc --noEmit` + test-suite gate was run by the implementer, **not**
independently re-run by the reviewer (`editors/vscode/node_modules` is absent and
installing is a mutation for a read-only role). The TS diff is 18 additive
declaration lines with no consumer.
