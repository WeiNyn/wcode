# wcode — markdown parser (pulldown-cmark) + a theme catalog

Status: **Part A shipped** (`377bf78`; second-layer APPROVED). **Part B (the theme
catalog) — design drafted, Q4–Q6 open.** Two coupled phases:
**A** migrates the TUI markdown parser to `pulldown-cmark`; **B** adds a built-in
**theme catalog** (a set of named themes + a per-theme syntect code theme).
Companion to [`tui-markdown-plan.md`](tui-markdown-plan.md) and
[`tui-theming-plan.md`](tui-theming-plan.md). Presentation-only, inside
`crates/wcode-tui` (+ one `[theme]` field in `wcode-cli`).

## 1. Why

**(A) The parser is hand-rolled and line-based.** `markdown.rs` (1119 lines) walks
`text.split('\n')` and dispatches per line (`leading_indent → quote → heading →
ordered → bullet → blank → paragraph`), with a hand-rolled inline pass and a
hand-rolled GFM-table detector. It is spec-incomplete (no task lists, footnotes,
setext headings, nested-list continuations, entity handling) and fragile by
construction. `pulldown-cmark` is the de-facto CommonMark+GFM parser: an **event
stream** that maps cleanly onto `ratatui` spans.

**(B) Theming is a per-role overlay with no preset.** `theme.rs` names 17 roles and
a `[theme]` table overrides their `fg`, but there is **no `theme = "name"` selector**
and the plan lists *"not a theme registry or built-in theme catalog"* as a non-goal.
The user wants a good starting **set** of themes. This overrides that non-goal
deliberately (recorded, since "minimalism is the point").

## 2. Ground truth (recon)

- **One entry point, one caller.** `markdown::render(text, width) -> Vec<Line<'static>>`
  (`markdown.rs:36`) → `render_mode(text, width, theme::color_mode())`. The only
  caller is `ui.rs:550` in `content_lines` (for `ContentBlock::Text`).
- **The cache is transparent to a parser swap.** `app.rs::CacheEntry { rev, width,
  lines: Vec<Line<'static>> }`; the hit test is `(rev, width)` — `rev` tracks only
  in-place `Block::Tool` edits (`0` = append-only). It never keys on rendered text.
  **The one contract is the return type `Vec<Line<'static>>`.**
- **Reusable:** `wrap(runs, width, first, cont, base)` (`markdown.rs:631`) + `line_with`,
  the syntect path (`Fenced`, `syntaxes()`, `syntax_theme()`, `fence_syntax`,
  `highlight_style`, `highlight_line`, `code_line`), `disp`, `hard_break`.
- **Line-based (a swap deletes them):** `is_fence`/`fence_info`, `bullet`,
  `leading_indent`, `heading`/`heading_level_style`, `ordered`, `quote`/`quote_gutter`/
  `quote_style`, `indent_prefix`; the whole table path (`table_at`, `is_delimiter`,
  `split_row`, `parse_aligns`, `table_lines`, `row_lines`, `wrap_cell`, `pad`); the
  inline parser (`inline`, `has_italic_edges`, `flush`).
- **Syntect theme:** hard-coded `base16-ocean.dark`, a process-wide `OnceLock`
  (`markdown.rs:113`), from `ThemeSet::load_defaults()`; `syntect` features
  `default-fancy`.
- **Theme plumbing:** `FileConfig.theme: BTreeMap<String,String>` (role→color) →
  `wcode_tui::parse_theme(&map) -> Result<ThemeSpec, String>` (hard error on a bad
  role/color) → `Options.theme: ThemeSpec` → `theme::install(theme)` at `lib.rs:183`,
  one-shot `OnceLock<Theme>`. `ROLE_NAMES: [&str; 17]` (`theme.rs:232`); `parse_color`
  accepts kebab-case ANSI names or `#rrggbb` hex (hex honored only under
  `ColorMode::Rgb`).
- **Tests:** 29 in `markdown.rs` pin exact text + styles. The whitespace/glue group
  encodes wcode-specific behavior; a spec parser will change output → **rewrite them**.
- **Dep:** `pulldown-cmark` is absent from the tree; latest `0.13.4`; its deps
  (`bitflags`, `memchr`, `unicase`) are already in `Cargo.lock` → **purely
  additive, no version bumps**.
- **Doc drift:** `README.md:74` lists 16 roles and omits `heading_sub` (17 in code).

## 3. Part A — decisions

- **D1 — Keep the contract.** `render(text, width) -> Vec<Line<'static>>` is
  unchanged (the cache depends on it). `render_mode` stays the color-mode seam.
- **D2 — `pulldown-cmark = "0.13"`**, `default-features = false` (no SIMD/`html`),
  with `Options::ENABLE_TABLES | ENABLE_STRIKETHROUGH | ENABLE_TASKLISTS |
  ENABLE_FOOTNOTES` (GFM-ish). No `ENABLE_SMART_PUNCTUATION` (keep source text).
- **D3 — Keep `wrap` + syntect; delete the line dispatch, `inline`, and the table
  detector.** The parser produces `(String, Style)` runs; `wrap` consumes them.
- **D4 — A block walk with a gutter stack.** A small state machine over
  `Parser::new_ext(text, opts)`: an accumulating `Vec<(String, Style)>` for the
  current leaf block, plus a **gutter stack** (one entry per open list/quote level)
  that composes `first`/`cont` prefixes. On a leaf block's `End`, emit
  `wrap(runs, width, first, cont, base)`.
- **D5 — Inline mapping.** `Event::Text` → body; `Event::Code` → `code_style`;
  `Tag::Emphasis` → italic; `Tag::Strong` → bold; `Tag::Link`/`Image` → the **label
  only** in `link_style` (the TUI can't follow links — matches today);
  `Event::SoftBreak` → a space; `Event::HardBreak` → a wrapped line break.
- **D6 — Code blocks via `Tag::CodeBlock`.** Collect the code text, then route each
  line through `Fenced::new(info, mode)` + `Fenced::line` (stateful syntect), exactly
  as today — no `wrap`.
- **D7 — Tables via `Tag::Table*`.** Build the existing `Table { header, aligns,
  rows }` from the `TableHead`/`TableRow`/`TableCell` events and **reuse
  `table_lines`** (the renderer is fine; only the *detector* was hand-rolled).
- **D8 — Headings, lists, quotes** map to the existing gutters/styles:
  `Tag::Heading{level}` → `heading`/`heading_sub`; `Tag::List(Some(start))` →
  ordered (keep the real marker, pad the continuation); `Tag::List(None)` → `• `;
  `Tag::BlockQuote` → one `│ ` per depth in `quote_style`; nesting composes via D4.
- **D9 — Rewrite the 29 tests** for spec-compliant output; keep the ones that assert
  *invariants* (wrapping to width, code styling, color-mode gating) and re-baseline
  the exact-string ones. Note which wcode-specific glue behaviors are **dropped**
  (the parser now owns tokenization).

## 4. Part A — the interface (what the sketch must pin)

```
// markdown.rs — unchanged public shape
pub fn render(text: &str, width: usize) -> Vec<Line<'static>>;
pub(crate) fn render_mode(text: &str, width: usize, mode: ColorMode) -> Vec<Line<'static>>;

// new: the event walk (private)
struct Blocks { /* runs, gutter stack, base, out */ }
impl Blocks {
    fn on_start(&mut self, tag: Tag, mode: ColorMode);
    fn on_end(&mut self, tag: TagEnd, width: usize);
    fn on_event(&mut self, ev: Event);           // Text/Code/SoftBreak/HardBreak…
    fn finish(self, width: usize) -> Vec<Line<'static>>;
}
```
`wrap`, `Fenced`, `table_lines`, `disp`, `hard_break` stay as-is (signatures
unchanged).

## 5. Part B — the theme catalog

- **D10 — A `name` selector.** `[theme] name = "gruvbox-dark"` selects a preset;
  the existing per-role keys still override on top. `parse_theme` grows a `name`
  field (a `ThemeSpec` gains `preset: Option<String>`), or a sibling
  `parse_theme_table(&BTreeMap)` that pulls `name` out first. Unknown name → a
  `ConfigError` (matching the "hard error" style).
- **D11 — A `Palette` data model.** Each catalog theme = a small set of **semantic
  colors** from which we derive *both* (a) the 17 UI roles and (b) a syntect
  `Theme` for code. So the code colors match the UI, per theme.
- **D12 — The starting set (6–8).** `default` (palette B, terminal-adaptive, named
  ANSI-16), `dark`, `light`, plus popular truecolor schemes — proposed:
  **`gruvbox-dark`**, **`nord`**, **`solarized-dark`**, **`solarized-light`**,
  **`catppuccin-mocha`**, **`tokyo-night`**. *(Q4: the exact list/names.)*
- **D13 — `ColorMode` degradation.** A preset's hex roles degrade like any hex
  override (honored only under `Rgb`; else the role's palette-B default). The
  `default` preset is named-16, so a non-truecolor terminal still gets a good look.
  *(Q5: should a preset author an explicit 256/16 fallback per role instead of
  degrading to palette B?)*
- **D14 — The syntect theme is part of the install.** The preset's syntax colors
  build a `syntect::highlighting::Theme`, installed into `markdown.rs`'s
  `SYNTAX_THEME` `OnceLock` in the same one-shot step as `theme::install` (both are
  process-global and install-once). *(Q6: build a custom syntect theme per preset vs
  map each preset to a `ThemeSet::load_defaults()` theme.)*

## 6. Open questions (for the first-layer review)

- **Q1 — GFM option set.** Which `pulldown_cmark::Options`? (task lists → a `[ ]`/
  `[x]` glyph? footnotes → render inline or drop? strikethrough → a modifier?)
- **Q2 — The `wrap` glue behavior.** Today `wrap` collapses spaces and glues runs
  (`a**b**c` → `abc`). With pulldown-cmark the runs come from events; does `wrap`'s
  `space_before` model still hold, or does it need adjusting? Which of the 29 tests
  are *invariants* (keep) vs *baselines* (re-baseline)?
- **Q3 — Soft break.** `Event::SoftBreak` → a space (CommonMark) vs a line break
  (the current per-line behavior keeps source newlines). Pick one.
- **Q4 — The theme set.** Confirm the 6–8 names above, or substitute.
- **Q5 — Preset degradation.** Degrade a preset's hex to palette-B per role, or
  author an explicit named-16 fallback per preset? (The theming plan prefers an
  authored fallback — §3 — but that's 6–8 × 17 colors.)
- **Q6 — Syntect theme.** Custom-per-preset (coherent, more data) vs map to a
  shipped `ThemeSet` theme (quick, incoherent).

**Part A — settled (first-layer review, verdict BLOCK → folded).** API shapes verified
from the crate source (a scratch crate in `/tmp`; the wcode tree was not modified);
`default-features = false` suffices. **Q1** tasklists → `[ ]`/`[x]`; footnotes →
text-only; strikethrough → enabled but **no modifier**; HTML → dropped. **Q2** `wrap`
is unchanged (the glue tests are invariants). **Q3** `SoftBreak` → a space;
`HardBreak` → a wrapped break. Two event-walk rules added to D4: **(B1)** every
block-level `Start` (`List`/`BlockQuote`/`CodeBlock`/`Table`/`Heading`/`Paragraph`)
flushes the open leaf first; **(B2)** an ordered `List` recomputes its marker and
continuation per `Item` (`ordered_next` seeded to `start - 1`).

**Part B — settled (first-layer review, verdict BLOCK → folded).** Q4: 7 presets
(`default` named-16 + `dark`/`light`/`gruvbox-dark`/`nord`/`solarized-dark`/
`solarized-light` truecolor). Q5: **degrade** a preset's hex to palette-B per role
under non-`Rgb` (the override of `tui-theming-plan.md` §3's authored-fallback
preference is accepted). Q6: a **custom per-preset syntect theme**, subject to the
blocker fix: `syntect::highlighting::Color` is RGBA-only, so add
`to_syntect(ratatui::Color) -> Option<syntect::Color>` and build the custom theme
**only when the palette is RGB** — otherwise `default_syntect_theme()`
(`markdown.rs`, today's `base16-ocean.dark`). Caveats recorded: the 6 truecolor
presets are no-ops on a non-truecolor terminal; a light preset needs a light
terminal (the palette's `fg`/`bg` reach only the syntect theme — prose keeps the
terminal default).

## 7. Non-goals

HTML rendering; images (the TUI can't show them); footnote *navigation* (render the
text, no links); math; mermaid (still a separate P4 stretch); a full syntax-theme
authoring format; behavior config beyond the theme selector.

## 8. Sizing

**Part A:** the event walk + gutter stack + inline/table/code mapping **M–L**; the
test rewrite **M**; **new dep** `pulldown-cmark` (additive). **Part B:** the catalog
+ the `name` selector + the syntect-theme build **M**; docs **S**. Together **L**.

## 9. References

- wcode: `crates/wcode-tui/src/{markdown,ui,theme,app}.rs`,
  `crates/wcode-tui/src/lib.rs` (`Options`, `install`), `crates/wcode-cli/src/config.rs`
  (`[theme]`), `README.md` (the theme table, doc-drifted).
- External: `pulldown-cmark` 0.13 (`Parser::new_ext`, `Event`, `Tag`, `TagEnd`).
