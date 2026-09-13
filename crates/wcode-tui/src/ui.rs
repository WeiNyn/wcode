//! Immediate-mode rendering: compose the whole frame from [`App`] each draw.
//!
//! Bands (top → bottom): transcript · rule · input · status. See
//! `docs/tui-design.md` for the visual spec.

use std::sync::OnceLock;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Paragraph};
use wcode_harness::message::{AgentMessage, ContentBlock};

use crate::app::{App, Block, Overlay, Tool};
use crate::markdown;

/// First/continuation prefixes for a thinking block (`···` then an aligned
/// continuation column).
const THINK_FIRST: &str = "   ··· ";
const THINK_CONT: &str = "       ";

/// Draw the full frame. Stateless: everything comes from `app`.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // The input grows with its line count (Shift-Enter adds a line).
    let input_height = (app.input().matches('\n').count() + 1).clamp(1, 6) as u16;
    let [body, rule, input, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_transcript(frame, body, app);
    draw_rule(frame, rule);
    draw_input(frame, input, app);
    draw_status(frame, status, app);
    // The modal, if any, is drawn last — over the bands.
    draw_overlay(frame, area, app);
}

/// Draw the open modal, centered over everything else. `Clear` first so the
/// bands beneath do not bleed through (design §1: an overlay never reflows the
/// base layout).
fn draw_overlay(frame: &mut Frame, area: Rect, app: &App) {
    let Some(Overlay::Pick(picker)) = app.overlay() else {
        return;
    };
    let rows = picker.rows();

    // Borders (2) + the filter line (1) + at least one item row.
    let max_items = area.height.saturating_sub(3).max(1) as usize;
    let visible = rows.len().min(max_items).max(1);
    let start = if rows.len() <= visible {
        0
    } else {
        picker
            .selected
            .saturating_sub(visible - 1)
            .min(rows.len() - visible)
    };

    let height = (visible as u16 + 3).min(area.height);
    let width = area.width.saturating_sub(4).clamp(1, 64);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let block = WidgetBlock::default()
        .borders(Borders::ALL)
        .border_style(dim())
        .title(Span::styled(format!(" {} ", picker.title), dim()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut lines = vec![Line::from(Span::styled(
        format!("/{}", picker.query),
        dim(),
    ))];
    for (i, (item, range)) in rows.iter().enumerate().skip(start).take(visible) {
        let marker = if i == picker.selected {
            Span::styled("❯ ", accent())
        } else {
            Span::styled("  ", dim())
        };
        let mut spans = vec![marker];
        spans.extend(highlight(item, range.as_ref()));
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Split an item into spans, emphasizing the matched substring (if any), all in
/// the dim base style. A match range off a char boundary is ignored rather than
/// sliced mid-codepoint.
fn highlight(text: &str, range: Option<&std::ops::Range<usize>>) -> Vec<Span<'static>> {
    let style = dim();
    let Some(range) = range else {
        return vec![Span::styled(text.to_string(), style)];
    };
    if range.end > text.len()
        || !text.is_char_boundary(range.start)
        || !text.is_char_boundary(range.end)
    {
        return vec![Span::styled(text.to_string(), style)];
    }
    vec![
        Span::styled(text[..range.start].to_string(), style),
        Span::styled(text[range.start..range.end].to_string(), accent()),
        Span::styled(text[range.end..].to_string(), style),
    ]
}

fn draw_transcript(frame: &mut Frame, area: Rect, app: &mut App) {
    let width = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    for (i, block) in app.transcript().iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        lines.extend(block_lines(block, width));
    }
    // The in-flight message trails the committed transcript.
    if let Some(message) = app.live() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        if let AgentMessage::Assistant { content, .. } = message {
            lines.extend(content_lines(content, width, true));
        }
    }
    // Follow the tail unless the user has scrolled up; the renderer measures
    // the transcript and reconciles the scroll window.
    let height = area.height as usize;
    let total = lines.len();
    app.sync_scroll(total, height, area.width as usize);
    let end = total.saturating_sub(app.scroll());
    let start = end.saturating_sub(height);
    let window: Vec<Line> = lines.drain(start..end).collect();
    frame.render_widget(Paragraph::new(window), area);
}

fn block_lines(block: &Block, width: usize) -> Vec<Line<'static>> {
    match block {
        Block::User(text) => wrap(text, width, " ❯ ", "   ", accent()),
        Block::Assistant(content) => content_lines(content, width, false),
        Block::Tool(tool) => tool_lines(tool),
        Block::Notice(text) => wrap(text, width, "   ", "   ", dim()),
        Block::Error(text) => wrap(text, width, "   ", "   ", error_style()),
    }
}

/// Render an assistant message's blocks in order, dropping tool calls (their
/// own line carries them). `live` appends a cursor to the last line.
fn content_lines(content: &[ContentBlock], width: usize, live: bool) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for block in content {
        match block {
            ContentBlock::Text { text } => {
                lines.extend(markdown::render(text, width));
            }
            ContentBlock::Thinking { text } => {
                lines.extend(wrap(text, width, THINK_FIRST, THINK_CONT, dim()));
            }
            ContentBlock::ToolCall { .. } => {}
        }
    }
    if live {
        match lines.last_mut() {
            Some(last) => last.spans.push(Span::styled("▌", accent())),
            None => lines.push(Line::from(Span::styled("   ▌", accent()))),
        }
    }
    lines
}

fn tool_lines(tool: &Tool) -> Vec<Line<'static>> {
    if !tool.done {
        let mut lines = vec![Line::from(vec![
            Span::styled("   ⚙ ", dim()),
            Span::styled(tool.name.clone(), dim()),
        ])];
        if let Some(tail) = last_line(&tool.output) {
            lines.push(Line::from(vec![
                Span::styled("     ", dim()),
                Span::styled(tail, dim()),
            ]));
        }
        return lines;
    }

    if let Some(diff) = &tool.diff {
        return diff_lines(tool, diff);
    }

    let (mark, style) = if tool.is_error {
        ("✗", error_style())
    } else {
        ("✓", dim())
    };
    let mut spans = vec![
        Span::styled(format!("   {mark} "), style),
        Span::styled(tool.name.clone(), style),
    ];
    let note = first_line(&tool.output);
    if !note.is_empty() {
        spans.push(Span::styled(format!(" · {note}"), dim()));
    }
    vec![Line::from(spans)]
}

/// A done tool that changed a file: its edit shown as a diff under the `⚙`
/// line, with a `+a −r` summary. The diff is UI-only (`ToolOutput::diff`), so
/// it never reached the model.
fn diff_lines(tool: &Tool, diff: &str) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("   ⚙ ", dim()),
        Span::styled(tool.name.clone(), dim()),
    ])];
    let (mut added, mut removed) = (0usize, 0usize);
    for raw in diff.lines() {
        let style = match raw.chars().next() {
            Some('+') => {
                added += 1;
                added_style()
            }
            Some('-') => {
                removed += 1;
                removed_style()
            }
            _ => dim(),
        };
        lines.push(Line::from(vec![
            Span::styled("     ", dim()),
            Span::styled(raw.to_string(), style),
        ]));
    }
    let (mark, style) = if tool.is_error {
        ("✗", error_style())
    } else {
        ("✓", dim())
    };
    lines.push(Line::from(vec![
        Span::styled(format!("   {mark} "), style),
        Span::styled(tool.name.clone(), style),
        Span::styled(format!(" · +{added} −{removed}"), dim()),
    ]));
    lines
}

/// First non-blank line, truncated — the one-line tool summary.
fn first_line(text: &str) -> String {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| truncate(l, 80))
        .unwrap_or_default()
}

/// Last non-blank line, truncated — the live tail of a running tool.
fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| truncate(l, 80))
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Greedy word-wrap `text` to `width`, prefixing the first line with `first`
/// and continuations with `cont` (same length, so text stays aligned).
fn wrap(text: &str, width: usize, first: &str, cont: &str, style: Style) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut first_line = true;
    for raw in text.split('\n') {
        let prefix = if first_line { first } else { cont };
        let avail = width.saturating_sub(prefix.chars().count()).max(1);
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
    frame.render_widget(Paragraph::new(Line::from(Span::styled(rule, dim()))), area);
}

fn draw_input(frame: &mut Frame, area: Rect, app: &App) {
    let (before, after) = split_at_char(app.input(), app.cursor());
    let before: Vec<&str> = before.split('\n').collect();
    let after: Vec<&str> = after.split('\n').collect();

    let mut lines: Vec<Line> = Vec::new();
    let last = before.len() - 1;
    for (i, segment) in before.iter().enumerate() {
        let prefix = if i == 0 { " ❯ " } else { "   " };
        let mut spans = vec![
            Span::styled(prefix, if i == 0 { accent() } else { dim() }),
            Span::raw((*segment).to_string()),
        ];
        if i == last {
            spans.push(Span::styled("▌", accent()));
            spans.push(Span::raw(after[0].to_string()));
        }
        lines.push(Line::from(spans));
    }
    for segment in &after[1..] {
        lines.push(Line::from(Span::raw(format!("   {segment}"))));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let status = app.status();
    let state = if app.running() { "⠹ running" } else { "⏸ idle" };
    let width = area.width as usize;

    // Fields are ordered most-important-first and dropped when space is tight:
    // session first, then effort, then tokens.
    let (mut show_tokens, mut show_effort, mut show_session) = (true, true, true);
    loop {
        let mut spans: Vec<Span> = vec![
            Span::raw(" "),
            Span::styled(status.model.clone(), dim()),
        ];
        if show_effort && let Some(effort) = &status.effort {
            spans.push(sep());
            spans.push(Span::styled(effort.clone(), dim()));
        }
        if show_tokens && let Some(tokens) = token_spans(app) {
            spans.push(sep());
            spans.extend(tokens);
        }
        if show_session && let Some(session) = &status.session {
            spans.push(sep());
            spans.push(Span::styled(format!("session {}", short_id(session)), dim()));
        }
        spans.push(sep());
        spans.push(Span::styled(state, dim()));
        if app.scroll() > 0 {
            spans.push(sep());
            spans.push(Span::styled(format!("↑ {}", app.scroll()), dim()));
        }
        spans.push(Span::raw(" "));

        let len: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        if len <= width || !(show_tokens || show_effort || show_session) {
            frame.render_widget(Paragraph::new(Line::from(spans)), area);
            return;
        }
        if show_session {
            show_session = false;
        } else if show_effort {
            show_effort = false;
        } else {
            show_tokens = false;
        }
    }
}

/// The dim ` · ` separator between status fields.
fn sep() -> Span<'static> {
    Span::styled(" · ", dim())
}

/// Context usage: a colored gauge plus `used / limit` (or just `used`).
fn token_spans(app: &App) -> Option<Vec<Span<'static>>> {
    let used = app.context_used()?;
    Some(match app.status().context_limit {
        Some(limit) => {
            let ratio = if limit == 0 {
                0.0
            } else {
                used as f64 / limit as f64
            };
            let color = if ratio >= 0.85 {
                Color::Red
            } else if ratio >= 0.6 {
                Color::Yellow
            } else {
                Color::Green
            };
            let bar_style = if no_color() {
                dim()
            } else {
                Style::new().fg(color)
            };
            vec![
                Span::styled(bar(ratio, 8), bar_style),
                Span::styled(
                    format!(" {} / {}", format_tokens(used), format_tokens(limit)),
                    dim(),
                ),
            ]
        }
        None => vec![Span::styled(format_tokens(used), dim())],
    })
}

/// An 8-cell gauge: `████░░░░`.
fn bar(ratio: f64, cells: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0) * cells as f64).round() as usize).min(cells);
    let mut out = "█".repeat(filled);
    out.push_str(&"░".repeat(cells - filled));
    out
}

/// Compact token count: `840`, `14.2k`, `272k`, `1M`.
fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format_scaled(n, 1_000_000, "M")
    } else if n >= 1_000 {
        format_scaled(n, 1_000, "k")
    } else {
        n.to_string()
    }
}

fn format_scaled(n: u64, unit: u64, suffix: &str) -> String {
    if n.is_multiple_of(unit) {
        format!("{}{suffix}", n / unit)
    } else {
        format!("{:.1}{suffix}", n as f64 / unit as f64)
    }
}

/// First 8 chars of a session id — enough to recognize, not enough to crowd.
fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn split_at_char(text: &str, n: usize) -> (String, String) {
    let idx = text
        .char_indices()
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    (text[..idx].to_string(), text[idx..].to_string())
}

pub(crate) fn dim() -> Style {
    Style::new().add_modifier(Modifier::DIM)
}

pub(crate) fn accent() -> Style {
    if no_color() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)
    }
}

/// Added (`+`) and removed (`-`) diff lines. Colored when available; bold (vs
/// the dim context) under `NO_COLOR`.
fn added_style() -> Style {
    if no_color() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Green)
    }
}

fn removed_style() -> Style {
    if no_color() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Red)
    }
}

fn error_style() -> Style {
    if no_color() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Red)
    }
}

/// Inline code and code blocks.
pub(crate) fn code_style() -> Style {
    if no_color() {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::Yellow)
    }
}

/// Honor `NO_COLOR` (https://no-color.org) — resolved once.
pub(crate) fn no_color() -> bool {
    static NO_COLOR: OnceLock<bool> = OnceLock::new();
    *NO_COLOR.get_or_init(|| std::env::var_os("NO_COLOR").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppEvent, Key};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
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
    fn status_compacts_tokens_and_shows_the_session() {
        let mut app = App::new();
        app.set_status(crate::app::Status {
            model: "m".into(),
            effort: Some("high".into()),
            session: Some("abcdef0123456789".into()),
            context_limit: Some(1_000_000),
        });
        app.handle(AppEvent::Agent(wcode_harness::event::AgentEvent::TurnEnd {
            message: AgentMessage::Assistant {
                content: vec![ContentBlock::Text { text: "x".into() }],
                stop_reason: wcode_harness::message::StopReason::Stop,
                usage: Some(wcode_harness::message::Usage {
                    input_tokens: 14_200,
                    output_tokens: 1,
                    cache_read_tokens: None,
                    cache_write_tokens: None,
                }),
                model: None,
            },
        }));
        let text = buffer_text(&render(&mut app, 80, 3));
        assert!(text.contains("14.2k / 1M"), "tokens missing: {text}");
        assert!(text.contains("session abcdef01"), "session missing: {text}");
    }

    #[test]
    fn format_tokens_compacts() {
        assert_eq!(format_tokens(840), "840");
        assert_eq!(format_tokens(14_200), "14.2k");
        assert_eq!(format_tokens(272_000), "272k");
        assert_eq!(format_tokens(1_000_000), "1M");
        assert_eq!(format_tokens(1_050_000), "1.1M");
    }

    #[test]
    fn bar_fills_proportionally() {
        assert_eq!(bar(0.0, 8), "░░░░░░░░");
        assert_eq!(bar(1.0, 8), "████████");
        assert_eq!(bar(0.5, 8), "████░░░░");
        assert_eq!(bar(2.0, 8), "████████"); // clamped
    }

    #[test]
    fn input_shows_multiple_lines() {
        let mut app = App::new();
        for c in "one".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Newline));
        for c in "two".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        let text = buffer_text(&render(&mut app, 40, 6));
        assert!(text.contains("one"), "first line missing: {text}");
        assert!(text.contains("two"), "second line missing: {text}");
    }

    #[test]
    fn draws_input_and_status() {
        let mut app = App::new();
        for c in "hi".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        let text = buffer_text(&render(&mut app, 40, 6));
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
        let text = buffer_text(&render(&mut app, 40, 8));
        assert!(text.contains("ping"), "transcript block missing: {text}");
        assert!(text.contains("❯ ping"));
    }

    #[test]
    fn draws_a_streaming_assistant_message() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(wcode_harness::event::AgentEvent::MessageStart {
            message: AgentMessage::Assistant {
                content: vec![ContentBlock::Text {
                    text: "streamed".into(),
                }],
                stop_reason: wcode_harness::message::StopReason::Stop,
                usage: None,
                model: None,
            },
        }));
        let text = buffer_text(&render(&mut app, 40, 6));
        assert!(text.contains("streamed"), "live block missing: {text}");
        assert!(text.contains("▌"), "live cursor missing: {text}");
    }

    #[test]
    fn greedy_wrap_respects_width() {
        let lines = greedy_wrap("one two three four", 8);
        assert!(lines.iter().all(|l| l.chars().count() <= 8));
        assert_eq!(lines.join(" "), "one two three four");
    }

    #[test]
    fn an_open_picker_renders_over_the_bands() {
        let mut app = App::new();
        app.set_models(vec!["gpt-4o".into(), "gpt-4o-mini".into()]);
        for c in "/model".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 60, 12));
        assert!(text.contains("model"), "picker title missing: {text}");
        assert!(text.contains("gpt-4o"), "picker item missing: {text}");
    }

    #[test]
    fn a_tool_diff_renders_with_a_plus_minus_summary() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t1".into(),
                name: "edit".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            wcode_harness::event::AgentEvent::ToolExecutionEnd {
                call_id: "t1".into(),
                name: "edit".into(),
                output: "edited f".into(),
                is_error: false,
                diff: Some("@@ -1,2 +1,2 @@\n ctx\n-old\n+new".into()),
                path: Some("f.rs".into()),
            },
        ));
        let text = buffer_text(&render(&mut app, 60, 12));
        assert!(text.contains("-old"), "removal missing: {text}");
        assert!(text.contains("+new"), "addition missing: {text}");
        assert!(text.contains("+1 −1"), "summary missing: {text}");
    }
}
