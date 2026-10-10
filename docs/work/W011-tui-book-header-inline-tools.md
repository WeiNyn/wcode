# W011 — TUI book header + tools inline in call order

- **Status:** in progress — decisions locked ([D009](../decisions/D009-tui-book-header-and-inline-tools.md)), human sign-off given 2026-10-09
- **Depends on:** the in-flight `Block::Turn` refactor (uncommitted working tree)
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §2, §4
- **Design source:** [`docs/design/tui-turn-notes.md`](../design/tui-turn-notes.md) §10, §15, §17; [`docs/design/tui-tool-glyphs.md`](../design/tui-tool-glyphs.md) §3, §7

---

## 1. The ask

Give the TUI a **book header** — a scroll-away top block
`WCODE · session <id> ──── project · ⎇ branch` — and render a turn's **tools
inline, in call order**, instead of gathering them into a foot ledger. The
model is `docs/design/tui-turn-notes.md` §17 (header) and §10 **R3** (tools in
place), over the shipping `Block::Turn` refactor.

The **must-have** (the human's words): a tool must **follow call order** and must
**NOT** be put at the end of the turn.

## 2. Scope / Non-scope

### 2.1 In scope
- A **session-head block** at the transcript's top: the header row + a `─` rule on
  the measure, `WCODE` reverse-video.
- **Drop the composer head row**; keep the composer foot line.
- **`model · effort`** as the header's far-right group (D009 §3).
- **Inline tool rendering**: walk the turn's `content`; emit tool `k` at the
  `k`-th `ToolCall`. Remove the `── notes ──` foot rule and the foot-ledger path.
- **System A marks** where not already shipped (`»`/`✓`/`✗`, `▣`).
- Update the **34 drifted tests** and add tests for the header + inline order.

### 2.2 Non-scope
- The `Block::Turn` model itself (`content` + `tools` already suffice).
- The speaker heads (`YOU`/`WCODE`) — already shipped.
- The **drop cap** (`tui-turn-notes.md` §9) and **R1 margin** / **R2 foot ledger**.
- System B/C tool marks. The live-buffer reflow (tracker row 62).

## 3. Interface (integration points)

- **New block** `Block::SessionHead(… )` (or a `Turn`-independent head) — the
  transcript's first committed block; **chrome** (not a browse selection target).
  - `app.rs` `Block` enum; `ui.rs` `block_lines` (the `§`-row removal, `§2` amend);
    `copy_text` (a head copies nothing).
- **The header strings** come from `Status` (`model`, `effort`, `session`) +
  cwd/git — the same source the composer head row reads today
  (`crates/wcode-tui/src/ui.rs:1451` `corner_titles` top row).
- **Composer**: drop the head row (`ui.rs:1451`); keep the foot line
  (`ui.rs:1509`, the gauge/state/folio).
- **Inline tools**: replace `ledger_lines` (`crates/wcode-tui/src/ui.rs:818`) with
  a walk of `turn.content` that interleaves prose and per-`ToolCall` tool rows;
  reuse `note_head_row`/`tool_summary_row`/`tool_panel_body`.
- **D32 map**: the per-tool hit offsets become the inline rows' offsets (mirror
  the current `note_height` walk).

## 4. Plan (deliverable + gate per step)

1. **Spec amendment** (designer) — amend `docs/tui-design.md` §2 (the `⚙`→`»`,
  `⧉`→`▣` rows; the `§` row removal) and §4 (the session-head block; tools inline;
  the composer head-row drop). **Gate:** a reviewer's first-layer approval of the
  amendment + the sketch delta.
2. **Sketch delta** (sketcher) — a review-only block for the `session_head` block
  + the inline render signatures, grounded in `file:line`. **Gate:** first-layer
  review.
3. **Implement** (developer) — the header block, the composer head-row drop, the
  inline render, System A marks, the test updates + new tests. **Gate:**
  `cargo test --workspace` + `cargo clippy --workspace --all-targets` clean; a
  live `examples/dump` frame at 80/120/48.
4. **Verify** (reviewer) — second layer against this brief. **Gate:** APPROVE or
   a `file:line` blocker list.
5. **Record** — this file + the tracker row.

## 5. Quality gates

```
cargo test --workspace
cargo clippy --workspace --all-targets
cargo run -p wcode-tui --example dump    # frames in target/tui-render/
```

## 6. Automated + human testing

- **Automated:** a header renders at row 0 with the session/branch groups; no
  composer head row; a multi-round turn's tools render inline in call order (the
  traced §1 defect: a tool does **not** land at a later reply's foot); the `¹²³`
  marks align 1:1 with the inline tools.
- **Human:** the `examples/dump` frames at 80×24 / 120×40 / 48×20 read as the
  agreed header + inline tools; the 48-col ladder drops `session` then the branch.

## 7. Expected outcome

See D009 and the composed frame in the conversation: a scroll-away header
(`WCODE · session ──── project · ⎇ branch`, model far right), the `YOU`/`WCODE`
speaker heads, prose with `¹²³` marks, and each tool **inline right where it was
called**, in call order. Composer head row gone; foot line (gauge · state ·
folio) kept.
