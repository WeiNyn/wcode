# wcode TUI — design (visual spec)

Status: **agreed; implemented through P3; the D29–D36 redesign has shipped** (§1, §2, §4); **amended by D018** (W014: in-flight thinking as an annotation, the sidebar's root row, transient command notices) **and by D023** (W018: P5 "prose only, fully bare" — no band rules, plain head labels, treeless tools, a text gauge, a grey default with red the only hue). Companion to [`tui-plan.md`](tui-plan.md) (the
architecture + phasing) and [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)
(the `Backend` seam the TUI is a client of). This doc is the *look*: bands,
glyphs, colors, and the mockups each phase should hit.

The REPL already has a vocabulary worth keeping — `❯` user, `⚙` tool start,
`✓`/`✗` tool end, `⋯` compaction/retry, "dim = secondary" (the TUI renders
thinking as a plain `thinking` row, D023). The
TUI reads as the same product, given a full screen.

## 1. Principles

1. **The base is the header over the transcript over the composer.** A pinned
   **header band** — ONE bare row, no rule (D023) — carries the running
   head `WCODE · session <id> ──── project · ⎇ branch` with `model · effort` far
   right (D010); the head row's only `─` is the **leader** between its own runs.
   Below it the transcript (flex) renders in a **measure** (a
   content column, default 68 and adjustable via `/width`, centered once the
   band reaches 84 cols), while the composer stays full-width. The composer keeps its **foot line** (the plain `used/limit` text left and `⏻ plan`/`▤ browse`/state/`↑ N`/folio right). There is **no
   session row, no status row, and no composer head row**. A **0–3-row team strip
   rides above the composer** only while a member is running; it is a rider, not a
   band. A palette, side panel, or diff is an *overlay* or an addition — never a
   reflow of the base.
2. **Role lives in the left gutter.** A 1-col margin, a marker column, content
   at a fixed column. The gutter is what makes a scrollback readable at a
   glance; wrapped continuation lines align under it. A **tool** is a **flat**
   row at the gutter column (col 3) — no tree (D023); its body sits at the same
   column.
3. **A small palette, dim is the workhorse.** The 17 roles live in one place —
   `theme.rs` (`accent, dim, muted, border, user, body, error, success, warn,
   code, heading, heading_sub, link, tool_name, thinking, diff_add, diff_del`).
   The **default (palette B) is bare (D023)**: `body`/`user` are default weight,
   **`error` red is the ONLY hue**, and every other role is the grey ramp.
   "One colour" means **no hue but red** — grey is the achromatic ramp, not a
   colour. The markdown structure roles keep only their non-colour modifiers
   (`heading` bold, `heading_sub`/`link` underlined, `code` plain), and the code
   fence is monochrome (a grey syntax theme). Assistant prose stays default and
   `dim` stays the workhorse.
   **`dim` is a legible grey, not a `DIM` modifier (D29).** It carries an
   explicit `fg` (a mid-grey) so the quiet half of the screen reads at ~7:1
   instead of the ~3:1 a `DIM` modifier lands on. The grey ramp is three steps —
   `dim` (secondary text) → `muted` (quietest) → `border` (frames) — and every
   role resolves per colour tier (**D30**): the bare grey steps, their `256`
   indices, or the `truecolor` hexes, chosen by the shipped capability ladder
   (`resolve_color_mode_from`, `theme.rs`); a hex is honoured only under `Rgb`.
   The seven presets (`nord`, `dark`, …) stay **coloured escape hatches** — a
   preset still overrides every chromatic role.
   **`NO_COLOR`** still yields `Theme::plain` — no `fg`, and there `dim` falls
   back to the `DIM` modifier, the only lever without colour; red is unavailable,
   so `error` degrades to **bold** (best-effort; D023/D29).
4. **Blocks are separated by a blank line between roles** (a tool's cluster sits
   tight at col 3). Noise is dim; only the model's prose and *your* prompt
   are full-strength. A **tool** is a flat row (name + right-aligned stats, its
   params inlined and body at the same column) and **thinking** is a plain dim
   `thinking` row — richer than a blank-line-only block (§4), not a reflow of the
   base.

## 2. Vocabulary

The **single** glyph table. §4 names elements but never re-declares a glyph.
`style` names a **role** (§1.3): under the bare default (D023) every role but
`error` resolves to the grey ramp, so e.g. `accent` and `border` read as grey.

| element | glyph | style |
|---|---|---|
| user prompt | `❯` | dim (the composer prefix; the prompt itself is default weight, D023) |
| assistant prose | — | default fg |
| speaker heads | `WCODE` / ` YOU` | dim (no longer reversed; D023) |
| thinking (in-flight annotation) | `thinking <t>` | dim row at col 3 + live cursor `▌` (D018; the marker glyph retired, D023) |
| thinking (collapsed row) | `thinking` | dim |
| tool head | `name  target  stats` | dim, flat at col 3 (no tree, D023); name `tool_name` |
| code fence / block quote | `│` | dim (kept — only the *tool* tree is gone, D023) |
| tool error head | `✗ name  stats` | red |
| compaction / retry | `⋯` | dim |
| live cursor | `▌` | accent, steady |
| selection bar (browse) | `▌` | accent |
| mode indicator (browse) | `▤ browse` | accent |
| spinner (running) | `⠋⠙⠹⠸…` | accent |
| status separators | `·` | dim |
| inline code / code block | `` ` `` | plain (monochrome by default; a coloured preset may tint, D023) |
| table | `─` | header bold; three `─` rules (top / header / bottom); columns separated by a whitespace gutter; cells wrap |
| overlay frame | `╭ ╮ ╰ ╯ ─ │` | border |
| changes tree | `├ └ │ ─` | dim; leaf stats right-aligned |
| todos | `☑ ☐` | dim |
| team state | `● ○ ✓ ✗` | running / idle / done / failed |
| branch / plan | `⎇` / `⏻` | dim / accent |
| idle / running | `⏸` / `⠹` | dim / accent |
| abort / paste chip | `⏹` / `❰ ❱` | dim |
| context gauge | `used/limit` | plain text (e.g. `14.2k/272k`), flush-left on the foot (D023) |

## 3. Layout drafts

> **Historical.** Drafts A–E below predate the composer redesign (D1b/D4b) and
> the D29–D36 work: they show a flat status line and a `────` rule, whereas the
> shipped screen carries the chrome in a pinned **header** band (D010) and
> the composer's **foot line**, and has no status row or session row. Read **§4**
> for the shipped layout; the drafts are kept for the reasoning, not as current
> targets. Draft B's in-flight thinking streams expanded — **retired by D018**: the
> live thinking row is now the one-row `thinking <t>` annotation of §4.

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


   thinking  The tests use an edit(src, from, to) helper; I'll add a case with two
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
kinds: `User`, `Turn`, `Tool`, `Notice`, `Error`, `Btw`, `Diff`, `Todos`
(a command notice is a rendering twin of `Notice` with a shorter life — see
**Notices** below).
Each block computes its own height at draw time. A wrapped-line cache keyed by
`(revision, width)` is the P2 optimization (do not re-wrap static history every
frame). Follow-tail while streaming; scroll-lock when the user scrolls up (P1).
At startup the loop asks `GetHistory` and `App::seed_history` rebuilds the
transcript from it, so a resumed session (or a reconnecting socket client) opens
on its earlier turns instead of an empty pane — a dim `⋯ N earlier message(s)`
divider marks the replayed prefix. With **no** history the transcript opens on a
dim seeded hint (`❯ type a message · /help for commands · F1 for keys`), not a
blank pane.

- **Notices** (D018) — a dim row in the content column, no glyph of its own (the
  `⏹` of `⏹ aborted` and the `⋯` prefixes are table glyphs). The **lifetime
  depends on the origin**: **command chatter is transient** — the echoed
  `/<command>` line and every reply, usage-hint, and error line the
  `/`-command path emits (`/help`, `/usage`, `/copy`, `/verify`, `/tasks`,
  `/team roster`, …), plus the submit-time `a turn is already running` guard —
  all retired at the **next submit on that surface** (notices are per-surface;
  a retire never crosses surfaces). A transient row renders **exactly** like a
  `Notice`; retirement removes it, it is not restyled, and the browse selection
  lands on the same block. **Run events are permanent** —
  `⋯ compacted N messages`, `⋯ compaction skipped: <reason>`,
  `⋯ retrying (n/m)`, `⏹ aborted`, the run's changes summary, and the
  `⋯ N earlier message(s)` divider stay for the session: they are the record of
  what the run did. The seeded hint has its own retirement (`clear_hint` at the
  first block) and is in neither list.

- **Header** (D010; amended by D023) — a **pinned top band**, not a transcript
  block: ONE bare row, no rule, carrying the running
  head `WCODE · session <id> ──── project · ⎇ branch`. The only `─` is the
  **leader** between the head's own runs (the `WCODE`/`session` group on the left,
  `project · ⎇ branch   model · effort` on the right). `WCODE` is a plain
  `dim` label (the reverse-video run is retired, D023), then `· session <id>`;
  `project · ⎇ branch` reads left-to-right and `model · effort` is a **far-right
  group** on the same row (option A). It is **chrome** — never a browse target.
  At 48 cols the ladder sheds `session` first, then the branch.
- **Speaker head** — the **user block** opens with a ` YOU` head (replacing the
  transcript's `❯`) — a plain `dim` run (the reverse-video run is retired, D023;
  no glyph, no role). The **assistant turn carries no head** (D012): it opens
  directly on its body (the `thinking` row or the prose), so a turn does not spend
  a row on a bare `WCODE` marker — the assistant is the page itself. The head
  replaces the retired `§N` section head (`§` is no longer in the glyph table,
  §2). It is **chrome** — never a browse selection target, never copied.
- **Code plate** — a fenced code block renders a **caption line** above it,
  `plate N — <info>` (the fence's info string verbatim; just `plate N` when
  there is none), all `dim`. `N` is the fenced block's ordinal, **per message,
  monotonic**. **No disclosure** — the code always shows (the design's
  disclosure collapse is deferred). No new glyph (`plate`/`—`/the digits/`/` are
  ASCII; `─` is declared).
- **Tools (inline, flat)** (D009/T1; amended by D023) — a tool renders **inline,
  in call order**: at the **position it was called**, flowed with the prose. The
  render walks the turn's `content` and emits tool `k` at the `k`-th `ToolCall`;
  the inline tool at the call site is the locator (no reference mark — D021). A
  tool renders as **one flat row at col 3** — `name  <params inlined>  …  <note ·
  ms>` — the `note · ms` stats **right-aligned**; the tree (`├`/`│`/`└`) is
  retired and so is the disclosure/copy affix with its mouse click geometry
  (D023). **Expanded**, the head sits over its body (`key  value`
  params + output/diff + a `… +N more` hint), all at the same column. The name
  prints **once** — no ordinal, no separate `✓ name · note` row. The **`bash`
  command is shown in full** (it wraps, never clips). A tool's **params are never
  hidden**. **Collapsed**, they ride the head row: the first param's value prints
  **bare** (its key dropped, its syntax highlight kept), the rest as `key=value`;
  the head wraps on a col-3-aligned continuation when it overflows, so the `bash`
  command still shows in full. **Expanded**, they re-draw as `key  value`
  rows. `Ctrl-T` governs the tool's **output body** only
  (params are the call's identity and never hide). A tool error is
  forced expanded. `Ctrl-T` toggles every tool's output; browse
  `Enter`/`Space`/`y` act per tool. The `── notes ──` **foot ledger is removed**.
- **Thinking** (D33/D010; **amended by D018 and D023**) — **in flight** it is ONE
  dim annotation row, not an expansion: `   thinking 1.2s▌` —
  `thinking`, the live elapsed, and the live cursor `▌` closing the
  row, all at col 3 (the marker glyph is retired, D023). **No reasoning body, no
  affordance** — the live block is never a
  selection target (see **Browse mode**), so nothing expands while the model
  reasons. The elapsed is the run's own clock: `Surface::run_elapsed`
  (`App::run_elapsed`), injected every 120 ms by the run loop — the same
  injection as the running chip and the sidebar's focused row, so the renderer
  never reads a clock. While the live message's **last content block is
  thinking** the foot chip reads `⠹ thinking <t>` instead of `⠹ running <t>`
  (a word change, not a new glyph) and reverts to `running` at the first text
  or tool delta — "thinking is active" is exactly `content.last()` is
  `Thinking`, which the kernel already maintains by opening a fresh thinking
  block per reasoning stretch. On **commit** it collapses to the unchanged
  quiet one-line `thinking` row (no char count, no timer), expandable with
  `Enter` — the reasoning text is
  reachable only after the turn commits, and its body sits at col 3 (the flat
  cluster).
- **Todos** (D35) — the `todo` tool's checklist is ONE block that updates in
  place (`── todos  d/t ──` header, `☑`/`☐` rows), in the transcript — removed
  from the sidebar.
- **Diff / changes** (D36) — a `/changes` selection re-shows the file's diff;
  the run's changeset renders as a directory **tree** (`├─`/`└─`, stats
  right-aligned) in the sidebar and the `/changes` modal.
- **Affordances** (D32; **retired by D023**) — the disclosure/copy affix and its
  mouse click geometry are gone: a tool row and a thinking row are plain lines.
  Expansion/copy is **keyboard-only** — `Ctrl-T` toggles every tool's output, and
  browse `Enter`/`Space`/`y` act per block. Preserving that keyboard parity is the
  deliberate trade-off for dropping the click cells.

**Gutter** — 1 col margin, marker column, content at a fixed column (so wrapped
continuation lines align under the text). The
gutter is also the natural home for a `▌` selection bar. A **tool**'s flat row
sits at col 3 (no tree, D023); a thinking row is a plain `thinking` line at the
same column (D010/D023).

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
0. the **header** band (pinned, D010; D023) — ONE bare row (the running head),
   no rule, always visible;
1. the **transcript** (flex, plain, scrollable). Committed assistant messages render as
   markdown — headings, bullets, fenced code, aligned tables, inline
   `code`/`**bold**`. It renders in a **measure** — a content column, **default
   **68 cols** and **runtime-adjustable** (`/width <cols>`, or `Alt-[` / `Alt-]`;
   clamped to the band), **centered** in its band once the band reaches **84 cols**
   (a real margin each side); below 84 the content uses the full band width (so
   an 80-col terminal keeps its `❯` at the gutter). The **composer band is never
   centered** — it is the one piece of furniture that stays edge-to-edge. A
   **tool** is a flat row (params + body, at col 3, D023); a **thinking** block is
   a one-row annotation while live and a collapsed row until expanded once
   committed (D018);
2. the **team region** — 0..=3 rows, only `Running` teammates (the root is the
   orchestrator, excluded), in the **canonical order** (D34). It is a rider, not
   a band: it collapses to nothing when no teammate runs;
3. the **composer** — an **open writing line** (the input) with a **foot line**
   below (no rule — D023). The composer's **head row is dropped**
   (D009): the chrome it carried (`project`/`branch`/`session`, `model`/`effort`)
   now rides the pinned **header** band (above), and the composer keeps
   only the **foot line** — the plain `used/limit` text left and
   `⏻ plan · ▤ browse · ⏸ idle`/`⠹ running 3.1s`/`⠹ thinking 3.1s`/`⠹ btw…` · `↑ N` `<N>/<M>` right
   (the `btw…` state while a `/btw` side ask is in flight; `⠹ thinking <t>`
   replaces `running` while the live message reasons — D018; the **folio**
   `<N>/<M>` is the surface's turn count, `N == M`, so a first turn reads
   `1/1`). There is **no box and no corners**, and **no session row, no status
   row, and no head row**.
The context gauge is plain text — `used/limit` (e.g. `14.2k/272k`), all `dim`,
flush-left on the composer foot (no parallelograms, no colour; D023). The header
and composer-foot
titles clip with `…` then drop least-important-first (header: the session
id, then branch; foot: the `↑ N` scroll, then the gauge text, then
`⏻ plan`/`▤ browse`). The session id is shortened to 8 chars.

**Sidebar** — `Ctrl-B` docks a 30-col left panel (only when the terminal is
≥ 80 cols; below that the layout is untouched), **off by default** so the base
layout stays byte-identical while it is closed. It stacks **two** sections, each
headed by a **ruled `── label ─────…` line** (D011 — the book-outline style; the
`─` fill reaches the panel edge), a blank line setting the second off from the
first:
1. **`agents`** — the **root/wcode session is the FIRST row** (D018): its label
   (`wcode` — the header's `WCODE` label, lowercased), its state
   glyph, its live action, the `*` when focused, and the run elapsed on the
   focused row — but **no number badge**. Below it the members' **numbered**
   rows (`1 ● explorer · read a.rs`, `2 ○ developer`, …): a `muted` number
   badge (so `Alt-N` is visible), the state glyph, the label, the live action,
   and a `*` on the focused surface. The badge counts **members, not drawn
   rows**, so `Alt-N` still means member N. The root row is **clickable to
   focus the root** — every drawn row publishes its index for hit-testing, so
   no new hit geometry — and its keyboard parity is the existing
   `Ctrl-N`/`Shift-Tab` surface cycle (the root is the first surface in the
   canonical order); `Alt-N` never targets the root. Rows are in the
   **canonical order** (D34: root first, then members in creation order —
   stable; `●` marks activity, it does not reorder), which the sidebar now
   draws in full. The **`/team` roster and the composer's working-team strip
   stay member-only**: both answer "which teammate is doing what" under a
   three-row budget, while the root is the orchestrator — the surface the user
   is typing into, whose head already rides the pinned header band on every
   frame.
2. **`changes`** — the run's changeset as a directory **tree** (D36):
   `crates/wcode-tui/src/` then `├─ ui.rs  +8 −0` …; `+a` added, `−r` removed,
   stats right-aligned.
An empty section keeps its header with a dim `—`. With the root row the
`agents` section is never empty in a live app, but the placeholder is **kept
as a defensive branch** (D018): `draw_sidebar` stays total over whatever row
list it is handed, and the `changes` section keeps the identical shape — one
rule for both sections. Rows clip to the panel and never wrap. The modal
overlay floats over the whole terminal so it covers the panel. *(No `Todos` section — todos are a transcript block, D35 — and no
`Context` section — the gauge lives on the composer's foot line.)*

**Keys** — `Enter` submit · `Shift-Enter`/`Ctrl-J` newline · `Del` forward-delete · `Up`/`Down` history ·
`Ctrl-A`/`Ctrl-E` move to the start/end of the input · `Ctrl-W` delete the previous word,
`Ctrl-U`/`Ctrl-K` delete to the start/end of the current line (readline word editing on the
atom buffer; a paste chip is one unit, never split) ·
`PgUp`/`PgDn` page, the mouse wheel nudges (3 lines) — either scrolls the
transcript, `↑ N` on the composer's foot line while scrolled ·
`Esc`/`Ctrl-C` cancel a run, quit when idle · `Ctrl-Y` (`/copy`) copies the last
reply (OSC-52) · `Ctrl-T` expands/collapses every tool's **output** (params stay
visible — inlined on a collapsed tool's single row, as `key  value` rows when
expanded; a failed tool always shows its error) ·
`Ctrl-N`/`Shift-Tab` focus the next/previous surface in the **canonical order**, `Alt-1..9` focuses **member row N** (the numbered order, not the raw index; the unbadged root row above member `1` belongs to the surface cycle, not to `Alt-N`) · `Ctrl-B`
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
- **Palette**: one `theme.rs` of named roles (`Theme::colored` / `Theme::plain`);
  the default (palette B) is the **bare grey ramp with `error` red the only hue**
  (D023), while a preset (`nord`, `dark`, …) overrides every chromatic role.
  Per-tier colours (`Named`/`256`/`truecolor`, D30) and a `[theme]` / `/theme`
  overlay (P4, landed); live-reloadable (`static THEME: RwLock<Theme>`).
- **Markdown**: hand-rolled, applied live and committed (P1/P2). **Clipboard**:
  OSC-52, no `arboard` (P2).
- **Theme / colour capability**: **shipped** — the ladder (`resolve_color_mode_from`) picks `Plain`/`Named`/`Indexed`/`Rgb`, a hex is honoured only under `Rgb`, and the palette is live-reloadable (`static THEME: RwLock<Theme>`); see **D29**/**D30**.
- **Wire deltas**: P0 accepts whole-message `MessageUpdate`; the O(n²) matters
  only for a socket-backed TUI (already reachable via `--socket`) and is
  resolved before that path is a goal.

## 6. Open questions

- **Thinking** — **resolved (D33/D010); amended by D018 and D023**: in flight it is ONE dim `thinking <t>` annotation row with the live cursor (no body — the reasoning text is reachable only after the turn commits); on commit it collapses to a quiet one-line `thinking` row (no char count), expandable. Tool output is a flat row (D009/T1, D023), collapsible per block (browse `Enter`) or all at once (`Ctrl-T`).
- **Timestamps** on turns: lean no.
- **Header/title bar**: **amended by D010 and D023** — the header IS a pinned top
  band now (the running head as ONE bare row, no rule); the composer keeps its
  **foot line**, and `Ctrl-B` docks the sidebar for the rest.
- **Block separation**: blank between *roles* (kept); a tool is a flat row and a thinking block is a plain row, so "between every block" is moot.
- **Gutter vs flat**: gutter (drafted) — it is the main thing the TUI buys over
  the line loop. Revisit only if it costs width on 80-col terminals.
