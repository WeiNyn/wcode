//! The TUI's named color roles, in one place. A [`Theme`] is a set of styles a
//! call site *names* (`accent`, `dim`, `error`, …) instead of a raw color, so the
//! look lives here and the renderer stays role-based.
//!
//! `Theme::plain()` is the `NO_COLOR` fallback: no `fg` at all, only
//! bold/italic/dim — byte-identical to the per-function branches it replaced.
//!
//! Palette B is the default (named ANSI colors). A [`ThemeSpec`] — built by
//! [`parse_theme`] from a `[theme]` table — overlays any role, including a hex
//! `#rrggbb` truecolor value (opt-in). Theme *detection* (`COLORTERM` /
//! 256-color) remains open (`tui-plan.md` P2).

use std::collections::BTreeMap;
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
    /// A low-emphasis grey, distinct from [`Theme::border`]: e.g. the team strip's
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
    /// A tool's name in its `⚙` / `✓` header.
    pub tool_name: Style,
    /// A thinking block.
    pub thinking: Style,
    /// An added (`+`) diff line.
    pub diff_add: Style,
    /// A removed (`-`) diff line.
    pub diff_del: Style,
}

impl Theme {
    /// The default palette — a handful of named ANSI colors. Assistant prose
    /// stays default and dim stays the workhorse (`tui-design.md` §1.3).
    const fn colored() -> Self {
        Theme {
            accent: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            muted: Style::new().fg(Color::Gray),
            border: Style::new().fg(Color::DarkGray),
            user: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            body: Style::new(),
            error: Style::new().fg(Color::Red),
            success: Style::new().fg(Color::Green),
            warn: Style::new().fg(Color::LightYellow),
            code: Style::new().fg(Color::Yellow),
            heading: Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            heading_sub: Style::new().fg(Color::Magenta),
            link: Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
            tool_name: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            thinking: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::DIM.union(Modifier::ITALIC)),
            diff_add: Style::new().fg(Color::Green),
            diff_del: Style::new().fg(Color::Red),
        }
    }

    /// The `NO_COLOR` palette: no foreground anywhere, only bold/italic/dim.
    const fn plain() -> Self {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().add_modifier(Modifier::DIM);
        Theme {
            accent: bold,
            dim,
            muted: dim,
            border: dim,
            user: bold,
            body: Style::new(),
            error: bold,
            success: bold,
            warn: bold,
            code: bold,
            heading: bold,
            heading_sub: Style::new().add_modifier(Modifier::UNDERLINED),
            link: Style::new().add_modifier(Modifier::UNDERLINED),
            tool_name: bold,
            thinking: Style::new().add_modifier(Modifier::DIM.union(Modifier::ITALIC)),
            diff_add: bold,
            diff_del: bold,
        }
    }
}

/// The active theme. A `RwLock` (not `OnceLock`) so [`set`] can rewrite it. The
/// seed is const (`Theme::colored()`); the `NO_COLOR`/`plain` choice is applied
/// by the always-run [`install`] before the first draw.
static THEME: RwLock<Theme> = RwLock::new(Theme::colored());

/// Bumped by every [`set`]; the TUI cache-invalidation watches it (`app.rs`).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// The current theme generation — the cache-invalidation signal.
pub(crate) fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
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
    let palette = spec.preset.as_deref().and_then(preset);
    *THEME.write().unwrap_or_else(PoisonError::into_inner) = resolve(spec, no_color());
    let syntax = palette
        .and_then(syntect_theme)
        .unwrap_or_else(crate::markdown::default_syntect_theme);
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
#[derive(Clone, Copy)]
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
    muted: Color::Gray,
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
    CATALOG.iter().map(|(n, _)| *n).collect()
}

impl Theme {
    /// ALL 17 roles from a palette (NOT an overlay). Starts from the palette-B
    /// MODIFIER skeleton (`Theme::colored()`) so only `fg` varies per role; the
    /// `body`/`dim` fg stay `None` (prose = the terminal default; `dim` = DIM).
    ///
    /// Per-role Rgb gating, EXPLICIT — the per-`Color` mirror of
    /// [`ThemeSpec::into_theme`]'s per-string rule (Q5): an `Rgb` palette color
    /// under a non-`Rgb` mode keeps that role's palette-B default.
    fn from_palette(p: &Palette, mode: ColorMode) -> Theme {
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
        theme
    }
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
/// falls back to [`crate::markdown::default_syntect_theme`]. So the `default`
/// preset (named-16/Reset) uses `base16-ocean.dark`; the six truecolor presets
/// get a custom theme whose scopes match the UI hues (D11/D14).
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

/// A preset selector plus a role→color overlay. `Default` (empty) is palette B;
/// a `preset` name (D10) selects a catalog palette and the `roles` overlay sits
/// on top. Built only through [`parse_theme_table`]/[`parse_theme`], which
/// validate every name, role, and color.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeSpec {
    /// A catalog preset name (D10); `None` = palette B. Resolved in `into_theme`.
    preset: Option<String>,
    roles: BTreeMap<String, String>,
}

impl ThemeSpec {
    /// This spec with its preset replaced by `name`; the role overrides are KEPT.
    /// An unknown name is an `Err` — never a silent palette B (B2).
    pub fn with_preset(mut self, name: &str) -> Result<ThemeSpec, String> {
        if preset(name).is_none() {
            return Err(format!("unknown theme `{name}`; known: {}", names().join(", ")));
        }
        self.preset = Some(name.to_string());
        Ok(self)
    }
    /// The theme for this spec: the preset palette (if any) with the role
    /// overrides applied. A preset seeds ALL 17 roles ([`Theme::from_palette`]);
    /// then each named role changes only its `fg` — its modifiers stay (`link`
    /// underlined, `tool_name` and `accent` bold, `thinking` dim+italic). A hex
    /// (`#rrggbb` → [`Color::Rgb`]) override is honored only when the terminal
    /// is truecolor ([`ColorMode::Rgb`]): under Plain / Named / Indexed it
    /// degrades to that role's palette-B default rather than emit an `Rgb` a
    /// non-truecolor terminal renders wrong. Named colors are honored in every
    /// color mode. [`parse_theme_table`] validated the values; a stray one
    /// cannot reach here.
    fn into_theme(self, mode: ColorMode) -> Theme {
        let mut theme = match self.preset.as_deref().and_then(preset) {
            Some(p) => Theme::from_palette(p, mode),
            None => Theme::colored(),
        };
        for (role, value) in &self.roles {
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
        theme
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
    if let Some(n) = &name
        && preset(n).is_none()
    {
        return Err(format!("unknown theme `{n}`; known: {}", names().join(", ")));
    }
    let mut spec = parse_theme(&roles)?; // role/color validation, unchanged
    spec.preset = name;
    Ok(spec)
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
    fn the_colored_theme_signals_errors_in_red() {
        assert_eq!(Theme::colored().error.fg, Some(Color::Red));
    }

    #[test]
    fn the_semantic_palette_keeps_roles_distinct() {
        // Palette "B": roles that used to be aliases now read as themselves.
        let t = Theme::colored();
        assert_ne!(t.muted.fg, t.border.fg, "muted (Gray) vs border (DarkGray)");
        assert_ne!(t.code.fg, t.warn.fg, "code (Yellow) vs warn (LightYellow)");
        assert_ne!(t.tool_name, t.link, "tool_name (bold) vs link (underline)");
        assert!(
            t.tool_name.add_modifier.contains(Modifier::BOLD),
            "tool_name is bold"
        );
        assert_eq!(
            t.thinking.fg,
            Some(Color::Magenta),
            "thinking carries the meta hue"
        );
        assert_ne!(t.heading, t.body);
        assert!(t.heading.fg.is_some(), "headings carry a color now");
        // Intentional aliases — not asserted distinct:
        // accent == user (prompt/running), success == diff_add, error == diff_del.
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
    fn an_override_sets_only_its_role_and_keeps_modifiers() {
        let roles = BTreeMap::from([
            ("link".to_string(), "light-cyan".to_string()),
            ("tool_name".to_string(), "light-green".to_string()),
        ]);
        let theme = parse_theme(&roles).unwrap().into_theme(ColorMode::Rgb);
        assert_eq!(theme.link.fg, Some(Color::LightCyan));
        assert_eq!(theme.tool_name.fg, Some(Color::LightGreen));
        assert!(theme.link.add_modifier.contains(Modifier::UNDERLINED));
        assert!(theme.tool_name.add_modifier.contains(Modifier::BOLD));
        // Untouched roles keep palette B.
        let base = Theme::colored();
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
        let base = Theme::colored(); // accent == Cyan + Bold

        assert_eq!(
            spec.clone().into_theme(ColorMode::Rgb).accent.fg,
            Some(Color::Rgb(0xff, 0x00, 0xcc)),
            "Rgb mode honors the hex"
        );
        for degraded in [ColorMode::Plain, ColorMode::Named, ColorMode::Indexed] {
            let theme = spec.clone().into_theme(degraded);
            assert_eq!(
                theme.accent.fg, base.accent.fg,
                "hex degrades to the role's palette default under {degraded:?}"
            );
            assert!(
                theme.accent.add_modifier.contains(Modifier::BOLD),
                "modifiers survive the degradation"
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
        assert_eq!(theme.error.fg, Some(Color::Red));
        assert_eq!(theme.muted.fg, Some(Color::Gray));
        assert_eq!(theme.border.fg, Some(Color::DarkGray));
        assert_eq!(theme.warn.fg, Some(Color::LightYellow));
        assert_eq!(
            theme.code.fg,
            Some(Color::Yellow),
            "code stays distinct from warn (palette B)"
        );
        assert_eq!(theme.body.fg, None, "prose stays the terminal default");
        assert_eq!(theme.dim.fg, None, "dim stays a modifier-only role");
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
        assert_eq!(spec.preset.as_deref(), Some("nord"));

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
        assert_eq!(switched.preset.as_deref(), Some("nord"));
        let theme = switched.into_theme(ColorMode::Rgb);
        assert_eq!(theme.warn.fg, Some(Color::LightCyan), "the override survived");
        assert_eq!(
            theme.accent.fg,
            Some(Color::Rgb(0x88, 0xc0, 0xd0)),
            "the preset applied"
        );
    }
}
