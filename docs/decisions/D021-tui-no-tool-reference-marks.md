# D021 — No tool-reference marks; the inline tool at the call site is the locator

- **Status:** accepted (human sign-off — this session)
- **Date:** this session
- **Relates to:** D009 (inline tools at the call site; this removes the marks it
  kept), D010 (the tool tree row), D012 (the assistant has no speaker head)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) — the §2 glyph table
  (drops the `footnote reference` row) and §4 (**Tools**)
- **Amends:** [`D009`](D009-tui-book-header-and-inline-tools.md) — the
  "`¹²³` reference marks stay" clause

## Context

D009 rendered each tool **inline, at the call site**, and kept a superscript
reference mark (`¹²³…`, `[n]` for n ≥ 10) on the prose/thinking line where the
call was. The mark was a nod to the retired `── notes ──` foot ledger: a
pointer from the sentence to a tool row.

The human reads those marks as noise — *"number annotations after
thinking/message"* — and wants them gone. With the tools inline at the call
site, the tool row **is** the locator: the sentence that names a tool sits
immediately above it, and the mark adds nothing.

## Decision

1. **No reference marks are printed.** The TUI no longer appends `¹²³` (or `[n]`)
   after prose/thinking at a tool-call site.
2. **The inline tool at the call site is the locator.** A tool renders at the
   position it was called (D009, unchanged); no mark points to it.

## Mechanism

- `turn_inline` (`crates/wcode-tui/src/ui.rs`) — the mark push and the
  `tail_is_prose` gate are gone. The `refn` counter stays: it selects the tool
  (`turn.tools.get(refn - 1)`) and feeds the D32 offsets.
- `content_lines` — the `ToolCall` arm emits nothing (`=> {}`); its per-message
  `refn` is gone.
- `footnote_mark` and the `link()` style helper are deleted. The `theme.link`
  role stays (markdown links still use it, via `markdown.rs`).

## Consequences

- A turn reads as prose interleaved with its inline tools — no superscript
  digits after sentences or `··· thinking` rows.
- The D009 model is untouched: `Turn.content` still holds each `ToolCall` in
  place and `tools[k]` is its result.
- **Foreclosed:** re-introducing a printed reference mark; a config knob for it
  (behaviour is code, per `AGENTS.md`).
