# D047 — One help page, and the root gets a digit

- **Status:** accepted — the human asked for the shortcut gaps to be fixed; ruled by
  `agent:designer` (the F1 merge with its entry row, the retired toggle, the
  `Alt-0..9` row), implemented and gated.
- **Date:** this session
- **Relates to:** [`app.rs`](../../crates/wcode-tui/src/app.rs) (`Overlay`, `help_rows`,
  `open_info_rows_at`, `open_help`, `focus_digit`, `KEYS`),
  [`ui.rs`](../../crates/wcode-tui/src/ui.rs) (`draw_overlay`, `draw_info_panel`, `item_line`),
  [`docs/tui-design.md`](../tui-design.md) §4 Keys / Information modals / D027 row
- **Supersedes:** the `Overlay::Info` doc note "its own variant rather than a widened
  `Help` (Q6): the two have different content sources and `F1` stays byte-identical"
  — that was a *scope* carve-out, and this is the item that closes it
- **Amends:** [`D027`](D027-settings-panel.md) (the item row and the panel's height
  contract); extends [`D046`](D046-one-chord-one-axis.md) (the digit row it left

## Context

Two shortcut gaps, both small in code and both blocked by a rule rather than by effort.

1. **The root surface had no digit.** `focus_digit` guarded `n >= 1`, so
   `Alt-1..9` addressed member rows and `Alt-0` was a no-op — the root reachable
   only by `Shift-Tab`, i.e. only backwards. D046 retired the *forward* surface
   cycle as redundant with the digits, which left that one direction with no
   chord at all.
2. **The keymap had no room and clipped silently.** `KEYS` was 36 rows and
   `info_visible_rows` = `line_count.min(height − 3)`, so the F1 body fit exactly at
   40 rows and a 37th row would clip with no hint — because `F1` drew its own
   panel (`Overlay::Help`, `draw_help`, `more: false`) while **`/help` drew the
   same rows through the scrollable info panel** (`more: true`). Two renderers for
   one table, and the one you reach by reflex was the worse one.

## Decision

**One help page, two entries.** `Overlay::Help` and `draw_help` are deleted; `F1`
opens the same `help_rows()` body `/help` does, through the same panel, the same
scroll and the same `— N more —` footer.

1. **The entries differ only in their starting row.** `F1` opens at the `keys`
   section, anchored by the section LABEL (`help_rows().iter().position(… Prose("keys"))`)
   rather than by arithmetic, so adding a command cannot shift where `F1` lands;
   `/help` opens at the top. At 40 rows `info_visible_rows(64, 40)` = 37 and the
   clamp lands exactly on 27, so `F1` shows the `keys` header plus all 36 rows —
   today's content, now with the document above it reachable by `↑`.
2. **The toggle retires.** `Overlay::Info` swallows every key but `Esc` and the
   scroll chords, so a merged `F1` would stop closing itself; the rule becomes the
   one `F2` has had since W032 — **an opener says *open*, `Esc` closes**. `KEYS`
   reads `("F1", "open help at the keys section")`.
3. **`Alt-0` is the root, by deleting the guard rather than adding a branch.**
   `canonical_order` sorts the root first, so `get(0)` *is* the active
   conversation's root — the numbering the sidebar badges already use, completed.
   The row becomes `("Alt-0..9", "focus sidebar row N (0 = the root)")`, so the
   table stays at 36 rows and the cliff is untouched even without the merge.
4. **No chord for a command.** A chord never echoes a line the user did not type
   (`copy_last`'s rule), every picker verb is command-only (`/model`, `/theme`,
   `/team`, `/surface`), and the usable letters collide with terminal reflexes
   (`Ctrl-M` ≡ Enter, `Ctrl-H` ≡ Backspace, `Ctrl-L` redraw, `Ctrl-V` paste).

**The measured consequence, recorded rather than assumed:** the `— N more —` hint
counts rows **below** the fold, so on a 40-row terminal `F1`'s page ends at the
bottom and shows **no** hint — nothing is cut off below, and the commands above are
reached with `↑`, which needs no announcement because the scroll is the affordance.
The designer's draft said the hint would read `— 27 more —` there; the render says
otherwise, and the test asserts the shipped truth.

## Consequences

- `item_line`'s "byte-for-byte" contract survives as a *single* renderer rather
  than two identical ones; its golden test (`the_item_row_is_indented_padded_and_dim`)
  replaces the pair-render comparison, which could only ever prove two identical
  functions agreed.
- `esc_closes_the_info_modal_and_nothing_else_does` now covers `F1`'s page too.
- The `KEYS` budget stops being a cliff: a 37th row is now safe, because the panel
  scrolls and says how much is missing below.
- One habit changes: `F1` no longer dismisses itself; `Esc` does.
