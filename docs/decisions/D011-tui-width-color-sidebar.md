# D011 — TUI: colour under 256, an adjustable measure, a book-outline sidebar

- **Status:** accepted (human sign-off 2026-10-10), shipped in W013
- **Amends:** [`docs/tui-design.md`](../tui-design.md) §1.3, §4 (Layout measure,
  Sidebar, Tools)
- **Extends:** D010 (the pinned header / tool tree)

## Context

Five follow-ups after the D010 render redesign: the user's prompt and the
assistant's reply shared one colour; tool input/output rendered flat; the
transcript width was a hard const; the sidebar read as unstyled words; and the
syntax highlighter only fired under truecolor.

## Decision

1. **The user prompt reads in the accent role** (`Theme.user`), the assistant's
   prose in `body` — §1.3 already specified this; the role simply had no call
   site.
2. **The measure is runtime state** — `App.measure`, default 68, clamped
   `40..=200` and to the band at draw time, driven by `/width <cols>` and
   `Alt-[` / `Alt-]` (step 8).
3. **Code colorizes under the 256 tier too** — a syntect token's RGB maps to the
   nearest xterm-256 index (the 6×6×6 cube or the 24-step gray ramp). `Plain`/
   `Named` still fall back to the uniform code style.
4. **Tool input and output are highlighted** — a file tool's body/params by the
   file's extension, `bash`'s `command` param as shell, via a prefix-agnostic
   `markdown::CodeHighlight`; each line is char-exact wrapped under the tree.
5. **The sidebar is a book outline** — each section is headed by a ruled
   `── label ─────…` line (the fill reaching the panel edge) and a blank line
   separates the sections.

## Consequences

- `App` gains a `measure` field; `ui::draw_transcript` reads it instead of a
  const. `/width` joins the command table and `Alt-[`/`Alt-]` the keymap.
- `markdown::CodeHighlight` is the reusable per-line tokenizer (the fenced
  `Fenced` type keeps its gutter).
- `ui::wrap_styled` wraps styled spans (coalescing runs), used by the tool body
  and params.
- No new glyph; `NO_COLOR` still yields `Theme::plain`.
