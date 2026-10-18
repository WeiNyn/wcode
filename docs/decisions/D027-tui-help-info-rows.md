# D027 — The ` help ` modal is a two-column table in `F1`'s row style

- **Status:** accepted (human sign-off on the W022 follow-up — this session)
- **Date:** this session
- **Relates to:** D026 (the information modal this restyles), D023 (fully bare —
  the item row is an `accent` label plus a `dim` detail: no new glyph, no new
  hue), the `F1` keymap (`Overlay::Help`)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) — the status line, §4's
  **Information modals** bullet (a new **Info modal body rows** bullet), §4
  **Keys** (the `KEYS` table is printed "in the same two-column style")
- **Work item:** [`work/W022-tui-info-overlay.md`](../work/W022-tui-info-overlay.md)

## Context

The user ran the TUI: `/help` shares `F1`'s frame but not its *content* styling.
`F1` (`crates/wcode-tui/src/ui.rs:242` `fn draw_help`) builds every row as two
styled spans — `Span::styled(format!(" {chord:<22}"), accent())` then
`Span::styled(what, dim())` — the tidy two-column look. `/help` goes through
`draw_info` (`ui.rs:280`), which renders the plain `help_text()` string
(`app.rs:951`) as **unstyled** lines, and `help_text()` emits the whole command
list as one long clipped line (`commands: /new /exit /model <id> …`) followed by
keys as `chord — desc`. Same panel, two different visual grammars — the worse one
on the more important command.

## Decision

- **The body is a list of rows**, each one of two kinds:
  - an **item** row — `F1`'s row byte-for-byte: a one-space indent, an `accent`
    label padded to a 22-col column, then a `dim` detail;
  - a **prose** row — one body-weight, unwrapped span.
- **`/help`** is the `F1` keymap with a commands table above it: a `commands`
  section, one item per `COMMANDS` entry (label `/name` + its `args`, or its
  aliases in brackets — `/new [clear]`; detail its `summary`), a blank row, a
  `keys` section, then one item per `KEYS` entry styled exactly as `F1` styles
  them. The old single-line `commands: …` is retired.
- **Section labels** (`commands`, `keys`) are prose rows — bare, lowercase, like
  every other head in the doc, no colon.
- **`/verify`, `/tasks`, `/team roster`, `/usage` are unchanged** — their text is
  not tabular, so it stays prose rows.
- **The panel geometry is shared with `F1`** (`area.width − 4` clamped to 64,
  centered, height from the row window), so ` help ` and ` keys ` are always the
  same size. One is never widened past the other.
- **`F1` is unchanged**: ` keys `, ` esc / F1 to close`, `more: false`.

## Mechanism

- `Overlay::Info`'s `text: String` becomes rows — `InfoRow::Item { label,
  detail }` / `InfoRow::Prose(String)` — beside `Pick`/`Help`
  (`app.rs:432-436`); `App::open_info` (`app.rs:2636`) takes rows; `help_text()`
  (`app.rs:951`) becomes `help_rows()`; `draw_info` (`ui.rs:280`) maps a row to
  `F1`'s two-span form; the command arms follow (`app.rs:3816` `/verify`,
  `:3833` `/team roster`, `:3836` `/tasks`, `:3838` `/help`).
- The scroll machinery is row-agnostic: one row is one line, so
  `info_lines()` (`app.rs:2692`), `info_visible_rows` (`app.rs:358`), and
  `info_scroll` (`app.rs:2663`, clamped against `terminal_height`) are unchanged
  — only what a row *is* changes.

## Consequences

- `/help` reads like `F1`: the command list becomes a scannable table instead of
  a clipped paragraph. The frame, title, scroll, `— N more —` hint, and
  `Esc`-only dismissal are all as D026 specified.
- **The command summaries are now a visible column, so length matters.** Panel
  width is `min(area.width − 4, 64)`; inner width is 62; the label field is
  `1 + 22 = 23` cols; the detail column is therefore **39 cols at the max
  width** (fewer on a narrow terminal). Rows do not wrap — they clip, exactly as
  `F1` clips. Four `COMMANDS` summaries overrun 39 today:

  | cmd | len | summary | trim |
  |-----|-----|---------|------|
  | `/team` | 63 | ``switch to a project team (picker), or `roster` to list the team`` | `switch to a project team` |
  | `/width` | 53 | `set the transcript width (40-200; also Alt-[ / Alt-])` | `set the transcript width` |
  | `/verify` | 44 | `check the plan's progress (are we finished?)` | `check the plan's progress` |
  | `/plan` | 40 | `toggle plan mode (explore, don't mutate)` | `toggle plan mode` |

  Trim them (or accept the clip); `Command::summary` is also the completion
  popup's one-line description (`app.rs:612`), so a trim is visible in both
  places — which is the point, since the shorter text is the better hint.
- **Foreclosed:** a wrapped detail column; a second panel width for ` help `; a
  `KEYS`-style table for `/verify`, `/tasks`, `/team roster`, `/usage` (their
  text is prose, not pairs); any change to `F1`'s bytes.
- Gates: `cargo test -p wcode-tui` (the `/help` assertion at `app.rs:7179`
  follows `help_text` → `help_rows`), the `F1` byte-identity regression
  (`f1_renders_the_help_overlay_with_the_keymap`), `the_overlays_use_rounded_borders`,
  `cargo clippy --workspace --all-targets`, and a live TUI session — `/help` and
  `F1` side by side, same panel size, same row rhythm.
