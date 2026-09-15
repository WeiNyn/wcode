# wcode TUI — input box: wrapping & paste (plan)

Status: **Phase 1 (wrap) and Phase 2 (paste) landed.** Companion to
[`tui-plan.md`](tui-plan.md) (the TUI project) and [`tui-design.md`](tui-design.md)
(the visual spec). This is the *input band*: how the user composes a message.

## Problem

Two defects in the input box, both about content that does not fit.

1. **No wrapping.** `draw_input` (`crates/wcode-tui/src/ui.rs`) renders one
   `Line` per logical line and never wraps, so a logical line wider than the
   band is **truncated at the right edge**. The band height is
   `('\n' count + 1).clamp(1, 6)` (`ui.rs:27`), so **more than six logical lines
   hides the cursor line** entirely. The band has no scroll offset either.
2. **Paste is verbatim.** A paste arrives as one `Event::Paste` and is inserted
   in full (`AppEvent::Paste → insert_str`). A large paste floods the box (then
   gets clipped per 1) with no sense of scale.

**Example.** Pasting a 200-line stack trace fills the (clipped) box; typing a
long one-line prompt hides everything past the terminal width. In both cases the
user cannot see what they are about to send.

## Design

### Phase 1 — wrap + visible multiline

- **Char-exact wrap** at `width − 3` (the gutter), preserving every space and
  hard-breaking an over-long token. (The existing `greedy_wrap` is
  whitespace-word-based — it collapses runs of spaces and is unusable for an
  editor.)
- **Height by visual rows**, not `\n` count: `rows.clamp(1, MAX)` where
  `MAX = 8.min(area.height / 2).max(1)`. When rows exceed `MAX`, **scroll the
  band** so the cursor row stays visible.
- **Gutter**: ` ❯ ` on the **first row only**; every other row — wrapped
  continuations *and* later logical lines — gets `   ` (blank). Mirrors the
  transcript's user block (`Block::User` uses the same first-line-only prefix).
- **Cursor** `▌` drawn at the cursor's *wrapped* (row, col).
- **Newline keys**: `Shift+Enter` (exists) plus `Ctrl+J` (portable — many
  terminals do not report a distinct Shift+Enter).
- **Fix `Delete`**: forward-delete is currently a no-op.

### Phase 2 — paste → placeholder

- **Model**: `input` is `Vec<Atom>` where
  `Atom::Char(char) | Atom::Paste(PasteBlock { id, text, lines, chars })`; the
  cursor is a plain gap index `0..=atoms.len()`. A `Paste` renders as a one-line
  chip and is **atomic** (one Backspace/Delete removes the whole block; the cursor
  steps over it as a unit). *(Simplest form of B4 — one atom per char, so no
  two-level cursor.)*
- **Threshold**: on `Event::Paste(text)`, if `chars > 100 || lines > 3`, insert a `Paste` atom;
  otherwise insert the text literally.
- **Chip**: `❰ pasted {lines} line(s) · {size} ❱`, `size` = `{chars} chars`, or
  `{bytes/1024:.1} KB` when `chars >= 1024`. Styled dim/accent like the chrome.
- **Multiple pastes**: each block keeps its own `id` (so a future command can
  address one).
- **Submit**: expand every block to its text, then the existing `trim` applies.
  `history` stores the **expanded** text; the transcript's `Block::User` shows
  the **full** text.
- **Overlay gate**: a paste with a picker open must not edit the input (today
  `on_key` is gated but `AppEvent::Paste` bypasses the gate).
- **No in-place peek/expand in v1** (see Open questions).

## Decisions (locked)

| # | question | decision |
|---|----------|----------|
| A1 | gutter | ` ❯ ` on the first row only; blank on wraps and later logical lines |
| A2 | growth | grow to ~8 rows, then internal scroll |
| A3 | newline keys | `Shift+Enter` + `Ctrl+J` |
| B4 | paste model | `Vec<Atom>` — `Char` or a `Paste` block (atomic) |
| B5 | thresholds | `> 100 chars` or `> 3 lines` |
| B6 | chip text | `❰ pasted N lines · M chars/KB ❱` |
| B7 | transcript + history | full/expanded text |
| B8 | multiple pastes | one block/id each |
| B9 | peek/expand | none in v1 |

## Phased tasks

**Phase 1 — wrap.** ☑ landed
- [x] `wrap_input(text, width) -> Vec<String>`: char-exact, spaces preserved,
      over-long token hard-broken (+ unit tests).
- [x] Height from visual rows in `ui.rs` (`MAX = 8.min(h/2).max(1)`), scroll that
      keeps the cursor row visible.
- [x] Cursor → (row, col) in wrapped coordinates; render `▌` there.
- [x] Gutter: ` ❯ ` on the first row only, `   ` elsewhere.
- [x] `Ctrl+J → Key::Newline` in `event.rs`; keep `Shift+Enter`.
- [x] Implement `Key::Delete` (forward-delete).
- [x] Tests (wrap semantics, cursor under wrap, non-truncating long line,
      scroll-to-cursor, `Ctrl+J`, `Delete`). *A true TTY run wasn't possible here;
      verified via the `TestBackend` render path + a pre-change comparison.*

**Phase 2 — paste.** ☑ landed
- [x] `Vec<Atom>` model (`Char`/`Paste`) + gap-index cursor; reimplement
      `insert_char`/`insert_str`/`backspace`/`delete`/`Left`/`Right`/`Home`/`End`.
- [x] `Event::Paste`: `> 100 chars || > 3 lines` → a `Paste` atom, else inline.
- [x] Chip rendering in `draw_input` (`InputView` → styled spans).
- [x] Atomic block deletion.
- [x] Expand on submit; `history` + transcript get the full text.
- [x] Overlay gate for paste.
- [x] Tests (small→inline, big→chip, submit expands to full text, atomic
      delete/step-over, two chips, overlay-gated, narrow-width chip). *A real TTY
      paste couldn't be driven here; verified via `TestBackend`.*

## Open questions

- **In-place peek/expand** of a chip (a key to toggle, or a `/paste` command) —
  deferred; add only if it bites.
- Should the **transcript** also collapse very large pastes? (Decision B7: no —
  the model needs the text and it is the record of truth.)
- **Configurable** thresholds vs constants — constants for v1.
- A **preview snippet** of the first pasted line in the chip — considered, not
  taken (keep the chip terse).
- **Scrolled input hides ` ❯ `**: the marker keys on the first row, so it scrolls
  off once the band is taller than `MAX`. Accepted (first-row-only gutter).
- **Unicode display width**: the wrap counts `chars()`, not columns, so double-width
  CJK/emoji can overflow the band. Pre-existing and shared with the transcript
  (`markdown.rs`, `greedy_wrap`); a follow-up if wide-character prompts matter.

## Progress

| phase | scope | status |
|-------|-------|--------|
| 1 | input wrap + visible multiline + `Ctrl+J` + `Delete` | ☑ done |
| 2 | paste → placeholder (`Vec<Atom>` chips) | ☑ done |

Legend: ☑ done · ◐ in progress · ☐ todo.
