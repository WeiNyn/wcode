# D026 — Informational `/`-commands open a scrollable modal

- **Status:** accepted (human sign-off on the W022 brief, "go" — this session)
- **Date:** this session
- **Relates to:** D018 (command chatter is transient — the line this amends),
  D023 (prose only, fully bare — the modal keeps the bare frame, no new glyph),
  the `F1` keymap modal (`Overlay::Help`)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) — the status line; §4's
  **Notices** bullet (five commands leave the transient-chatter list), a new
  **Information modals** bullet, and one clause in §4 **Keys**
- **Work item:** [`work/W022-tui-info-overlay.md`](../work/W022-tui-info-overlay.md)

## Context

D018 made every `/`-command reply a **transient** `Notice` row, retired at the
next submit. That is right for chatter — an echo, a usage hint, an error — but
wrong for a **report**: `/help` (~30 `KEYS` lines), `/verify` (unbounded), and
`/tasks`, `/team roster`, `/usage` are prose you *read*, printed into a
scrollback that clamps and then scrolls away. The `F1` keymap already solves the
same shape of problem — a centered modal over the frame — so the report
commands become modals rather than inventing a second reading surface.

## Decision

- **Which commands.** Exactly five open the modal: `/help`, `/verify`, `/usage`,
  `/tasks`, `/team roster`. They push **no** reply row; the echoed `/<command>`
  line still prints (it is pushed generically in `command()`,
  `crates/wcode-tui/src/app.rs:3539`) and retires at the next submit, so the
  record of what was asked survives.
- **Which stay lines.** Bare usage hints and errors (`/width` with no argument or
  out of range, an unknown command, …), the `/copy` reply, action confirmations
  (`theme: <name>`, `width: N cols`), and the submit-time
  `a turn is already running` guard remain transient (**W029 retired that
  guard** — a mid-run submit now queues and confirms with `queued · N waiting`).
  They are hints, not
  reports; a modal is the wrong weight, and model/effort already ride the header
  and the width the foot.
- **The modal.** A `Clear`ed, centered, rounded-border panel — §2's `overlay
  frame` glyph row (`╭ ╮ ╰ ╯ ─ │`), **no new glyph**. Title is the lowercase
  command name, matching `" keys "`: ` help `, ` usage `, ` verify `, ` tasks `,
  ` team `. Body is that command's report text, scrolled by `↑`/`↓` (one line),
  `PgUp`/`PgDn` (a page), and the wheel (three lines), with a trailing dim
  `— N more —` hint while clipped.
- **Dismissal.** `Esc` only. While open, every other key is swallowed — the same
  rationale as `F1` (`app.rs:4222` `fn on_overlay_key`): a modal that leaks a
  stray character into the composer is a bug, not a courtesy. `Esc` here closes
  the modal and does **not** cancel or quit.
- **`F1` is unchanged.** `Overlay::Help` keeps its content (`KEYS`) and its
  `Esc`/`F1` dismissal, byte-identical; giving it scroll is a follow-up, not
  this decision.

## Mechanism

- `Overlay::Info { title, text, scroll }` beside `Pick`/`Help`
  (`app.rs:432-436`), with `App::open_info` mirroring `open_help`
  (`app.rs:2517`), an `Info` arm in `on_overlay_key` (`app.rs:4222`) that clamps
  `scroll` against the drawn height (a new `terminal_height` beside
  `terminal_width`, `app.rs:2244`), and a `draw_info` arm in `draw_overlay`
  (`crates/wcode-tui/src/ui.rs:226`), refactoring the panel out of `draw_help`
  (`ui.rs:235`) so `F1`'s bytes do not move.
- `/usage`'s reply arrives out of frame from `AgentEvent::History`
  (`app.rs:1771`) via `render_usage` (`app.rs:1905`), so it needs a
  `usage_pending` flag set by the command arm: a reply the user did not ask for
  never opens a modal, and `replay_history`'s startup `GetHistory` cannot
  misfire one.

## Consequences

- The report commands read at full frame size, scroll when they overflow, and
  leave the scrollback clean; the transcript keeps what *happened*, and the
  scrollback no longer accumulates multi-line reports.
- `↑ N` on the composer foot keeps meaning transcript scroll — the modal owns
  `↑`/`↓`/`PgUp`/`PgDn` only while it is open.
- **Foreclosed:** a modal for the bare hints; a `q` dismiss key; a second
  reading surface for reports; a scrollable `F1` in this change.
- Gates: W022 §6/§7 — `an_info_command_opens_a_popup`,
  `esc_closes_the_info_modal_and_nothing_else_does`,
  `the_info_modal_scrolls_and_clamps`, `a_usage_reply_opens_the_usage_modal`,
  the F1-help and transient-twin regressions, plus `cargo test --workspace`,
  `cargo clippy --workspace --all-targets`, and a live TUI session.
