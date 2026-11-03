# W048 — The tab band is clickable: direct addressing without a chord — brief

- **Status:** **SHIPPED** (see §9 for what the ruling settled and what the implementation measured), No code written. The one design question (§4.2) is the designer's; the mechanism (§4.1) is the orchestrator's.
- **Item:** W048 (W046, W047 and D047 are closed; W041–W045 reserved per `docs/work/W040-multi-root-conversations.md` §14).
- **Predecessor:** W040, whose **first** surviving `SKETCH` block is this work (`crates/wcode-tui/src/app.rs:2673`–`:2734`, labelled *"Waiting for: the clickable band (W040/W041 follow-up). Retired by: a click item that ships TabsHit/tab_at/activate_conversation_id"*). W046 recorded the band as *not* clickable (`ui.rs:309`-region "**Not yet clickable**: the row's tab extents are not published into a hit map") and D046 named the click as the tabs' deferred **direct** affordance.
- **Decisions:** proposes **D048** — *a tab click moves the ACTIVE CONVERSATION and nothing else; the surface axis is untouched by the tab band*. Retires one of the two remaining `SKETCH` blocks.

---

## 1. The ask (inherited, not new)

The human's ask was "how is shortcut support" and, on the gaps: *"the honest fix is the clickable band"*. Nothing in this item is new scope — it is the direct tab addressing W046/W047 left out, and it is the last disabled affordance on the tab band.

Why a click and not a chord: D046 established that `Alt-N` is the **surface** vocabulary and that an N-dependent meaning is the defect it deleted. A click has no chord to steal, addresses any tab in one gesture, and is the only direct jump a tab band can have on a terminal without the kitty protocol.

---

## 2. Scope / Non-scope

### 2.1 In scope

1. **`TabsHit`** — the band's drawn geometry per frame: its rect and, per drawn tab, `(ConversationId, x, width)` in draw order (the sketch's shape, `app.rs:2673`-region).
2. **`HitMap.tabs: Option<TabsHit>`** — `None` on every frame where the band has height 0 (N = 1), so a click there falls through exactly as today. Reset by the existing per-frame `clear_hit_map`.
3. **`App::set_tabs_hit`** (called by `draw_tab_bar`), **`App::tab_at(col, row) -> Option<ConversationId>`**, and **`App::activate_conversation_id(id) -> bool`** (the by-id sibling of `activate_conversation(idx)`).
4. **The click branch in `on_mouse_down`, before the sidebar branch**: a tab hit activates that conversation and returns (a tab click never starts a transcript selection); a click on the band's row that misses every tab is a **no-op**, not a fall-through to the transcript.
5. **The `SKETCH` block at `app.rs:2673`-`:2734` is deleted** in the same commit that ships its contents — `grep -rn "==== SKETCH" crates/` goes 2 → 1.
6. **Tests** (§7.1) and one `docs/tui-design.md` clause (§4.2).

### 2.2 Non-scope (each names something a reasonable reader would assume is in scope)

- **Hover, press/release styling, a close `×` on a tab, drag-to-reorder, a scrollable band.** Each is a second gesture grammar; the band is one bare row with a structural shed rule (`docs/tui-design.md` §4), and W046 already refused a legend on it for the same reason.
- **A new chord for direct tab addressing.** D046 refused it; this item is the alternative, not a companion.
- **Changing the shed rule.** R6 draws the active tab first and drops the tabs to its right when clipped. A clipped tab is therefore not clickable — that is the rule working, not a gap (§4.3 states it).
- **The band's contents.** No label, glyph, count or badge changes; the click changes no pixel.
- **`Alt-0..9`'s meaning** (D047 settled it: rows of the canonical order within the active tab). The band must not touch surface focus (§4.2).
- **Mouse in browse mode / behind a modal.** `on_mouse` already returns early for an overlay or the browse search prompt; unaffected. Browse-mode *block* clicks are untouched.
- **W041's browser hub.** Unrelated; the same `Roots` will drive it.

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|---|---|---|---|---|
| W1 | `TabsHit` + the `HitMap` field | medium | low | low | Two structs and one field, the idioms the sidebar and transcript already use (`app.rs:4274` `set_sidebar_hit`, `:4262` `set_transcript_hit`). |
| W2 | publishing from `draw_tab_bar` | **high** | low | **medium** | The renderer must report the **drawn** extents, including the shed rule and the `▌ `/`⠋ `/`✗ `/` remote` spans — the risk is a hit map that disagrees with the pixels, which is a click that focuses a tab the user did not click. It is why `ui.rs:4812`-region exists for the sidebar ("the drawn-row proof `set_sidebar_hit`'s"). |
| W3 | the click branch + `activate_conversation_id` | **high** | low | low | One `if` before the sidebar branch, and a by-id wrapper over the existing `index_of` + `activate_conversation`. |
| W4 | tests | high | low | **medium** | The band's geometry is computed by the renderer, so the honest test renders and clicks; a unit test over a hand-built `TabsHit` proves nothing (§7.1). |
| W5 | the `SKETCH` deletion + the doc clause | medium | low | low | Text only, and the deletion is the point of the item. |

---

## 4. Interface & structure

### 4.1 The mechanism (the sketch's shape, confirmed against today's code)

`crates/wcode-tui/src/app.rs`:

- `pub(crate) struct TabsHit { pub(crate) rect: Rect, pub(crate) tabs: Vec<(ConversationId, u16, u16)> }` — the sibling of `SidebarHit`/`TranscriptHit`; `x` is absolute (screen col), `width` includes the `·` separator on the tab's RIGHT (the sidebar's badge-column precedent: a click between rows is a no-op, a click on a row is precise).
- `HitMap` (`:2820`-region) gains `pub(crate) tabs: Option<TabsHit>`; `clear_hit_map` (`:4332`-region) resets the whole value each frame, so a band that disappears at N = 1 cannot leave stale geometry.
- `pub(crate) fn set_tabs_hit(&mut self, rect: Rect, tabs: Vec<(ConversationId, u16, u16)>)`.
- `pub(crate) fn tab_at(&self, col: u16, row: u16) -> Option<ConversationId>` — `row == rect.y` and a span containing `col`; no other row of the band area counts (the band is one row, and the header below it must not be clickable through a stale field).
- In `on_mouse_down` (`:4296`-region), **before** the sidebar branch: `if let Some(id) = self.tab_at(col, row) { self.activate_conversation_id(id); self.mouse_down = None; return; }`.

`crates/wcode-tui/src/ui.rs`:

- `draw_tab_bar` (`:264`-region) builds `spans` today; it must also accumulate the per-tab extents **in draw order** and call `app.set_tabs_hit(area, tabs)` — where `area` is the band's `Rect` and the extents are measured from the same `spans` it draws. **Contracts:** the publisher is called on every frame the band is drawn; every drawn tab appears exactly once; the active tab appears first (R6's order); the total never exceeds the rect's width.

### 4.2 The one design question (designer's ruling — filled before implementation)

**What does a tab click move?** Two candidate readings, and they are mutually exclusive:

- **A — the active conversation, and nothing else.** The tab band is *selection chrome* above the header; `docs/tui-design.md` §4 says it "is not the session band … and as chrome it is never a browse target and never copied". A click selects the conversation; the surface focus follows to that conversation's root only because `activate_conversation` already does that (`app.rs:3353`-region) — the one equivalence every other activation path has.
- **B — the active conversation AND a surface**, i.e. a second interpretation where clicking a tab also lands on a particular surface (the last-focused one in that tab?). Rejected as a candidate because it needs a per-tab remembered focus that does not exist, and because it would let a click write into a conversation's surface state — the class W040's review blocked twice.

**Also to settle:** does a click on the band's row but NOT on a tab (the trailing gutter, the clip boundary) fall through to the transcript (today's behaviour for an unhandled Down, which starts a text selection) or become a no-op? My lean: **no-op** — the band is chrome, and a drag-select that starts on chrome would be surprising.

*Placeholder: the ruling and its `docs/tui-design.md` clause land here before implementation.*

### 4.3 What the shed rule implies (stated, not a defect)

At N > 1 the band draws the **active tab first** and drops tabs to its right when the row is crowded (R6). A clipped tab is therefore not clickable — it is not drawn, so it has no extent to hit. The ways to reach it are unchanged: `Ctrl-N`/`Ctrl-P` (which wrap) or `/tab <n>`.

### 4.4 What is NOT added

No new `Action`, no `ConversationEvent`, no protocol field, no new crate, no dependency, no chord, no glyph, no palette role, no change to `KEYS` (a click is not a key), and no change to the band's rendering.

---

## 5. Plan

1. **`tui:` the band publishes its extents** — `TabsHit`, the `HitMap` field, `set_tabs_hit`, and `draw_tab_bar` calling it. Gate: a render-level test (§7.1 item 1) asserting the published extents cover the drawn labels and that the band at N = 1 publishes nothing.
2. **`tui:` a tab click switches the conversation** — `tab_at`, `activate_conversation_id`, the branch in `on_mouse_down`. Gate: §7.1 items 2–4 (including "a click on the band's row but not on a tab does not start a text selection").
3. **`tui:` the sketch becomes code** — delete `app.rs:2673`-`:2734`; `grep -rn "==== SKETCH" crates/` returns exactly `main.rs:1919` with its label. Gate: build + that grep.
4. **`docs:`** the `docs/tui-design.md` clause per §4.2, and D048.

---

## 6. Quality gates

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets
grep -rn "==== SKETCH" crates/     # exactly one pair (main.rs), labelled
python3 scripts/check-book.py      # only if a book page is touched
```

## 7. Testing

### 7.1 Automated (each must fail if the logic is wrong)

1. **`the_band_publishes_one_extent_per_drawn_tab`** — render at N = 2 and N = 1: at N = 2 the hit map has one extent per drawn tab, in draw order, the active tab first, each `x..x+width` inside the band's rect and covering its label's first cell; at N = 1 the map has `None` (height 0). Teeth: hand-computing extents that ignore the `▌ ` prefix or the separators reddens it.
2. **`clicking_a_tab_activates_that_conversation`** — a render-and-click test at N = 2: click the *second drawn* tab's first cell; the active conversation changes to it and the surface focus follows to its root; `take_actions()` is empty (a click is not a `Request`).
3. **`clicking_the_band_off_a_tab_is_a_no_op`** — a click on the band's row past the last tab, and on the row below it (the header), changes nothing: same active conversation, same focus, `text_sel` unchanged (this is the "does not start a text selection" half, and the reason the branch returns rather than falls through).
4. **`a_tab_click_does_not_write_into_the_other_conversation`** — after the click, the previously focused surface's state is untouched (the W040 regression class), asserted by comparing the two surfaces' transcripts before and after.
5. **`the_band_is_not_clickable_at_n1`** — one conversation: a click on row 0 (where the band would be) does nothing at all, and no `TabsHit` exists.

### 7.2 How a human verifies it

```sh
cargo build --bin wcode
./target/debug/wcode --base-url http://127.0.0.1:9/v1 --model probe
```

1. `/new` twice → three tabs; **click the third tab** → the `▌` moves there and the transcript is that tab's; the other two are untouched (click back and read them).
2. **Click the band's empty right side** → nothing happens; the transcript selection is not started.
3. **Click the header row just below the band** → nothing happens (the band's rect is one row).
4. With one tab (press `/exit`-less: just start fresh) → row 0 is the header and a click there does nothing.
5. `F1` still lists no click (a click is not a chord) and `Ctrl-N`/`Ctrl-P` still work — the click is additive.

## 8. Expected outcome

- A tab is reachable in one gesture, at any N, with no chord and no N-dependent meaning.
- The band's hit map is published by the renderer that draws it, so pixels and hit extents cannot disagree.
- `grep -rn "==== SKETCH" crates/` returns exactly one labelled pair (the `serve()` adoption), because this item's sketch became code.
- A tab click moves the active conversation and nothing else; the surface axis keeps `Shift-Tab` and `Alt-0..9`.

---

## 9. The ruling, and what shipped

**The designer ruled (A)**: a tab click moves the active conversation and nothing else — the band addresses conversations, `Alt-0..9` addresses surfaces, and a click must never write surface state into a conversation the user is not in. The click is not a new kind of activation; it *is* `activate_conversation`, which already points the focus at that tab's root.

**The miss rule is a no-op**, and the review of the ruling produced the sharper statement: "fall through to the transcript" was never live behaviour — `transcript_pos_at` and `block_at` are rect-bounded, so a press in chrome already selected nothing. What the spec now says is the *rule*, not the accident: no press outside the transcript's own rect may ever begin a selection in it, because the failure mode is a future `y`-clamp, not a wrong row.

**Implemented as briefed**, with three additions the ruling implied:
1. `tab_row` returns `(Line, Vec<(ConversationId, u16, u16)>)` — the extents are measured from the same spans that are drawn, and a tab that does not fit the band's width is not published, so the shed rule has one implementation.
2. `self.mouse_down = None` in the tab branch (the sidebar branch's ritual), so a drag starting on a tab cannot become a transcript selection.
3. The stale doc comments the ruling named were fixed in the same diff: `activate_conversation`'s "`Ctrl-N`/`Shift-Tab` call this" (D046/D047 made it `Ctrl-N`/`Ctrl-P` and the click) and `ui.rs`'s "**Not yet clickable**" note, which is now the published-hits contract.

**Mutation-checked:** removing the activation from the click branch reddens `clicking_a_tab_activates_that_conversation` (`left: ConversationId(0) / right: ConversationId(1)`); restoring it greens. The sketch block at `app.rs` is deleted, so `grep -rn "==== SKETCH" crates/` returns exactly one labelled pair (`main.rs`'s `serve()` adoption).

### 9.1 The second layer found a real arithmetic bug, and it is worth naming

**A separator that is not drawn was still credited to a tab.** The separator credit was unconditional on the tab on its left having been *published*: once a tab was clipped, every later iteration still added 5 cells to the last **published** extent. At a narrow band that let the active tab's extent reach *past* the band's width — `(c0, 0, 19)` against `width == 18` — so a click on the drawn fragment of the next tab activated the previous one. That contradicted three things the item had stated as shipped: the separator ownership rule, "the hit map cannot disagree with its pixels", and the new spec sentence "any cell it does not draw is a no-op".

**The fix is one guard** (`if prev_published && let Some(last) = extents.last_mut()`), with the separator still pushed — a separator after a clipped tab begins past `width`, so it is never drawn, and only the credit was wrong.

**And the test that would have caught it did not exist**, which is the more useful finding: `the_band_publishes_one_extent_per_drawn_tab` claimed "one extent per DRAWN tab" with a doc line "a tab the clip dropped publishes nothing", but its fixture was two tabs at 80 columns — it could never clip, so the half of its name that mattered was unexercised, and its own invariant (`x + w <= rect.width`) never saw a clipped band. The new `a_clipped_band_publishes_only_the_tabs_it_draws_whole` renders **three** tabs at **16** columns (a three-tab fixture did not exist in the crate at all), asserts exactly one published extent of `(0, 12)` — the active tab's 7 cells plus its drawn 5-cell separator — and clicks the drawn fragment of the clipped tab to prove it resolves to nothing. Mutation-proved: restoring the unguarded credit reddens it with `left: (0, 17) / right: (0, 12)`.

Two smaller findings, both now stated rather than assumed:

- **`a_drag_starting_on_a_tab_selects_nothing` guarded the outcome, not the line.** The outcome was already prevented by the rect-bounded `transcript_pos_at`, so removing `mouse_down = None` would not have reddened it. A `#[cfg(test)] mouse_gesture()` accessor now asserts the field directly after the `Down`.
- **`cell_len` counts `chars()`,** so a wide-character label (`/rename 日本語`) drifts the extents by one cell per wide char. Harmless while the band was only pixels; it now decides hit-testing, so the assumption ("every glyph the bar can emit is single-width") is called out in `tab_row` rather than left implicit.

### 9.2 The live-verification gap, stated instead of implied

A click can be wrong about (a) which conversation a cell resolves to, (b) what activation does, and (c) whether the terminal delivers the event. The tests cover (a) against **real rendered geometry** and (b) through the real reducer; (c) rides the pre-existing `translate_mouse` → `AppEvent::Mouse` path the sidebar and transcript clicks already use. **Terminal delivery was not exercised live**: injecting an SGR mouse sequence through `tmux send-keys -l` did not move the active tab, and that is not evidence about this code — tmux arbitrates mouse input itself, so the injection may never have reached the pane. The manual check is two tabs, a click, and `/tab` before and after.
