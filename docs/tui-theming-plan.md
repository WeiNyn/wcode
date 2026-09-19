# TUI palette & theming — plan

Status: **T1 landed (palette B); T2/T3 open.** Parent: [`tui-plan.md`](tui-plan.md)
(P2 truecolor detection, P4 configurable theme) and [`tui-design.md`](tui-design.md) §1.3.

Presentation-only, inside `crates/wcode-tui`. No new deps.

## 1. Problem

The shipped palette (`crates/wcode-tui/src/theme.rs`, `Theme::colored`) *names* 16
roles but they collapse to ~9 looks, so distinct components render identically:

- `muted` == `border` (both `DarkGray`) — the sidebar's `done` row looks like the
  pane border, and every border reads as "quiet prose".
- `warn` == `code` (both `Yellow`) — the mid context gauge looks like inline code.
- `tool_name` == `link` (both `Blue`) — a tool header and a hyperlink look alike.

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
| `tool_name` | **Blue + Bold** | was `Blue` (= link) |
| `thinking` | **Magenta + DIM + Italic** | shares the "meta" hue |
| `diff_add` | Green | alias of `success` |
| `diff_del` | Red | alias of `error` |

Changed vs. before: `muted`, `warn`, `heading`, `tool_name`, `thinking`. Everything
else is untouched. A test pins the three collisions above **open** (they must never
re-merge); the intentional aliases are documented, not asserted distinct.

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

- **T1 (landed — `847b569`) — palette B.** Rewrite `Theme::colored` per §2; add a test that
  `muted≠border`, `warn≠code`, `tool_name≠link` and that `heading` carries a color.
  Commit `tui: adopt the semantic palette (B) for role distinctness`.
- **T2 — capability detection (P2).** Introduce `ColorMode { Plain, Named, Indexed,
  Rgb }`, resolve it once in `theme.rs`, and make `colored()` the `Named` tier.
  Commit `tui: detect terminal color support (P2)`.
- **T3 — `[theme]` overlay (P4).** A `[theme]` table in `config.toml` (roles as hex,
  absent = inherit), opt-in truecolor, with the curated 16-color fallback from §3.
  Commit `tui: configurable [theme] overlay (P4)`.

## 5. Non-goals

Not a theme registry or built-in theme catalog; not behavior config (see the
repo's "minimalism is the point"); not a change to the kernel — the palette lives
entirely in `crates/wcode-tui`.
