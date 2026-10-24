//! The TUI's named color roles, in one place. A [`Theme`] is a set of styles a
//! call site *names* (`accent`, `dim`, `error`, …) instead of a raw color, so the
//! look lives here and the renderer stays role-based.
//!
//! `Theme::plain()` is the `NO_COLOR` fallback: no `fg` at all, only
//! bold/italic/dim — byte-identical to the per-function branches it replaced.
//!
//! Palette B is the default — the bare grey ramp, `error` the only colour (W018).
//! A [`ThemeSpec`] — built by
//! [`parse_theme`] from a `[theme]` table — overlays any role, including a hex
//! `#rrggbb` truecolor value (opt-in). Theme *detection* (`COLORTERM` /
//! 256-color) remains open (`tui-plan.md` P2).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, PoisonError, RwLock};

use ratatui::style::{Color, Modifier, Style};

/// The TUI's color roles. Add a field only when a real call site needs one.
#[derive(Clone, Copy)]
pub(crate) struct Theme {
    /// The user prompt, the live cursor, and running state.
    pub accent: Style,
    /// The workhorse: secondary chrome and quiet prose.
    pub dim: Style,
    /// A low-emphasis grey, distinct from [`Theme::border`] above the named-16
    /// tier (they share one quiet grey there, D29): e.g. the team strip's
    /// `done` state.
    pub muted: Style,
    /// The overlay and popup borders and their titles.
    pub border: Style,
    /// The user's own prompt block.
    pub user: Style,
    /// Assistant prose — the uncolorized default.
    pub body: Style,
    /// A failure: the `✗` mark, error blocks, the full context gauge.
    pub error: Style,
    /// A success: the `✓` mark, the empty context gauge.
    pub success: Style,
    /// A warning: the mid context gauge.
    pub warn: Style,
    /// Inline code and fenced code blocks.
    pub code: Style,
    /// Markdown headings.
    pub heading: Style,
    /// Deeper markdown headings (levels 2–6).
    pub heading_sub: Style,
    /// Markdown links.
    pub link: Style,
    /// A tool's name in its `»` / `✓` header.
    pub tool_name: Style,
    /// A thinking block.
    pub thinking: Style,
    /// An added (`+`) diff line.
    pub diff_add: Style,
    /// A removed (`-`) diff line.
    pub diff_del: Style,
}

impl Theme {
    /// The bare skeleton (W018 §4.1) that BOTH palette B and every preset seed
    /// from: `error` red is the ONLY colour; `body`/`user` are default weight; every
    /// other role is the grey ramp; the markdown structure roles carry only a
    /// non-colour modifier. `dim` is the workhorse grey and `muted`/`border` the
    /// quiet pair (`tui-design.md` §1.3, D29).
    const fn colored() -> Self {
        Theme {
            // default weight (no fg)
            body: Style::new(),
            user: Style::new(),
            // THE ONLY COLOUR
            error: Style::new().fg(Color::Red),
            // the grey ramp (D29)
            dim: Style::new().fg(Color::Gray),
            muted: Style::new().fg(Color::DarkGray),
            border: Style::new().fg(Color::DarkGray),
            // everything else -> the dim grey
            accent: Style::new().fg(Color::Gray),
            success: Style::new().fg(Color::Gray),
            warn: Style::new().fg(Color::Gray),
            tool_name: Style::new().fg(Color::Gray),
            thinking: Style::new().fg(Color::Gray).add_modifier(Modifier::ITALIC),
            diff_add: Style::new().fg(Color::Gray),
            diff_del: Style::new().fg(Color::Gray),
            // markdown structure: NO fg, only the non-colour modifier (Q3)
            code: Style::new(),
            heading: Style::new().add_modifier(Modifier::BOLD),
            heading_sub: Style::new().add_modifier(Modifier::UNDERLINED),
            link: Style::new().add_modifier(Modifier::UNDERLINED),
        }
    }

    /// The `NO_COLOR` palette: no foreground anywhere — only bold/italic/dim. The
    /// bare palette modulo the single red→bold step (red is unavailable), so `error`
    /// degrades to bold; the markdown structure roles keep their modifier.
    const fn plain() -> Self {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().add_modifier(Modifier::DIM);
        Theme {
            body: Style::new(),
            user: Style::new(),
            error: bold,
            dim,
            muted: dim,
            border: dim,
            accent: dim,
            success: dim,
            warn: dim,
            tool_name: dim,
            thinking: Style::new().add_modifier(Modifier::DIM.union(Modifier::ITALIC)),
            diff_add: dim,
            diff_del: dim,
            heading: bold,
            heading_sub: Style::new().add_modifier(Modifier::UNDERLINED),
            link: Style::new().add_modifier(Modifier::UNDERLINED),
            code: Style::new(),
        }
    }
}

/// The active theme. A `RwLock` (not `OnceLock`) so [`set`] can rewrite it. The
/// seed is const (`Theme::colored()`); the `NO_COLOR`/`plain` choice is applied
/// by the always-run [`install`] before the first draw.
static THEME: RwLock<Theme> = RwLock::new(Theme::colored());

/// Bumped by every [`set`]; the TUI cache-invalidation watches it (`app.rs`).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The NAME the last [`set`] applied — `None` for palette B. Its own lock (not a
/// field on `Theme`) because a `Theme` carries resolved colours, and because it
/// must survive `NO_COLOR`'s `Theme::plain()`, where the colours say nothing
/// about the requested name (W032 A9).
static ACTIVE_NAME: RwLock<Option<&'static str>> = RwLock::new(None);

/// The current theme generation — the cache-invalidation signal.
pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// The active palette's catalogue name, recorded by [`set`] — `None` for
/// palette B or an inline `[theme]` palette, neither of which has a name to show
/// (W032 first-layer A9).
///
/// # Contracts
/// - `None` is a real answer, not a failure: palette B (and an inline `[theme]`
///   with no name) has no catalogue name. The `Some` case is always a
///   `&'static str` — the applied name is either a `CATALOG` entry or the
///   [`OMARCHY`] sentinel — so it re-binds through the catalogue, and a
///   `[theme] name` from config and a `/theme` switch are the same fact.
/// - The row's word for `None` (`palette b`) is `settings.rs`'s label, NOT a
///   value this returns: `theme.rs` stays presentation-free.
/// - Reflects the LAST [`set`] — [`install`] and [`set_preset`] both route
///   through it. Under `NO_COLOR` the name is still recorded: the row claims a
///   name, not a rendering.
pub fn active_name() -> Option<&'static str> {
    *ACTIVE_NAME.read().unwrap_or_else(PoisonError::into_inner)
}

/// The active theme — a COPY (the lock scope is one deref-copy).
///
/// POISONING POLICY: only [`set`] takes the write lock and it assigns plain data,
/// so a poison is practically impossible — but both here and in `set` we recover
/// via `unwrap_or_else(PoisonError::into_inner)` so a poisoned theme can never
/// abort a draw.
pub(crate) fn theme() -> Theme {
    *THEME.read().unwrap_or_else(PoisonError::into_inner)
}

/// Install/REPLACE the process theme — the re-callable form of the old one-shot
/// `install`. Rewrites the UI lock, swaps the syntect theme, and bumps the
/// generation so caches invalidate.
pub(crate) fn set(spec: ThemeSpec) {
    let palette = spec.palette();
    // Recorded DERIVED FROM THE SPEC (A9), before `resolve` consumes it: no side
    // effect leaks out of a builder, and a `with_preset` that is called and
    // discarded records nothing.
    *ACTIVE_NAME.write().unwrap_or_else(PoisonError::into_inner) = spec.name();
    *THEME.write().unwrap_or_else(PoisonError::into_inner) = resolve(spec, no_color());
    let syntax = palette
        .as_ref()
        .and_then(syntect_theme)
        .unwrap_or_else(crate::markdown::grey_syntect_theme);
    crate::markdown::install_syntax_theme(syntax);
    GENERATION.fetch_add(1, Ordering::Relaxed); // cache-invalidation signal
}

/// The BASE spec — the config `[theme]` spec — retained so a runtime switch can
/// compose on it. `OnceLock`, set once by [`install`]; the ONE place a runtime
/// switch reads its base.
static BASE_SPEC: OnceLock<ThemeSpec> = OnceLock::new();

/// The one-shot startup entry: retain the base spec, then apply it.
pub(crate) fn install(spec: ThemeSpec) {
    let _ = BASE_SPEC.set(spec.clone());
    set(spec);
}

/// The SINGLE runtime-switch entry — the TUI `/theme` and the REPL/CLI
/// `set_theme` both route through it. Composes the retained base with the chosen
/// preset (so a `[theme]` role override survives the switch), applies it, and
/// surfaces an unknown name as `Err`.
pub(crate) fn set_preset(name: &str) -> Result<(), String> {
    let base = BASE_SPEC.get().cloned().unwrap_or_default();
    set(base.with_preset(name)?);
    Ok(())
}

/// Honor `NO_COLOR` (<https://no-color.org>) — resolved once.
pub(crate) fn no_color() -> bool {
    static NO_COLOR: OnceLock<bool> = OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some())
}

/// The terminal's color capability, from the environment ladder (plan §3).
/// Resolved once, like [`no_color()`]; used to gate what a [`ThemeSpec`] may
/// emit — see [`ThemeSpec::into_theme`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMode {
    /// Modifiers only — the `Theme::plain` palette.
    Plain,
    /// The terminal's own 16-color palette (`Theme::colored`).
    Named,
    /// A 256-color palette.
    Indexed,
    /// 24-bit truecolor — hex overrides are honored here.
    Rgb,
}

/// The capability ladder exactly as planned: `NO_COLOR` or a dumb terminal is
/// intentionally colorless (nothing later rescues it), `COLORTERM=24bit|truecolor`
/// claims truecolor, a `TERM` containing `256color` claims the 256-tier, and
/// everything else stays on the symbolic 16 palette.
fn resolve_color_mode_from(
    no_color: impl Fn() -> bool,
    colorterm: Option<&str>,
    term: Option<&str>,
) -> ColorMode {
    if no_color() || term == Some("dumb") {
        return ColorMode::Plain;
    }
    let colorterm = colorterm.map(str::to_ascii_lowercase);
    if colorterm.as_deref() == Some("truecolor") || colorterm.as_deref() == Some("24bit") {
        return ColorMode::Rgb;
    }
    if term.is_some_and(|t| t.contains("256color")) {
        return ColorMode::Indexed;
    }
    ColorMode::Named
}

/// The resolved color mode for this process, memoized like [`no_color()`].
pub fn color_mode() -> ColorMode {
    static MODE: OnceLock<ColorMode> = OnceLock::new();
    *MODE.get_or_init(|| {
        let colorterm = std::env::var_os("COLORTERM").and_then(|v| v.into_string().ok());
        let term = std::env::var_os("TERM").and_then(|v| v.into_string().ok());
        resolve_color_mode_from(no_color, colorterm.as_deref(), term.as_deref())
    })
}

/// A preset's SEMANTIC colors (D11) — the single source from which BOTH the 17 UI
/// roles ([`Theme::from_palette`]) AND the syntect theme ([`syntect_theme`])
/// derive, so code and prose cohere per theme.
///
/// ## Caveats
/// - The six truecolor presets are **no-ops on a non-truecolor terminal** (their
///   RGB roles degrade to palette B per role, Q5); `default` (named-16) is the
///   safe everywhere-case. 256/16 quantization stays a follow-up.
/// - A `light` preset is meaningful only on a **light** terminal: the UI has no
///   `bg` role (prose background is the terminal's), and the palette's `fg`/`bg`
///   reach **only** the syntect theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Default code foreground (syntect `settings.foreground`); the UI `body`
    /// stays `None` (prose = the terminal default).
    fg: Color,
    /// Code background (syntect `settings.background`); the UI has no bg role.
    bg: Color,
    /// Primary accent (→ `accent`, `user`).
    accent: Color,
    /// Quiet chrome (→ `muted`).
    muted: Color,
    /// Rules/borders (→ `border`).
    border: Color,
    /// The six hues, reused across the roles and the syntect scopes.
    red: Color,
    green: Color,
    yellow: Color,
    /// Code foreground (→ `code`); palette B keeps it distinct from `warn`.
    code: Color,
    blue: Color,
    magenta: Color,
    cyan: Color,
}

/// A `const`-constructible truecolor helper, so the palette tables stay short.
const fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgb(r, g, b)
}

/// `default` — palette B (named-16); the terminal-adaptive everywhere-case.
const DEFAULT: Palette = Palette {
    fg: Color::Reset,
    bg: Color::Reset,
    accent: Color::Cyan,
    muted: Color::DarkGray,
    border: Color::DarkGray,
    code: Color::Yellow,
    red: Color::Red,
    green: Color::Green,
    yellow: Color::LightYellow,
    blue: Color::Blue,
    magenta: Color::Magenta,
    cyan: Color::Cyan,
};

/// VS Code Dark+.
const DARK: Palette = Palette {
    fg: rgb(0xd4, 0xd4, 0xd4),
    bg: rgb(0x1e, 0x1e, 0x1e),
    accent: rgb(0x56, 0x9c, 0xd6),
    muted: rgb(0x80, 0x80, 0x80),
    border: rgb(0x40, 0x40, 0x40),
    code: rgb(0xdc, 0xdc, 0xaa),
    red: rgb(0xf4, 0x47, 0x47),
    green: rgb(0x6a, 0x99, 0x55),
    yellow: rgb(0xdc, 0xdc, 0xaa),
    blue: rgb(0x56, 0x9c, 0xd6),
    magenta: rgb(0xc5, 0x86, 0xc0),
    cyan: rgb(0x4e, 0xc9, 0xb0),
};

/// VS Code Light+.
const LIGHT: Palette = Palette {
    fg: rgb(0x1f, 0x1f, 0x1f),
    bg: rgb(0xff, 0xff, 0xff),
    accent: rgb(0x00, 0x00, 0xff),
    muted: rgb(0x76, 0x76, 0x76),
    border: rgb(0xd0, 0xd0, 0xd0),
    code: rgb(0x94, 0x98, 0x00),
    red: rgb(0xcd, 0x31, 0x31),
    green: rgb(0x00, 0xbc, 0x00),
    yellow: rgb(0x94, 0x98, 0x00),
    blue: rgb(0x04, 0x51, 0xa5),
    magenta: rgb(0xbc, 0x05, 0xbc),
    cyan: rgb(0x05, 0x98, 0xbc),
};

/// gruvbox dark.
const GRUVBOX_DARK: Palette = Palette {
    fg: rgb(0xeb, 0xdb, 0xb2),
    bg: rgb(0x28, 0x28, 0x28),
    accent: rgb(0xfa, 0xbd, 0x2f),
    muted: rgb(0x92, 0x83, 0x74),
    border: rgb(0x50, 0x49, 0x45),
    code: rgb(0xfa, 0xbd, 0x2f),
    red: rgb(0xfb, 0x49, 0x34),
    green: rgb(0xb8, 0xbb, 0x26),
    yellow: rgb(0xfa, 0xbd, 0x2f),
    blue: rgb(0x83, 0xa5, 0x98),
    magenta: rgb(0xd3, 0x86, 0x9b),
    cyan: rgb(0x8e, 0xc0, 0x7c),
};

/// Nord.
const NORD: Palette = Palette {
    fg: rgb(0xd8, 0xde, 0xe9),
    bg: rgb(0x2e, 0x34, 0x40),
    accent: rgb(0x88, 0xc0, 0xd0),
    muted: rgb(0x4c, 0x56, 0x6a),
    border: rgb(0x43, 0x4c, 0x5e),
    code: rgb(0xeb, 0xcb, 0x8b),
    red: rgb(0xbf, 0x61, 0x6a),
    green: rgb(0xa3, 0xbe, 0x8c),
    yellow: rgb(0xeb, 0xcb, 0x8b),
    blue: rgb(0x81, 0xa1, 0xc1),
    magenta: rgb(0xb4, 0x8e, 0xad),
    cyan: rgb(0x88, 0xc0, 0xd0),
};

/// Solarized dark.
const SOLARIZED_DARK: Palette = Palette {
    fg: rgb(0x83, 0x94, 0x96),
    bg: rgb(0x00, 0x2b, 0x36),
    accent: rgb(0xb5, 0x89, 0x00),
    muted: rgb(0x58, 0x6e, 0x75),
    border: rgb(0x07, 0x36, 0x42),
    code: rgb(0xb5, 0x89, 0x00),
    red: rgb(0xdc, 0x32, 0x2f),
    green: rgb(0x85, 0x99, 0x00),
    yellow: rgb(0xb5, 0x89, 0x00),
    blue: rgb(0x26, 0x8b, 0xd2),
    magenta: rgb(0xd3, 0x36, 0x82),
    cyan: rgb(0x2a, 0xa1, 0x98),
};

/// Solarized light.
const SOLARIZED_LIGHT: Palette = Palette {
    fg: rgb(0x65, 0x7b, 0x83),
    bg: rgb(0xfd, 0xf6, 0xe3),
    accent: rgb(0xb5, 0x89, 0x00),
    muted: rgb(0x93, 0xa1, 0xa1),
    border: rgb(0xee, 0xe8, 0xd5),
    code: rgb(0xb5, 0x89, 0x00),
    red: rgb(0xdc, 0x32, 0x2f),
    green: rgb(0x85, 0x99, 0x00),
    yellow: rgb(0xb5, 0x89, 0x00),
    blue: rgb(0x26, 0x8b, 0xd2),
    magenta: rgb(0xd3, 0x36, 0x82),
    cyan: rgb(0x2a, 0xa1, 0x98),
};

/// The built-in catalog (Q4: 7 presets).
const CATALOG: &[(&str, Palette)] = &[
    ("default", DEFAULT),
    ("dark", DARK),
    ("light", LIGHT),
    ("gruvbox-dark", GRUVBOX_DARK),
    ("nord", NORD),
    ("solarized-dark", SOLARIZED_DARK),
    ("solarized-light", SOLARIZED_LIGHT),
];

/// The palette for a preset `name`, if the catalog knows it.
pub fn preset(name: &str) -> Option<&'static Palette> {
    CATALOG.iter().find(|(n, _)| *n == name).map(|(_, p)| p)
}

/// The catalog's preset names, in order (for an error message / `--list-themes`).
pub fn names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = CATALOG.iter().map(|(n, _)| *n).collect();
    if omarchy_available() {
        names.push(OMARCHY);
    }
    names
}

/// The Omarchy sentinel preset name — recognized wherever a preset name is.
const OMARCHY: &str = "omarchy";

/// Resolve a source NAME to a [`Source`]: a catalog preset, the `omarchy`
/// sentinel (the live desktop palette), or an `Err` naming the known presets.
fn resolve_source(name: &str) -> Result<Source, String> {
    if preset(name).is_some() {
        return Ok(Source::Named(name.into()));
    }
    if name == OMARCHY {
        return omarchy_source();
    }
    Err(format!(
        "unknown theme `{name}`; known: {}",
        names().join(", ")
    ))
}

/// The live Omarchy palette path:
/// `$HOME/.local/state/omarchy/current/theme/colors.toml`. Omarchy writes this
/// literal (it ignores `XDG_STATE_HOME`), so we match it.
fn omarchy_colors_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/state/omarchy/current/theme/colors.toml"))
}

/// Whether a live Omarchy palette is present — gates `omarchy` in [`names`].
fn omarchy_available() -> bool {
    omarchy_colors_path().is_some_and(|p| p.is_file())
}

/// Read and parse the live Omarchy `colors.toml` into a flat key→value map.
fn read_omarchy_colors() -> Result<BTreeMap<String, String>, String> {
    let path = omarchy_colors_path().ok_or("no `HOME` to locate the Omarchy theme")?;
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("no Omarchy theme at {} ({e})", path.display()))?;
    Ok(parse_colors_toml(&text))
}

/// Parse the flat `key = "value"` shape of an Omarchy `colors.toml`: one pair
/// per line, `#` comment lines and blanks skipped, quotes stripped. Omarchy
/// writes a flat map (its own `omarchy-theme-color` reads it the same way), so
/// no TOML parser is needed.
fn parse_colors_toml(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let value = match value.chars().next() {
            // A quoted value ends at its closing quote (a trailing comment drops).
            Some('"' | '\'') => value[1..].split(['"', '\'']).next().unwrap_or(""),
            // An unquoted value is its first whitespace-delimited token.
            _ => value.split_whitespace().next().unwrap_or(""),
        };
        if !key.is_empty() && !value.is_empty() {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}

/// Omarchy's window border, `rgba(6e6f69ee)` — the RGB, alpha dropped. `None`
/// when absent or not an `rgba(rrggbb[aa])` string.
fn omarchy_border(c: &BTreeMap<String, String>) -> Option<Color> {
    let inner = c
        .get("hyprland_active_border")?
        .trim()
        .strip_prefix("rgba(")?
        .strip_suffix(')')?;
    if inner.len() < 6 {
        return None;
    }
    hex_to_rgb(&inner[..6])
}

/// Map a resolved Omarchy palette (key → hex/rgba) to a wcode [`Palette`]. Omarchy
/// `colors.toml` ships in **two schemas**: the built-in **semantic** themes carry
/// `muted`/`red`/`green`/…; third-party **ANSI** themes carry `color0`…`color15`
/// and no semantic keys. `foreground`/`background`/`accent` are required in both;
/// the semantic-only fields fall back to their ANSI slot (`muted`→`color8`,
/// `red`→`color1`, … `cyan`→`color6`). Every field is RGB, so the result also
/// yields a matching syntect theme. A missing required key, or a value
/// `parse_color` rejects, is an `Err`.
fn omarchy_palette(c: &BTreeMap<String, String>) -> Result<Palette, String> {
    // The first present key wins (semantic, else its ANSI fallback); all absent
    // is an error naming every candidate.
    let color_or = |keys: &[&str]| -> Result<Color, String> {
        for key in keys {
            if let Some(v) = c.get(*key) {
                return parse_color(v);
            }
        }
        Err(format!("no `{}` in the Omarchy palette", keys.join("`/`")))
    };
    Ok(Palette {
        fg: color_or(&["foreground"])?,
        bg: color_or(&["background"])?,
        accent: color_or(&["accent"])?,
        // semantic `muted`, else the ANSI bright-black `color8`.
        muted: color_or(&["muted", "color8"])?,
        // Borders are `rgba(...)`, not hex; fall back to `muted` when absent.
        border: match omarchy_border(c) {
            Some(border) => border,
            None => color_or(&["muted", "color8"])?,
        },
        red: color_or(&["red", "color1"])?,
        green: color_or(&["green", "color2"])?,
        yellow: color_or(&["yellow", "color3"])?,
        // Documented collapse: Omarchy has no distinct `code`; palette B already
        // shares its Rgb `code`/`warn` step (`palette_b`).
        code: color_or(&["yellow", "color3"])?,
        blue: color_or(&["blue", "color4"])?,
        magenta: color_or(&["magenta", "color5"])?,
        cyan: color_or(&["cyan", "color6"])?,
    })
}

/// Resolve the `omarchy` sentinel: read the live palette into an inline source.
/// `dim` is not a [`Palette`] field, so it rides in the source's `roles` from
/// Omarchy's `dark_foreground`.
fn omarchy_source() -> Result<Source, String> {
    let colors = read_omarchy_colors().map_err(|e| format!("theme `{OMARCHY}`: {e}"))?;
    let palette = omarchy_palette(&colors).map_err(|e| format!("theme `{OMARCHY}`: {e}"))?;
    let mut roles = BTreeMap::new();
    if let Some(dim) = colors.get("dark_foreground")
        && parse_color(dim).is_ok()
    {
        roles.insert("dim".to_string(), dim.clone());
    }
    Ok(Source::Inline { palette, roles })
}

impl Theme {
    /// ALL 17 roles from a palette (NOT an overlay). Palette B (`DEFAULT`) takes
    /// the D30 tier path ([`palette_b`]) — each role's own Named-16 / 256 / hex
    /// step. Any other preset starts from the bare skeleton
    /// (`Theme::colored()`) so only `fg` varies per role, and its `Rgb` colors
    /// are gated (below); `dim` still takes the grey ramp.
    ///
    /// Per-role Rgb gating, EXPLICIT — the per-`Color` mirror of
    /// [`ThemeSpec::into_theme`]'s per-string rule (Q5): an `Rgb` palette color
    /// under a non-`Rgb` mode keeps that role's palette-B default.
    fn from_palette(p: &Palette, mode: ColorMode) -> Theme {
        if *p == DEFAULT {
            return palette_b(mode); // palette B resolves through its D30 tiers
        }
        let mut theme = Theme::colored();
        let set = |slot: &mut Style, color: Color| {
            if matches!(color, Color::Rgb(..)) && mode != ColorMode::Rgb {
                return;
            }
            slot.fg = Some(color);
        };
        set(&mut theme.accent, p.accent);
        set(&mut theme.user, p.accent);
        set(&mut theme.muted, p.muted);
        set(&mut theme.border, p.border);
        set(&mut theme.error, p.red);
        set(&mut theme.diff_del, p.red);
        set(&mut theme.success, p.green);
        set(&mut theme.diff_add, p.green);
        set(&mut theme.warn, p.yellow);
        set(&mut theme.code, p.code);
        set(&mut theme.heading, p.magenta);
        set(&mut theme.heading_sub, p.magenta);
        set(&mut theme.thinking, p.magenta);
        set(&mut theme.link, p.blue);
        set(&mut theme.tool_name, p.blue);
        apply_dim_ramp(&mut theme, mode);
        theme
    }
}

/// The D29 grey ramp for a color `mode`: `(dim, muted, border)`. `dim` is the
/// universal workhorse grey (no palette defines one); `muted`/`border` are the
/// palette-B steps. `Plain` has no color lever, so callers keep the modifier.
fn grey_ramp(mode: ColorMode) -> (Color, Color, Color) {
    match mode {
        ColorMode::Plain | ColorMode::Named => (Color::Gray, Color::DarkGray, Color::DarkGray),
        ColorMode::Indexed => (Color::Indexed(250), Color::Indexed(243), Color::Indexed(238)),
        ColorMode::Rgb => (
            Color::Rgb(0xa9, 0xb1, 0xba),
            Color::Rgb(0x7d, 0x87, 0x90),
            Color::Rgb(0x3b, 0x42, 0x52),
        ),
    }
}

/// Palette B's universal `dim` grey for a color `mode` — the only grey ramp a
/// named preset inherits (no palette defines a `dim`). `Plain` has no color
/// lever, so the caller keeps the modifier.
fn apply_dim_ramp(theme: &mut Theme, mode: ColorMode) {
    if mode == ColorMode::Plain {
        return;
    }
    theme.dim.fg = Some(grey_ramp(mode).0);
}

/// Palette B at a color tier (D30): the bare skeleton ([`Theme::colored`]) with a
/// grey ramp per tier and a per-tier red for `error` — the only chromatic role.
/// `body`/`user`/`heading`/`heading_sub`/`link`/`code` keep no `fg` (prose = the
/// terminal default; markdown = modifier-only).
fn palette_b(mode: ColorMode) -> Theme {
    let mut theme = Theme::colored();
    let (dim, muted, border) = grey_ramp(mode);
    theme.dim.fg = Some(dim);
    theme.muted.fg = Some(muted);
    theme.border.fg = Some(border);
    // `error` is the ONLY chromatic role — a red per tier:
    let red = match mode {
        ColorMode::Plain | ColorMode::Named => Color::Red,
        ColorMode::Indexed => Color::Indexed(203),
        ColorMode::Rgb => Color::Rgb(0xe0, 0x6c, 0x75),
    };
    theme.error.fg = Some(red);
    // every other non-markdown role -> the SAME dim grey for this tier:
    theme.accent.fg = Some(dim);
    theme.success.fg = Some(dim);
    theme.warn.fg = Some(dim);
    theme.tool_name.fg = Some(dim);
    theme.thinking.fg = Some(dim);
    theme.diff_add.fg = Some(dim);
    theme.diff_del.fg = Some(dim);
    theme
}

/// Map a ratatui color to syntect: ONLY an `Rgb` maps; `Reset`/named → `None`
/// (`syntect::highlighting::Color` is RGBA-only — no named/Reset concept).
fn to_syntect(c: Color) -> Option<syntect::highlighting::Color> {
    match c {
        Color::Rgb(r, g, b) => Some(syntect::highlighting::Color { r, g, b, a: 0xff }),
        _ => None,
    }
}

/// A syntect theme from the palette — built ONLY when EVERY palette color is
/// `to_syntect`-able (an all-RGB palette); `None` otherwise, so the caller
/// ([`set`]) falls back to the bare grey theme
/// ([`crate::markdown::grey_syntect_theme`]). So the `default` preset
/// (named-16/Reset) yields a monochrome fence; the six truecolor presets get a
/// custom theme whose scopes match the UI hues (D11/D14).
fn syntect_theme(p: &Palette) -> Option<syntect::highlighting::Theme> {
    use syntect::highlighting::{ScopeSelectors, ThemeItem, ThemeSettings};

    // EVERY palette color must be RGB (the named-16 `default` palette is not).
    let fg = to_syntect(p.fg)?;
    let bg = to_syntect(p.bg)?;
    let muted = to_syntect(p.muted)?;
    let green = to_syntect(p.green)?;
    let yellow = to_syntect(p.yellow)?;
    let blue = to_syntect(p.blue)?;
    let magenta = to_syntect(p.magenta)?;
    let cyan = to_syntect(p.cyan)?;
    // In the palette but not mapped to a scope; still must be RGB.
    to_syntect(p.accent)?;
    to_syntect(p.border)?;
    to_syntect(p.red)?;

    let item = |scope: &str, foreground: syntect::highlighting::Color| ThemeItem {
        scope: scope.parse::<ScopeSelectors>().expect("valid scope selector"),
        style: syntect::highlighting::StyleModifier {
            foreground: Some(foreground),
            ..Default::default()
        },
    };

    Some(syntect::highlighting::Theme {
        name: None,
        author: None,
        settings: ThemeSettings {
            foreground: Some(fg),
            background: Some(bg),
            ..Default::default()
        },
        scopes: vec![
            item("keyword", magenta),
            item("string", green),
            item("comment", muted),
            item("entity.name.function", blue),
            item("entity.name.type", cyan),
            item("constant", yellow),
            item("variable", fg),
        ],
    })
}

/// A theme's color source (D10): palette B, a catalog preset, or a resolved
/// inline palette. `Default` is palette B.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
enum Source {
    /// Palette B — no preset.
    #[default]
    PaletteB,
    /// A catalog preset name; resolved to a `&'static Palette` in `into_theme`.
    Named(Box<str>),
    /// A resolved inline palette (currently `omarchy`), plus role additions a
    /// [`Palette`] cannot carry — only `dim`, which palette B derives from the
    /// grey ramp instead of a palette field.
    Inline {
        palette: Palette,
        roles: BTreeMap<String, String>,
    },
}

/// A theme source plus a role→color overlay. `Default` (empty) is palette B; a
/// named source selects a palette (or an inline one) and the `roles` overlay
/// sits on top. Built only through [`parse_theme_table`]/[`parse_theme`], which
/// validate every name, role, and color.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeSpec {
    /// The color source (D10). Resolved in `into_theme`.
    source: Source,
    roles: BTreeMap<String, String>,
}

impl ThemeSpec {
    /// This spec with its source replaced by `name` (a catalog preset or the
    /// `omarchy` sentinel); the role overrides are KEPT. An unknown name, or an
    /// unavailable `omarchy`, is an `Err` — never a silent palette B (B2).
    pub fn with_preset(mut self, name: &str) -> Result<ThemeSpec, String> {
        self.source = resolve_source(name)?;
        Ok(self)
    }

    /// The catalogue NAME this spec came from — `None` for palette B, and
    /// [`OMARCHY`] for an inline palette (only [`omarchy_source`] builds one).
    /// Recorded by [`set`] and read by [`active_name`] (W032 A9).
    fn name(&self) -> Option<&'static str> {
        match &self.source {
            Source::PaletteB => None,
            // Re-bound to the CATALOG's own `&'static str`, so the value is a
            // name the `/theme` picker also offers.
            Source::Named(name) => CATALOG
                .iter()
                .find(|(n, _)| *n == &**name)
                .map(|(n, _)| *n),
            Source::Inline { .. } => Some(OMARCHY),
        }
    }

    /// The palette this spec resolves to, if it has one — a catalog preset or a
    /// resolved inline palette. Palette B (`None`) has none. Used by [`set`] to
    /// pick a matching syntect theme.
    fn palette(&self) -> Option<Palette> {
        match &self.source {
            Source::PaletteB => None,
            Source::Named(name) => preset(name).copied(),
            Source::Inline { palette, .. } => Some(*palette),
        }
    }

    /// The theme for this spec: the preset palette (if any) with the role
    /// overrides applied. A preset seeds ALL 17 roles ([`Theme::from_palette`]);
    /// then each named role changes only its `fg` — its modifiers stay (`link`
    /// underlined, `thinking` italic). A hex
    /// (`#rrggbb` → [`Color::Rgb`]) override is honored only when the terminal
    /// is truecolor ([`ColorMode::Rgb`]): under Plain / Named / Indexed it
    /// degrades to that role's palette-B default rather than emit an `Rgb` a
    /// non-truecolor terminal renders wrong. Named colors are honored in every
    /// color mode. [`parse_theme_table`] validated the values; a stray one
    /// cannot reach here.
    fn into_theme(self, mode: ColorMode) -> Theme {
        let ThemeSpec { source, roles } = self;
        let mut theme = match source {
            // Palette B resolves through its D30 tiers; a hex palette degrades
            // per-role under a non-`Rgb` mode (see `from_palette`).
            Source::PaletteB => palette_b(mode),
            Source::Named(name) => preset(&name)
                .map_or_else(|| palette_b(mode), |p| Theme::from_palette(p, mode)),
            Source::Inline { palette, roles } => {
                let mut theme = Theme::from_palette(&palette, mode);
                apply_roles(&mut theme, &roles, mode);
                theme
            }
        };
        apply_roles(&mut theme, &roles, mode);
        theme
    }
}

/// Apply a role→color overlay to `theme`. Each named role changes only its `fg`
/// — its modifiers stay (`link` underlined, `thinking` italic). A hex
/// (`#rrggbb` → [`Color::Rgb`]) value is honored
/// only when the terminal is truecolor ([`ColorMode::Rgb`]); under Plain / Named
/// / Indexed it is skipped, keeping the role's base color. [`parse_theme_table`]
/// validated the values; a stray one cannot reach here.
fn apply_roles(theme: &mut Theme, roles: &BTreeMap<String, String>, mode: ColorMode) {
    for (role, value) in roles {
        let Ok(color) = parse_color(value) else {
            continue;
        };
        if matches!(color, Color::Rgb(..)) && mode != ColorMode::Rgb {
            continue;
        }
        let slot = match role.as_str() {
            "accent" => &mut theme.accent,
            "dim" => &mut theme.dim,
            "muted" => &mut theme.muted,
            "border" => &mut theme.border,
            "user" => &mut theme.user,
            "body" => &mut theme.body,
            "error" => &mut theme.error,
            "success" => &mut theme.success,
            "warn" => &mut theme.warn,
            "code" => &mut theme.code,
            "heading" => &mut theme.heading,
            "heading_sub" => &mut theme.heading_sub,
            "link" => &mut theme.link,
            "tool_name" => &mut theme.tool_name,
            "thinking" => &mut theme.thinking,
            "diff_add" => &mut theme.diff_add,
            "diff_del" => &mut theme.diff_del,
            _ => continue,
        };
        slot.fg = Some(color);
    }
}

/// The 17 role names a `[theme]` table may override.
const ROLE_NAMES: [&str; 17] = [
    "accent",
    "dim",
    "muted",
    "border",
    "user",
    "body",
    "error",
    "success",
    "warn",
    "code",
    "heading",
    "heading_sub",
    "link",
    "tool_name",
    "thinking",
    "diff_add",
    "diff_del",
];

/// Pull `name` out, resolve the preset, then validate/overlay the role keys.
/// An unknown `name` (or role/color) is a hard `Err` (the existing style).
pub fn parse_theme_table(table: &BTreeMap<String, String>) -> Result<ThemeSpec, String> {
    let mut roles = table.clone();
    let name = roles.remove("name");
    let spec = parse_theme(&roles)?; // role/color validation, unchanged
    match name {
        // One resolution path for every source (catalog preset or `omarchy`).
        Some(n) => spec.with_preset(&n),
        None => Ok(spec),
    }
}

/// Validate a `[theme]` table (role → color) into a [`ThemeSpec`]. An unknown
/// role or an unparseable color is a hard error — the caller surfaces it.
pub fn parse_theme(roles: &BTreeMap<String, String>) -> Result<ThemeSpec, String> {
    let mut spec = ThemeSpec::default();
    for (role, value) in roles {
        if !ROLE_NAMES.contains(&role.as_str()) {
            return Err(format!("unknown theme role `{role}`"));
        }
        if parse_color(value).is_err() {
            return Err(format!("invalid color `{value}` for role `{role}`"));
        }
        spec.roles.insert(role.clone(), value.clone());
    }
    Ok(spec)
}

/// A color value: a `#rrggbb` hex (truecolor, opt-in) or a named ANSI color.
/// Kebab-case names only; the input is trimmed and lower-cased.
fn parse_color(s: &str) -> Result<Color, String> {
    let s = s.trim().to_ascii_lowercase();
    if let Some(hex) = s.strip_prefix('#') {
        return hex_to_rgb(hex).ok_or_else(|| format!("invalid hex color `{s}`"));
    }
    let color = match s.as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" => Color::Magenta,
        "cyan" => Color::Cyan,
        "gray" | "grey" => Color::Gray,
        "dark-gray" | "dark-grey" => Color::DarkGray,
        "light-red" => Color::LightRed,
        "light-green" => Color::LightGreen,
        "light-yellow" => Color::LightYellow,
        "light-blue" => Color::LightBlue,
        "light-magenta" => Color::LightMagenta,
        "light-cyan" => Color::LightCyan,
        "white" => Color::White,
        "reset" | "default" => Color::Reset,
        other => return Err(format!("unknown color `{other}`")),
    };
    Ok(color)
}

/// Exactly six hex digits → a truecolor [`Color::Rgb`].
fn hex_to_rgb(hex: &str) -> Option<Color> {
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// The palette for `spec`: `NO_COLOR` wins (plain, overrides ignored), else the
/// overlay on palette B.
fn resolve(spec: ThemeSpec, no_color: bool) -> Theme {
    if no_color {
        Theme::plain()
    } else {
        spec.into_theme(color_mode())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_error_carries_a_foreground_colour() {
        // W018 §4.1: palette B is the grey ramp + a per-tier red for `error` — no
        // other role carries a hue (its fg is `None` or on the WHOLE grey ramp).
        for mode in [ColorMode::Named, ColorMode::Indexed, ColorMode::Rgb] {
            let (d, m, b) = grey_ramp(mode);
            let t = ThemeSpec::default().into_theme(mode);
            let red = match mode {
                ColorMode::Indexed => Color::Indexed(203),
                ColorMode::Rgb => Color::Rgb(0xe0, 0x6c, 0x75),
                _ => Color::Red,
            };
            assert_eq!(t.error.fg, Some(red), "error is the only colour ({mode:?})");
            let roles: [(&str, &Style); 16] = [
                ("accent", &t.accent),
                ("dim", &t.dim),
                ("muted", &t.muted),
                ("border", &t.border),
                ("user", &t.user),
                ("body", &t.body),
                ("success", &t.success),
                ("warn", &t.warn),
                ("code", &t.code),
                ("heading", &t.heading),
                ("heading_sub", &t.heading_sub),
                ("link", &t.link),
                ("tool_name", &t.tool_name),
                ("thinking", &t.thinking),
                ("diff_add", &t.diff_add),
                ("diff_del", &t.diff_del),
            ];
            for (role, style) in roles {
                assert!(
                    style.fg.is_none()
                        || matches!(style.fg, Some(c) if c == d || c == m || c == b),
                    "{role} must be on the grey ramp (or default), not a hue ({mode:?}): {style:?}"
                );
            }
        }
    }

    #[test]
    fn the_bare_default_code_fence_is_monochrome() {
        // Defect #1 (Option A): the bare default's fence theme declares NO scoped hue,
        // so every token resolves to the one grey foreground. Built locally (the
        // process-wide syntax-theme store would race other fence tests). The grey is
        // checked by RAMP MEMBERSHIP, not a literal.
        let theme = crate::markdown::grey_syntect_theme();
        assert!(
            theme.scopes.is_empty(),
            "the bare fence theme declares no scoped hue"
        );
        let (d, m, b) = grey_ramp(ColorMode::Rgb);
        let on_ramp = |c: syntect::highlighting::Color| {
            [d, m, b].iter().any(
                |x| matches!(x, Color::Rgb(rr, gg, bb) if *rr == c.r && *gg == c.g && *bb == c.b),
            )
        };
        let fg = theme.settings.foreground.expect("a grey foreground");
        assert!(
            on_ramp(fg),
            "the fence foreground {fg:?} must be on the grey ramp"
        );

        // Every highlighted token of a real Rust fence takes that one grey (a
        // per-token list, not a role list): no keyword/string/comment hue survives.
        use syntect::easy::HighlightLines;
        use syntect::parsing::SyntaxSet;
        let ss = SyntaxSet::load_defaults_nonewlines();
        let syntax = ss.find_syntax_by_token("rust").expect("the rust syntax");
        let mut hl = HighlightLines::new(syntax, &theme);
        let ranges = hl
            .highlight_line("let x = 1; // a comment\n", &ss)
            .expect("a highlighted line");
        let tokens: Vec<(&str, syntect::highlighting::Color)> =
            ranges.into_iter().map(|(st, txt)| (txt, st.foreground)).collect();
        assert!(tokens.len() >= 2, "the fence yields several tokens: {tokens:?}");
        for (token, c) in tokens {
            assert!(on_ramp(c), "the fence token {token:?} carries a hue: {c:?}");
        }
    }
    /// Every role, so a test can assert over all of them.
    fn fields(theme: &Theme) -> [&Style; 17] {
        [
            &theme.accent,
            &theme.dim,
            &theme.muted,
            &theme.border,
            &theme.user,
            &theme.body,
            &theme.error,
            &theme.success,
            &theme.warn,
            &theme.code,
            &theme.heading,
            &theme.heading_sub,
            &theme.link,
            &theme.tool_name,
            &theme.thinking,
            &theme.diff_add,
            &theme.diff_del,
        ]
    }

    #[test]
    fn the_plain_theme_sets_no_foreground_on_any_role() {
        // The NO_COLOR contract: no role may carry an `fg`, or a colorless
        // terminal would still get escapes. (Do not flip the process-global
        // `NO_COLOR` in a test — build the palette directly.)
        for style in fields(&Theme::plain()) {
            assert_eq!(style.fg, None, "NO_COLOR must not set an fg: {style:?}");
        }
    }

    #[test]
    fn dim_carries_an_explicit_grey_and_no_dim_modifier() {
        // D29: `dim` is an explicit grey under color, not the `DIM` modifier.
        let colored = Theme::colored();
        assert!(colored.dim.fg.is_some(), "dim must carry an explicit grey");
        assert!(
            !colored.dim.add_modifier.contains(Modifier::DIM),
            "dim must not use the DIM modifier under color"
        );
        // `thinking` drops DIM too, keeping the grey + italic.
        assert!(!colored.thinking.add_modifier.contains(Modifier::DIM));
        assert!(colored.thinking.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(colored.thinking.fg, Some(Color::Gray));

        // NO_COLOR has no color lever, so `dim` keeps the modifier.
        assert!(
            Theme::plain().dim.add_modifier.contains(Modifier::DIM),
            "NO_COLOR keeps the DIM modifier"
        );
    }

    #[test]
    fn each_tier_yields_its_own_grey_steps() {
        // D29: the grey ramp is picked per color mode (palette B — the `default`
        // preset / no preset). Each literal below is pinned, so a transposed
        // 238/243, a 250→251 typo, or a swapped hex byte would fail.
        let named = ThemeSpec::default().into_theme(ColorMode::Named);
        assert_eq!(named.dim.fg, Some(Color::Gray));
        assert_eq!(named.muted.fg, Some(Color::DarkGray));
        assert_eq!(named.border.fg, Some(Color::DarkGray));

        let indexed = ThemeSpec::default().into_theme(ColorMode::Indexed);
        assert_eq!(indexed.dim.fg, Some(Color::Indexed(250)));
        assert_eq!(indexed.muted.fg, Some(Color::Indexed(243)));
        assert_eq!(indexed.border.fg, Some(Color::Indexed(238)));

        let rgb = ThemeSpec::default().into_theme(ColorMode::Rgb);
        assert_eq!(rgb.dim.fg, Some(Color::Rgb(0xa9, 0xb1, 0xba)));
        assert_eq!(rgb.muted.fg, Some(Color::Rgb(0x7d, 0x87, 0x90)));
        assert_eq!(rgb.border.fg, Some(Color::Rgb(0x3b, 0x42, 0x52)));

        // The three steps are distinct within a tier — except Named-16, where
        // `muted` and `border` share the one quiet grey (documented, D29).
        assert_ne!(named.dim.fg, named.muted.fg);
        assert_eq!(named.muted.fg, named.border.fg, "the named-16 coincidence");
        for (label, tier) in [("indexed", &indexed), ("rgb", &rgb)] {
            assert_ne!(tier.dim.fg, tier.muted.fg, "{label}: dim vs muted");
            assert_ne!(tier.muted.fg, tier.border.fg, "{label}: muted vs border");
            assert_ne!(tier.dim.fg, tier.border.fg, "{label}: dim vs border");
        }
    }

    #[test]
    fn each_role_resolves_its_tier_value() {
        // D30/W018: palette B is the grey ramp at each mode, plus a per-tier red for
        // `error` — the only chromatic role. Every literal is pinned.
        let named = ThemeSpec::default().into_theme(ColorMode::Named);
        assert_eq!(named.error.fg, Some(Color::Red));
        assert_eq!(named.dim.fg, Some(Color::Gray));
        assert_eq!(named.muted.fg, Some(Color::DarkGray));
        assert_eq!(named.border.fg, Some(Color::DarkGray));
        for role in [&named.accent, &named.success, &named.warn, &named.tool_name, &named.thinking, &named.diff_add, &named.diff_del] {
            assert_eq!(role.fg, Some(Color::Gray), "collapses to the dim grey");
        }
        for role in [&named.body, &named.user, &named.heading, &named.heading_sub, &named.link, &named.code] {
            assert_eq!(role.fg, None, "keeps the default fg");
        }

        let indexed = ThemeSpec::default().into_theme(ColorMode::Indexed);
        assert_eq!(indexed.error.fg, Some(Color::Indexed(203)));
        assert_eq!(indexed.dim.fg, Some(Color::Indexed(250)));
        assert_eq!(indexed.muted.fg, Some(Color::Indexed(243)));
        assert_eq!(indexed.border.fg, Some(Color::Indexed(238)));
        for role in [&indexed.accent, &indexed.success, &indexed.warn, &indexed.tool_name, &indexed.thinking, &indexed.diff_add, &indexed.diff_del] {
            assert_eq!(role.fg, Some(Color::Indexed(250)), "collapses to the dim grey");
        }
        for role in [&indexed.body, &indexed.user, &indexed.heading, &indexed.heading_sub, &indexed.link, &indexed.code] {
            assert_eq!(role.fg, None, "keeps the default fg");
        }

        let rgb = ThemeSpec::default().into_theme(ColorMode::Rgb);
        assert_eq!(rgb.error.fg, Some(Color::Rgb(0xe0, 0x6c, 0x75)));
        assert_eq!(rgb.dim.fg, Some(Color::Rgb(0xa9, 0xb1, 0xba)));
        assert_eq!(rgb.muted.fg, Some(Color::Rgb(0x7d, 0x87, 0x90)));
        assert_eq!(rgb.border.fg, Some(Color::Rgb(0x3b, 0x42, 0x52)));
        for role in [&rgb.accent, &rgb.success, &rgb.warn, &rgb.tool_name, &rgb.thinking, &rgb.diff_add, &rgb.diff_del] {
            assert_eq!(role.fg, Some(Color::Rgb(0xa9, 0xb1, 0xba)), "collapses to the dim grey");
        }
        for role in [&rgb.body, &rgb.user, &rgb.heading, &rgb.heading_sub, &rgb.link, &rgb.code] {
            assert_eq!(role.fg, None, "keeps the default fg");
        }
    }

    #[test]
    fn the_colored_theme_signals_errors_in_red() {
        assert_eq!(Theme::colored().error.fg, Some(Color::Red));
    }

    #[test]
    fn the_semantic_palette_keeps_roles_distinct() {
        // W018: palette B collapses to the grey ramp — roles stay distinct by GREY
        // STEP and non-colour modifier, never by hue.
        let t = Theme::colored();
        // D29: `muted`/`border` share the one named-16 quiet grey but diverge
        // above it (243 vs 238, #7d8790 vs #3b4252).
        assert_eq!(t.muted.fg, t.border.fg, "both the named-16 quiet grey");
        let rgb = ThemeSpec::default().into_theme(ColorMode::Rgb);
        assert_ne!(rgb.muted.fg, rgb.border.fg, "muted vs border, truecolor");
        assert_ne!(t.code.fg, t.warn.fg, "code (default) vs warn (grey)");
        assert_ne!(t.tool_name, t.link, "tool_name (grey) vs link (underline)");
        assert!(
            !t.tool_name.add_modifier.contains(Modifier::BOLD),
            "W018: tool_name drops its BOLD (the skeleton is bare)"
        );
        assert_eq!(
            t.thinking.fg,
            Some(Color::Gray),
            "thinking carries the grey, not a meta hue"
        );
        assert_ne!(t.heading, t.body, "headings stay bold, body default");
        assert!(
            t.heading.fg.is_none() && t.heading.add_modifier.contains(Modifier::BOLD),
            "headings are BOLD with the default fg"
        );
        // Intentional aliases — not asserted distinct:
        // accent == success == warn == the dim grey; error == the one red.
    }

    #[test]
    fn parse_theme_accepts_a_named_color_and_a_hex() {
        let roles = BTreeMap::from([
            ("accent".to_string(), "light-magenta".to_string()),
            ("muted".to_string(), "#123456".to_string()),
        ]);
        let theme = parse_theme(&roles).expect("a valid spec").into_theme(ColorMode::Rgb);
        assert_eq!(theme.accent.fg, Some(Color::LightMagenta));
        assert_eq!(theme.muted.fg, Some(Color::Rgb(0x12, 0x34, 0x56)));
    }

    #[test]
    fn parse_theme_rejects_an_unknown_role_or_color() {
        let role = BTreeMap::from([("nope".to_string(), "red".to_string())]);
        assert!(parse_theme(&role).is_err(), "an unknown role is an error");
        let color = BTreeMap::from([("accent".to_string(), "chartreuse".to_string())]);
        assert!(parse_theme(&color).is_err(), "an unknown color is an error");
    }

    #[test]
    fn an_override_changes_only_its_role() {
        let roles = BTreeMap::from([
            ("link".to_string(), "light-cyan".to_string()),
            ("tool_name".to_string(), "light-green".to_string()),
        ]);
        let theme = parse_theme(&roles).unwrap().into_theme(ColorMode::Rgb);
        assert_eq!(theme.link.fg, Some(Color::LightCyan));
        assert_eq!(theme.tool_name.fg, Some(Color::LightGreen));
        assert!(theme.link.add_modifier.contains(Modifier::UNDERLINED));
        // W018: the bare skeleton drops the `tool_name` BOLD.
        assert!(!theme.tool_name.add_modifier.contains(Modifier::BOLD));
        // Untouched roles keep palette B at the chosen tier (D30).
        let base = ThemeSpec::default().into_theme(ColorMode::Rgb);
        assert_eq!(theme.error.fg, base.error.fg);
        assert_eq!(theme.heading, base.heading);
    }

    #[test]
    fn resolve_color_mode_follows_the_ladder() {
        let none = || false;
        // NO_COLOR beats everything, even a truecolor claim.
        assert_eq!(
            resolve_color_mode_from(|| true, Some("truecolor"), Some("xterm-256color")),
            ColorMode::Plain,
            "NO_COLOR wins"
        );
        // A dumb TERM is colorless regardless of COLORTERM.
        assert_eq!(
            resolve_color_mode_from(none, Some("truecolor"), Some("dumb")),
            ColorMode::Plain,
            "TERM=dumb is plain"
        );
        // Both truecolor spellings.
        assert_eq!(
            resolve_color_mode_from(none, Some("truecolor"), None),
            ColorMode::Rgb
        );
        assert_eq!(
            resolve_color_mode_from(none, Some("24bit"), None),
            ColorMode::Rgb
        );
        assert_eq!(
            resolve_color_mode_from(none, Some("TrueColor"), Some("xterm-256color")),
            ColorMode::Rgb,
            "truecolor claims beat a 256color TERM; COLORTERM is case-insensitive"
        );
        // A 256-color TERM.
        assert_eq!(
            resolve_color_mode_from(none, None, Some("xterm-256color")),
            ColorMode::Indexed
        );
        // Nothing claimed: the symbolic 16 palette.
        assert_eq!(resolve_color_mode_from(none, None, None), ColorMode::Named);
        assert_eq!(
            resolve_color_mode_from(none, None, Some("xterm")),
            ColorMode::Named
        );
    }

    #[test]
    fn hex_overrides_are_honored_only_under_truecolor() {
        let roles = BTreeMap::from([("accent".to_string(), "#ff00cc".to_string())]);
        let spec = parse_theme(&roles).unwrap();
        assert_eq!(
            spec.clone().into_theme(ColorMode::Rgb).accent.fg,
            Some(Color::Rgb(0xff, 0x00, 0xcc)),
            "Rgb mode honors the hex"
        );
        for degraded in [ColorMode::Plain, ColorMode::Named, ColorMode::Indexed] {
            let theme = spec.clone().into_theme(degraded);
            // D30: a role's palette-B default is its tier step for the mode.
            let tier = ThemeSpec::default().into_theme(degraded).accent.fg;
            assert_eq!(
                theme.accent.fg, tier,
                "hex degrades to the role's palette-B tier under {degraded:?}"
            );
            assert!(
                !theme.accent.add_modifier.contains(Modifier::BOLD),
                "W018: the bare skeleton drops the accent BOLD"
            );
        }
    }

    #[test]
    fn named_colors_are_honored_in_every_color_mode() {
        let roles = BTreeMap::from([("warn".to_string(), "light-cyan".to_string())]);
        let spec = parse_theme(&roles).unwrap();
        for mode in [
            ColorMode::Plain,
            ColorMode::Named,
            ColorMode::Indexed,
            ColorMode::Rgb,
        ] {
            assert_eq!(
                spec.clone().into_theme(mode).warn.fg,
                Some(Color::LightCyan),
                "a named color is honored under {mode:?}"
            );
        }
    }

    #[test]
    fn no_color_wins_over_a_spec() {
        let roles = BTreeMap::from([("accent".to_string(), "red".to_string())]);
        let spec = parse_theme(&roles).expect("a valid spec");
        let plain = resolve(spec, true);
        for style in fields(&plain) {
            assert_eq!(style.fg, None, "NO_COLOR must not set an fg: {style:?}");
        }
    }

    // ---- Part B: the catalog, the `name` selector, and the syntect theme ----

    #[test]
    fn the_default_preset_is_palette_b() {
        let theme = Theme::from_palette(preset("default").unwrap(), ColorMode::Named);
        assert_eq!(theme.error.fg, Some(Color::Red), "the only colour");
        assert_eq!(theme.muted.fg, Some(Color::DarkGray), "D29 named grey");
        assert_eq!(theme.border.fg, Some(Color::DarkGray));
        assert_eq!(theme.warn.fg, Some(Color::Gray), "warn collapses to the grey");
        assert_eq!(theme.code.fg, None, "code drops its fg (Q3)");
        assert_eq!(theme.body.fg, None, "prose stays the terminal default");
        assert_eq!(theme.dim.fg, Some(Color::Gray), "dim carries the D29 grey");
    }

    #[test]
    fn a_truecolor_preset_sets_rgb_roles() {
        let p = preset("gruvbox-dark").unwrap();
        let theme = Theme::from_palette(p, ColorMode::Rgb);
        assert_eq!(theme.accent.fg, Some(Color::Rgb(0xfa, 0xbd, 0x2f)));
        assert_eq!(theme.error.fg, Some(Color::Rgb(0xfb, 0x49, 0x34)));
        assert_eq!(theme.link.fg, Some(Color::Rgb(0x83, 0xa5, 0x98)));
        // The intentional aliases: accent == user; heading/heading_sub/thinking.
        assert_eq!(theme.user.fg, theme.accent.fg);
        assert_eq!(theme.heading_sub.fg, theme.thinking.fg);
    }

    #[test]
    fn a_truecolor_preset_degrades_under_a_non_rgb_mode() {
        let p = preset("nord").unwrap();
        let base = Theme::colored();
        for mode in [ColorMode::Plain, ColorMode::Named, ColorMode::Indexed] {
            let theme = Theme::from_palette(p, mode);
            assert_eq!(theme.accent.fg, base.accent.fg, "{mode:?}");
            assert_eq!(theme.error.fg, base.error.fg, "{mode:?}");
            assert_eq!(theme.link.fg, base.link.fg, "{mode:?}");
        }
    }

    #[test]
    fn parse_theme_table_resolves_a_preset_and_rejects_an_unknown_name() {
        let table = BTreeMap::from([("name".to_string(), "nord".to_string())]);
        let spec = parse_theme_table(&table).expect("a known preset");
        assert_eq!(spec.source, Source::Named("nord".into()));

        let bad = BTreeMap::from([("name".to_string(), "chartreuse".to_string())]);
        let err = parse_theme_table(&bad).unwrap_err();
        assert!(err.contains("unknown theme"), "{err}");
        assert!(err.contains("nord"), "the error lists known names: {err}");
    }

    #[test]
    fn role_overrides_apply_on_top_of_a_preset() {
        let table = BTreeMap::from([
            ("name".to_string(), "nord".to_string()),
            ("warn".to_string(), "light-cyan".to_string()),
        ]);
        let theme = parse_theme_table(&table).unwrap().into_theme(ColorMode::Rgb);
        // The override wins for its role...
        assert_eq!(theme.warn.fg, Some(Color::LightCyan));
        // ...while the preset still seeds the others.
        assert_eq!(theme.error.fg, Some(Color::Rgb(0xbf, 0x61, 0x6a)));
        assert_eq!(theme.accent.fg, Some(Color::Rgb(0x88, 0xc0, 0xd0)));
    }

    #[test]
    fn no_color_wins_over_a_preset() {
        let table = BTreeMap::from([("name".to_string(), "nord".to_string())]);
        let spec = parse_theme_table(&table).unwrap();
        let plain = resolve(spec, true);
        for style in fields(&plain) {
            assert_eq!(style.fg, None, "NO_COLOR must not set an fg: {style:?}");
        }
    }

    #[test]
    fn to_syntect_maps_only_rgb() {
        assert_eq!(
            to_syntect(Color::Rgb(1, 2, 3)),
            Some(syntect::highlighting::Color {
                r: 1,
                g: 2,
                b: 3,
                a: 0xff
            })
        );
        assert_eq!(to_syntect(Color::Red), None);
        assert_eq!(to_syntect(Color::Reset), None);
    }

    #[test]
    fn syntect_theme_is_built_only_for_an_all_rgb_palette() {
        assert!(
            syntect_theme(preset("default").unwrap()).is_none(),
            "a named-16 palette has no RGB to map"
        );

        let p = preset("nord").unwrap();
        let theme = syntect_theme(p).expect("an all-RGB palette yields Some");
        assert_eq!(theme.settings.foreground, to_syntect(p.fg));
        assert_eq!(theme.settings.background, to_syntect(p.bg));
        assert_eq!(theme.scopes.len(), 7);
        let fg = |i: usize| theme.scopes[i].style.foreground;
        assert_eq!(fg(0), to_syntect(p.magenta), "keyword");
        assert_eq!(fg(1), to_syntect(p.green), "string");
        assert_eq!(fg(2), to_syntect(p.muted), "comment");
        assert_eq!(fg(3), to_syntect(p.blue), "entity.name.function");
        assert_eq!(fg(4), to_syntect(p.cyan), "entity.name.type");
        assert_eq!(fg(5), to_syntect(p.yellow), "constant");
        assert_eq!(fg(6), to_syntect(p.fg), "variable");
    }

    // ---- T4: the mutable store, the runtime switch, the syntect swap ----

    #[test]
    fn a_set_rewrites_the_process_theme() {
        // The `RwLock` store: a `set` is visible to a later `theme()` copy. The
        // global is restored immediately (it is process-wide).
        if no_color() {
            return; // NO_COLOR forces `plain`; a role override is ignored by design
        }
        let spec =
            parse_theme(&BTreeMap::from([("accent".to_string(), "red".to_string())])).unwrap();
        set(spec);
        assert_eq!(theme().accent.fg, Some(Color::Red));
        set(ThemeSpec::default());
    }

    #[test]
    fn with_preset_rejects_an_unknown_name() {
        let base = ThemeSpec::default();
        let err = base.with_preset("chartreuse").unwrap_err();
        assert!(err.contains("unknown theme"), "{err}");
        assert!(err.contains("nord"), "lists the known names: {err}");
    }

    #[test]
    fn with_preset_keeps_the_role_overrides() {
        // The base-spec guarantee: a `[theme]` role override survives a switch.
        let base =
            parse_theme(&BTreeMap::from([("warn".to_string(), "light-cyan".to_string())])).unwrap();
        let switched = base.with_preset("nord").expect("a known preset");
        assert_eq!(switched.source, Source::Named("nord".into()));
        let theme = switched.into_theme(ColorMode::Rgb);
        assert_eq!(theme.warn.fg, Some(Color::LightCyan), "the override survived");
        assert_eq!(
            theme.accent.fg,
            Some(Color::Rgb(0x88, 0xc0, 0xd0)),
            "the preset applied"
        );
    }

    // ---- Omarchy: the flat colors.toml, the palette mapping, the source ----

    #[test]
    fn parse_colors_toml_reads_a_flat_map() {
        let text = "\
# a comment
mode = \"dark\"

accent = \"#7fa66e\"
background = '#161716'
hyprland_active_border = \"rgba(6e6f69ee)\"
unquoted = dark
";
        let map = parse_colors_toml(text);
        assert_eq!(map.get("mode").map(String::as_str), Some("dark"));
        assert_eq!(map.get("accent").map(String::as_str), Some("#7fa66e"));
        // A single-quoted value — the `#` inside is not a comment.
        assert_eq!(map.get("background").map(String::as_str), Some("#161716"));
        assert_eq!(
            map.get("hyprland_active_border").map(String::as_str),
            Some("rgba(6e6f69ee)")
        );
        assert_eq!(map.get("unquoted").map(String::as_str), Some("dark"));
        assert_eq!(map.len(), 5, "comments and blanks are skipped");
    }

    #[test]
    fn omarchy_border_takes_the_rgb_and_drops_the_alpha() {
        let c = BTreeMap::from([(
            "hyprland_active_border".to_string(),
            "rgba(6e6f69ee)".to_string(),
        )]);
        assert_eq!(omarchy_border(&c), Some(Color::Rgb(0x6e, 0x6f, 0x69)));
        // Absent, or a non-hex rgba → no border (the caller falls back to muted).
        assert_eq!(omarchy_border(&BTreeMap::new()), None);
        let decimal = BTreeMap::from([(
            "hyprland_active_border".to_string(),
            "rgba(110,111,105,0.9)".to_string(),
        )]);
        assert_eq!(omarchy_border(&decimal), None);
    }

    /// A full Omarchy palette, as its `colors.toml` defines it (this machine's).
    fn omarchy_fixture() -> BTreeMap<String, String> {
        [
            ("accent", "#7fa66e"),
            ("foreground", "#cfcdc4"),
            ("background", "#161716"),
            ("muted", "#62635e"),
            ("dark_foreground", "#7c7d77"),
            ("red", "#c98378"),
            ("green", "#7fa66e"),
            ("yellow", "#d6bd8a"),
            ("blue", "#869fae"),
            ("magenta", "#a798b0"),
            ("cyan", "#88aea7"),
            ("hyprland_active_border", "rgba(6e6f69ee)"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    /// A third-party Omarchy theme in the **ANSI** schema — the `kanso` keys
    /// (`color0`…`color15`, `foreground`/`background`/`accent`), with NO semantic
    /// `muted`/`red`/… — so the semantic-only fields must fall back to their ANSI slot.
    fn omarchy_ansi_fixture() -> BTreeMap<String, String> {
        [
            ("accent", "#8ba4b0"),
            ("foreground", "#C5C9C7"),
            ("background", "#090E13"),
            ("color0", "#090E13"),
            ("color1", "#c4746e"),
            ("color2", "#8a9a7b"),
            ("color3", "#c4b28a"),
            ("color4", "#8ba4b0"),
            ("color5", "#a292a3"),
            ("color6", "#8ea4a2"),
            ("color7", "#c8c093"),
            ("color8", "#393B44"),
            ("color9", "#e46876"),
            ("color10", "#87a987"),
            ("color11", "#e6c384"),
            ("color12", "#7fb4ca"),
            ("color13", "#938aa9"),
            ("color14", "#7aa89f"),
            ("color15", "#A4A7A4"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    #[test]
    fn omarchy_palette_maps_every_field() {
        let p = omarchy_palette(&omarchy_fixture()).expect("a full palette");
        assert_eq!(p.fg, Color::Rgb(0xcf, 0xcd, 0xc4));
        assert_eq!(p.bg, Color::Rgb(0x16, 0x17, 0x16));
        assert_eq!(p.accent, Color::Rgb(0x7f, 0xa6, 0x6e));
        assert_eq!(p.muted, Color::Rgb(0x62, 0x63, 0x5e));
        assert_eq!(p.border, Color::Rgb(0x6e, 0x6f, 0x69), "the hyprland border RGB");
        assert_eq!(p.red, Color::Rgb(0xc9, 0x83, 0x78));
        assert_eq!(p.green, Color::Rgb(0x7f, 0xa6, 0x6e));
        assert_eq!(p.yellow, Color::Rgb(0xd6, 0xbd, 0x8a));
        assert_eq!(p.code, p.yellow, "the documented code/warn collapse");
        assert_eq!(p.blue, Color::Rgb(0x86, 0x9f, 0xae));
        assert_eq!(p.magenta, Color::Rgb(0xa7, 0x98, 0xb0));
        assert_eq!(p.cyan, Color::Rgb(0x88, 0xae, 0xa7));
    }

    #[test]
    fn omarchy_palette_falls_back_to_muted_without_a_border() {
        let mut c = omarchy_fixture();
        c.remove("hyprland_active_border");
        let p = omarchy_palette(&c).expect("a palette");
        assert_eq!(p.border, p.muted, "no rgba border → muted");
    }

    #[test]
    fn omarchy_palette_resolves_the_ansi_schema() {
        // W019: a third-party theme (kanso) uses the ANSI schema, so the semantic-only
        // fields fall back to their `colorN` slot; `border` (no
        // `hyprland_active_border`) falls back to `muted` (= `color8`).
        let p = omarchy_palette(&omarchy_ansi_fixture()).expect("a full ANSI palette");
        assert_eq!(p.fg, Color::Rgb(0xc5, 0xc9, 0xc7), "foreground");
        assert_eq!(p.bg, Color::Rgb(0x09, 0x0e, 0x13), "background");
        assert_eq!(p.accent, Color::Rgb(0x8b, 0xa4, 0xb0), "accent");
        assert_eq!(p.muted, Color::Rgb(0x39, 0x3b, 0x44), "muted <- color8");
        assert_eq!(p.border, p.muted, "no rgba border -> muted");
        assert_eq!(p.red, Color::Rgb(0xc4, 0x74, 0x6e), "red <- color1");
        assert_eq!(p.green, Color::Rgb(0x8a, 0x9a, 0x7b), "green <- color2");
        assert_eq!(p.yellow, Color::Rgb(0xc4, 0xb2, 0x8a), "yellow <- color3");
        assert_eq!(p.code, p.yellow, "the documented code/warn collapse");
        assert_eq!(p.blue, Color::Rgb(0x8b, 0xa4, 0xb0), "blue <- color4");
        assert_eq!(p.magenta, Color::Rgb(0xa2, 0x92, 0xa3), "magenta <- color5");
        assert_eq!(p.cyan, Color::Rgb(0x8e, 0xa4, 0xa2), "cyan <- color6");
    }

    #[test]
    fn an_incomplete_omarchy_palette_is_an_err() {
        let mut c = omarchy_fixture();
        c.remove("accent");
        assert!(omarchy_palette(&c).is_err(), "a missing key is an error");
        // A full palette with one unreadable color: errors on `parse_color`
        // (not on a missing key — every key is present).
        let mut bad = omarchy_fixture();
        bad.insert("accent".to_string(), "chartreuse".to_string());
        assert!(
            omarchy_palette(&bad).is_err(),
            "an unreadable color is an error"
        );
    }

    #[test]
    fn an_inline_source_seeds_the_palette_and_the_dim_role() {
        // The `omarchy` shape: a resolved palette plus the one role a Palette can
        // not carry (`dim`). Built directly — reading the live file is
        // machine-dependent and is exercised by the live check, not here.
        let spec = ThemeSpec {
            source: Source::Inline {
                palette: *preset("nord").unwrap(),
                roles: BTreeMap::from([("dim".to_string(), "#123456".to_string())]),
            },
            roles: BTreeMap::new(),
        };
        let theme = spec.into_theme(ColorMode::Rgb);
        assert_eq!(
            theme.accent.fg,
            Some(Color::Rgb(0x88, 0xc0, 0xd0)),
            "nord seeds accent"
        );
        assert_eq!(
            theme.dim.fg,
            Some(Color::Rgb(0x12, 0x34, 0x56)),
            "the inline dim wins over the grey ramp"
        );
    }

    #[test]
    fn a_user_role_overrides_an_inline_source() {
        let mut base = ThemeSpec {
            source: Source::Inline {
                palette: *preset("nord").unwrap(),
                roles: BTreeMap::new(),
            },
            roles: BTreeMap::new(),
        };
        base.roles.insert("accent".to_string(), "red".to_string());
        let theme = base.into_theme(ColorMode::Rgb);
        assert_eq!(theme.accent.fg, Some(Color::Red), "the user's role wins");
        assert_eq!(
            theme.error.fg,
            Some(Color::Rgb(0xbf, 0x61, 0x6a)),
            "the inline palette seeds the rest"
        );
    }

    #[test]
    fn an_inline_hex_degrades_under_a_non_rgb_mode() {
        // The inline `dim` is a hex string, so below truecolor it is skipped and
        // the palette's grey ramp stands (from_palette / apply_dim_ramp).
        let spec = ThemeSpec {
            source: Source::Inline {
                palette: *preset("nord").unwrap(),
                roles: BTreeMap::from([("dim".to_string(), "#123456".to_string())]),
            },
            roles: BTreeMap::new(),
        };
        let named = spec.into_theme(ColorMode::Named);
        assert_eq!(named.dim.fg, Some(Color::Gray), "dim degrades to the D29 grey");
    }
}
