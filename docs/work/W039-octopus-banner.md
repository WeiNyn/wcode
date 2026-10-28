# W039 — Octopus ASCII-art banner in the TUI empty state — brief

- **Status:** shipped
- **Work item:** W039
- **Decisions:** D042 (wcode mark — amended by D045), D023 (bare/prose-only), D009 (D5 empty-state hint), D045 (octopus banner)
- **Author:** `agent:brainstormer` + orchestrator
- **Date:** 2026-05-01

## 1. The ask

The user wants a **big, multi-line ASCII-art octopus banner** in the wcode TUI that acts as wcode's avatar, shown **when starting a new session**. Today the empty state is the D042 mark. The request replaces/augments that two-row mark with an octopus ASCII banner. This directly collides with D042 ("zero new glyphs", "§2 byte-unchanged") and with the closed glyph table in `docs/tui-design.md` §2; the user is the human owner and may override D042. **Resolved: Option A — amend D042 + §2.**

## 2. Scope / Non-scope

**In scope.**

- Define the octopus ASCII art and where it lives in the empty state (new session only; retired on the first real block, exactly like the hint today).
- Amend D042 / `docs/design/wcode-mark.md` / `docs/tui-design.md` §2.
- Renderer support so multi-line art with significant whitespace survives (bypass `wrap`/`greedy_wrap`).
- Seeding + retirement integration so the banner and hint stay in lockstep with the four parallel vecs (`transcript`/`block_revs`/`cache`/`seqs`).
- Root-vs-member placement decision (root-only).
- Tests updated to the new empty-state contract, plus a live dump check.

**Non-scope.**

- **VS Code / px tier.** `editors/vscode/media/wcode.svg`, `icon.svg`, `icon.png` left as the `❯▌` mark for now (follow-up designer task).
- **CLI REPL banner.** stays as-is.
- **Hue / new theme roles.** banner resolves to existing `dim()` role only.
- **A fourth band.** banner is a transcript block.
- **Persistent banner.** retires on first block.
- **Session storage / kernel.** no change outside `wcode-tui` (and docs).

## 3. Estimates

| # | workstream | value | complexity | risk |
|---|------------|-------|------------|------|
| P0 | Design/spec: octopus art + D042 amendment | medium | medium | medium |
| P1 | Renderer support (`Block::Banner` + raw render) | medium | high | high |
| P2 | Seeding + retirement (`seed_hint_if_empty`, `clear_hint`, root-only) | medium | medium | medium |
| P3 | Width handling (art already ≤48 cols) | low | low | low |
| P4 | Test contract update + regression | medium | medium | low |
| P5 | Live verification (dump example + human terminal) | high | low | low |

## 4. Interface & structure

### Added

- `Block::Banner` — unit variant in `pub enum Block` (`app.rs`). Unit because the art is a render-time concern; retirement is a variant match.
- `OCTOPUS_BANNER: &str` const (`app.rs`) — the 5-line art.
- `banner_lines` behaviour folded into `block_lines`'s `Block::Banner` arm (`ui.rs`) — raw lines, no `wrap`.

### Changed

- `crates/wcode-tui/src/app.rs` `pub enum Block` — gains `Banner`.
- `crates/wcode-tui/src/app.rs` `EMPTY_MARK` / `ROOT_EMPTY_HINT` — retained as aliases.
- `crates/wcode-tui/src/app.rs` `seed_hint_if_empty` — pushes `Block::Banner(OCTOPUS_BANNER)` on root, `Block::Notice(EMPTY_HINT)` on member.
- `crates/wcode-tui/src/app.rs` `clear_hint` — retires `Block::Banner(_)` and `Block::Notice(EMPTY_HINT)`.
- `crates/wcode-tui/src/ui.rs` `block_lines` — new `Block::Banner` arm.
- `crates/wcode-tui/src/ui.rs` empty-state tests — rewritten to the new contract.

### Integration points

- `crates/wcode-tui/src/app.rs:226` `pub enum Block`
- `crates/wcode-tui/src/app.rs:416` `copy_text`
- `crates/wcode-tui/src/app.rs:1794` `seed_hint_if_empty`
- `crates/wcode-tui/src/app.rs:1817` `clear_hint`
- `crates/wcode-tui/src/ui.rs:924` `block_lines`
- `crates/wcode-tui/src/ui.rs:5204`/`:5235`/`:5261`/`:5287` named empty-state tests
- `crates/wcode-tui/examples/dump.rs:71`/`:112` empty mode

## 5. Plan (executed)

1. Decision gate (human) — Option A, root-only, replace mark, px-tier follow-up. ✅
2. Spec amendment — art into `docs/design/wcode-mark.md`, D042 amended via D045, §2 row added. ✅
3. Renderer support — `Block::Banner` + raw arm. ✅
4. Seeding + retirement — updated. ✅
5. Width fallback — art already ≤48 cols; no separate variant needed. ✅
6. Tests — rewritten. ✅
7. Live verification — dump example inspected. ✅
8. Clippy + workspace tests — clean. ✅

## 6. Quality gates

- `cargo test --workspace` → all pass (564 tui tests).
- `cargo clippy --workspace --all-targets` → clean.
- `cargo run -p wcode-tui --example dump -- empty` → octopus present on root empty frame at 80 and 48 cols; retired after resumed session.

## 7. Testing

### Automated

- `a_fresh_surface_shows_the_empty_state_hint` — banner row above hint row.
- `the_mark_is_seeded_only_on_the_root_surface` — member has hint, no banner.
- `the_root_empty_state_is_the_octopus_banner` — composition assertions.
- `the_empty_state_fits_48_cols` — hint on one row at 48 cols.
- `the_transcript_centers_its_measure_on_a_wide_band` — hint probe moved to banner's 5th line.

### Human

Run `cargo run -p wcode-tui --example dump -- empty` and inspect `target/tui-render/empty-80x24.txt` and `empty-48x20.txt`.

## 8. Expected outcome

- A new session's root empty state shows the octopus banner above the hint, both dim.
- The banner retires on the first real block; resumed sessions never show it.
- Member surfaces keep the bare hint only.
- `cargo test --workspace` and `cargo clippy --workspace --all-targets` green.
- D042 amended (terminal tier); §2 amended; px tier follow-up tracked.

## 9. References

- `docs/decisions/D042-wcode-mark.md`, `D045-wcode-octopus-banner.md`
- `docs/design/wcode-mark.md`, `docs/tui-design.md` §2
- `crates/wcode-tui/src/app.rs:226,416,480-505,1794,1817`
- `crates/wcode-tui/src/ui.rs:924,5204,5235,5261,5287,6572`

**Not verified.** The px-tier octopus icon was not redrawn (follow-up). The banner was inspected via the dump example, not a live interactive terminal session.
