# D009 — the TUI book header, and tools inline in call order

- **Status:** accepted (human sign-off, 2026-10-09)
- **Supersedes in part:** the foot-ledger preference in
  [`docs/design/tui-turn-notes.md`](../design/tui-turn-notes.md) §10 (R2) and §12.
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §2 (glyph rows), §4
  (Transcript: a session-head block; tools render **inline**; the `§N` section
  head → the shipped speaker head), §4 Layout (the composer head row is
  dropped), §1.1 and §6 (the chrome now rides the session-head block + the
  composer foot line).
- **Work item:** W011 — [`docs/work/W011-tui-book-header-inline-tools.md`](../work/W011-tui-book-header-inline-tools.md).

## Context

The turn model is one `Block::Turn` per exchange (`docs/sketches/turn-block.md`,
partly implemented in the working tree). Its head is a **speaker** (`WCODE`, §15
(b)) and its apparatus is currently gathered into a **foot ledger**
(`── notes ──` at the turn's end). Two gaps against what the human asked for:

1. **The header is absent.** The session/project/branch chrome still rides the
   **composer head row** (`crates/wcode-tui/src/ui.rs:1451` "Head line — always
   drawn"), and there is no session-head block.
2. **Tools pile at the turn's foot** — the human's must-have.

## Decision

1. **Book header — §17 H1, reading (i), single-rule.** A **scroll-away transcript
   block**, the session's first: `WCODE · session <id> ──── project · ⎇ branch`,
   one row + a `─` rule **on the measure**. `WCODE` is a reverse-video run
   (`Modifier::REVERSED`; no glyph, no role).
2. **The composer head row is dropped** — the composer keeps only the foot line
   (gauge · state · folio).
3. **`model · effort` rides the header's far-right group (option A).** This
   resolves §17's internal conflict: it recommends model-on-composer (B), but §17
   also drops the composer head row, so (B) has nowhere to live; (A) keeps the
   model id visible.
4. **Tools render inline, in call order** (overriding the foot ledger, R2). Each
   tool sits **at the position it was called**, flowed with the prose. Render:
   walk the turn's `content`; at the `k`-th `ToolCall`, emit the tool whose result
   is `tools[k]`. The **`¹²³` reference marks stay**.
5. **Tool marks: System A** (`docs/design/tui-tool-glyphs.md` §3, `:315`) — `»`
   (retires `⚙`), `✓`/`✗` kept; the shared `⧉`→`▣` fix (`:333`).
6. **No new glyph, no new `theme.rs` role.**

## Consequences

- `docs/tui-design.md` §2/§4 must be amended **first** (design-taste §7.3); the
  amended spec is the authority the sketch and the diff are reviewed against.
- The `Block::Turn` **model is unchanged** — `content` already holds the prose +
  each `ToolCall` in order and `tools[k]` is its result in call order, so the
  inline render is a **render-time** change (`ui.rs`), not a model change.
- The header is a new committed block **type** (a session-head), rendered once at
  the transcript's top.
- The retired `── notes ──` foot rule and the `Block::Notes`-shaped ledger go.
- The 34 drifted tests in `crates/wcode-tui` must be updated to the new model, and
  new tests added for the header + inline order.
