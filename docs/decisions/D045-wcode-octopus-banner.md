# D045 — The wcode avatar becomes an octopus banner (W039)

- **Status:** accepted — implemented in this commit.
- **Date:** this session
- **Relates to:** [`docs/work/W039-octopus-banner.md`](../work/W039-octopus-banner.md),
  [`D042`](D042-wcode-mark.md) (amended: the terminal tier is now the octopus),
  [`docs/design/wcode-mark.md`](../design/wcode-mark.md),
  `crates/wcode-tui/src/app.rs` (`Block::Banner`, `OCTOPUS_BANNER`,
  `seed_hint_if_empty`, `clear_hint`),
  `crates/wcode-tui/src/ui.rs` (`block_lines` `Block::Banner` arm).
- **Amends:** `D042` (terminal tier only), `docs/tui-design.md` §2 (admits the
  banner's glyphs), `docs/design/wcode-mark.md` (terminal tier).

## Context

The human owner asked for a "real banner" and a "real avatar" for wcode in the
TUI: a big, multi-line ASCII octopus shown when starting a new session. D042
had locked the terminal tier as the two-row `❯▌ wcode` mark and §2 as a closed
glyph table, so this request requires amending both (Option A of the brief).

## Decision

1. **The avatar is an octopus**, rendered as a preformatted `Block::Banner`
   (a new unit variant) carrying the art verbatim — spaces and newlines
   preserved, bypassing `wrap`/`greedy_wrap` (which collapse whitespace). This
   is the shape W037 originally sketched and then withdrew for the mark; it is
   correct here because the art's spacing is load-bearing.
2. **The art (8-wide body, 5 lines):**
   ```
    ▄████▄
    █ •  • █
    ████████
    ▀█ ▀▀ █▀
    ❯ type a message · /help · F1 keys
   ```
   (The hint is the banner's last line, so the whole empty state is one block.)
3. **Root surface only.** Member surfaces keep the bare hint
   (`EMPTY_HINT`), exactly as D042 §5 prescribed for the mark. The banner is
   seeded by `Surface::seed_hint_if_empty` and retired by `Surface::clear_hint`
   (the single retire path), so the five call sites are untouched.
4. **§2 admits the banner's glyphs** — `▄ █ ▀ •` — as a dedicated
   `banner (octopus)` row; the rest of the table is unchanged and §1.3's
   bare default (red-only hue) still holds. The banner resolves to the
   existing `dim()` role only.
5. **D042's px tier** (the VS Code activity-bar icon / marketplace tile) is
   left as the `❯▌` mark for now; redrawing it as an octopus is a separate
   designer task (out of scope of this commit, tracked in W039 §2 non-scope
   — the user asked for it; follow-up). The terminal tier and the px tier
   may diverge until that lands.
6. **The mark (`❯▌ wcode`) is retired** from the terminal empty state. The
   `EMPTY_MARK` / `ROOT_EMPTY_HINT` constants are retained as aliases for
   back-compat with tests.

## Consequences

- `Block` gains a `Banner(String)` variant; every exhaustive `match` was
  updated (`copy_text`, `block_lines`, and the two `_ => {}` filters are
  already catch-all).
- `clear_hint` now retires `Block::Banner(_)` and `Block::Notice(EMPTY_HINT)`.
- The empty-state tests were rewritten to the new contract; the measure-
  centering test's "hint row" probe moved to the banner's 5th line.

## Verification

- `cargo test --workspace` and `cargo clippy --workspace --all-targets` clean.
- `cargo run -p wcode-tui --example dump -- empty` shows the octopus on the
  root empty frame at 80 and 48 cols, retired after a resumed session.
- `docs/tui-design.md` §2 md5 changed (intentionally — the amendment is the
  point); every other § is byte-unchanged.

**Not verified.** The px-tier octopus icon was not redrawn (follow-up). The
banner was inspected via the dump example, not a live interactive terminal
session.
