# wcode TUI — design (visual spec)

Status: **draft, agreed**. Companion to [`tui-plan.md`](tui-plan.md) (the
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
3. **A small palette, dim is the workhorse.** Accent (cyan) for the user prompt
   and running state; red for errors; yellow for inline code; a green→yellow→red
   gauge for context fill. Everything else is default-fg or dim. Honor
   `NO_COLOR` (styles collapse to bold/dim); degrade to 256-color when
   truecolor is absent.
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
replayed prefix.

**Gutter** — 1 col margin, marker column, content at a fixed column (so wrapped
continuation lines align under the text, as in draft B's `···` block). The
gutter is also the natural home for a `▌` selection bar (P3, copy).

**Input** — `❯ ` prefix, block cursor `▌`. One line, growing with its content
(Shift-Enter inserts a newline; the band is capped at 6 lines). Up/Down recall
prompts, saving the draft and restoring it at the bottom; history persists one
prompt per line beside the sessions. (Shift-Enter needs a terminal that reports
the modifier — kitty/xterm-`modifyOtherKeys`; elsewhere it is Enter.)

**Status** — one dim "chrome" row, full width, left-aligned:
`model · effort · tokens · session · state`, where tokens is an 8-cell gauge
colored by fill (green/yellow/red) plus `used / limit` (e.g. `██████░░ 150k / 200k`,
shown once a turn has reported usage) and state is `⏸ idle` / `⠹ running`. The
state glyph is the single source of "am I running"; the spinner also rides the
active tool line, so a long tool never looks frozen. Committed assistant
messages are rendered as markdown (headings, bullets, fenced code, tables with
alignment, inline `code`/`**bold**`), live and committed; long words and table
cells wrap rather than overflow.
When narrow, fields drop least-important-first: **session, then effort, then
tokens** (model and state always stay). The session id is shortened to 8 chars.

**Keys** — `Enter` submit · `Shift-Enter` newline · `Up`/`Down` history ·
`PgUp`/`PgDn` page, the mouse wheel nudges (3 lines) — either scrolls the
transcript, `↑ N` in the status while scrolled ·
`Esc`/`Ctrl-C` cancel a run, quit when idle · `Ctrl-Y` (`/copy`) copies the last
reply (OSC-52). `/changes` lists the files the current run changed (`path · +a −r`)
in the overlay; selecting one re-shows its diff in the transcript. The changeset
is per-run and ephemeral — durable history is git's.

## 5. Decisions

Agreed for P0 (see also `tui-plan.md` §9):

- **Data source**: a `wcode-protocol` [`Backend`](../crates/wcode-protocol/src/backend.rs)
  (`Local`/`Remote`), never `Agent` directly. Local↔remote becomes a transport
  swap, and the TUI is already the jcode architecture minus the work.
- **Crate**: new `wcode-tui`, depending on `wcode-protocol`.
- **Versions**: `ratatui` 0.30, `crossterm` 0.29 (jcode's; both exist).
- **Alt-screen**: yes, with a custom scrollback (P1).
- **Palette**: accent + red only at P0; theme detection at P2.
- **Markdown**: hand-rolled, applied live and committed (P1/P2). **Clipboard**:
  OSC-52, no `arboard` (P2).
- **Theme detection** (truecolor via `COLORTERM`): still open.
- **Wire deltas**: P0 accepts whole-message `MessageUpdate`; the O(n²) matters
  only for a socket-backed TUI (already reachable via `--socket`) and is
  resolved before that path is a goal.

## 6. Open questions

- **Thinking**: inline (drafted) vs a collapsed one-liner `⋯ thinking · N chars`,
  expandable. Lean inline at P0, collapsible at P1.
- **Timestamps** on turns: lean no.
- **Header/title bar** (session, cwd): lean no — the status line carries it.
- **Block separation**: blank between *roles* (drafted) vs between every block.
- **Gutter vs flat**: gutter (drafted) — it is the main thing the TUI buys over
  the line loop. Revisit only if it costs width on 80-col terminals.
