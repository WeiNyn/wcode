# D010 — TUI: a pinned header and a tool tree

- **Status:** accepted (human sign-off 2026-10-10), shipped in W012
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §1.1, §4 (header, tools,
  thinking), §5 (the "no title bar" decision)
- **Supersedes:** D009's **scroll-away** session head (D009's *inline-tools* half
  stands)

## Context

The book header (D009) was a scroll-away transcript block, and the spec decided
against a title bar (`tui-design.md`). Three rendering complaints drove a
redesign: the header scrolled out of sight, a tool printed its name twice (a
`1 » name` head plus a separate `✓ name` row), and the message / thinking / tool
gutters drifted across three columns so a prompt never lined up with its reply.

## Decision

1. **The header is a pinned top band** — the running head
   `WCODE · session <id> ──── project · ⎇ branch` over a full-width `─` rule,
   above the transcript. It is **chrome**, never a browse target.
2. **A tool is one tree row** (`T1`): `├ name  target` with the `note · ms` (or
   `+a −r · ms`) stats and the `▸`/`▣` affordances, over its body (`│   ` params
   plus output/diff/preview plus a `… +N more` hint), closed by a `└` terminator
   when it has a body. The name prints **once** — no ordinal, no separate `✓`
   row. The glyphs reuse the changes-tree set (`├ └ │`), so §2's table is
   unchanged.
3. **One content column** (7) for the message body, the thinking body, and the
   tool body; the block markers (`···`, `├`, `│`) share a gutter column (3).
4. **Thinking** collapses to a quiet `··· thinking` row (the `N chars` count is
   dropped).

## Consequences

- `Block::SessionHead` and `sync_session_head` are retired; the header renders in
  `draw`'s band stack, so the transcript no longer carries a chrome block.
- The markdown `GUTTER`, the speaker heads, and the tool/thinking prefixes all
  key off one `CONTENT_COL`.
- The 48-col ladder still holds; `NO_COLOR` still yields `Theme::plain`; no new
  glyph was added.
