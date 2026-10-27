# W037 — wcode identity: one mark for the TUI banner and the VS Code icon

- **Status:** **landed, not committed.** The human signed off ("follow your
  recommendation"): candidate **A** (`❯▌`), all-dim banner, `#3fb950` on
  `#0b0e13`, lowercase `wcode`, root-surface only. Second-layer review:
  **APPROVE** (nits N1–N5, no blockers). Open: the nit fixes, the commit, and
  closing `docs/next-steps.md` row 109
- **Decisions:** D042 (`docs/decisions/D042-wcode-mark.md`) — **accepted, locked**
- **Scope collapsed after design round 1** — see AMENDMENT 1 below: no new `Block`
  variant, no `banner_lines`, so §4/§5's `Block::Banner` work and §7.1's
  space-preservation tests are withdrawn. The Rust change is 3 constants, one
  branch, one two-way match, and four tests
- **Author:** `agent:brainstormer` (recon: `agent:explorer`)
- **Date:** 2026-10-27

> **AMENDMENT 1 (design round 1, `docs/design/identity/`) — the `Block::Banner`
> variant in §4/§5 is SUPERSEDED; do not build it.**
>
> The brief assumed ASCII art, which forced a new transcript block because
> `greedy_wrap` (`ui.rs:2083` `for word in text.split_whitespace()`) collapses
> space runs. The chosen banner is a **mark of two cells**, so there is no art
> and no space run: `❯▌ wcode` renders correctly through the existing
> `Block::Notice` path, and `wrap` (`ui.rs:2054`) already splits on `'\n'` — a
> two-line `Notice` is two rows with the `"   "` gutter, unchanged.
>
> Consequence: **zero new `Block` variants, zero new renderer functions, zero
> new tests about space preservation.** The wiring is:
>
> 1. `app.rs:480` — `EMPTY_HINT` becomes two lines (`❯▌ wcode` + a hint
>    shortened to fit 48 cols — the shipped one already wraps there, a live
>    defect).
> 2. `clear_hint` (`app.rs:1784`) keeps working verbatim: it compares against the
>    same constant, so its five call sites are untouched.
> 3. Root-surface-only seeding (Q5) is NOT free under this shape — the constant is
>    per-surface today; decide whether a member's empty state also carries the
>    mark, or the seed branches on `is_root` (`app.rs:1457`).
>
> Everything in §4 about `copy_text`, four-vec lockstep, `banner_lines`, and the
> five space-preservation tests is **withdrawn**. §7.1's tests reduce to the
> banner's presence/retirement plus the 48-col assertion.

---

## 1. The ask

The human wants wcode to have an **identity**: one mark, designed once, that
shows up as the **startup banner in the TUI** and as the **extension icon in
VS Code**. Verbatim:

> "I now want to improve the identity for wcode, so we will need to design a
> Icon that apply to both TUI as startup Banner and vscode extension Icon"

The problem it solves is real but narrow: wcode currently has **two
disconnected pieces of identity** — a drawn `>_` chevron SVG that exists *only*
in the activity bar (`editors/vscode/media/wcode.svg`, referenced once at
`editors/vscode/package.json:39` `"icon": "media/wcode.svg"` — `:38` before
this change's top-level `icon` insert — a pre-existing mark), and a
**font-rendered** `>_` favicon (`favicon.svg`, `<text …font-family="ui-monospace,
Menlo, Consolas, monospace"…>&gt;_</text>`), byte-identical into the book
(`diff favicon.svg docs/book/favicon.svg` → 0). The terminal — the product —
shows **no mark at all**. There is no mark spec, no D-record, and no marketplace
tile.

---

## 2. Scope / Non-scope

**In scope.**

1. **One mark spec doc** — `docs/design/wcode-mark.md`: the geometry on a
   24-unit grid, the ink rule, and the **tiers** (px tier for the SVG/PNG;
   terminal tier for the banner). Two or three candidates, drawn, then chosen.
2. **The VS Code mono icon** — refine `editors/vscode/media/wcode.svg` to the
   chosen geometry (it is already `fill="currentColor"` with two path runs, so
   it is monochrome-by-construction; only the geometry changes).
3. **A marketplace tile** — commit `editors/vscode/media/icon.png` and set a
   **top-level** `"icon"` in `editors/vscode/package.json` (today only the
   activity-bar contribution at `:38` exists). `media/` is not in
   `.vscodeignore`, so a committed PNG ships in the VSIX with no build step.
4. **The TUI startup banner** — the empty state opens on the mark, derived from
   the same spec, and retires with the existing hint when the first real block
   lands.
5. **The records** — `D042-wcode-mark.md`, the `docs/tui-design.md` §2/§4
   amendment (only the parts the mark actually needs), and the
   `docs/next-steps.md` W037 row.

**Non-scope.**

- **A fourth band.** `design-taste` §3: "Three bands, always — transcript
  (flex) → input → status; **there is no fourth**". The banner is a *block in
  the transcript*, not new chrome. Reason: a band is a permanent reflow of the
  base (§1.1); the banner is one-shot and must scroll/vanish with the empty
  state.
- **Any new `theme.rs` role.** `theme.rs:21` — "Add a field only when a real
  call site needs one"; `ROLE_NAMES` is a fixed 17 (`theme.rs:865`). The banner
  resolves to existing roles (`dim`/`body`/`accent`).
- **Hue in the TUI default theme.** D023 / `docs/tui-design.md:39` — "The
  **default (palette B) is bare (D023)** … `error` red is the ONLY hue". The
  terminal mark is grey, always.
- **The REPL's `❯` vocabulary.** `crates/wcode-cli/src/repl.rs:886` /
  `:893` stays as-is (flagged as a nearly-free follow-on, not scoped).
- **The site/book redesign.** Root `style.css` and `docs/book/style.css` are
  untouched (see Q3 — the brand hex question, which is a *record*, not a
  restyle).
- **npm publish / a new package.**
- **Favicon unification (root + book).** *Flagged, not scoped.* They are
  already byte-identical to each other, but both render `>_` through a
  **monospace font** rather than drawn paths — so the favicon's shape varies by
  machine. That is an identity defect, but fixing it is a separate one-file
  change with its own visual check (and would make the favicon the third
  consumer of the spec, which is fine *after* D042 exists).

---

## 3. Estimates

| # | workstream | value | complexity | risk | why |
|---|------------|-------|------------|------|-----|
| P0 | The mark spec + candidates (`docs/design/wcode-mark.md`) | **medium** | **low** | **low** | prose + drawings; worst case is a page of unused candidates. Value: it is the thing every later step cites, and it settles the pixel-vs-terminal tension *before* code. |
| P1 | Terminal tier: the TUI startup banner (`Block::Banner`) | **medium** | **medium** | **medium** | a new `Block` variant threaded through a renderer whose **four parallel vecs** must stay in lockstep (`transcript`/`block_revs`/`cache`/`seqs`), and through **five** `clear_hint` call sites; risk is a desynced vec or a banner that fails to retire — both silent, neither crashes. |
| P2 | VS Code mono icon (`media/wcode.svg`) | **low** | **low** | **low** | two path runs in a 24×24 viewBox; the file already is `currentColor` + monochrome. Value is polish on the one surface that already has a mark. |
| P3 | Marketplace tile (`media/icon.png` + `package.json` `"icon"`) | **medium** | **low** | **low** | one committed PNG plus one JSON key; value is that the *listing* (which people see before the product) stops being a placeholder. |
| — | *(flagged)* favicon as drawn paths instead of `<text>` | low | low | low | out of scope now; it is the third consumer of P0's spec. |
| — | *(flagged)* a one-line mark in the CLI REPL banner (`repl.rs:886`) | low | low | low | nearly free but it changes a startup line users read, and `❯` there is currently only the REPL prompt. |

Reading: **P0 is the gate**; **P1 is the only medium-risk item** and carries the
named regression tests. Nothing here is high-risk — no boot path, no session
write, no default behind a flag.

---

## 4. Interface & structure

### Added

- **`docs/design/wcode-mark.md`** — the mark spec. Contents: the grid (24
  units), the ink rule, the **tiers** (px tier = the SVG/PNG; terminal tier =
  the banner rows), the sanctioned-symbol audit, and the rejected candidates
  with their measured row cost.
- **`docs/decisions/D042-wcode-mark.md`** — the lock: the mark, its one ink,
  the tiers, and why each alternative was rejected.
- **`editors/vscode/media/icon.png`** — the marketplace tile, one ink, the tile
  colour baked in. Committed, no build step (`.vscodeignore` excludes
  `src/**`, `scripts/**`, `test/**`, `node_modules/**`, `out/**/*.map`, and
  build config — **not** `media/`).
- **`crates/wcode-tui/examples/dump.rs` — an `empty` mode** (or a second
  fixture in the same example): the existing fixture is a **populated** session,
  so it can never show the banner. The ladder needs an empty-transcript frame.

### Changed

**`crates/wcode-tui/src/app.rs`**

- `pub enum Block {` — `app.rs:226`. Gains a **unit** variant `Banner`. Unit,
  not `Banner(Vec<String>)`, because the art is a rendering concern: the
  renderer already owns width (`block_lines(block, width)`) and the art must be
  picked *by width* at render time. It also makes retirement a **variant match**
  instead of string identity.
- `fn seed_hint_if_empty(&mut self)` — `app.rs:1771`, at
  `self.transcript.push(Block::Notice(EMPTY_HINT.into()));` (`:1774`). Pushes
  `Block::Banner` **before** the hint, and only on the root surface
  (`self.is_root`, `app.rs:1457`). Both pushes keep the four vecs in lockstep
  exactly as the hint does today (`cache.push(CacheEntry::never())`,
  `seqs.push(0)`).
- `fn clear_hint(&mut self)` — `app.rs:1784`, at
  `|b| matches!(b, Block::Notice(t) if t.as_str() == EMPTY_HINT),` (`:1786`).
  Retires **both** empty-state blocks, by variant:
  `matches!(b, Block::Banner) || matches!(b, Block::Notice(t) if t.as_str() == EMPTY_HINT)`,
  scanning the leading slots (`.take(2)` already covers both) and removing
  **all** matches (indices in reverse, so the four vecs stay aligned). Keeping
  this in `clear_hint` — rather than adding a second function — is what keeps
  the **five existing call sites** untouched: `push_block` (`app.rs:1759`),
  `seed` (`:2370`), the `AppEvent::Agent` arm (`:4185`, whose comment warns the
  hint must clear *before* `before` is captured), `push_on` (`:5137`), and
  `seed_empty_hint` (`:5974`).
  - **Why a variant match over string identity:** the hint's identity is a
    literal (`EMPTY_HINT`, `app.rs:480`) that the retirer must compare against —
    a second literal, so a change to one without the other silently leaks the
    block. The banner has no comparable literal (it is art, chosen by width,
    several rows), so string identity would need the *composed* art at the *same*
    width — unknowable at retirement time. A variant match cannot drift.
- `fn copy_text(block: &Block) -> Option<String>` — `app.rs:416`, arms at
  `:417-422`. Its match is **exhaustive with no `_` arm**, so the compiler
  forces an arm: `Block::Banner => return None`. Also note `fn selectable`
  (`app.rs:1742`) is `i < self.transcript.len()`, so the banner **is** a
  browse-mode target (as the hint is today).

**`crates/wcode-tui/src/ui.rs`**

- `pub(crate) fn block_lines(block: &Block, width: usize)` — `ui.rs:924`, arms
  from `:926`; the `Notice` arm is `ui.rs:934` `Block::Notice(text) => wrap(text, width, "   ", "   ", dim()),`.
  Gains `Block::Banner => banner_lines(width)`, **not** a `wrap` call.
  - **Why not `wrap`:** `ui.rs:2054` `fn wrap(…)` → `ui.rs:2080` `fn greedy_wrap`
    → `ui.rs:2083` `for word in text.split_whitespace()`, re-joining with a
    single `' '` (`:2087` `cur.push(' ')`). **Every run of spaces collapses.**
    Contrast the composer's preserving wrapper `ui.rs:2107` `fn wrap_input` with
    `wrap_line`, documented "no whitespace is collapsed".
- New `fn banner_lines(width: usize) -> Vec<Line<'static>>` + the art `const`s,
  placed next to `wrap_input` (`ui.rs:2107`) — the preserving-wrapper precedent,
  in the presentation crate only. It emits **pre-formatted** rows (one `Span`
  per row, `dim()`/`body()`), so no wrapping, no collapsing; a row wider than
  `width` is **clipped, not reflowed** (the D038 `…` rule: a cut is never
  silent — clip with `…` or fall back to the narrower tier).
- `banner_lines` picks a **width tier**: the measure is the full band below 84
  cols (`ui.rs:74` `const MEASURE_MIN_BAND: usize = 84;`, used at `:749`), so
  the banner must be ≤48 glyphs at 48 cols and may be wider at 84+. The tiers
  are decided in the **design step** (P0), not here.

**`editors/vscode/package.json`**

- A top-level `"icon": "media/icon.png"` (marketplace tile). The existing
  `"icon": "media/wcode.svg"` at `:38` **stays** — it is
  `contributes.viewsContainers.activitybar[0].icon`, a different field with a
  different contract (monochrome, theme-tinted).

**`editors/vscode/media/wcode.svg`** — geometry only, if the chosen mark differs
from today's two runs:
```
<path d="M4.4 4.6 3 6.2 8.6 12 3 17.8l1.4 1.6L11.4 12 4.4 4.6Z" />
<path d="M12.5 16.5h8v2.2h-8z" />
```

**`docs/tui-design.md`**

- §2 (`:65`, the table; the authority: `:67` "The **single** glyph table. §4
  names elements but never re-declares a glyph.") — a row **only if** the
  terminal tier uses a symbol not already present. `❯` is already
  `| user prompt | ❯ | dim |`, and `_` is ASCII; a mark built from those adds
  **no glyph**. A block-letter tier (`█ ▀ ▄`) **would** add three, and needs the
  amendment + a real call site.
- §4 Transcript (`:204-216`) — the block-kind list `User, Turn, Tool, Notice,
  Error, Btw, Diff, Todos` (`:205-206`) and the sentence "With **no** history
  the transcript opens on a dim seeded hint (`❯ type a message · /help for
  commands · F1 for keys`), not a blank pane" (`:214-216`) both gain the banner.

**`docs/next-steps.md`** — a W037 row pointing at this file.

### Integration points (verify before coding)

- `crates/wcode-tui/src/app.rs:480` — `const EMPTY_HINT: &str = "❯ type a message · /help for commands · F1 for keys";`
- `app.rs:1757` `fn push_block` → `:1759` `self.clear_hint();` (the "first real
  block retires the empty state" seam).
- `app.rs:1771` `fn seed_hint_if_empty` → `:1774` the push (the seed seam).
- `app.rs:1784` `fn clear_hint` / `:1786` the string-identity match.
- `app.rs:5974` `pub(crate) fn seed_empty_hint`; `:5976` the All-mode guard.
- `crates/wcode-tui/src/ui.rs:104` `app.seed_empty_hint();` — called **every
  frame**; a seed must therefore be idempotent, which the
  `transcript.is_empty()` guard (`app.rs:1772`) provides.
- `ui.rs:934` the `Notice` arm (the arm the banner must *not* copy).
- `ui.rs:2083` `for word in text.split_whitespace()` (why art cannot ride
  `wrap`).
- `ui.rs:743` `fn draw_transcript`, `:812-814` — with content shorter than the
  band, `sync_scroll` (`app.rs:2454`, `:2462`) clamps `scroll` to 0 and the
  window is `lines[0..total]`, so a seeded block renders at the **top** of the
  transcript band, immediately under the header. **This is the geometry the
  banner inherits: a top-of-band block on an empty screen that never scrolls
  (it is removed, not scrolled past).**
- `editors/vscode/package.json:39` — the existing activity-bar icon (`:38` at
  HEAD; the W037 top-level `"icon"` insert shifts it by one).
- `editors/vscode/.vscodeignore` — `media/` is not excluded, so a new PNG ships.

---

## 5. Plan

1. **Design the mark — candidates, rendered.** Deliverable:
   `docs/design/wcode-mark.md` with **2–3 candidates** (A/B/C), each shown three
   ways: (a) the 24-unit path geometry, (b) the **terminal projection** as raw
   text at 48, 80 and 120 cols (one row; and, priced separately, the 3-row
   block-letter form), (c) a 16 px and 24 px raster mock of the activity-bar
   icon. The doc also records the **row cost**, the **48-col behaviour**, the
   **`NO_COLOR`** behaviour, and the sanctioned-symbol audit for each. Gate: the
   human writes the pick (one letter) on the doc. *The designer owns the
   artwork.*
2. **Lock the decision + amend the spec.** Deliverable:
   `docs/decisions/D042-wcode-mark.md` (the mark, **one ink**, the tiers, the
   rejected candidates each with its cost) and the `docs/tui-design.md` edits —
   §4's block-kind list + the empty-state sentence, plus a §2 row *only* if a
   new symbol entered. Gate: `grep -n 'D042' docs/decisions/D042-wcode-mark.md
   docs/tui-design.md`; if the terminal tier is `❯_`, the D-record states in
   one line that **no glyph was added**.
3. **Ship the px assets.** Deliverable: the refined
   `editors/vscode/media/wcode.svg`, `editors/vscode/media/icon.png`
   (≥128×128 — see §11.2), and the top-level `"icon"` key. Gate:
   `npx --yes @vscode/vsce@3 ls --tree` lists `media/icon.png`;
   `node -e 'JSON.parse(require("fs").readFileSync("editors/vscode/package.json"))'`
   parses; the file at 16 px and 24 px in the activity bar shows a legible
   single-ink mark with the same silhouette as the banner below.
4. **Wire the TUI banner.** Deliverable: `Block::Banner` + the
   `seed_hint_if_empty` push + the `clear_hint` variant retirement + the
   `copy_text` arm + `banner_lines` with its art consts. Gate:
   `cargo test -p wcode-tui` green **including the named tests** (§7.1);
   `cargo clippy -p wcode-tui --all-targets -- -D warnings` clean.
5. **Prove the frame — the empty-state ladder.** Deliverable:
   `cargo run -p wcode-tui --example dump -- empty` writes
   `target/tui-render/empty-{48x20,80x24,120x40}.{txt,ansi}` from an **empty**
   `App`. Gate: the `.txt` files show the mark at the content column, the hint
   **below** it, the header **above** it, nothing clipped at 48 cols; the
   `.ansi` files colour the mark with the **grey ramp only** (no hue) at
   default, and with `NO_COLOR` set the `.txt` is unchanged.
6. **Live check.** Deliverable: a real binary run. Gate:
   `cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1` (refused port)
   shows the banner on the first frame; typing a message and pressing Enter
   retires it (gone in the next frame; a `/help` overlay does not disturb it).
   `WCODE_RETRY_MAX=2` for a fast error path.
7. **Record it in the tracker.** Deliverable: the `docs/next-steps.md` W037 row
   with the commit ids. Gate: the row names each commit and the review verdict.

---

## 6. Quality gates

```sh
cargo test --workspace                                  # all pass
cargo clippy --workspace --all-targets -- -D warnings    # clean — CI's exact gate
```

CI already runs both, with `-D warnings` (`.github/workflows/ci.yml:28`,
`:33`) — **the `-D warnings` form is the gate**; a plain `cargo clippy` is
weaker than CI. Plus the two live checks:

```sh
cargo run -p wcode-tui --example dump -- empty               # → target/tui-render/empty-*.txt
cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1   # banner on frame 1, gone after Enter
```

**Regression tests that must stay green (by name):**

- `crates/wcode-tui/src/ui.rs:5201` `a_fresh_surface_shows_the_empty_state_hint`
  — currently asserts `text.contains("type a message")` and
  `text.contains("/help for commands")` at 80×12, then that `seed_history`
  retires the hint. **It must be extended**, not merely kept: the mark row joins
  the assertions, and the retirement assertion gains "and the mark is gone".
- `crates/wcode-tui/src/ui.rs:4497`
  `a_transient_row_renders_exactly_like_a_notice_row` — asserts
  `block_lines(&Block::Notice(..), w) == block_lines(&Block::Transient(..), w)`
  for 20/40/80 cols. **Must stay untouched**: it is the guard that the banner
  arm did not get folded into `Notice`'s `wrap`.
- `crates/wcode-tui/src/app.rs:11509`
  `seqs_stay_in_lockstep_across_push_insert_and_retire` — **will fail as
  written**: it asserts `assert_eq!(app2.focused().seqs.len(), 1, "the hint
  keeps seqs in step")` after `seed_hint_if_empty()`, and `:11524`
  `assert_eq!(s2.transcript.len(), 1, …)`. Both become `2` / `0` once the banner
  is seeded alongside; updating them **is** the lockstep proof for the new
  variant.
- `crates/wcode-tui/src/ui.rs:4518` `a_drag_highlight_changes_no_row_width` — a
  `Banner` block should be added to its list, because browse can select it.

---

## 7. Testing

### 7.1 Automated

- `a_fresh_surface_opens_on_the_mark_and_the_hint` (extend `ui.rs:5201`) —
  asserts the fresh frame at 80×12 contains the mark's first row **and**
  `type a message`, and that the mark row sits **above** the hint row (compare
  line indices, not just presence).
- `the_mark_retires_with_the_hint` — asserts that after one
  `push_block`/`seed_history` the transcript contains **no** `Block::Banner` and
  no `Notice(EMPTY_HINT)`, and that `block_revs`/`cache`/`seqs` all have the
  same length as `transcript` (the four-vec invariant, which is what actually
  breaks).
- `the_mark_is_seeded_only_on_the_root_surface` — asserts a member surface
  seeded empty carries the hint but **not** the mark (Q5), so `/team`-switching
  never repeats the brand.
- `the_banner_survives_the_narrow_ladder` — for widths 48/80/120, that
  `block_lines(&Block::Banner, w)` yields the promised **row count** and that
  **no row exceeds `w`** (a hard assert on row width). This makes the 48-col
  claim real.
- `the_banner_preserves_its_spaces` — asserts `block_lines(&Block::Banner, 80)`
  is **not** equal to `wrap`ping the same joined text (the `greedy_wrap` trap,
  pinned).
- `copy_text` arm — asserts `copy_text(&Block::Banner)` is `None`.

### 7.2 How a human verifies it

```sh
# 1. The frame, no TTY, three widths — the design review.
cargo run -p wcode-tui --example dump -- empty
cat target/tui-render/empty-48x20.txt
cat target/tui-render/empty-80x24.txt
cat target/tui-render/empty-120x40.txt
cat target/tui-render/empty-80x24.ansi     # on a real terminal: grey only, no hue

# 2. The ink constraint.
NO_COLOR=1 cargo run -p wcode-tui --example dump -- empty && cat target/tui-render/empty-80x24.txt

# 3. The live banner, and its retirement.
cargo run -p wcode-cli -- --base-url http://127.0.0.1:9/v1
#   → the mark is on the first frame, under the header
#   → type "hi" + Enter; the error path unwinds — the mark must be GONE

# 4. The activity-bar icon (VS Code).
code --extensionDevelopmentPath=editors/vscode .
#   → the wcode icon at 16 px and 24 px: one ink, legible, same silhouette as the banner
```

Look for: the mark **top of the transcript band, above the hint**, identical
silhouette at all three widths (clipped, never reflowed), **no hue** anywhere in
the `.ansi`, and the mark **absent** from every frame after the first submit.

---

## 8. Expected outcome

- `docs/design/wcode-mark.md` exists, names one mark, and shows 2–3 candidates
  with measured row costs; `D042` locks it.
- `editors/vscode/media/wcode.svg` and the terminal banner are **the same drawn
  geometry** — a reviewer can lay the banner rows next to the SVG paths and see
  one mark.
- `editors/vscode/package.json` has a top-level `"icon": "media/icon.png"` and
  `vsce ls` lists the PNG.
- A fresh `wcode` opens on the mark (top of the transcript band) + the hint; the
  first message retires both, and no later frame shows either.
- `cargo test --workspace` green with the named tests updated/added;
  `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `empty-48x20.txt` shows no row wider than 48.
- The TUI still has **exactly one hue** (`error` red), **zero** new `theme.rs`
  roles, and **zero** new glyphs (if `❯_` is the terminal tier).

---

## 9. References

- `docs/tui-design.md:65-101` — §2, the single glyph table (the authority;
  `:67` "§4 names elements but never re-declares a glyph").
- `docs/tui-design.md:39` — palette B is bare, `error` red the only hue (D023).
- `docs/tui-design.md:204-216` — §4 Transcript: the block-kind list and the
  seeded-hint sentence to amend.
- `docs/decisions/D023-tui-prose-only-bare.md` — the bare-palette lock.
- `docs/design/tui-turn-notes.md:676-710`, `:780-810` — prior art: the drawn
  3-row banner was **costed and rejected** (+3 rows/message; `█ ▀ ▄` absent from
  §2) in favour of a one-row reverse-video head. **A startup-only banner is a
  different cost profile** (one-shot, empty screen only, retired at the first
  block — it never repeats per message), so the rejection does not carry over
  verbatim; its *measurements* do.
- `docs/design/tui-book-directions.md:225-250` — the other drawn-initial
  direction and its ≥120-col requirement.
- `docs/design/vscode-ui-exploration.md:36` — "Identity is a word"; `:70-76` §3.1
  "One glyph vocabulary, shared with the terminal … No invented glyphs"; `:111`
  "one accent (`--vscode-charts-blue`) for wcode identity".
- `.wcode/skills/design-taste/SKILL.md` — §3 terminal rules. **Note the skill's
  §3 glyph list is stale**: it quotes `❯ ··· ⚙ ✓ ✗ ⋯ ▌ ⠋ ▤ · ` │ ─ ┼ █░`, but
  `█ ░` appear **nowhere** in `crates/wcode-tui/src` and nowhere in
  `docs/tui-design.md`. Reconcile it in step 2.
- `editors/vscode/media/wcode.svg`, `favicon.svg`, `docs/book/favicon.svg` — the
  existing mark, twice (drawn, and font-rendered).
- `crates/wcode-tui/src/lib.rs:88` `render_text` / `:95` `render_plain`;
  `crates/wcode-tui/examples/dump.rs:55`
  `const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (48, 20)];`.
- `.github/workflows/ci.yml:28,33` — the exact CI gates.

---

## 10. Open questions — for the human, each with a recommendation

**Q1 — What is the mark?** Evolve `>_`, or a new concept (a `w` from chevrons, a
bracket, a block cursor, a terminal prompt)?

> **Recommend: evolve `>_`.** It already exists twice (`media/wcode.svg`'s two
> path runs; the favicon), it is **one ink by construction**, and every
> alternative that tries to be clever fails the constraint that kills most logos
> here: **it must read at 16×16 in the activity bar AND survive the TUI's bare
> grey ramp with no hue to lean on.** A chevron + bar is the only shape in that
> space that is unambiguous at 16 px. The *design work* is not the concept, it
> is the **proportion** (stroke weight vs. the 3 px padding the current file
> already has, the chevron's angle, the bar's baseline and gap) and the
> **terminal tier** — and the terminal tier is where `>_`'s ASCII nature pays
> off: `❯_` uses **only** the already-sanctioned `❯` and ASCII `_`.
> *Designer's brief:* propose 2–3 *proportion* candidates of the same concept,
> plus at most one genuinely different concept as a control.

**Q2 — What does the TUI banner cost?** (i) one row, reverse-video/filled
wordmark (0 new glyphs); (ii) 3-row block letters (needs a §2 amendment for
`█ ▀ ▄`); (iii) the mark as a small glyph + the existing hint line.

> **Recommend (iii), with (i) as the loud fallback — and (ii) only if the human
> explicitly wants drawn letters.** Concretely, (iii) is **1 row for the mark +
> the hint row that already exists** (so +1 row net, 0 new glyphs, 48-col-safe:
> `❯_` is 2 cells):
> ```
>  ❯_   wcode
>
>  ❯ type a message · /help for commands · F1 for keys
> ```
> (i) is **+0** rows but reuses `Modifier::REVERSED`, and **`REVERSED` already
> means SELECTION** in this TUI (`paint_bar`, the drag highlight, and
> `recolor_range` at `ui.rs:860`) — a static filled wordmark dilutes that
> meaning, and D023 retired the reverse-video speaker head. (ii) is **+3 rows**
> and requires a §2 amendment admitting `█ ▀ ▄` with a real call site — at 48
> cols the letters still fit (`WCODE` = 5 cells wide × 3 rows) but they eat a
> fifth of a 14-row transcript band.
> **48-col behaviour:** (iii) `❯_  wcode` = 9 cells, safe; (i) `wcode` reversed
> = 5 cells, safe; (ii) `WCODE` block letters = 5×3, safe — the *width* never
> fails, the *height* is the cost.

**Q3 — One accent, or per-surface tiers?**

> **Recommend: the mark itself carries NO colour; the one hex is reserved for
> the tile, and the tile hex is `#3fb950` (the prompt green).** `media/wcode.svg`
> is already `fill="currentColor"`, so the activity-bar tier inherits the editor
> theme and is correct by construction. The only place a hex must be **baked**
> is the PNG tile (and, if ever fixed, the favicon background). `#3fb950` is
> already the mark's colour on the favicon and is `--prompt` on the root site —
> "the prompt" is a meaning, not decoration. The TUI stays **ink-only**. This
> needs a sentence in `D042` because it touches `design-taste` §4's "≤1 accent"
> reading: the tile hex is **not** an accent, it is the mark's ground.

**Q4 — Marketplace icon now, or activity-bar-only?**

> **Recommend: ship the PNG now.** The listing is seen before the product,
> `.vscodeignore` already ships `media/`, and the release workflow packages the
> VSIX — so the cost is one committed file and one JSON key.

**Q5 (folded in, needs no answer) — the banner on member surfaces.** Recommend
**root surface only** (`app.rs:1457` `is_root`). The hint belongs on every empty
surface; a **brand** banner repeated each time the human tabs to an idle worker
reads as chrome, and the team surfaces are already the brand.

**Q6 (folded in) — retirement.** Recommend the banner retires **through
`clear_hint`** (one function retiring both), not a second retire path.

---

## 11. Facts NOT verified

1. **Nothing was run.** The brainstormer is read-only (`bash` mutation
   refused). No render was observed — the 48/80/120 behaviour, the top-of-band
   placement, and the absence of clipping are **reasoned from `ui.rs:743-814` +
   `app.rs:2454-2468`**, not seen. Step 5 converts the reasoning into pixels.
2. **The marketplace icon requirement** (PNG, ≥128×128, SVG not accepted) is
   quoted from memory, not from VS Code docs. If wrong, only the tile's size in
   step 3 changes.
3. **The activity-bar render at 16 px / 24 px** was not inspected in a running
   VS Code.
4. **`❯`'s cell width** (East-Asian-Ambiguous class, like `▌`) is unverified; if
   it renders double-width, `❯_` misaligns and the terminal tier must fall back
   to ASCII `>_`. Worth one `printf '❯_' | cat -A` check.
5. **The book's cover markup** was not checked for a mark slot (non-scope).
6. **"The seeded hint renders at the TOP of the band"** holds only while the
   content fits (`total < height`); on a screen short enough to overflow it
   would render as the tail — untested, irrelevant at 20+ rows.
7. **"`media/` ships in the VSIX"** is inferred from `.vscodeignore`, not from a
   `vsce package` run.

---

## 12. Process constraints

- **`docs/tui-design.md` §2 is the authority.** A new symbol needs a **§2
  amendment with a real call site** (`:67`). If the terminal tier is `❯_`, no
  amendment is owed — `❯` is already the `user prompt` row and `_` is ASCII —
  and the D-record must say so in one line.
- **A D-record is required**: `docs/decisions/D042-wcode-mark.md` (D041 is the
  highest today).
- **`design-taste` runs before the screen is called done**, and its §3 glyph
  list must be reconciled with §2 as part of step 2.
- **One logical change per commit**, area-prefixed: `docs:`, `vscode:`, `tui:` —
  so steps 2, 3, 4 are three commits, not one.
- **Minimalism**: no config toggle for the banner. It is the empty state,
  unconditionally — a `[tui] banner = false` would be exactly the kind of
  behaviour config this repo refuses.

---

*End of brief — no code written, nothing edited.*
