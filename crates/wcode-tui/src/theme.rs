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
use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};

/// The TUI's color roles. Add a field only when a real call site needs one.
pub(crate) struct Theme {
    /// The user prompt, the live cursor, and running state.
    pub accent: Style,
    /// The workhorse: secondary chrome and quiet prose.
    pub dim: Style,
    /// A low-emphasis grey, distinct from [`Theme::border`]: e.g. the sidebar's
    /// `done` state.
    pub muted: Style,
    /// The overlay and sidebar borders and their titles.
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
    fn colored() -> Self {
        Theme {
            accent: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            dim: Style::new().add_modifier(Modifier::DIM),
            muted: Style::new().fg(Color::Gray),
            border: Style::new().fg(Color::DarkGray),
            user: Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            body: Style::default(),
            error: Style::new().fg(Color::Red),
            success: Style::new().fg(Color::Green),
            warn: Style::new().fg(Color::LightYellow),
            code: Style::new().fg(Color::Yellow),
            heading: Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
            link: Style::new().fg(Color::Blue).add_modifier(Modifier::UNDERLINED),
            tool_name: Style::new().fg(Color::Blue).add_modifier(Modifier::BOLD),
            thinking: Style::new()
                .fg(Color::Magenta)
                .add_modifier(Modifier::DIM | Modifier::ITALIC),
            diff_add: Style::new().fg(Color::Green),
            diff_del: Style::new().fg(Color::Red),
        }
    }

    /// The `NO_COLOR` palette: no foreground anywhere, only bold/italic/dim.
    fn plain() -> Self {
        let bold = Style::new().add_modifier(Modifier::BOLD);
        let dim = Style::new().add_modifier(Modifier::DIM);
        Theme {
            accent: bold,
            dim,
            muted: dim,
            border: dim,
            user: bold,
            body: Style::default(),
            error: bold,
            success: bold,
            warn: bold,
            code: bold,
            heading: bold,
            link: Style::new().add_modifier(Modifier::UNDERLINED),
            tool_name: bold,
            thinking: Style::new().add_modifier(Modifier::DIM | Modifier::ITALIC),
            diff_add: bold,
            diff_del: bold,
        }
    }
}

/// The active theme — an installed [`ThemeSpec`] overlay on palette B, or
/// `plain()` under `NO_COLOR`. Resolved once, before the first draw.
static THEME: OnceLock<Theme> = OnceLock::new();

/// The active theme, resolved once from [`THEME`].
pub(crate) fn theme() -> &'static Theme {
    THEME.get_or_init(|| if no_color() { Theme::plain() } else { Theme::colored() })
}

/// Install `spec` as the process theme (called once, before the first draw); a
/// later call, or one after the theme was first read, is a no-op.
pub(crate) fn install(spec: ThemeSpec) {
    let _ = THEME.set(resolve(spec, no_color()));
}

/// Honor `NO_COLOR` (<https://no-color.org>) — resolved once.
pub(crate) fn no_color() -> bool {
    static NO_COLOR: OnceLock<bool> = OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some())
}

/// A role→color overlay on the default palette (palette B). `Default` (empty) is
/// palette B; overrides are built only through [`parse_theme`], which validates
/// every role and color.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ThemeSpec {
    roles: BTreeMap<String, String>,
}

impl ThemeSpec {
    /// The default palette with this spec's overrides applied. For each named
    /// role only the `fg` changes — its modifiers stay (`link` underlined,
    /// `tool_name` and `accent` bold, `thinking` dim+italic).
    fn into_theme(self) -> Theme {
        let mut theme = Theme::colored();
        for (role, value) in &self.roles {
            // `parse_theme` validated both; a stray value cannot reach here.
            let Ok(color) = parse_color(value) else {
                continue;
            };
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

/// The 16 role names a `[theme]` table may override.
const ROLE_NAMES: [&str; 16] = [
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
    "link",
    "tool_name",
    "thinking",
    "diff_add",
    "diff_del",
];

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
        spec.into_theme()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every role, so a test can assert over all of them.
    fn fields(theme: &Theme) -> [&Style; 16] {
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
        let theme = parse_theme(&roles).expect("a valid spec").into_theme();
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
        let theme = parse_theme(&roles).unwrap().into_theme();
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
    fn no_color_wins_over_a_spec() {
        let roles = BTreeMap::from([("accent".to_string(), "red".to_string())]);
        let spec = parse_theme(&roles).expect("a valid spec");
        let plain = resolve(spec, true);
        for style in fields(&plain) {
            assert_eq!(style.fg, None, "NO_COLOR must not set an fg: {style:?}");
        }
    }
}
