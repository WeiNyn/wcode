# TUI palette & theming — plan

Status: **T1 landed (palette B); T2 landed (capability detection, hex-gated truecolor); T3 landed (T3a `010e7ab`, T3b `6f7053f`).** Parent: [`tui-plan.md`](tui-plan.md)
(P2 truecolor detection, P4 configurable theme) and [`tui-design.md`](tui-design.md) §1.3.

Presentation-only, inside `crates/wcode-tui`. No new deps.

## 1. Problem

The shipped palette (`crates/wcode-tui/src/theme.rs`, `Theme::colored`) *names* 16
roles but several share a look, so distinct components are hard to tell apart:

- `muted` == `border` (both `DarkGray`) — the sidebar's `done` row looks like the
  pane border, and every border reads as "quiet prose".
- `warn` == `code` (both `Yellow`) — the mid context gauge looks like inline code.
- `tool_name` / `link` share the same `Blue` fg (differing only by underline) — a
  tool header and a hyperlink read alike.

Three further pairs are **intentional aliases**, not bugs: `accent` == `user` (the
prompt and running state), `success` == `diff_add`, `error` == `diff_del` (same
semantics — good/bad).

The result is the complaint that opened this work: components are boring and hard
to tell apart.

## 2. Decision — palette B ("semantic families")

Each component *family* owns a hue: **meta** (headings + reasoning) is magenta,
**tools** are blue+bold, **code** is yellow, **warnings** are light-yellow,
**semantics** are green/red, **chrome** is grey, and the **accent** (cyan) stays the
user prompt + running state. Named ANSI 16 only; under `NO_COLOR` every role drops
its `fg` and the `Theme::plain` palette is used unchanged.

| role | style (colored) | note |
|---|---|---|
| `accent` | Cyan + Bold | prompt + running |
| `user` | Cyan + Bold | alias of `accent` |
| `dim` | DIM | the workhorse |
| `muted` | **Gray** | was `DarkGray` (= border) |
| `border` | DarkGray | chrome |
| `body` | default | uncolorized prose |
| `error` | Red | |
| `success` | Green | |
| `warn` | **LightYellow** | was `Yellow` (= code) |
| `code` | Yellow | unchanged |
| `heading` | **Magenta + Bold** | was Bold-only |
| `link` | Blue + Underline | |
| `tool_name` | **Blue + Bold** | was plain `Blue` (same hue as `link`) |
| `thinking` | **Magenta + DIM + Italic** | shares the "meta" hue |
| `diff_add` | Green | alias of `success` |
| `diff_del` | Red | alias of `error` |

Changed vs. before: `muted`, `warn`, `heading`, `tool_name`, `thinking`. Everything
else is untouched. A test pins the collisions above **open** — `muted≠border`,
`code≠warn`, `heading` colored, `tool_name` bold — while the intentional aliases
(`accent`/`user`, `success`/`diff_add`, `error`/`diff_del`) are documented, not
asserted distinct.

## 3. Theming direction — tiers, not a registry

Custom theming is **opt-in and truecolor**; the **default stays symbolic**
(named-16), because a symbolic default *inherits the user's terminal palette*
(Solarized/Gruvbox users want their cyan, not ours). Colors are reinforcement only:
glyphs (`⚙ ✓ ✗ ···`) and modifiers carry the meaning, so the TUI stays usable at
8 colors or none.

- **Data model:** reuse `ratatui::style::Color` — it is already an enum of
  `Named` / `Indexed(u8)` / `Rgb(u8,u8,u8)` / `Reset`. A theme is a struct of 16
  `Color`s; no color crate.
- **Capability ladder** (resolved once, like the existing `no_color()`):

  ```
  NO_COLOR / !tty / TERM=dumb        → Plain   (modifiers only — Theme::plain)
  COLORTERM in {truecolor, 24bit}    → Rgb     (24-bit)
  TERM contains 256color (or ≥256)   → Indexed (256)
  otherwise                          → Named   (16 — palettes like B)
  ```

  Downgrade prefers an **authored 16-color fallback per role** (the designer picks
  the stand-in) with automatic nearest-color only as a backstop — auto-quantizing
  RGB→16 can re-collapse two roles onto one slot, i.e. reintroduce §1.
- **Gotchas even on modern emulators:** tmux/ssh often drop `COLORTERM`, so the
  256/16 tiers must be good, not afterthoughts; truecolor fixes *absolute* colors,
  so explicit themes need a **dark and light variant** (do not rely on `COLORFGBG`).

## 4. Phases

- **T1 (landed — `847b569`) — palette B.** Rewrite `Theme::colored` per §2; add a test
  pinning `muted≠border`, `code≠warn`, `heading` colored, and `tool_name` bold.
  Commit `tui: adopt the semantic palette (B) for role distinctness`.
- **T2 — capability detection (P2).** *Landed.* `ColorMode { Plain, Named,
  Indexed, Rgb }` resolved once from the §3 ladder (`NO_COLOR`/`TERM=dumb` →
  Plain; `COLORTERM` in {truecolor,24bit} → Rgb; `TERM` containing `256color` →
  Indexed; else Named) via a pure `resolve_color_mode_from(no_color, colorterm,
  term)` under test. It has a real, observable effect today: a hex `#rrggbb`
  override in a `[theme]` spec is honored only when `color_mode() == Rgb`; under
  Plain/Named/Indexed it degrades to the role's palette-B default (a truecolor
  value on a non-truecolor terminal renders wrong), while named colors are
  honored in every mode. Config stays terminal-agnostic — `parse_theme` still
  accepts and validates hex; the degradation happens at apply time in
  `ThemeSpec::into_theme(mode)`, and `resolve()`'s NO_COLOR-wins behavior is
  unchanged. Still open: the actual tier palettes (Basic 8 / 256 / truecolor),
  which land only alongside a tier that needs them.
- **T3 — `[theme]` overlay (P4).** A `[theme]` table in `config.toml` overrides
  roles; absent keys inherit palette B. This is the user-facing theming feature, so
  it lands in two commits:
  - **T3a (TUI).** A public `ThemeSpec` (a role→color map, `Default` = palette B)
    plus `parse_theme(&BTreeMap<String,String>) -> Result<ThemeSpec, String>` in
    `wcode-tui`. Color values are **either** a named ANSI color (`cyan`,
    `light-yellow`, `dark-gray`, …) **or** a hex `#rrggbb` (→ `Color::Rgb`, opt-in
    truecolor). Unknown role/color is a hard error. `Options` gains `theme:
    ThemeSpec`; `run` installs it into the `theme()` `OnceLock` before the first
    draw. `NO_COLOR` still wins (forces `plain`, overrides ignored). No CLI change
    yet. Test: parse named + hex; unknown role/color errors; an override changes
    only its role; `NO_COLOR` wins.
  - **T3b (CLI).** `[theme]` in `config.rs` (`FileConfig` → `Config`), validated at
    load time into a `ThemeSpec` (a bad value is a `ConfigError`, matching the
    "unparseable overlay is a hard error" style), passed through `main.rs` at both
    `Options` literals. README + config example.

  Commits (landed): `010e7ab` `tui: a configurable [theme] spec (P4)`, `6f7053f`
  `cli: load the [theme] table into the TUI`.

## 5. Non-goals

Not a theme registry or built-in theme catalog; not behavior config (see the
repo's "minimalism is the point"); not a change to the kernel — the palette lives
entirely in `crates/wcode-tui`.
