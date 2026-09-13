//! Immediate-mode rendering: compose the whole frame from [`App`] each draw.
//!
//! Bands (top → bottom): transcript · rule · input · status. See
//! `docs/tui-design.md` for the visual spec.

use std::sync::OnceLock;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Block};

/// Draw the full frame. Stateless: everything comes from `app`.
pub fn draw(frame: &mut Frame, app: &App) {
    let [body, rule, input, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_transcript(frame, body, app);
    draw_rule(frame, rule);
    draw_input(frame, input, app);
    draw_status(frame, status, app);
}

fn draw_transcript(frame: &mut Frame, area: Rect, app: &App) {
    let width = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (i, block) in app.transcript().iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.extend(block_lines(block, width));
    }
    // Follow the tail: keep the most recent lines when they overflow.
    let height = area.height as usize;
    if lines.len() > height {
        lines.drain(0..lines.len() - height);
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn block_lines(block: &Block, width: usize) -> Vec<Line<'static>> {
    match block {
        Block::User(text) => wrap(text, width, " ❯ ", "   ", accent()),
        Block::Notice(text) => wrap(text, width, "   ", "   ", dim()),
    }
}

/// Greedy word-wrap `text` to `width`, prefixing the first line with `first`
/// and continuations with `cont` (kept the same length so text stays aligned).
fn wrap(text: &str, width: usize, first: &str, cont: &str, style: Style) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut first_line = true;
    for raw in text.split('\n') {
        let prefix = if first_line { first } else { cont };
        let indent = prefix.chars().count();
        let avail = width.saturating_sub(indent).max(1);
        let segments = greedy_wrap(raw, avail);
        let mut iter = segments.into_iter();
        let head = iter.next().unwrap_or_default();
        lines.push(prefixed(prefix, &head, style));
        first_line = false;
        for segment in iter {
            lines.push(prefixed(cont, &segment, style));
        }
    }
    lines
}

fn prefixed(prefix: &str, text: &str, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(prefix.to_string(), style),
        Span::styled(text.to_string(), style),
    ])
}

/// Break a single line into ≤`width`-wide chunks on word boundaries.
fn greedy_wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        if cur.is_empty() {
            cur.push_str(word);
        } else if cur.chars().count() + 1 + word.chars().count() <= width {
            cur.push(' ');
            cur.push_str(word);
        } else {
            out.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

fn draw_rule(frame: &mut Frame, area: Rect) {
    let rule = "─".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(rule, dim()))),
        area,
    );
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let (before, after) = split_at_char(app.input(), app.cursor());
    let line = Line::from(vec![
        Span::styled(" ❯ ", accent()),
        Span::raw(before),
        Span::styled("▌", accent()),
        Span::raw(after),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let status = app.status();
    let mut parts = vec![status.model.clone()];
    if let Some(effort) = &status.effort {
        parts.push(effort.clone());
    }
    parts.push(if app.running() { "⠹ running" } else { "⏸ idle" }.to_string());
    let text = format!(" {} ", parts.join(" · "));
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, dim()))), area);
}

fn split_at_char(text: &str, n: usize) -> (String, String) {
    let idx = text
        .char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    (text[..idx].to_string(), text[idx..].to_string())
}

fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

fn accent() -> Style {
    if no_color() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    }
}

/// Honor `NO_COLOR` (https://no-color.org) — resolved once.
fn no_color() -> bool {
    static NO_COLOR: OnceLock<bool> = OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppEvent, Key};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(app: &App, width: u16, height: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn draws_input_and_status() {
        let mut app = App::new();
        for c in "hi".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        let text = buffer_text(&render(&app, 40, 6));
        assert!(text.contains("❯ hi"));
        assert!(text.contains("idle"), "status line missing: {text}");
    }

    #[test]
    fn draws_a_committed_block() {
        let mut app = App::new();
        for c in "ping".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&app, 40, 8));
        assert!(text.contains("ping"), "transcript block missing: {text}");
        assert!(text.contains("❯ ping"));
    }

    #[test]
    fn greedy_wrap_respects_width() {
        let lines = greedy_wrap("one two three four", 8);
        assert!(lines.iter().all(|l| l.chars().count() <= 8));
        assert_eq!(lines.join(" "), "one two three four");
    }
}
