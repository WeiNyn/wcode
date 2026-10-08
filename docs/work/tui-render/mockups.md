# TUI redesign — selection mockups

Screen drafts for the visible decisions in the redesign proposal, each as two
options at 80×24 in the locked spec's glyph/palette language. **Only what
changes is drawn** — bands that do not change are elided, not redrawn.

Style is not visible in plain text, so each frame carries a one-line legend
naming the roles used. Palette roles are the 17 in
[`theme.rs`](../../../crates/wcode-tui/src/theme.rs) — `accent` (cyan, bold),
`dim`, `muted` (grey), `user`, `body`, `error`, `success`, `warn`, `code`,
`border` (dark-grey), `tool_name`, `thinking`, `heading`.

At a glance — the options I would pick:

| decision | pick | one-line why |
|---|---|---|
| D1 session row | **(b)** folded into the box | removes a full lonely row; session id drops first under width pressure |
| D3 gauge glyph | **`▰ ▱`** | shipped already; lighter than solid blocks |
| D4 state emphasis | **accent state + muted project** | the one thing that must be readable at a glance is the run |
| D5 empty state | **seeded hint** | a blank first screen teaches nothing |
| D6 roster split | **keep both, document the split** | strip = live running glance; sidebar = full roster; §4's 4th "Context" section is rot, not layout |
| D7 borders | **rounded** | one chrome voice, not two |

---

## D1 — session row

### (a) standalone `session …` row

A whole 80-col row for a 16-char dim string, sitting above the transcript and
competing with it for the eye.

````
 session a1b2c3d4
 ❯ how does edit resolve an anchor?

   An anchor is a 5-char hash of a line's raw content, so a line's
   address includes its indentation — a reformatter moves it.

   ⚙ read  crates/wcode-cli/src/tools/edit.rs
   ✓ read · 128 lines · 12ms

   The drift-proofing is the point: every edit targets one snapshot.
╭──────────────────────────────────────────────────────────────────────────╮
│ ❯ ▌                                                                      │
╰──────────────────────────────────────────────────────────────────────────╯
````

*legend:* `session a1b2c3d4` = `dim`. Top border `wcode ⎇ main` = `accent`,
`gpt-5-codex · high` = `dim`. Bottom-left gauge `▰▰▰▱▱▱▱▱ 14.2k / 272k` =
`success`; bottom-right `⏻ plan · ⏸ idle` = `accent` · `dim`.

*tradeoff (a):* the id is always visible; but it is a full row of near-empty
chrome at the top of every frame, and it is the first thing lost on a short
terminal.

### (b) folded into the input-box top-left

The id rides the box's top-left corner beside `project ⎇ branch`; there is no
standalone row.

````
 ❯ how does edit resolve an anchor?

   An anchor is a 5-char hash of a line's raw content, so a line's
   address includes its indentation — a reformatter moves it.

   ⚙ read  crates/wcode-cli/src/tools/edit.rs
   ✓ read · 128 lines · 12ms

   The drift-proofing is the point: every edit targets one snapshot.
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

*legend:* `wcode` = `accent`; `⎇ main · session a1b2c3d4` = `dim` (the `·` is
the locked status separator). Everything else as in (a).

*tradeoff (b):* frees the top row and consolidates chrome; but at 48 cols the
session id is the first field the width ladder drops (branch, then the gauge),
so a narrow terminal loses it.

**Recommend:** (b). The gain is structural; the loss is a field that is already
low-value next to `⎇ branch`.

---

## D3 — gauge glyph

The context gauge in the input-box bottom-left corner. Same reading
(`14.2k / 272k`, ~42% full), two glyphs.

### `▰ ▱` — parallelograms (shipped today)

````
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ──────────────────────────────── ⏻ plan · ⏸ idle ╯
````

*legend:* `▰` filled / `▱` empty, colored `success` → `warn` → `error` by fill.

*tradeoff:* reads light; dihedral parallelograms sit on the baseline cleanly at
small terminal fonts. Not in the spec's §2 table (see the proposal) — this
option makes the code the reference and fixes §2.

### `█ ░` — blocks (spec §2 today)

````
╰ ███░░░░░ 14.2k / 272k ───────────────────────────────── ⏻ plan · ⏸ idle ╯
````

*legend:* `█` filled / `░` empty, same color ramp.

*tradeoff:* heavier, more conventional meter look; `░` is a dithered cell that
some terminals render muddy, and `█` is the heaviest ink on the frame — it
weighs more than the state token beside it.

**Recommend:** `▰ ▱`. It ships, and its lighter ink keeps the eye on the run
state, not the meter.

---

## D4 — state emphasis

Same frame text, two stylings. The question is which signal gets the accent
budget (≤1 accent role): the run state, or the project name.

### (a) current — project `accent`, run state `dim`

````
╭ wcode ⎇ main ─────────────────────────────── gpt-5-codex · high ╮
│ ❯ run the tests                                                 │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ─────────────── ⏻ plan · ⠹ running 3.1s ╯
````

*legend:* `wcode` = `accent` (bold cyan); `⎇ main` = `dim`; `⠹ running 3.1s` =
`dim` (as dim as `⏸ idle`); `⏻ plan` = `accent`.

*tradeoff (a):* the loudest text on an idle screen is a static project name,
while the one dynamic thing a user looks for — *is it running?* — is as quiet as
*idle*.

### (b) proposed — run state `accent`, project `muted`

````
╭ wcode ⎇ main ─────────────────────────────── gpt-5-codex · high ╮
│ ❯ run the tests                                                 │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ─────────────── ⏻ plan · ⠹ running 3.1s ╯
````

*legend:* `wcode` = `muted` (grey); `⠹ running 3.1s` = `accent` (bold cyan);
`⏻ plan` = `accent`; `⏸ idle` = `dim`.

*tradeoff (b):* the state scans first, matching §1.3's "accent for the user
prompt and running state"; the project name recedes to `muted`. Costs nothing
but a restyle of two spans.

**Recommend:** (b), with the animated `⠋⠙⠹⠸` spinner (motion dial ≤2, frozen
under `NO_COLOR`/reduced-motion) replacing the static `⠹`.

---

## D5 — empty state

The first frame of a fresh session. Only the transcript interior changes.

### (a) blank pane

````
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````
*(transcript area above the box is empty.)*

*tradeoff (a):* nothing competes; but the user is taught nothing — the keymap
is behind `F1`, the commands behind `/help`, and neither is advertised.

### (b) seeded hint line

````
   ❯ type a message · /help for commands · F1 for keys
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

*legend:* the hint is a dim `Block::Notice`; `❯` is the locked user-prompt
glyph, used here as a prompt-to-act. It scrolls away with the first turn.

*tradeoff (b):* one dim line teaches the three entry points; costs a Notice
block that must be cleared once the session has real content (or it lingers as
noise on a resumed session).

**Recommend:** (b). The seeded hint is the cheapest fix for the biggest
discoverability gap; drop it whenever `GetHistory` seeds real turns.

---

## D6 — sidebar / roster (corrected)

**Correction:** the shipped sidebar stacks **three** sections — **Team ·
Todos · Changes** (`crates/wcode-tui/src/ui.rs:WRkiC`, `:5R7Lv`, `:na8uD`); a
rendered-frame test pins exactly those three (`:IBBPK`). There is **no shipped
Context section**. `docs/tui-design.md:4qLNe`–`:0GQ7a` claims a fourth
("Context", the gauge) — that is **doc rot, not a layout bug**: the fix is to
amend §4, with no code change. The gauge repeats nowhere; it lives once, in the
input-box corner.

The only *shipped* roster duplication is between the **working-team strip**
(`crates/wcode-tui/src/ui.rs:3yhde`, running members only) and the **sidebar
Team section** (`:WRkiC`, all members). Both options below are about that.

### (a) keep both — document the split (shipped today)

````
 working strip (always on)      sidebar Team (on Ctrl-B)
 ─────────────────────         ──────────────────────────────────
 ● explorer  read crates/…      Team
                                 ● explorer  read crates/…
                                 ○ developer
                                 ✓ reviewer  done
````

*legend:* `●` running `success`, `○` idle `dim`, `✓` done `muted`, `✗` failed
`error`; the live action is `dim`.

*tradeoff (a):* a running member shows in both places while the sidebar is open.
Accepted: the strip is the always-visible glance (the sidebar is off by
default) and additively shows the "latest event is the bottom row" ordering
(`docs/tui-design.md:v3lUS`) the sidebar lacks; the sidebar adds the full roster
+ per-member state the strip omits.

### (b) suppress the strip while the sidebar is docked

````
 (no strip)                     sidebar Team (on Ctrl-B)
                                ──────────────────────────────────
                                Team
                                  ● explorer  read crates/…
                                  ○ developer
                                  ✓ reviewer  done

                                ❯ ▌
                                ╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ─ ⏻ plan · ⠹ running 3.1s ╯
````

*tradeoff (b):* no duplication; but the always-on live glance disappears
*exactly* when the sidebar is open, and the strip's ordering is lost. The
coarse run state (`⠹ running`) still rides the box corner.

**Recommend:** (a) — keep both, document the split in §4; the duplication is a
subset, not a contradiction. Separately, **amend §4 to drop the phantom
Context section** (text-only; no code change).

---

## D7 — borders

The picker overlay (`/model`, `/theme`, `/resume`). Only the frame changes.

### (a) square (today)

````
❯ /m

    ┌──────────────────────────────────────────────┐
    │ /m                                           │
    │ ❯ gpt-5-codex                                │
    │   gpt-5-codex-mini                           │
    │   o4-mini                                    │
    └──────────────────────────────────────────────┘
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

*legend:* overlay border + title = `border`; filter `dim`; selected marker `❯` =
`accent`; match highlight `accent`.

*tradeoff (a):* square corners read as "system dialog"; but they clash with the
rounded input box right below them — two chrome languages on one screen.

### (b) rounded (proposed)

````
❯ /m

    ╭──────────────────────────────────────────────╮
    │ /m                                           │
    │ ❯ gpt-5-codex                                │
    │   gpt-5-codex-mini                           │
    │   o4-mini                                    │
    ╰──────────────────────────────────────────────╯
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

*tradeoff (b):* one border language for the box, the picker, the `F1` help, and
the completion/search popups. Cosmetic only; touches no behavior.

**Recommend:** (b) — rounded everywhere. It is the smallest change with the
largest coherence gain.

---

## Summary of recommendations

- **D1 (b)** fold `session …` into the input-box top-left
- **D3** lock the gauge as `▰ ▱` (fix §2)
- **D4 (b)** run state `accent`, project `muted`, animated spinner
- **D5 (b)** seed the dim hint line, cleared once history exists
- **D6 (a)** keep the strip + sidebar Team, document the split; fix §4's phantom Context section (text only)
- **D7 (b)** rounded borders on the box and every overlay

Each is presentation-only, inside `crates/wcode-tui` + `docs/tui-design.md`; the
ones that amend the spec (D1 §1.1/§4, D3 §2, D4 §1.3, D6 §4 doc-only, D7 §4) name their
sections in the redesign proposal.
