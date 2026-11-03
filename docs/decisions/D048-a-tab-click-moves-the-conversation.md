# D048 — A tab click moves exactly one thing: the active conversation

- **Status:** accepted — the human's ask ("how is shortcut support" → the direct tab
  jump), ruled by `agent:designer` (the (A) reading and the miss rule), briefed in
  [`W048`](../work/W048-clickable-tab-band.md), implemented from W040's own
  surviving sketch.
- **Date:** this session
- **Relates to:** [`app.rs`](../../crates/wcode-tui/src/app.rs) (`TabsHit`, `HitMap.tabs`,
  `set_tabs_hit`, `tab_at`, `activate_conversation_id`, `on_mouse_down`),
  [`ui.rs`](../../crates/wcode-tui/src/ui.rs) (`tab_row`, `draw_tab_bar`),
  [`docs/tui-design.md`](../tui-design.md) §4 Tab band
- **Extends:** [`D046`](D046-one-chord-one-axis.md) — which named the click as the tabs'
  *direct* affordance while `Alt-0..9` covers the surface axis — and retires the
  `TabsHit`/`tab_at`/`activate_conversation_id` sketch W040 left behind
- **Nothing on the wire:** `ConversationId` is already the tab's key (`D046`'s
  neighbour); `PROTOCOL_VERSION` stays 1

## Context

W046 gave tabs a cycle (`Ctrl-N`/`Ctrl-P`) and `/tab <n>`, and D046 recorded — in
writing — that a chord for a *direct* tab jump would re-create the N-dependent
ambiguity it deleted. The band itself was disabled: `ui.rs`'s own doc said **"Not yet
clickable: the row's tab extents are not published into a hit map"**, and W040's sketch
for the fix sat in `app.rs` labelled *"Waiting for: the clickable band"*.

So the last shortcut gap was not a missing chord but a missing hit map: at five tabs,
reaching the fifth was four `Ctrl-N` presses or a typed command.

## Decision

**A tab click moves the active conversation, and nothing else.**

1. **It is `activate_conversation`, not a second kind of activation.** `/tab <n>`,
   `Ctrl-N`/`Ctrl-P` and the `Added` reply all go through it, and pointing the surface
   focus at that conversation's root is part of what activation *is* — so the click
   inherits it rather than deciding it. The click is "land `Ctrl-N` on this tab".
2. **The renderer publishes, the pointer reads.** `tab_row` already builds the spans;
   it now also returns one `(conversation, x, width)` per **drawn** tab, in draw order,
   and `draw_tab_bar` hands them to `App::set_tabs_hit`. The shed rule is therefore
   implemented once: a tab the clip dropped is not among the extents, so it has no
   click target.
3. **The `·` separators belong to the tab on their left**, because the band draws no
   boundary glyph — a dead strip between two labels would be a rule the user cannot
   see. (The sidebar's "a click between rows is a no-op" applies *vertically*, where
   every row has its own line; it does not transfer to a glyph-less horizontal run.)
4. **A miss is a no-op, and the branch returns.** The band is chrome: a press on its
   gutter, on the clip boundary, or on any cell it does not draw changes nothing and
   begins no selection. `mouse_down = None` is load-bearing — `on_mouse_drag` returns
   early on it, so a drag that starts on a tab and ends in the transcript selects
   nothing and copies nothing.
5. **N = 1 publishes nothing.** The band has height 0, so `HitMap.tabs` is `None`;
   row 0 is the header, which is chrome too.

**The invariant the ruling turned an accident into:** the miss rule was already
prevented, not merely unexercised — `transcript_pos_at` and `block_at` are rect-bounded
(`if !hit.rect.contains((col, row).into()) { return None; }`), so a press in chrome
could never anchor a transcript selection. That is now *stated* in the spec rather than
relied upon, because the failure mode is not "the wrong row is selected" — it is a
future author adding a `y`-clamp to the transcript hit-test and quietly turning the
band into a selection surface.

## Consequences

- A tab is reachable in one gesture at any N, with no chord and no N-dependent meaning.
- The band's hit map cannot disagree with its pixels: both come from the same `spans`,
  in the same order, with the same clipping.
- `grep -rn "==== SKETCH" crates/` drops to **one** labelled pair — `main.rs`'s
  `serve()` adoption — because this item's sketch became code.
- A click on the already-active tab is a no-op (`activate_conversation` is idempotent
  apart from `dirty`); a click on a `Running` or `Failed` tab is allowed, because
  switching is reading, unlike `/tab close`'s `Running` refusal.
- No chord, no glyph, no palette role, no `KEYS` row: a click is not a key.
