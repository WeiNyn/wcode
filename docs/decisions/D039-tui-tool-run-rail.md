# D039 — A run of continuous tool calls carries a rail

- **Status:** accepted (human sign-off; design fixed before implementation)
- **Date:** this session
- **Relates to:** [`ui.rs`](../../crates/wcode-tui/src/ui.rs) (`turn_inline`,
  `run_membership`, the `gutter` threaded through every tool row),
  [`tui-design.md`](../tui-design.md) §2 (the glyph table) and §4 (**Tools**,
  **Gutter**)
- **Complement to:** [`D038`](D038-tui-one-row-tool-head.md) — D038 makes a head
  one row, D039 makes a batch of heads legible as a batch
- **Distinct from:** [`D023`](D023-tui-prose-only-bare.md) — the **retired tree**
  (`├`/`│`/`└`) is not coming back; see *What this is not* below

## Context

Tools render flat and inline in call order (D009/D021/D023): tool `k` prints at
the `k`-th `ToolCall`. That is the right model, but a **batch** of calls has no
form. An assistant that emits eight `read`s in one message produces eight
identical three-space-indented rows; nothing on screen distinguishes "one tool"
from "eight tools of one batch", or says where a batch stops. The per-row
information is complete; the *grouping* is missing.

The retired tree expressed grouping through **hierarchy** (a call's output nested
under it). That is the part D023 removed and this decision does not restore: the
rows stay flat and non-nested.

## Decision

1. **A run is a maximal span of ≥2 consecutive `ToolCall` blocks** in a turn's
   `content`. A `Text` block breaks the span; the turn's end breaks it.

2. **A `Thinking` block inside a run is TRANSPARENT.** It neither breaks the span
   nor stands outside it: the call before it and the call after it are the same
   run, and the thinking block's own row (+ its expanded body) carries the rail.
   That holds at a run's **edge** too: `[Thinking, call, call]` is one run and the
   leading thinking row is railed — a thinking row is never the thing that decides
   the span, only the calls are.

3. **The run's rows carry ` │ ` at the gutter** — the `│` at **col 1**, in place
   of the three-space indent's blanks. The bar is **col 1 only**: content stays at
   **col 3** on every row, so no content column moves and browse's `▌` at col 0
   coexists.

4. **A lone tool is byte-identical to its D038 rendering.** One call — or two
   calls separated by prose — gets no rail. The rail is a signal, so it must not
   appear where there is nothing to signal.

5. **Membership is computed from the BLOCKS, never from `turn.tools`.** A
   `ToolCall` whose result has not landed yet emits no panel (`ui.rs:1109-1111`,
   *"no panel is emitted for it"*); keying the rail off the landed tools would
   make the rail appear and shift on arrival.

## What this is not

**Not the D023 tree.** The tree expressed hierarchy — a call *owned* its output,
and `├`/`└` drew that ownership. The rail expresses **batch continuity** only: it
says "these calls arrived as one batch" and nothing about scope. It never nests,
never branches, never terminates with a corner glyph, and an expanded tool's body
carries the same single bar as its head. `│` is already declared in the glyph
table for code fences and block quotes; the rail reuses it, so no new mark enters
the vocabulary.

## Mechanism

- `turn_inline` (`ui.rs:1090`) precomputes `run_membership(&turn.content) ->
  Vec<bool>` (`ui.rs:1129`) in one pass over the blocks (a running list of the
  current span's indices plus a call count; a `Text` block or the end flushes it,
  marking the span iff it carried ≥2 calls), then picks the gutter per block:
  `ui.rs:1098` `let gutter = if rail[i] { TOOL_RAIL } else { TOOL_INDENT };`.
- The gutter (`TOOL_RAIL` = ` │ `, `ui.rs:43`) is threaded as a `&str` prefix
  (` │ ` or `   `) down every row builder of a tool: the collapsed head, its live
  tail, the expanded head, the params block (including a wrapped param's
  continuation rows), the body rows, the `… +N more` hint, and the thinking row +
  body.
- `Block::Tool` (a standalone/replayed tool, no turn) passes the plain indent: it
  cannot be in a run.
- `copy_text` is untouched — it reads `Tool`/`Turn` fields, never rendered lines,
  so the clipboard gains no `│`.

## Consequences

- An 8-call batch reads as one band; the eye can skip it or enter it.
- Two calls split by a sentence stay visually separate — prose is the batch
  boundary, exactly as it is in the model's own message.
- The rail costs **zero** content columns: col 3 is unchanged, so every existing
  column assertion still holds and a lone tool's rendering is unchanged
  byte-for-byte.
- **Foreclosed:** colouring or numbering a run (`A`/`B` tokens, a run index); a
  rail that continues through prose; a rail on the collapsed head of a tool whose
  result never lands (there is no row to carry it).
- Gate: `cargo test --workspace` green · `cargo clippy --workspace --all-targets`
  clean · a live `dump` frame showing two contiguous calls railed, a lone call not.
