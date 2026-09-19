//! The TUI's named color roles, in one place. A [`Theme`] is a set of styles a
//! call site *names* (`accent`, `dim`, `error`, …) instead of a raw color, so the
//! look lives here and the renderer stays role-based.
//!
//! `Theme::plain()` is the `NO_COLOR` fallback: no `fg` at all, only
//! bold/italic/dim — byte-identical to the per-function branches it replaced.
//!
//! Named ANSI colors only (no truecolor), so it works on any terminal. Theme
//! *detection* (`COLORTERM` / 256-color) and a *configurable* theme remain open
//! (`tui-plan.md` P2/P4) — this is a structural refactor onto roles, nothing more.

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

/// The active theme, chosen once from `NO_COLOR`.
pub(crate) fn theme() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        if no_color() {
            Theme::plain()
        } else {
            Theme::colored()
        }
    })
}

/// Honor `NO_COLOR` (<https://no-color.org>) — resolved once.
pub(crate) fn no_color() -> bool {
    static NO_COLOR: OnceLock<bool> = OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some())
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
}
