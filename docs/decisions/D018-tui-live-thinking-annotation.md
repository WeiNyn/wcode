# D018 — Live thinking is an annotation, the sidebar starts with the root row, command notices are transient

- **Status:** accepted (human sign-off 2026-10-10, this session — W014 slices b/c/d)
- **Date:** 2026-10-10
- **Relates to:** D033/D010 (in-flight thinking streams expanded — the line this
  amends), D011/D034 (the sidebar's two sections and canonical order), D032
  (affordances), D017 (the `/team roster` the root row must not leak into)
- **Amends:** `docs/tui-design.md` — the status line; §2's glyph table
  ("thinking (in flight) | `···` | dim, italic"); §3's draft-B note; §4
  (**Transcript**, a new **Notices** bullet, **Thinking**, the **Layout** foot
  ladder, **Sidebar**); §6's "Thinking — resolved (D33/D010)" line. One record
  covers all three because they agree: the live surface shows *status*, not
  content; the roster names the surface you are typing into; the scrollback
  keeps what *happened*, not what you asked for.

## Context

The human named three TUI defects in W014 (the fourth, the compaction window,
is W014a — a harness change with no presentation surface):

1. **In-flight thinking floods the transcript.** The live arm streams the
   reasoning body expanded — `ui.rs:941-942` (pre-W014d) `if live { /* In-flight:
   stream inline, expanded — as before D33. */ … }` — so a model that reasons for minutes
   buries the turn it belongs to, and the only honest number (how long have I
   been waiting) rides the composer foot.
2. **The sidebar has no root/wcode row to click.** The roster filters the root
   out — `app.rs:3280` `.filter(|&i| !self.surfaces[i].is_root)` — although the
   D34 canonical order already puts the root first, every drawn row publishes
   its index for hit-testing (`ui.rs:2158` `app.set_sidebar_hit(area, members);`),
   and the lookup is index-agnostic (`app.rs:3267` `hit.members.iter().find(|(_, y)| *y == row).map(|(idx, _)| *idx)`).
   The one row a user most wants to click is the one row the roster filters out.
3. **Command chatter never leaves.** Every `/`-command echo is a permanent
   `Notice` — `app.rs:3450` (pre-W014c) `self.notice(line);` in `fn command` —
   rendered by `ui.rs:692` `Block::Notice(text) => wrap(text, width, "   ", "   ", dim())`.
   A `/usage` reply read once sits in the scrollback forever.

## Decision

### 1. In-flight thinking is one dim annotation row (W014d)

- While the model reasons, the live thinking block renders as **one row** —
  `   ··· thinking 1.2s▌` — the `···` gutter marker, `thinking`, the live
  elapsed, and the live cursor closing the row. **No reasoning body, no
  affordance** (the live block is never a selection target), so nothing expands
  in flight; the body is reachable only after the turn commits.
- The elapsed is the clock the TUI already has: `Surface::run_elapsed`
  (`app.rs:4514` `pub fn run_elapsed(&self) -> Option<std::time::Duration> { self.focused().run_elapsed }`),
  injected every 120 ms by the run loop (`lib.rs:274` `const TICK: Duration = Duration::from_millis(120);`,
  `lib.rs:565` `app.set_run_elapsed(id, started.elapsed());`) — the same
  injection the running chip and the sidebar's focused row already read. No new
  clock; the renderer stays pure (the elapsed is a parameter, like `width`).
- The composer-foot state chip reads `⠹ thinking <t>` instead of
  `⠹ running <t>` while the live message's last content block is `Thinking`;
  it reverts at the first text/tool delta. A **word change, not a new glyph** —
  `⠹` is already the running glyph (`ui.rs:1798`).
- "Thinking is active" needs no kernel change: the kernel already separates the
  deltas (`loop_.rs:259` `Some(LlmStreamEvent::ThinkingDelta(delta)) => { append_thinking(&mut content, &delta); … }`
  vs `:247` `TextDelta => append_text(...)`), so `content.last()` is `Thinking`
  exactly while the model reasons.
- Committed thinking is **unchanged**: the collapsed `··· thinking` row, `Enter`
  expands.
- `set_run_elapsed` must bump the surface's `live_rev`, or the cached live
  render (`app.rs:1390` `let stale = self.live_cache.rev != self.live_rev || self.live_cache.width != width;`)
  freezes the timer between deltas.

### 2. The sidebar's `agents` section starts with the root/wcode session (W014b)

- Row 0 is the root: its label (`wcode` — the header's reverse-video `WCODE`
  run, lowercased), state glyph, live action, `*` when focused, the focused
  row's run elapsed — and **no number badge**. The badge counts **members, not
  drawn rows**, so members keep `1..N` and `Alt-N` still means member N; the
  root row's keyboard parity is the existing `Ctrl-N`/`Shift-Tab` surface cycle
  (the root is the first surface in the canonical order). Clicking the row
  focuses the root through the already-published hit geometry.
- The **`/team` roster and the composer's working-team strip stay member-only**:
  both answer "which teammate is doing what" under a three-row budget, while
  the root is the orchestrator the user is typing into, whose head already
  rides the pinned header band every frame.
- The empty-section `—` placeholder is **kept as a defensive branch**: with a
  root row `agents` never renders empty in a live app, but `draw_sidebar` stays
  total over its input and the `changes` section keeps the same shape.

### 3. Command chatter is transient; run events are permanent (W014c)

- **Transient**, retired at the next submit on that surface (per-surface; a
  retire never crosses surfaces): the echoed `/<command>` line and every reply,
  usage-hint, and error line the `/`-command path emits (`/help`, `/usage`,
  `/copy`, `/verify`, `/tasks`, `/team roster`, …), plus the submit-time
  `a turn is already running` guard — a rejected submit is chatter, not a run
  event. A transient row renders exactly like a `Notice`; retirement removes
  it and clamps the browse selection onto the same block.
- **Permanent**: `⋯ compacted N messages`, `⋯ compaction skipped: <reason>`,
  `⋯ retrying (n/m)`, `⏹ aborted`, the run's changes summary, and the
  `⋯ N earlier message(s)` divider — the record of what the run did, so a
  later reader can still see why the context reset or the run died. The seeded
  hint keeps its own `clear_hint` retirement.

## Mechanism

- `sidebar_rows_indexed()` (`app.rs:3330`) — a sibling of `member_rows_indexed` (`app.rs:3277`)
  that does not filter `is_root`; `member_rows`/`member_rows_indexed` keep
  their non-root contract, so `/team` and the strip are untouched.
- `Block::Transient(String)` (`app.rs:190`, the `Block` enum) — a rendering
  twin of `Notice` — and `Surface::retire_transient()`, the `clear_hint`
  splice (`app.rs:1471`) across all three parallel vecs, plus the browse
  `selected` clamp.
- `live_lines`/`content_lines` gain `Option<Duration>` (`ui.rs:918`/`:936`),
  threaded from `append_live_lines` (`app.rs:1379`); `corner_titles`
  (`ui.rs:1789`) picks *running*/*thinking* from `content.last()`.

## Consequences

- A reasoning model's turn reads as: one dim `··· thinking 1.2s▌` row, then the
  answer; the reasoning text is one `Enter` away after commit.
- The sidebar's `agents` section always has the root row; member badges stay
  `1..N`, so `alt_n_focuses_the_numbered_sidebar_row` (`app.rs:7535`) survives
  unchanged — its comment ("the FIRST MEMBER ... not `surfaces[0]`, the root")
  stays true.
- A transcript holds at most one generation of command chatter; the
  compaction/abort records persist.
- **Foreclosed**: a live-thinking expand/peek (the live row is never a target);
  a root row in `/team` or the strip; a new spinner glyph, a new clock, or a
  per-thinking timer.
- Gates live in W014 §7.1 (the named tests) and §7.2 (headless `dump` frames
  plus a keyless local endpoint); the doc diff names every anchor above.
