# TUI markdown — plan

Status: **planned.** Motivation: `docs/gap-analysis-jcode.md` §3 ("markdown maturity")
and `docs/tui-design.md:147`, which already names the primary fix — *"A wrapped-line
cache keyed by `(revision, width)` … do not re-wrap static history every frame."*
Presentation-only, inside `crates/wcode-tui`.

## 1. Problem

Two independent defects in the transcript renderer.

**(a) The whole transcript is re-parsed every frame.** `draw_transcript`
(`ui.rs::draw_transcript`) iterates *every* committed block and calls
`block_lines` → `content_lines` (`ui.rs::content_lines`) → `markdown::render`
(`markdown.rs:30`) — the expensive path — on every frame, then drains the visible
window. A long session re-parses (and re-allocates) the entire history at 60 Hz.
`theme.rs` is the only memo in the crate; there is no render cache.

**(b) The parser is thin.** `markdown.rs` supports fenced code, headings, bullets,
GFM tables, and inline `code` / **bold** / `[label](url)`. Missing: **ordered
lists**, **blockquotes**, **nested indentation**, **heading levels**, **italic**,
**syntax-highlighted code** (fenced lines are styled uniformly; fences are not
language-tagged), and true display width (wide glyphs count as one,
`markdown.rs::disp`).

## 2. Design

### Phase 1a — a per-block render cache (the foundation)

A cache *above* `block_lines`, in `Surface` (the renderer already feeds back into
`App`: `set_block_ranges`, `sync_scroll`), keyed by **(per-block revision, width)**.

- **Soundness — the crux.** Committed blocks are **not** immutable. `Block::Tool`
  is mutated in place after it is pushed:
  - expand/collapse — `toggle_selected`; Ctrl-T expand/collapse-all;
  - live growth — `ToolExecutionUpdate` (on every update), `ToolExecutionEnd`
    (sets `done`, `is_error`, `expanded` on error, `path`, `diff`).

  A key of `(index, width)` alone is **wrong**. Each block needs a **revision**
  bumped at those mutation sites; every other `Block` variant is append-only and
  its revision stays `0`. Hit iff `rendered_rev == rev[i] && rendered_width == width`.
- **No interior mutability.** Store a `Vec<CacheEntry>` parallel to `transcript`
  in `Surface` (matching the existing `last_total`/`last_width`/`ranges` style);
  `RefCell`/`RwLock` have no precedent here.
- **Borrow.** `transcript()` returns `&self`; the loop needs `&mut` for the cache.
  Resolve with a `Surface` method that owns both borrows and appends into a caller
  `&mut Vec<Line>` (e.g. `append_block_lines(i, width, &mut out)`), rather than
  holding `transcript().iter()` across a `&mut` borrow.
- **Excluded:** the in-flight `live()` message (never a `Block`; re-renders
  legitimately — the `▌` cursor rides the last line).
- **Pre-bar lines.** The browse selection bar is a second pass over the *window*
  (`paint_bar`); the cache stores pre-bar lines — the key is unaffected.
- **Scope note.** The per-frame `Vec<Line>` assembly (extending the window) still
  copies pre-styled lines; 1a removes the markdown/wrap work, not the extend.
  Rendering *only* the visible window is a larger refactor — out of scope.

### Phase 1b — parser features

Add to `markdown.rs`, in `render`'s dispatch loop: **heading levels** (1–6 styled
distinctly), **ordered lists** (`1.`/`2.` …), **blockquotes** (`>`), **nested
indentation**, **italic** (`*x*`/`_x_`), and **syntax-highlighted code** (language
tag on the fence → `syntect`). Keep the hand-rolled block model — do **not**
migrate to `cmark` (gap-analysis §3). Real display width (`unicode-width`) is a
candidate for a follow-up, not this phase.

### Non-goals

- **Mermaid** — deferred, gated stretch (TUI P4): a parser subset (flowchart /
  sequence) in halfblock, or delegation to an external `mmdc`/`npx`. Its own doc.
- **Side panel** (pin a diff/file beside the chat) — a *later* layer, its own doc;
  it reuses the diff renderers, not this work.

## 3. Phases & progress

- [ ] **1a — per-block render cache.** `Surface` cache + per-block revision +
      the borrow-safe append; tests: identical `buffer_text` across frames, and a
      cache hit when a frame renders unchanged content/width (and a miss after a
      tool-block mutation or a resize). `tui:` commit.
- [ ] **1b — parser features** (heading levels, ordered lists, blockquotes, nested
      indent, italic, `syntect` highlighting). Tests in `markdown.rs` per feature;
      keep the existing `markdown.rs` tests intact (the cache sits *above* `render`).
      `tui:` commit(s); a new `syntect` dep.
- [ ] *(optional, alongside 1b)* **live theme reload** — `theme::THEME` is
      `OnceLock` install-once; an `RwLock` swap enables runtime recolor (gap §3, S).

## 4. Open questions

- **Cache granularity** — one cache for all `Block` variants, or only the
  expensive ones (`Assistant`/`Tool`/`Diff`)? `User`/`Notice`/`Error`/`Btw` are a
  cheap `wrap` and may not be worth caching.
- **Revision plumbing** — a `rev` field on `Block::Tool`, or a parallel
  `Vec<u64>` in `Surface` bumped by the reducer? The parallel vec keeps `Block`
  pure (it is a UI view type); the field is simpler at the mutation sites.
- **`syntect` weight** — a `Theme`/`SyntaxSet` set is a sizeable dep + first-use
  load; confirm it is worth it vs a hand-rolled keyword highlighter for a few
  languages. Decide at 1b.

## 5. References

- wcode: `crates/wcode-tui/src/markdown.rs`, `ui.rs` (`draw_transcript`,
  `block_lines`, `content_lines`), `app.rs` (`Block`, `Surface`, the `apply`
  reducer), `theme.rs`; `docs/tui-design.md:147`/`:185`,
  `docs/gap-analysis-jcode.md:25`/§3, `docs/tui-plan.md` (P2).
- jcode: `crates/jcode-tui*/…` markdown/syntax paths (reference only).
