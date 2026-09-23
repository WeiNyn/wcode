# TUI markdown — plan

Status: **1a + 1b-i shipped; 1b-ii (syntax highlighting) open.** Motivation:
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

**Ratified & shipped.** Cache **all `Block` variants** (not only the expensive
ones); revisions live in a parallel `Vec<u64>` in `Surface` — `Block` stays a pure
UI view type, no `rev` field; the inter-block separator is owned by
`append_block_lines` (which returns the block's own pre-bar range); the hit/miss
probe is a **per-`Surface`** counter (a global atomic would race the parallel
render tests). `app.rs` now names one render-backend type (`ratatui::text::Line`)
for the cache.

### Phase 1b — parser features

**1b-i — block features (shipped).** In `render`'s dispatch loop, in order:
`leading_indent` → `quote` → `heading` → `ordered` → `bullet` → blank → paragraph.
Ratified decisions:
- **Indent split first**, **2 columns** per level (a tab is two; an odd space
  floors), so `  - x` is a nested bullet. `indent_prefix(level) = {GUTTER} + 2·level
  spaces`; deeper content just gains those spaces.
- **Heading marker dropped** (`heading -> Option<(usize, &str)>`, level 1–6), styled
  by `heading_level_style`: one new **`heading_sub`** role carries levels 2–6
  (`theme.rs` grew one field — field, `colored`, `plain`, the `into_theme` match,
  and `ROLE_NAMES` 16→17).
- **Ordered lists** keep their real marker (`1. `, `2) `, `10. `) and pad the
  continuation to the marker width.
- **Blockquotes** get **one `│ ` per `>` level** (depth), with a matching space run
  on continuations; the body is `theme.muted` (no new role).
- **Italic** is matched **last** (after bold/link/code), supporting `*x*` and `_x_`:
  non-empty, non-space content edges, and — for `_` — a word boundary on both sides
  (tracked with `prev: Option<char>`), so `snake_case` stays plain.

**1b-ii — syntax highlighting (open).** Language-tagged fences → `syntect`. A
sizeable dep plus a first-use load; decide there.

**1b-iii — spacing fidelity (shipped).** `wrap` flattened styled runs with
`split_whitespace` and re-joined with one space, so *every* inline-run boundary
became a space (`see `foo`.` → `see foo .`; `a**b**c` → `a b c`; `*i*,` → `i ,`).
Fixed by modelling each atom as `(text, style, space_before)`: a style change
splits the atom, `>=1` source spaces collapse to one `space_before`, and a
separator is emitted only when the source had whitespace (and the line continues).
The **signature is unchanged**; `hard_break`/`used`/`avail` accounting is
untouched. Accepted: a glued oversized word (`a**verylongbold**c`) breaks
mid-word between its atoms (no separators).

### Non-goals

- **Mermaid** — deferred, gated stretch (TUI P4): a parser subset (flowchart /
  sequence) in halfblock, or delegation to an external `mmdc`/`npx`. Its own doc.
- **Side panel** (pin a diff/file beside the chat) — a *later* layer, its own doc;
  it reuses the diff renderers, not this work.

## 3. Phases & progress

- [x] **1a — per-block render cache.** `Surface` cache + per-block revision +
      the borrow-safe append; tests: identical `buffer_text` across frames, and a
      cache hit when a frame renders unchanged content/width (and a miss after a
      tool-block mutation or a resize). `tui:` `d585ebf`.
- [x] **1b-i — parser block features** (heading levels, ordered lists, blockquotes,
      nested indent, italic). Per-feature tests in `markdown.rs`; the 8 existing
      tests stay green. `tui:` `c9ceba1`.
- [ ] **1b-ii — syntax highlighting** (`syntect`, language-tagged fences); a new dep.
- [x] **1b-iii — preserve source spacing in `wrap`.** Inline-run boundaries no longer
      become spaces; `wrap` carries a per-word "space-before" flag (signature
      unchanged). Regression tests in `markdown.rs`. `tui:` `eb8427f`.
- [ ] *(optional, alongside 1b)* **live theme reload** — `theme::THEME` is
      `OnceLock` install-once; an `RwLock` swap enables runtime recolor (gap §3, S).

## 4. Open questions

- **Cache granularity — resolved (1a).** One cache for **all** `Block` variants;
  the uniform path beats special-casing the cheap `User`/`Notice`/`Error`/`Btw`.
- **Revision plumbing — resolved (1a).** A parallel `Vec<u64>` in `Surface`,
  bumped by the reducer, rather than a `rev` field on `Block` — the view type
  stays pure.
- **`syntect` weight — open (1b-ii).** A `Theme`/`SyntaxSet` set is a sizeable dep +
  first-use load; confirm it is worth it vs a hand-rolled keyword highlighter for a
  few languages. Decide at 1b-ii.

## 5. References

- wcode: `crates/wcode-tui/src/markdown.rs`, `ui.rs` (`draw_transcript`,
  `block_lines`, `content_lines`), `app.rs` (`Block`, `Surface`, the `apply`
  reducer), `theme.rs`; `docs/tui-design.md:147`/`:185`,
  `docs/gap-analysis-jcode.md:25`/§3, `docs/tui-plan.md` (P2).
- jcode: `crates/jcode-tui*/…` markdown/syntax paths (reference only).
