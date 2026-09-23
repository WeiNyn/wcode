# wcode TUI — design (visual spec)

Status: **agreed; implemented through P3**. Companion to [`tui-plan.md`](tui-plan.md) (the
architecture + phasing) and [`interface-protocol-brainstorm.md`](interface-protocol-brainstorm.md)
(the `Backend` seam the TUI is a client of). This doc is the *look*: bands,
glyphs, colors, and the mockups each phase should hit.

The REPL already has a vocabulary worth keeping — `❯` user, `⚙` tool start,
`✓`/`✗` tool end, `···` thinking, `⋯` compaction/retry, "dim = secondary". The
TUI reads as the same product, given a full screen.

## 1. Principles

1. **Three bands, always.** Transcript (flex) → input → status. The only fixed
   structure. A palette, side panel, or diff is an *overlay* or an addition —
   never a reflow of the base.
2. **Role lives in the left gutter.** A 1-col margin, a marker column, content
   at a fixed column. The gutter is what makes a scrollback readable at a
   glance; wrapped continuation lines align under it.
3. **A small palette, dim is the workhorse.** The roles live in one place —
   `theme.rs` (`accent, dim, muted, border, user, body, error, success, warn,
   code, heading, link, tool_name, thinking, diff_add, diff_del`) — named ANSI
   colors only. Accent (cyan) for the user prompt and running state; red for
   errors; green for success; yellow for inline code; a green→yellow→red gauge
   for context fill. Assistant prose stays default and dim stays the workhorse.
   Honor `NO_COLOR` (every role drops its `fg`, keeping only bold/italic/dim);
   **truecolor / 256-color detection is still open** (§5).
4. **Blocks are separated by a blank line between roles** (tools cluster tight
   under their `⚙`). Noise is dim; only the model's prose and *your* prompt are
   full-strength.

## 2. Vocabulary

| element | glyph | style |
|---|---|---|
| user prompt | `❯` | accent, bold |
| assistant prose | — | default fg |
| thinking | `···` | dim, italic |
| tool start | `⚙ name  args` | dim |
| tool done | `✓ name · note` | dim (green-dim ok) |
| tool error | `✗ name · note` | red |
| compaction / retry | `⋯` | dim |
| live cursor | `▌` | accent, steady |
| selection bar (browse) | `▌` | accent |
| mode indicator (browse) | `▤ browse` | accent |
| spinner (running) | `⠋⠙⠹⠸…` | accent |
| status separators | `·` | dim |
| inline code / code block | `` ` `` | yellow (bold under `NO_COLOR`) |
| table | `│ ─ ┼` | header bold; columns aligned, cells wrap |
| context gauge | `█░` | green → yellow → red by fill |

## 3. Layout drafts

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

**Transcript** — `Vec<Block>` of committed blocks plus one live block. Block
kinds: `User`, `Assistant`, `Thinking`, `Tool`, `Notice`. Each block computes
its own height at draw time. A wrapped-line cache keyed by `(revision, width)`
is the P2 optimization (do not re-wrap static history every frame). Follow-tail
while streaming; scroll-lock when the user scrolls up (P1). At startup the loop
asks `GetHistory` and `App::seed_history` rebuilds the transcript from it, so a
resumed session (or a reconnecting socket client) opens on its earlier turns
instead of an empty pane — a dim `⋯ N earlier message(s)` divider marks the
replayed prefix. A completed tool line carries its wall-clock duration —
`✓ read · 128 lines · 12ms` — from `ToolExecutionEnd::duration_ms` (UI-only,
like the diff and path).

**Gutter** — 1 col margin, marker column, content at a fixed column (so wrapped
continuation lines align under the text, as in draft B's `···` block). The
gutter is also the natural home for a `▌` selection bar.

**Browse mode** — `Ctrl-G` moves a `▌` selection over the committed transcript.
The bar occupies the gutter's first column (the 1-col margin), *replacing* the
blank there so no glyph shifts; a `▤ browse` token rides the status band just
before `state`. Movement is `j`/`k`, `g`/`G`, `PgUp`/`PgDn`; `Esc`/`q`/`Ctrl-G`
leave — and in browse `Esc` leaves the mode, it does **not** cancel or quit.
**Browse owns the text and navigation keys**; `F1` / `?` (help) and `Ctrl-C`
(cancel / quit) stay **global**, and every other key is ignored while browsing
(so the composer cannot be edited behind the mode). The live block is never a
target (it is transient). Phase 1 selects and
reveals only; expand / copy / `$PAGER` actions are later phases.

**Input** — `❯ ` prefix on the first row, block cursor `▌`. Wraps and grows with
its content (Shift-Enter or Ctrl-J inserts a newline); the band grows to ~8 rows
then scrolls to keep the cursor visible. A paste over 100 chars or more than 3
lines collapses to a dim chip `❰ pasted 12 lines · 340 chars ❱` — atomic
(Backspace removes it whole) and expanded on submit (outer whitespace trimmed). Up/Down recall prompts, saving
the draft and restoring it at the bottom; history persists one prompt per line
beside the sessions. (Shift-Enter needs a terminal that reports the modifier —
kitty/xterm-`modifyOtherKeys`; Ctrl-J is the portable newline.)

**Layout** — top to bottom (the sidebar, `Ctrl-B`, is an optional left column):
1. a dim **session line** (`session a1b2c3d4`) — collapses to zero rows when there is no session;
2. the **transcript** (flex, plain, scrollable; committed assistant messages render as markdown — headings, bullets, fenced code, aligned tables, inline `code`/`**bold**`);
3. the **team region** — 0..=3 rows, only `Running` teammates (the root is the orchestrator, excluded), ordered oldest→newest so the **latest event is the bottom row**;
4. the **input box** — a rounded border whose four corners carry the chrome: `project ⎇ branch` top-left, `model · effort` top-right, the context gauge bottom-left, and `[⏻ plan] · [▤ browse] · ⏸ idle`/`⠹ running 3.1s` · `[↑ N]` bottom-right.
The context gauge is 8 parallelograms (`▰` filled, `▱` empty) colored green→yellow→red by fill, then `used / limit`. Corner titles clip with `…` then drop least-important-first (top: branch; bottom: the gauge). The session id is shortened to 8 chars.

**Sidebar** — `Ctrl-B` docks a 30-col left panel (only when the terminal is
≥ 80 cols; below that the layout is untouched), **off by default** so the base
layout stays byte-identical while it is closed. It stacks four dim-headed
sections: **Team** (each member's state glyph, label, a `*` on the focused
surface, and its live action), **Todos** (`☑`/`☐` + text, header `done/total`),
**Changes** (`path · +added −removed`), and **Context** (the gauge). An empty
section keeps its header with a dim `—`; rows clip to the panel and never wrap.
The modal overlay floats over the whole terminal so it covers the panel.

**Keys** — `Enter` submit · `Shift-Enter`/`Ctrl-J` newline · `Del` forward-delete · `Up`/`Down` history ·
`Ctrl-A`/`Ctrl-E` move to the start/end of the input · `Ctrl-W` delete the previous word,
`Ctrl-U`/`Ctrl-K` delete to the start/end of the current line (readline word editing on the
atom buffer; a paste chip is one unit, never split) ·
`PgUp`/`PgDn` page, the mouse wheel nudges (3 lines) — either scrolls the
transcript, `↑ N` in the status while scrolled ·
`Esc`/`Ctrl-C` cancel a run, quit when idle · `Ctrl-Y` (`/copy`) copies the last
reply (OSC-52) · `Ctrl-T` expands/collapses every tool's output (a collapsed tool
shows a 4-line preview, a failed tool always shows its error) ·
`Ctrl-N`/`Shift-Tab` focus the next/previous surface, `Alt-1..9` jumps to the Nth · `Ctrl-B`
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
  named ANSI colors only; not yet configurable (that is P4).
- **Markdown**: hand-rolled, applied live and committed (P1/P2). **Clipboard**:
  OSC-52, no `arboard` (P2).
- **Theme detection** (truecolor via `COLORTERM`): still open.
- **Wire deltas**: P0 accepts whole-message `MessageUpdate`; the O(n²) matters
  only for a socket-backed TUI (already reachable via `--socket`) and is
  resolved before that path is a goal.

## 6. Open questions

- **Thinking**: inline (drafted) vs a collapsed one-liner `⋯ thinking · N chars`,
  expandable. Lean inline at P0; thinking still stays inline. Tool *output* is now
  collapsible (`Ctrl-T`, or one block at a time in browse mode) — see [`tui-polish-plan.md`](tui-polish-plan.md).
- **Timestamps** on turns: lean no.
- **Header/title bar**: decided **no** — the status line carries session/cwd/branch, and `Ctrl-B` docks the sidebar for the rest.
- **Block separation**: blank between *roles* (drafted) vs between every block.
- **Gutter vs flat**: gutter (drafted) — it is the main thing the TUI buys over
  the line loop. Revisit only if it costs width on 80-col terminals.
