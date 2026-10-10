# W012 — TUI: pinned header + transcript render (tools · thinking · messages)

- **Status:** implemented — 3 commits (`2d9e501` one content column · `fa20066` pinned header · `903dbdd` tool tree/thinking/rule spacing) + D010; second-layer review pending
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §1, §2, §4 + the Decisions list (this is a *redesign*, not a restyle)
- **Recorded as:** D010 (pending) under `docs/decisions/`
- **Design source:** the four proposals agreed in-session (H1 / T1 / `···` kept / one content column)

---

## 1. The ask

Four things the human named as "ugly":
1. **Header** — not pinned, and weak.
2. **Tool calls** — awkward.
3. **Thinking** — awkward.
4. **Messages** — awkward.

This is a **transcript-rendering redesign**, not a palette pass (the palette was
already the subject of D29/D30).

## 2. Scope / Non-scope

### 2.1 In scope
- **H1 — pin the header.** The session head moves from a scroll-away transcript
  block (`session_head_lines` `ui.rs:733`) to a fixed top band in `draw`
  (`ui.rs:142`). Content unchanged.
- **T1 — one tree row per tool.** `├ name  target … stats ▸ ▣`, name printed
  once; retire the `1` ordinal and the duplicate `✓ name · note · ms` row. Keep
  `▸`/`▾`/`▣`, their hit columns, error-forced-expanded, `Ctrl-T`.
- **One content column.** Fix the col-3 / col-6 / col-8 gutter drift so a prompt
  and its reply align.
- **Thinking.** Keep the declared `···` glyph (no new glyph); quiet collapsed row.

### 2.2 Non-scope
- The palette (D29/D30) — already shipped.
- The kernel (`crates/wcode-harness`) — untouched.
- New glyphs (the `┆` in the mock is illustrative only).
- The sidebar, overlays, browse model, input band.

## 3. Interface (integration points)

- Bands: `draw` `ui.rs:142` (`Layout::vertical`).
- Session head: `session_head_lines` `ui.rs:733`, `sync_session_head` `app.rs:4188`, `Block::SessionHead` `app.rs:194`.
- Tool rows: `note_head_row` `ui.rs:970`, `tool_summary_row` `ui.rs:1160`, `note_affordance_cols` `ui.rs:993`.
- Speaker/message gutter: `speaker_lines` `ui.rs:704`, markdown `GUTTER = "   "` `markdown.rs:20`.
- Thinking: `thinking_header_line` `ui.rs:1018`.

## 4. Plan

1. Pin the header (band 0); retire `Block::SessionHead`. Amend spec + D010.
2. Tool tree row (T1). Update affordance hit map + tests.
3. Unify the message gutter.
4. Tidy the thinking row.

Each step: `cargo test --workspace` + `cargo clippy --workspace --all-targets`.

## 5. Quality gates

```
cargo test --workspace
cargo clippy --workspace --all-targets
```
Plus a live check against a keyless endpoint (`--base-url http://localhost:11434/v1`)
where available; the TUI itself needs a TTY (not scriptable here).

## 6. Expected outcome

The target screen (acceptance):

```
 ⎇ main · claude-sonnet-4 · high                                 ⏻ plan  ▤ browse
──────────────────────────────────────────────────────────────────────────────
 you │ how does edit resolve an anchor?
 wcode │ An anchor is a 5-char hash of a line's raw content, so a line's
       │ address includes its indentation.
       ├ read  crates/wcode-cli/src/tools/edit.rs      128 lines · 12ms   ▸ ▣
       │   An anchor is a 5-char hash of a line's raw…
       │   … +12 more lines
       ··· thinking                                                    ▸ ▣
 ❯ ▌
 ──────────────────────────────────────────────────────────────────────────────
 ▰▰▱▱▱▱▱▱ 14.2k / 272k                                                     1/1
```
