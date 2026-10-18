# W023 — TUI: `/help` renders in `F1`'s two-column style

- **Status:** ☑ **landed** — `7018a70`; first-layer APPROVED (7 amendments), second-layer APPROVED; zero `SKETCH` markers; workspace tests + clippy clean.
- **Follow-up to:** W022 (info modals). Tracker row 53.
- **Decision:** `docs/decisions/D027-tui-help-info-rows.md`; amends `docs/tui-design.md` §4 Information modals + §4 Keys.

## The ask
After W022 shipped, the user ran the TUI and reported `/help` looked worse than `F1`: same frame, but the content was styled differently. They preferred F1's popup and asked for `/help` to match ("make sure the pop up looking good").

## Why they differed (recon)
- `F1` (`draw_help`, `ui.rs:242`) builds every row as two styled spans — `accent` label padded to 22 cols + `dim` detail.
- `/help` (`draw_info`, `ui.rs:280`) rendered the plain `help_text()` string (`app.rs:951`) as **unstyled** lines, with the commands emitted as one long clipped `commands: …` line.

## The change
`/help` now renders as F1's two-column table:
- `Overlay::Info` carries `rows: Vec<InfoRow>` instead of a `String`; `InfoRow::Item { label, detail }` (an `accent` 22-col label + `dim` detail — F1's row byte-for-byte) or `InfoRow::Prose(String)` (a body-weight line).
- The one row builder is `ui::item_line` with `const ITEM_LABEL_COLS = 22` (beside `CONTENT_COL`), so `/help` and `F1` can never re-diverge; `draw_help` is now a thin caller (F1 bytes unchanged — `f1_renders_the_help_overlay_with_the_keymap` and `the_overlays_use_rounded_borders` pass unmodified).
- `/help` = a `commands` section (one `Item` per `COMMANDS` entry: label `/name` + args, e.g. `/new [clear]`; detail the `summary`) then a `keys` section (one `Item` per `KEYS` entry, styled as F1). Same panel geometry, so ` help ` and ` keys ` are the same size.
- `/verify`, `/tasks`, `/team roster`, `/usage` stay prose rows (`open_info` wraps text into `Prose`); `info_scroll`/`info_lines` count `rows.len()`.
- Four `Command::summary` strings trimmed to fit the 39-col detail budget (`/plan`, `/verify`, `/width`, `/team`).

## Gates
- `cargo test --workspace` → all green (+6 new tests); `cargo clippy --workspace --all-targets` → clean; zero `SKETCH` markers.
- Headless `/help` vs `F1` render confirmed identical in style (throwaway example, since removed).

## Non-blocking residuals (reviewer's second layer)
- **N2** `command_label` joins multiple aliases with `|` (prose said space-joined) — unobservable today (every aliased command has one alias).
- **N3** `info_body`'s `Item` flatten arm is unexercised (no caller opens `/help` through it; the `/help` test uses `info_rows`).
- **N4** `the_help_section_labels_are_bare_prose` probes the rendered `fg` (`commands` vs `/new`), which holds for the default colored palette but would fail under `NO_COLOR`/a `plain` preset — same class as the pre-existing `markdown` theme race.
- **N5** `f1_stays_byte_identical_after_the_item_row_extraction` checks the first row + no-hint + footer, not a full-frame golden (F1 is otherwise covered by its dedicated tests).
- Live TUI session not run (no reachable keyless endpoint); the `/help`↔`F1` style equality is proven by a buffer-level test.

## Commits
- `7018a70 tui: /help renders in the F1 two-column style`
- docs: this item + `D027` + the spec amendment + tracker row 53.
