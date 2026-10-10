# wcode TUI — design (visual spec)

Status: **agreed; implemented through P3; the D29–D36 redesign has shipped** (§1, §2, §4). Companion to [`tui-plan.md`](tui-plan.md) (the
architecture + phasing) and [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)
(the `Backend` seam the TUI is a client of). This doc is the *look*: bands,
glyphs, colors, and the mockups each phase should hit.

The REPL already has a vocabulary worth keeping — `❯` user, `⚙` tool start,
`✓`/`✗` tool end, `···` thinking, `⋯` compaction/retry, "dim = secondary". The
TUI reads as the same product, given a full screen.

## 1. Principles

1. **The base is the header over the transcript over the composer.** A pinned
   **header band** (the top row over a full-width `─` rule) carries the running
   head `WCODE · session <id> ──── project · ⎇ branch` with `model · effort` far
   right (D010); below it the transcript (flex) renders in a **measure** (a
   ≤68-col content column, centered once the band reaches 84 cols), while the
   composer stays full-width. The composer keeps its **foot line** (the context
   gauge left and `⏻ plan`/`▤ browse`/state/`↑ N`/folio right). There is **no
   session row, no status row, and no composer head row**. A **0–3-row team strip
   rides above the composer** only while a member is running; it is a rider, not a
   band. A palette, side panel, or diff is an *overlay* or an addition — never a
   reflow of the base.
2. **Role lives in the left gutter.** A 1-col margin, a marker column, content
   at a fixed column. The gutter is what makes a scrollback readable at a
   glance; wrapped continuation lines align under it. A **tool** hangs a tree in
   the gutter (`├`/`│`/`└`), its body on the content column (D010).
3. **A small palette, dim is the workhorse.** The 17 roles live in one place —
   `theme.rs` (`accent, dim, muted, border, user, body, error, success, warn,
   code, heading, heading_sub, link, tool_name, thinking, diff_add, diff_del`). Accent (cyan)
   for the user prompt and running state; red for errors; green for success;
   yellow for inline code; a green→yellow→red gauge for context fill. Assistant
   prose stays default and `dim` stays the workhorse.
   **`dim` is a legible grey, not a `DIM` modifier (D29).** It carries an
   explicit `fg` (a mid-grey) so the quiet half of the screen reads at ~7:1
   instead of the ~3:1 a `DIM` modifier lands on. The grey ramp is three steps —
   `dim` (secondary text) → `muted` (quietest) → `border` (frames) — and every
   role resolves per colour tier (**D30**): the 16-`Named` hues, their `256`
   indices, or the `truecolor` hexes, chosen by the shipped capability ladder
   (`resolve_color_mode_from`, `theme.rs`); a hex is honoured only under `Rgb`.
   **`NO_COLOR`** still yields `Theme::plain` — no `fg`, and there `dim` falls
   back to the `DIM` modifier, the only lever without colour (best-effort; D29).
4. **Blocks are separated by a blank line between roles** (a tool's tree clusters
   tight under its `├`). Noise is dim; only the model's prose and *your* prompt
   are full-strength. A **tool** is a `├` tree row (name + stats, its params/body
   under `│`, `└`-terminated) and **thinking** is a collapsed `··· thinking`
   affordance row — richer than a blank-line-only block (§4), not a reflow of the
   base.

## 2. Vocabulary

The **single** glyph table. §4 names elements but never re-declares a glyph.

| element | glyph | style |
|---|---|---|
| user prompt | `❯` | accent, bold |
| assistant prose | — | default fg |
| footnote reference | `¹²³` (`[n]` for n ≥ 10) | link |
| thinking (in flight) | `···` | dim, italic |
| thinking (collapsed row) | `··· thinking` | dim |
| tool head | `├ name  target  stats` | dim; name `tool_name` |
| tool body / end | `│   …` / `└` | dim |
| tool error head | `✗ name  stats` | red |
| compaction / retry | `⋯` | dim |
| live cursor | `▌` | accent, steady |
| selection bar (browse) | `▌` | accent |
| mode indicator (browse) | `▤ browse` | accent |
| spinner (running) | `⠋⠙⠹⠸…` | accent |
| status separators | `·` | dim |
| inline code / code block | `` ` `` | yellow (bold under `NO_COLOR`) |
| table | `─` | header bold; three `─` rules (top / header / bottom); columns separated by a whitespace gutter; cells wrap |
| overlay frame | `╭ ╮ ╰ ╯ ─ │` | border |
| collapsed affordance | `▸` | dim |
| expanded affordance | `▾` | dim |
| copy affordance | `▣` | dim |
| changes tree | `├ └ │ ─` | dim; leaf stats right-aligned |
| todos | `☑ ☐` | dim |
| team state | `● ○ ✓ ✗` | running / idle / done / failed |
| branch / plan | `⎇` / `⏻` | dim / accent |
| idle / running | `⏸` / `⠹` | dim / accent |
| abort / paste chip | `⏹` / `❰ ❱` | dim |
| context gauge | `▰ ▱` | green → yellow → red by fill |

## 3. Layout drafts

> **Historical.** Drafts A–E below predate the composer redesign (D1b/D4b) and
> the D29–D36 work: they show a flat status line and a `────` rule, whereas the
> shipped screen carries the chrome in a pinned **header** band (D010) and
> the composer's **foot line**, and has no status row or session row. Read **§4**
> for the shipped layout; the drafts are kept for the reasoning, not as current
> targets.

### A — idle, a completed exchange (78 cols)

```
 ❯ how does edit resolve an anchor?


   An anchor is a 5-char hash of a line's raw content, so a line's address
   includes its indentation — a reformatter moves the anchors.


   ⚙ read  crates/wcode-cli/src/tools/edit.rs
   ✓ read · 128 lines · 12ms


   The drift-proofing is the point: every edit targets one snapshot, and an
   edit above a target never shifts it.


 ❯ ▌
 ────────────────────────────────────────────────────────────────────────────
  gpt-5-codex · high · 14.2k / 272k · session a1b2c3 · ⏸ idle
```

### B — running (thinking, tool in flight, live cursor)

```
 ❯ add a test for the ambiguous-anchor case


   ··· The tests use an edit(src, from, to) helper; I'll add a case with two
       identical lines and assert the ambiguity error is returned.
   I'll add the test, then run the suite to be sure.

   ⚙ edit  crates/wcode-cli/src/tools/edit.rs
   ✓ edit · 84 lines changed · 9ms

   ⚙ bash  cargo test -p wcode-cli
   ⠹ bash · running 1.2s

 ❯ ▌
 ────────────────────────────────────────────────────────────────────────────
  gpt-5-codex · high · 15.8k / 272k · session a1b2c3 · ⠹ running 3.1s
```

### C — narrow (48 cols; status truncates by priority)

```
 ❯ run the tests

   ⚙ bash  cargo test
   ✓ bash · 12 lines
   All 41 tests pass.

 ❯ ▌
 ──────────────────────────────────────
  gpt-5-codex · 15.8k · ⏸ idle
```

### D — command palette (P1, overlay)

```
   ┌──────────────────────────────────────────────┐
   │ /                                            │
   │ ❯ model      set the model                   │
   │   models     list available models           │
   │   effort     set reasoning effort            │
   │   compact    summarize older messages        │
   │   resume     open another session            │
   │   usage      token usage                     │
   └──────────────────────────────────────────────┘
 ❯ /m▌
 ────────────────────────────────────────────────────────────────────────────
  gpt-5-codex · high · 14.2k / 272k · session a1b2c3 · ⏸ idle
```

### E — tool result with a diff (P1/P2, inline)

```
   ⚙ edit  crates/wcode-cli/src/tools/edit.rs
     @@ -40,6 +40,9 @@
      fn resolve(anchor: &str) -> Result<usize, EditError> {
     +    if matches.len() > 1 {
     +        return Err(EditError::Ambiguous);
     +    }
          Ok(matches[0])
      }
   ✓ edit · +3 −0 · 9ms
```

## 4. Component specs

**Transcript** — a `Vec<Block>` of committed blocks plus one live block. Block
kinds: `User`, `Turn`, `Tool`, `Notice`, `Error`, `Btw`, `Diff`, `Todos`.
Each block computes its own height at draw time. A wrapped-line cache keyed by
`(revision, width)` is the P2 optimization (do not re-wrap static history every
frame). Follow-tail while streaming; scroll-lock when the user scrolls up (P1).
At startup the loop asks `GetHistory` and `App::seed_history` rebuilds the
transcript from it, so a resumed session (or a reconnecting socket client) opens
on its earlier turns instead of an empty pane — a dim `⋯ N earlier message(s)`
divider marks the replayed prefix. With **no** history the transcript opens on a
dim seeded hint (`❯ type a message · /help for commands · F1 for keys`), not a
blank pane.

- **Header** (D010) — a **pinned top band**, not a transcript block: the running
  head `WCODE · session <id> ──── project · ⎇ branch`, over a **full-width**
  `─` rule — one row + one rule, always visible. `WCODE` is a **reverse-video**
  run (`Modifier::REVERSED`; no glyph, no role), then `· session <id>`;
  `project · ⎇ branch` reads left-to-right and `model · effort` is a **far-right
  group** on the same row (option A). It is **chrome** — never a browse target.
  At 48 cols the ladder sheds `session` first, then the branch.
- **Speaker head** — each message opens with a **speaker** head: `YOU` for the
  user block (replacing the transcript's `❯`) and `WCODE` for the assistant turn —
  a **reverse-video** run (`Modifier::REVERSED`; no glyph, no role). It replaces
  the retired `§N` section head (`§` is no longer in the glyph table, §2). It is
  **chrome** — never a browse selection target, never copied.
- **Code plate** — a fenced code block renders a **caption line** above it,
  `plate N — <info>` (the fence's info string verbatim; just `plate N` when
  there is none), all `dim`. `N` is the fenced block's ordinal, **per message,
  monotonic**. **No disclosure** — the code always shows (the design's `▸`
  collapse is deferred). No new glyph (`plate`/`—`/the digits/`/` are ASCII;
  `─` is declared).
- **Tools (inline, tree)** (D009/T1) — a tool renders **inline, in call order**:
  at the **position it was called**, flowed with the prose. The render walks the
  turn's `content` and emits tool `k` at the `k`-th `ToolCall`; the prose keeps a
  `¹²³` mark where the call was (suppressed on a text-less tool-call round). A tool
  is **one tree row** — `├ name  target` with the `note · ms` (or `+a −r · ms`)
  stats and the `▸`/`▣` affordance right-aligned — over its body (`│   ` params +
  output/diff/preview + a `… +N more` hint), closed by a `└` terminator when it has
  a body. The name prints **once** — no ordinal, no separate `✓ name · note` row.
  The **`bash` command is shown in full** (it wraps, never clips). A tool error is
  forced expanded. `Ctrl-T` toggles every tool's output; browse
  `Enter`/`Space`/`y` act per tool. The `── notes ──` **foot ledger is removed**.
- **Thinking** (D33/D010) — **in flight** it streams expanded inline (the `···`
  gutter plus the `thinking` italic body); on **commit** it collapses to a quiet
  one-line `··· thinking` row (no char count) with the same `▸`/`▾`/`▣`
  affordances, expandable. Its `···` marker shares the tool tree's gutter column.
- **Todos** (D35) — the `todo` tool's checklist is ONE block that updates in
  place (`── todos  d/t ──` header, `☑`/`☐` rows), in the transcript — removed
  from the sidebar.
- **Diff / changes** (D36) — a `/changes` selection re-shows the file's diff;
  the run's changeset renders as a directory **tree** (`├─`/`└─`, stats
  right-aligned) in the sidebar and the `/changes` modal.
- **Affordances** (D32) — every **expandable** block (a tool's head row, or an
  assistant block whose first content is thinking) carries a `▸`/`▾` disclosure
  and a `▣` copy cell on its first row, right-aligned at the panel's columns;
  click toggles/copies, and browse `Enter`/`Space` and `y` are the keyboard
  parity.

**Gutter** — 1 col margin, marker column, content at a fixed column (so wrapped
continuation lines align under the text, as in draft B's `···` block). The
gutter is also the natural home for a `▌` selection bar. A **tool**'s tree
(`├`/`│`/`└`) rides the gutter on the content column; a thinking row keeps the
plain `···` marker column (D010).

**Browse mode** — `Ctrl-G` moves a `▌` selection over the committed transcript.
The bar occupies the gutter's first column (the 1-col margin), *replacing* the
blank there so no glyph shifts; a `▤ browse` token rides the composer's foot line just
before `state`. Movement is `j`/`k`, `g`/`G`, `PgUp`/`PgDn`; `Esc`/`q`/`Ctrl-G`
leave — and in browse `Esc` leaves the mode, it does **not** cancel or quit.
**Browse owns the text and navigation keys**; `F1` / `?` (help) and `Ctrl-C`
(cancel / quit) stay **global**, and every other key is ignored while browsing
(so the composer cannot be edited behind the mode). The live block is never a
target (it is transient). Select and reveal; `Enter`/`Space` expands the
selected block and `y` copies it (D32). A `$PAGER` action is still a later phase.

**Input** — `❯ ` prefix on the first row, block cursor `▌`. Wraps and grows with
its content (Shift-Enter or Ctrl-J inserts a newline); the band grows to ~8 rows
then scrolls to keep the cursor visible. A paste over 100 chars or more than 3
lines collapses to a dim chip `❰ pasted 12 lines · 340 chars ❱` — atomic
(Backspace removes it whole) and expanded on submit (outer whitespace trimmed). Up/Down recall prompts, saving
the draft and restoring it at the bottom; history persists one prompt per line
beside the sessions. (Shift-Enter needs a terminal that reports the modifier —
kitty/xterm-`modifyOtherKeys`; Ctrl-J is the portable newline.)

**Layout** — top to bottom (the sidebar, `Ctrl-B`, is an optional left column):
0. the **header** band (pinned, D010) — the running head over a full-width
   `─` rule, always visible;
1. the **transcript** (flex, plain, scrollable). Committed assistant messages render as
   markdown — headings, bullets, fenced code, aligned tables, inline
   `code`/`**bold**`. It renders in a **measure** — a content column at most
   **68 cols** wide, **centered** in its band once the band reaches **84 cols**
   (a real margin each side); below 84 the content uses the full band width (so
   an 80-col terminal keeps its `❯` at the gutter). The **composer band is never
   centered** — it is the one piece of furniture that stays edge-to-edge. A
   **tool** is a panel (params + body); a **thinking** block is a collapsed row
   until expanded;
2. the **team region** — 0..=3 rows, only `Running` teammates (the root is the
   orchestrator, excluded), in the **canonical order** (D34). It is a rider, not
   a band: it collapses to nothing when no teammate runs;
3. the **composer** — an **open writing line** (the input) over a full-width
   `─` rule, with a **foot line** below. The composer's **head row is dropped**
   (D009): the chrome it carried (`project`/`branch`/`session`, `model`/`effort`)
   now rides the pinned **header** band (above), and the composer keeps
   only the **foot line** — the context gauge left and
   `[⏻ plan] · [▤ browse] · ⏸ idle`/`⠹ running 3.1s`/`⠹ btw…` · `[↑ N]` `<N>/<M>` right
   (the `btw…` state while a `/btw` side ask is in flight; the **folio**
   `<N>/<M>` is the surface's turn count, `N == M`, so a first turn reads
   `1/1`). There is **no box and no corners**, and **no session row, no status
   row, and no head row**.
The context gauge is 8 parallelograms (`▰` filled, `▱` empty) colored
green→yellow→red by fill, then `used / limit`. The header and composer-foot
titles clip with `…` then drop least-important-first (header: the session
id, then branch; foot: the `[↑ N]` scroll, then the gauge, then
`⏻ plan`/`▤ browse`). The session id is shortened to 8 chars.

**Sidebar** — `Ctrl-B` docks a 30-col left panel (only when the terminal is
≥ 80 cols; below that the layout is untouched), **off by default** so the base
layout stays byte-identical while it is closed. It stacks **two** dim-headed
sections:
1. **`agents`** — each member's **numbered** row (`1 ● explorer · read a.rs`,
   `2 ○ developer`, …): a `muted` number badge (so `Alt-N` is visible), the state
   glyph, the label, the live action, and a `*` on the focused surface. Rows are
   in the **canonical order** (D34: root first, then members in creation order —
   stable; `●` marks activity, it does not reorder).
2. **`changes`** — the run's changeset as a directory **tree** (D36):
   `crates/wcode-tui/src/` then `├─ ui.rs  +8 −0` …; `+a` added, `−r` removed,
   stats right-aligned.
An empty section keeps its header with a dim `—`; rows clip to the panel and
never wrap. The modal overlay floats over the whole terminal so it covers the
panel. *(No `Todos` section — todos are a transcript block, D35 — and no
`Context` section — the gauge lives on the composer's foot line.)*

**Keys** — `Enter` submit · `Shift-Enter`/`Ctrl-J` newline · `Del` forward-delete · `Up`/`Down` history ·
`Ctrl-A`/`Ctrl-E` move to the start/end of the input · `Ctrl-W` delete the previous word,
`Ctrl-U`/`Ctrl-K` delete to the start/end of the current line (readline word editing on the
atom buffer; a paste chip is one unit, never split) ·
`PgUp`/`PgDn` page, the mouse wheel nudges (3 lines) — either scrolls the
transcript, `↑ N` on the composer's foot line while scrolled ·
`Esc`/`Ctrl-C` cancel a run, quit when idle · `Ctrl-Y` (`/copy`) copies the last
reply (OSC-52) · `Ctrl-T` expands/collapses every tool's output (a collapsed tool
shows a 4-line preview, a failed tool always shows its error) ·
`Ctrl-N`/`Shift-Tab` focus the next/previous surface in the **canonical order**, `Alt-1..9` focuses **sidebar row N** (the numbered order, not the raw index) · `Ctrl-B`
docks/undocks the left sidebar · `F1` opens the keymap overlay (dismissed only by `Esc`/`F1`; the
same `KEYS` table is printed by `/help`) · `Ctrl-G` enters **transcript browse**
(a `▌` selection over the committed blocks — `j`/`k` next/prev, `g`/`G` first/last,
`PgUp`/`PgDn` by a page, the wheel scrolls the view), where `Esc`/`q`/`Ctrl-G`
leave (in browse `Esc` leaves the mode and does **not** cancel or quit). Browse owns
the text and navigation keys; `F1`/`?` (help) and `Ctrl-C` (cancel / quit) stay
**global**, and every other key is ignored while browsing. `/changes` lists
the files the current run changed (`path · +a −r`)
in the overlay; selecting one re-shows its diff in the transcript. The changeset
is per-run and ephemeral — durable history is git's. `/resume` opens the same
overlay over the sessions on disk; choosing one quits and re-execs into it
(`--resume`), since a pure client cannot rebuild a session in place.

## 5. Decisions

Agreed for P0 (see also `tui-plan.md` §9):

- **Data source**: a `wcode-protocol` [`Backend`](../crates/wcode-protocol/src/backend.rs)
  (`Local`/`Remote`), never `Agent` directly. Local↔remote becomes a transport
  swap, and the TUI is already the jcode architecture minus the work.
- **Crate**: new `wcode-tui`, depending on `wcode-protocol`.
- **Versions**: `ratatui` 0.30, `crossterm` 0.29 (jcode's; both exist).
- **Alt-screen**: yes, with a custom scrollback (P1).
- **Palette**: one `theme.rs` of named roles (`Theme::colored` / `Theme::plain`),
  with per-tier colours (`Named`/`256`/`truecolor`, D30) and a `[theme]` / `/theme`
  overlay (P4, landed); live-reloadable (`static THEME: RwLock<Theme>`).
- **Markdown**: hand-rolled, applied live and committed (P1/P2). **Clipboard**:
  OSC-52, no `arboard` (P2).
- **Theme / colour capability**: **shipped** — the ladder (`resolve_color_mode_from`) picks `Plain`/`Named`/`Indexed`/`Rgb`, a hex is honoured only under `Rgb`, and the palette is live-reloadable (`static THEME: RwLock<Theme>`); see **D29**/**D30**.
- **Wire deltas**: P0 accepts whole-message `MessageUpdate`; the O(n²) matters
  only for a socket-backed TUI (already reachable via `--socket`) and is
  resolved before that path is a goal.

## 6. Open questions

- **Thinking** — **resolved (D33/D010)**: in flight it streams **expanded** inline; on commit it collapses to a quiet one-line `··· thinking` row (no char count), expandable. Tool output is a tree row (D009/T1), collapsible per block (`▸`/`▾`, or browse `Enter`) or all at once (`Ctrl-T`).
- **Timestamps** on turns: lean no.
- **Header/title bar**: **amended by D010** — the header IS a pinned top band now
  (the running head over a full-width rule); the composer keeps its **foot line**,
  and `Ctrl-B` docks the sidebar for the rest.
- **Block separation**: blank between *roles* (kept); a tool is a panel and a thinking block is an affordance row, so "between every block" is moot.
- **Gutter vs flat**: gutter (drafted) — it is the main thing the TUI buys over
  the line loop. Revisit only if it costs width on 80-col terminals.
