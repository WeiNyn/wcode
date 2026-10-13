# W019 · `omarchy` theme source fails on ANSI-schema themes

**Status:** fix in progress (human-approved approach).
**Author:** orchestrator (recon + brief).
**Bug, not a W018 phase.**

## The bug

`./target/debug/wcode` fails to start for a user whose `~/.config/wcode/config.toml`
sets `[theme] name = "omarchy"`:

```
error: invalid [theme]: theme `omarchy`: no `muted` in the Omarchy palette
```

**Root cause.** Omarchy `colors.toml` comes in **two schemas**:

- **semantic** — the built-in themes (`tusche`, `platin`, `papier`, `kokemusu`,
  `chrom`, `quattrocento-light`): `accent`, `muted`, `red`, `green`, `yellow`,
  `blue`, `magenta`, `cyan`, `hyprland_active_border`, …
- **ANSI** — third-party themes such as `kanso`: `accent`, `cursor`, `foreground`,
  `background`, `selection_*`, and `color0`…`color15` — **no** semantic `muted`/
  `red`/… keys.

`omarchy_palette` (`crates/wcode-tui/src/theme.rs`) only understands the semantic
schema and hard-errors on the first missing key (`muted`, the 4th it reads). The
reader landed in `5ab249b`; **W018 did not touch it** (verified: `git diff
7002466..HEAD -- theme.rs` has no change on those lines).

Evidence (this machine, theme `kanso`):
- `~/.local/state/omarchy/current/theme/colors.toml` → `accent/foreground/background` + `color0..15`, no `muted`.
- `~/.config/omarchy/themes/kanso/colors.toml` → same.
- `~/.config/omarchy/themes/tusche/colors.toml` → the semantic keys.

## The ask

Make the `omarchy` source resolve **both** schemas, so `name = "omarchy"` works
with any Omarchy theme.

## Scope / non-scope

**In scope:** `omarchy_palette` (`theme.rs`) — resolve the semantic-only fields
with an ANSI fallback; a test with an ANSI-schema (kanso-shaped) fixture.

**Non-scope:** the semantic path is unchanged; `omarchy_border`, `parse_colors_toml`,
`read_omarchy_colors`, the `Palette` struct, and the W018 palette collapse are
untouched; no new config knob.

## The mapping

Anchors `foreground` / `background` / `accent` stay **required** (both schemas
define them; a file missing them is not a colour scheme — keep the loud error).
The semantic-only fields fall back to their ANSI slots:

| `Palette` field | semantic key | ANSI fallback |
|---|---|---|
| `muted` | `muted` | `color8` (bright black) |
| `red` | `red` | `color1` |
| `green` | `green` | `color2` |
| `yellow` | `yellow` | `color3` |
| `blue` | `blue` | `color4` |
| `magenta` | `magenta` | `color5` |
| `cyan` | `cyan` | `color6` |
| `code` | `yellow` (documented collapse) | same as `yellow` |
| `border` | `hyprland_active_border` | `muted` (unchanged) |

## Plan

1. Add a fallback-aware resolver in `omarchy_palette` (`color_or(primary, &[alt])`).
2. Apply the table.
3. Tests: a new ANSI-schema fixture (kanso-shaped) → a full `Palette` with the
   mapped values; keep every existing semantic test (`omarchy_palette_maps_every_field`,
   `…_falls_back_to_muted_without_a_border`, `an_incomplete_omarchy_palette_is_an_err`
   — removing `accent` must still error).

## Gates

```
cargo test -p wcode-tui
cargo clippy --workspace --all-targets
cargo test --workspace            # isolated HOME; the omarchy env trap now *disappears* for this user
```
**Live check:** after the fix, `./target/debug/wcode --dump-config` must succeed
with the real `~/.config/wcode/config.toml` (`name = "omarchy"`, theme `kanso`)
instead of erroring — and `wcode --tui` must open.

## Expected outcome

`name = "omarchy"` resolves against an ANSI-schema theme; the built-in semantic
themes are byte-identical to before; the CLI no longer aborts at config load.
