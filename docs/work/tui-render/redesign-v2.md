# wcode TUI — deeper redesign (v2): richer blocks, legible palette, interactivity

Status: **proposal** — no code, no spec changed. Extends
[`mockups.md`](mockups.md) (v1: the session row, gauge, state, empty state,
borders, roster split). Presentation-only, inside `crates/wcode-tui` +
`docs/tui-design.md`; the kernel is out of scope.

Surface read: *a terminal surface, for wcode users, in the locked wcode spec
language, 80×24 first (legible to 48 cols) with 120×40 where it earns its keep.*

The locked decisions the spec amends are cited by section; the numbered
**D-decisions** (D1–D28) live in a *different* doc —
[`docs/team-and-tui-plan.md:224`](../../team-and-tui-plan.md) (`bQzob`) — so each
new decision below carries a `(team-and-tui-plan §D)` cross-reference.

---

## 0. What changes at a glance

| # | change | `docs/tui-design.md` section to amend | new D |
|---|---|---|---|
| D29 | legible grey ramp; `dim` loses the DIM modifier | §1.3 (`yQCWu`) | D29 |
| D30 | land the tier palettes (Basic-8 / 256 / truecolor) | §1.3, §5 (`1c4bV` stale) | D30 |
| D31 | tool block is a **panel** showing its **params** (bash = full cmd) | §4 Transcript (`nZDLN`,`a0dAT`), §2 (`pMZOm`) | D31 |
| D32 | per-block **expand/collapse** + **copy** affordances (click + key) | §4 Transcript, §4 Keys (`yZ4Cg`) | D32 |
| D33 | **thinking** collapsed by default, expandable | §4 Transcript, §6 (`QwBGB`) | D33 |
| D34 | one **canonical surface order**; sidebar numbered; `Alt-N` matches | §4 Sidebar (`4qLNe`) | D34 |
| D35 | **Todos** leave the sidebar → a transcript block | §4 Sidebar, §4 Transcript | D35 |
| D36 | **Changes** render as a directory **tree** | §4 Sidebar, §4 Transcript | D36 |
| — | panels → block structure; separators; the fixed glyph table | §1.2 (`zwmWX`), §1.4 (`vHHsu`), §2 (`pMZOm`) | — |

Everything below is drawn **only where it changes**.

---

## 1. Palette — legibility first (D29, D30)

### The problem, precisely

`dim` is the workhorse role — it paints tool lines, thinking, chrome, notices,
the whole quiet half of the screen — and today it is `Style::new().add_modifier(Modifier::DIM)`
with **no foreground** (`crates/wcode-tui/src/theme.rs:7YBTt`). `DIM` is a
*toggle the terminal interprets*; on xterm/iTerm/Terminal.app it renders at
roughly half the default intensity, which on a dark background lands near
**3:1** — below WCAG AA (4.5:1) and, for many users, below comfortable reading.
The complaint is exactly this: dim text is hard to read.

### The scheme

Keep all 17 roles, the color-mode ladder
(`crates/wcode-tui/src/theme.rs:7wN6X`,`:QNCsA`), and `NO_COLOR`. Change **what
the grey roles emit**: an explicit grey ramp instead of a modifier. `DIM`
survives **only** under `NO_COLOR`, where there is no color lever and the role
must fall back (documented best-effort).

Three greys, three jobs: `dim` (secondary text — the workhorse), `muted`
(quietest text, e.g. a `done` row), `border` (frames and rules). Each tier picks
its own steps.

| role | before | Named-16 | 256 | truecolor |
|---|---|---|---|---|
| `accent` | Cyan + Bold | Cyan + Bold | 45 | `#56b6c2` |
| **`dim`** | **`DIM`, no fg** | **`Gray`** | **250** | **`#a9b1ba`** |
| `muted` | `Gray` | `DarkGray` | 243 | `#7d8790` |
| `border` | `DarkGray` | `DarkGray` | 238 | `#3b4252` |
| `body` | default | default | default | default |
| `user` | Cyan + Bold | Cyan + Bold | 45 | `#56b6c2` |
| `error` | Red | Red | 203 | `#e06c75` |
| `success` | Green | Green | 114 | `#98c379` |
| `warn` | LightYellow | LightYellow | 180 | `#d6b06a` |
| `code` | Yellow | Yellow | 180 | `#d6b06a` |
| `heading` | Magenta + Bold | Magenta + Bold | 176 | `#c678dd` |
| `heading_sub` | Magenta | Magenta | 176 | `#c678dd` |
| `link` | Blue + Underline | Blue + Underline | 39 | `#61afef` |
| `tool_name` | Blue + Bold | Blue + Bold | 39 | `#61afef` |
| `thinking` | Magenta + **DIM** + Italic | Magenta + Italic | 176 | `#c678dd` |
| `diff_add` | Green | Green | 114 | `#98c379` |
| `diff_del` | Red | Red | 203 | `#e06c75` |

Hues are unchanged — they were legible. **The fix is the grey ramp and dropping
`DIM` from `dim` and `thinking`.** `NO_COLOR` → `Theme::plain` unchanged in
shape; there, `dim` stays `DIM` (no color lever).

### Legibility, before vs after (approximate, on a `#1e1e1e` dark background)

| role | before | after (truecolor) | note |
|---|---|---|---|
| `dim` | `DIM` ≈ **3.0:1** | `#a9b1ba` ≈ **7.7:1** | passes AA/AAA; the main win |
| `muted` | `Gray` ≈ 4.3:1 | `#7d8790` ≈ **4.6:1** | passes AA |
| `border` | `DarkGray` ≈ 1.6:1 | `#3b4252` ≈ **1.7:1** | decorative frame; fine |

`(ratios approximate; WCAG AA body text = 4.5:1.)`

### Render — before (80×24)

````
 ❯ why does the anchor move when I reformat?

   ··· I should confirm how anchors hash indentation, then read edit.rs before
       answering.
   The anchor hashes the line's raw content, so indentation is part of its
   address.

   ⚙ read  crates/wcode-cli/src/tools/edit.rs
   ✓ read · 128 lines · 12ms

   Reformatting reindents the line, so its hash — and every anchor below it —
   changes. Re-read after a formatter.
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

*legend (before):* the two prose paragraphs and the `❯` prompt are full strength;
**everything else — `···` thinking, `⚙`/`✓` tool lines, the box corners — is
`dim` = `DIM` modifier ≈ half intensity.** On a low-contrast theme this is the
"wall of faint text" the user reports.

### Render — after (80×24)

````
 ❯ why does the anchor move when I reformat?

   ··· I should confirm how anchors hash indentation, then read edit.rs before
       answering.
   The anchor hashes the line's raw content, so indentation is part of its
   address.

   ⚙ read  crates/wcode-cli/src/tools/edit.rs
   ✓ read · 128 lines · 12ms

   Reformatting reindents the line, so its hash — and every anchor below it —
   changes. Re-read after a formatter.
╭ wcode ⎇ main · session a1b2c3d4 ──────────── gpt-5-codex · high ╮
│ ❯ ▌                                                            │
╰ ▰▰▰▱▱▱▱▱ 14.2k / 272k ────────────── ⏻ plan · ⏸ idle ╯
````

*legend (after):* the same text, but `dim` is now an **explicit legible grey**
(`#a9b1ba` / index 250 / `Gray`), so the quiet half of the screen reads at ~7:1
instead of ~3:1. The hierarchy holds (prose > secondary > frames); it is just
*readable*.

**Recommend:** land D29 + D30. The greys are the whole point; the hues don't move.

---

## 2. The fixed glyph table (§2 amendment)

`docs/tui-design.md:34-50` (`pMZOm`) is stale — it omits most shipped glyphs
(see v1 critique C2/C3). This is the **single** table; §4 stops re-declaring
glyphs. Rows marked **＋** are new (each has a real call site below).

| element | glyph | style |
|---|---|---|
| user prompt | `❯` | accent, bold |
| assistant prose | — | default fg |
| thinking (collapsed) | `···` | dim |
| thinking (expanded) | `···` | dim, italic |
| tool start | `⚙ name  args` | dim |
| tool done | `✓ name · note` | dim |
| tool error | `✗ name · note` | error |
| compaction / retry | `⋯` | dim |
| live cursor | `▌` | accent, steady |
| selection bar / browse | `▌` · `▤ browse` | accent |
| spinner (running) | `⠋⠙⠹⠸` | accent |
| status separator | `·` | dim |
| inline code / fence | `` ` `` | code |
| table | `│ ─ ┼` | header bold |
| **＋ tree branches** | `├ └ │ ─` | dim |
| **＋ collapsed affordance** | `▸` | dim |
| **＋ expanded affordance** | `▾` | dim |
| **＋ copy affordance** | `⧉` | dim |
| **＋ panel frame** | `╭ ╮ ╰ ╯ ─ │` | border |
| context gauge | `▰ ▱` | success→warn→error |
| team state | `● ○ ✓ ✗` | by state |
| todos | `☑ ☐` | dim |
| branch / plan / idle / running | `⎇` · `⏻` · `⏸` · `⠹` | dim / accent |
| abort / paste chip | `⏹` · `❰ ❱` | dim |

New glyphs: `▸ ▾ ⧉ ├ └` (and the panel corners, already used by the input box).

---

## 3. Tool block — panel, params, expand/collapse, copy (D31, D32)

Today a finished tool is `⚙ name  <target-clipped-to-60>` + `✓ name · note`
(`crates/wcode-tui/src/ui.rs:rvJud`,`:ogadO`); the **bash command is clipped at
60 chars** and the args are not laid out. The redesign renders each tool as a
**panel** whose header is the tool, whose body is a keyed **params** block, and
whose top-right carries the `▸`/`▾` and `⧉` affordances.

### (a) collapsed (80×24)

````
   ╭─ ⚙ bash ─────────────────────────────── ▸ ⧉ ─╮
   │  cmd  cargo test -p wcode-cli                  │
   │  ✓ 41 passed · 12 lines · 9ms                  │
   ╰────────────────────────────────────────────────╯
````

*legend:* frame `border`; `⚙ bash` `tool_name`; `cmd` key `dim`, value `body`;
`▸`/`⧉` `dim`; `✓ …` `dim` (`success` on the mark).

### (b) expanded (80×24)

````
   ╭─ ⚙ bash ─────────────────────────────── ▾ ⧉ ─╮
   │  cmd  cargo test -p wcode-cli                  │
   ├────────────────────────────────────────────────┤
   │  running 41 tests                              │
   │  test result: ok. 41 passed; 0 failed          │
   │  …                                             │
   │  ✓ 41 passed · 12 lines · 9ms                  │
   ╰────────────────────────────────────────────────╯
````

*legend:* the params row stays visible when expanded (never scrolls away);
the `├─…─┤` rule separates params from output.

### (c) the bash command is never clipped — 120×40

````
   ╭─ ⚙ bash ────────────────────────────────────────────────────── ▾ ⧉ ─╮
   │  cmd   cargo test -p wcode-cli --all-targets -- --nocapture          │
   │  cwd   /Users/wei/Workspace/wcode                                     │
   ├──────────────────────────────────────────────────────────────────────┤
   │  running 41 tests                                                     │
   │  test result: ok. 41 passed; 0 failed; 0 ignored                      │
   │                                                                       │
   │  ✓ 41 passed · 3 lines · 412ms                                         │
   ╰──────────────────────────────────────────────────────────────────────╯
````

*legend:* `cmd` and `cwd` are **keys** (`dim`) with full `body`-strength values;
a command longer than the panel **wraps** under the value column, never clips.

### Per-block affordances (D32)

- **Click** the panel header (or the `▸`/`▾`) toggles that block.
- **Click** `⧉` copies that block's text via OSC-52 — the same payload browse
  mode's `y` copies (`crates/wcode-tui/src/app.rs:copy_text`).
- **Keyboard parity** (the skill rule: every affordance has a key): browse
  `Enter`/`Space` toggles the selected block, browse `y` copies it
  (`docs/tui-design.md:mMR5N`), both already in `KEYS`
  (`crates/wcode-tui/src/app.rs:Spmdk`). `Ctrl-T` still toggles all.

Params per tool (first present arg among the call's keys, from
`crates/wcode-tui/src/app.rs:ACTION_KEYS`): `bash` → `cmd`, `cwd`; `read` →
`path`, `offset`/`limit`; `edit`/`write` → `path`, then the diff; `grep` →
`pattern`, `path`; `find` → `pattern`.

---

## 4. Thinking block — collapsed by default, expandable (D33)

Today thinking is always inline (`··· text…`, `crates/wcode-tui/src/ui.rs:3vxfD`,
`:Nd8Id`) — it eats the transcript. The redesign collapses it to a one-liner,
resolving §6's open question (`docs/tui-design.md:QwBGB`).

### collapsed (80×24)

````
   ··· thinking · 218 chars                    ▸ ⧉
````

### expanded (80×24)

````
   ··· thinking · 218 chars                    ▾ ⧉
       I should confirm how anchors hash indentation, then read edit.rs
       before answering; the rule is that the hash covers the raw line, so
       reindenting moves it.
````

*legend:* `··· thinking · N chars` `dim` (the `···` marker per the table);
expanded body `thinking` (italic); `▸`/`▾`/`⧉` `dim`. No panel frame — thinking
is lighter than a tool; a bare affordance row keeps it cheap.

---

## 5. Sidebar — restyle, defined order, `Alt-N` (D34, D35, D36)

### The ordering bug, precisely

Three orderings disagree today:

- `Alt-1..9` → `focus_digit` → `set_focus(n-1)` → **raw `surfaces` index**
  (`crates/wcode-tui/src/app.rs:kf8T6`,`:wOITP`); `Alt-1` focuses `surfaces[0]`
  = the **root**, not the top sidebar row.
- The **sidebar Team** list → `member_rows_indexed`, sorted
  **active-first, then by recency** (`:mqoN0`,`:7wDvL`).
- The **working strip** → sorted **oldest→newest, last 3**
  (`crates/wcode-tui/src/app.rs:FQAXh`).

So the Nth sidebar row is *not* `Alt-N`. Fix: **one canonical order** drives all
three, and the sidebar **numbers** its rows.

### Before — recency order, `Alt-N` mismatched (80×24, sidebar 30 cols)

````
 Team                          ❯ run the tests
   ● reviewer   bash
   ○ explorer
   ✓ developer
 Changes  2                    ╭ wcode ⎇ main · session a1b2c3d4 ──── gpt-5-codex · high ╮
   src/app.rs · +3 −1          │ ❯ ▌                                                  │
   src/ui.rs · +8 −0           ╰ ▰▰▰▱▱▱▱▱ 14.2k/272k ─────── ⏻ plan · ⠹ running 3.1s ╯
````

*legend:* header + rows `dim`; `●` `success`, `○` `dim`, `✓` `muted`.

*problem:* the top row is `reviewer` (most recent), but `Alt-1` focuses the
**root** and `Alt-3` focuses `developer` — the list reorders under load, so the
mapping is unlearnable.

### After — stable canonical order, numbered (80×24)

````
 agents                        ❯ run the tests
   1 ● explorer
   2 ○ developer
   3 ✓ reviewer   bash
 ─────────────────────────
 changes  3 files  +12 −4
   crates/wcode-tui/src/
   ├─ ui.rs            +8 −0
   ├─ app.rs           +3 −1
   └─ theme.rs         +1 −3

                               ╭ wcode ⎇ main · session a1b2c3d4 ─── gpt-5-codex · high ╮
                               │ ❯ ▌                                                  │
                               ╰ ▰▰▰▱▱▱▱▱ 14.2k/272k ─────── ⏻ plan · ⠹ running 3.1s ╯
````

*legend:* `agents`/`changes` headers `dim`; the number badge `muted`; state
glyph by state; `●` now means *running* **without** reordering (a marker, not a
sort key).

**Canonical order (new locked decision D34):** root first, then members in
**creation order** (stable — never recency). The sidebar lists the numbered
non-root rows `1..N`; `Alt-1..N` selects **row N** (not raw index); the working
strip uses the same order. Stable order = learnable muscle memory; activity is
shown by `●`, not by reordering.

### Sidebar restyle

- A header+rule per section (`agents`, `changes`) using `border`; the panels and
  the input box share the rounded frame language.
- Number badge (`1`…`N`) so `Alt-N` is visible, not guessed.
- The **working strip** drops its own sort and mirrors D34 (same order, running
  only).

### `Alt-N` mapping (new `KEYS` entry wording)

| key | action |
|---|---|
| `Alt-1..9` | focus sidebar row **N** (the numbered order; not raw index) |
| `Ctrl-N` / `Shift-Tab` | next / previous in the same canonical order |

*`docs/tui-design.md:3Kx5S` (the Keys block) currently says "`Alt-1..9`
jumps to the Nth" — amend to "focus sidebar row N".*
surface" — amend to "focus sidebar row N".*

---

## 6. Todos in the transcript (D35)

Today Todos are a **sidebar section** (`docs/tui-design.md:KpqSU`,
`crates/wcode-tui/src/ui.rs:5R7Lv`) plus a `Block::Notice` dumped by the todo
tool (`crates/wcode-tui/src/app.rs:render_todos`). The user wants them **out of
the sidebar** for width. New: a first-class **`Block::Todos`** in the transcript,
rendered live, and the sidebar section is **removed**.

````
   ── todos  1/3 ─────────────────────────────────────────
   ☑ map the seam
   ☐ write the test
   ☐ run the suite
````

*legend:* `── todos  d/t ───` header `dim`; `☑`/`☐` `dim`; completed text
`muted`, pending `body`. Updates in place (the existing `TodoUpdate` feed),
scrolls with the transcript.

---

## 7. Changes as a tree (D36)

Today `Changes` is a flat `path · +a −r` list (sidebar) / `/changes` overlay
(`docs/tui-design.md:KpqSU`, `:zpKsa`). New: a **directory tree**, `├─`/`└─`
per the amended table, grouped by directory, stats right-aligned.

### in the transcript / `/changes` (80×24)

````
   ── changes  3 files  +12 −4 ───────────────────────────
   crates/wcode-tui/src/
   ├─ ui.rs                                   +8 −0
   ├─ app.rs                                  +3 −1
   └─ theme.rs                                +1 −3
````

### the same tree in the 30-col sidebar

````
 changes  3 files  +12 −4
   crates/wcode-tui/src/
   ├─ ui.rs          +8 −0
   ├─ app.rs         +3 −1
   └─ theme.rs       +1 −3
````

*legend:* directory lines `dim`; file leaves `body`; `+a` `diff_add`, `−r`
`diff_del`; `├─`/`└─` `dim`. Selecting a leaf still re-shows its diff
(`/changes`, `docs/tui-design.md:zpKsa`).

---

## 8. Keys (amend the Keys block `docs/tui-design.md:yZ4Cg`)

New / changed bindings (all with a `KEYS` entry,
`crates/wcode-tui/src/app.rs:Spmdk`):

| key / gesture | action | note |
|---|---|---|
| click block header · `▸`/`▾` | toggle that block | mouse; keyboard parity = browse `Enter`/`Space` |
| click `⧉` | copy that block (OSC-52) | keyboard parity = browse `y` |
| `Alt-1..9` | focus **sidebar row N** (canonical order) | **changed** from raw index |
| `Ctrl-N` / `Shift-Tab` | next / prev **in canonical order** | unchanged wording, new semantics |
| `Ctrl-T` | toggle all tools | unchanged |
| browse `Enter` / `Space` / `y` | toggle / copy selected | unchanged |

Mouse routing already exists for the transcript and sidebar hit maps
(`crates/wcode-tui/src/app.rs:hit`, `:sidebar_member_at`); D32 adds a per-block
affordance hit region.

---

## 9. Spec amendments + locked decisions

Amend in `docs/tui-design.md`, then add the decisions to the D-table in
`docs/team-and-tui-plan.md:224` (`bQzob`):

- **§1.2 Gutter (`zwmWX`)**: panels start in the content column; the marker
  column keeps `⚙`/`···`.
- **§1.3 Palette (`yQCWu`)**: the legible grey ramp; `dim` has an explicit fg;
  `DIM` only under `NO_COLOR`.
- **§1.4 Block separation (`vHHsu`)**: tools and thinking are panels/affordance
  rows, not blank-line-only blocks.
- **§2 Vocabulary (`pMZOm`)**: the fixed table (§2 here) — new `▸ ▾ ⧉ ├ └`.
- **§4 Transcript (`nZDLN`,`a0dAT`)**: new block kinds — Tool panel (params),
  Thinking collapsed, Todos, Changes tree.
- **§4 Sidebar (`4qLNe`)**: drop the phantom **Context** section; **remove
  Todos**; **two** sections (`agents`, `changes`); numbered rows; canonical order.
- **§4 Keys (`yZ4Cg`)**: the new bindings (§8).
- **§5 Decisions (`1c4bV`)**: strike the stale "theme detection still open"
  (the ladder shipped, `crates/wcode-tui/src/theme.rs:115` also shipped live
  reload).
- **§6 Open questions (`QwBGB`)**: **resolve** thinking → collapsed.
- **Cross-ref**: `docs/team-and-tui-plan.md:224` (`bQzob`) — add **D29–D36**.

New locked decisions:

- **D29** — Legible grey ramp; `dim`/`thinking` lose the `DIM` modifier.
- **D30** — Land the tier palettes (Basic-8 / 256 / truecolor); `NO_COLOR` keeps
  `DIM` as the only lever.
- **D31** — Tool block = panel with a keyed params block; **bash shows the full
  command** (wraps, never clips).
- **D32** — Per-block disclosure (`▸`/`▾`) and copy (`⧉`) affordances; click =
  mouse, browse keys = keyboard parity.
- **D33** — Thinking collapsed to `··· thinking · N chars`, expandable.
- **D34** — One canonical surface order (stable, root-first) drives the sidebar,
  the strip, and `Alt-1..N`; sidebar rows numbered.
- **D35** — Todos live in the transcript (`Block::Todos`); removed from the sidebar.
- **D36** — Changes render as a directory tree.

---

## 10. Tradeoffs + open questions

**Tradeoffs**
- **D31 panels** cost 3 rows of chrome per tool (top, params, bottom) and 2
  columns of frame. On an 80×24 tool-heavy session that is real estate. Mitigation:
  a *compact* mode (no frame, `⚙ name  cmd …` one line + `▸`) for when many tools
  are on screen — worth a follow-up render if the human wants it.
- **D32 click** must not fight the existing drag-to-select: a click on a header
  toggles, a drag *through* the block still selects — the hit region must exclude
  the drag path. Needs care; not free.
- **D34 stable order** means a running member no longer floats to the top; the
  `●` marker carries activity instead. Learnable, but a behavior change from the
  shipped active-first sort.
- **D33 thinking collapsed** hides reasoning by default — some users read it as
  it streams. Default-collapse is the recommendation; a config/`/think` toggle is
  the escape hatch if wanted.
- **D29 tier palettes** are the design-taste skill's named open item
  (`docs/next-steps.md:zft6w`, item 14: "tier palettes open"); landing them is
  scope, not a tweak.

**Open questions for the human**
1. **Panels (D31):** always-on, or a compact one-line fallback when N tools are
   visible? (affects 80×24 real estate most)
2. **Grey ramp (D29):** are the proposed greys the right steps, or do you want a
   warmer/cooler cast? (the exact hexes are the only subjective part)
3. **Thinking (D33):** default collapsed, or default-expanded until the first
   turn commits?
4. **Order (D34):** stable creation order (recommended), or a *fixed* running-first
   order that `Alt-N` follows exactly (still reorders, but deterministically)?
5. **bash panel (D31):** show `cmd`/`cwd` always, or `cwd` only when it differs
   from the project root?

**What I did NOT do**
- No live terminal run — all renders are hand-authored text, not captured frames.
- No measuring of the exact column counts in every frame (illustrative; a couple
  of ruler offsets are approximate).
- No implementation; no `theme.rs`/`ui.rs` edits; no `TestBackend` snapshot audit
  of what D29–D36 would move.
- Did not verify crossterm click-key separation for the D32 drag-vs-click guard.
