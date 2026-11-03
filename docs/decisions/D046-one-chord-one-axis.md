# D046 — One chord, one axis: conversations get the cycle, surfaces keep the direct keys

- **Status:** accepted — the human's request ("we should [have] a shortcut to move
  around tab"), ruled by `agent:designer`, briefed in
  [`W046`](../work/W046-tabs-new-resume-and-nav.md) §12, second-layer review **BLOCKED
  then cleared** (three cheap items: two assertions without teeth, one comments
  commit).
- **Date:** this session
- **Relates to:** [`app.rs`](../../crates/wcode-tui/src/app.rs) (`on_key`,  `cycle_conversation`, `KEYS`,
  `focus_step`, `focus_digit`), [`docs/tui-design.md`](../tui-design.md) §4 Tab band + Keys + §5,
  [`event.rs`](../../crates/wcode-tui/src/event.rs) (`translate_key`, untouched),
  [`terminal.rs`](../../crates/wcode-tui/src/terminal.rs) (no keyboard-enhancement flags)
- **Supersedes:** `W040` §13.2 item 6 and §13.4 bullet 1 (the N-dependent
  `Ctrl-N`/`Shift-Tab`), recorded as errata rows in
  [`W040`](../work/W040-multi-root-conversations.md) §15
- **Extends:** [`D027`](D027-settings-panel.md)'s rule that the keymap table is the single
  source of chords and their descriptions

## Context

W040 gave the TUI conversations (tabs) but no unambiguous way to move between them.
The chord that existed was **modal on an invisible count**: `Key::Ctrl('n') =>
self.cycle_or_focus(1)` cycled conversations when `conversations.len() > 1` and
surfaces otherwise, while `KEYS` and `docs/tui-design.md` both advertised only the
surface meaning — so at N > 1 the keymap lied, and the state that decided the binding
was the state the band hides (height 0 at N = 1).

Three facts fixed the design space:

1. **`Ctrl-Tab` is not expressible.** `event.rs` matches `KeyCode::Tab` before any
   modifier guard (it is not a `Char`), and `terminal.rs` pushes no
   keyboard-enhancement flags — so `Ctrl-Tab` arrives as plain `Tab`, i.e. the
   completion accept.
2. **`Alt-Left`/`Alt-Right` is a hazard, not a chord.** `KeyCode::Left => Key::Left`
   ignores modifiers and `Key` has no arrow-with-modifier variant, so it would cost a
   variant plus `translate_key` arms plus every exhaustive match — and on terminals
   without xterm-keys it arrives as `Esc` + `Left`, which **cancels the run**.
3. **The two axes are asymmetric.** Tabs are unbounded, have no direct key and no
   click target (the band is not clickable), so a cycle is their only movement.
   Surfaces are ≤ 9 badged rows, already directly addressable by `Alt-1..9`, the
   sidebar click and `/surface` — but exactly one direction is unreachable: back to
   the unbadged root (`focus_digit` guards `n >= 1`), which is what `Shift-Tab` alone
   provides.

## Decision

**One chord, one axis.** The cycle goes to the axis with no direct addressing; the
bounded axis keeps its direct keys plus the one direction it cannot address.

| axis | chord | note |
|---|---|---|
| next / previous **conversation tab** | `Ctrl-N` / `Ctrl-P` | every N, wraps, **inert** at N = 1, **silent** |
| previous **surface** | `Shift-Tab` | unchanged at N ≤ 1 (that is the contract the four surface tripwires pin); at N > 1 it stops switching conversations — which *was* the defect — and it is now the only surface-cycle key |
| direct **surface** jump | `Alt-1..9` | unchanged |
| direct **tab** jump | `/tab <n>` | no chord — the band prints no numbers and draws the active tab first, so a digit would have two meanings |

1. **The forward surface cycle retires.** It is redundant with `Alt-1..9` on a ≤ 9-row
   badged axis; `Shift-Tab` stays because nothing else reaches the root backwards.
2. **A switch writes nothing.** The old `tab N of M` transient is deleted: it was the
   only transcript-writing navigation key in the app, and because `transient` pushes
   onto the focused surface *after* `activate_conversation` moved the focus, the block
   landed **in the tab you just arrived at**. Rule: **commands ack, keys don't** —
   `/tab <n>` keeps its `tab N` echo, `/theme`, `/width` keep theirs.
3. **No legend on the band.** The row has exactly one reading (`▌` = you are here) and
   a structural shed rule that must never have to shed something that is not a tab;
   `F1`/`/help` are the disclosure channel, and `KEYS` is their single source.
4. **`KEYS` grows by exactly one row (35 → 36).** `KEYS.len() + 1` = 37 is exactly the
   F1 panel's body height at 64×40, so a further row would clip silently; the
   `f1_renders_the_help_overlay_with_the_keymap` test is the tripwire.

## Consequences

- `Shift-Tab`'s N = 1 contract is preserved for the right reason: the four tests that
  used `Ctrl-N` to move between surfaces were re-pointed at `Shift-Tab` (one at
  `Alt-1`, which moves off the root rather than back to it), so
  `backtab_moves_focus_backwards_and_wraps` is untouched.
- Tab switching is input-mode-only, shadowed by overlays, the settings panel, the
  completion popup and browse — exactly as every other chord is.
- The clickable band (deferred, sketched in `app.rs`) is the future *direct* tab
  affordance; when it lands, `Alt-N` stays the surface vocabulary and no chord is
  repurposed.
- A user with `Ctrl-N`→next-member muscle memory loses that one movement; `Alt-N` and
  the sidebar click cover it.

## Alternatives rejected

| option | why not |
|---|---|
| keep the N-dependent `Ctrl-N` | the ambiguity is the reported defect; and the deciding condition is invisible |
| `Ctrl-Tab` / `Ctrl-Shift-Tab` | not expressible without the kitty protocol (§Context 1) |
| `Alt-←` / `Alt-→` | modifier-dropping arrows, plus an `Esc`-prefix hazard that would cancel a run (§Context 2) |
| `Alt-h` / `Alt-l` | same ESC-prefix class, no established meaning, and `Alt-` + punctuation is already the measure's corner |
| `Ctrl-F` / `Ctrl-B` | `Ctrl-B` is the sidebar |
| `Ctrl-D/U`, `Ctrl-R/V`, `Ctrl-X/Z`, `Ctrl-S/Q` | no next/prev meaning; `Ctrl-Z`/`Ctrl-S`/`Ctrl-Q` are terminal-hostile |
| a digit chord for tabs | the band prints no numbers and draws the active tab first, so "3" would not be the third thing on screen |
| `Shift-Tab` as a second tab key | one chord, one axis — and the F1 height cliff cannot afford the row |
