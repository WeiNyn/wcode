//! Immediate-mode rendering: compose the whole frame from [`App`] each draw.
//!
//! Bands (top → bottom): transcript · rule · input · status. See
//! `docs/tui-design.md` for the visual spec.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, Borders, Clear, Paragraph};
use wcode_harness::message::{AgentMessage, ContentBlock};

use crate::TeamState;
use crate::app::{App, Block, InputView, KEYS, Mode, Overlay, Picker, Tool};
use crate::markdown;
use crate::theme;

/// First/continuation prefixes for a thinking block (`···` then an aligned
/// continuation column).
const THINK_FIRST: &str = "   ··· ";
const THINK_CONT: &str = "       ";

/// Indent for a tool's body lines, aligning under the `⚙` marker column.
const TOOL_INDENT: &str = "     ";
/// Output lines a collapsed tool shows before a `… +N more` hint.
const TOOL_PREVIEW_LINES: usize = 4;
/// Output lines a fully expanded tool shows before the hint returns.
const TOOL_EXPANDED_LINES: usize = 200;
/// Diff lines a collapsed tool shows before the hint.
const TOOL_DIFF_PREVIEW_LINES: usize = 8;

/// The team sidebar appears only when the terminal is at least this wide.
const SIDEBAR_MIN_WIDTH: u16 = 60;
/// The sidebar's fixed width (`Constraint::Length(26)`).
const SIDEBAR_WIDTH: u16 = 26;

/// Draw the full frame. Stateless: everything comes from `app`.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    // The input grows with its *wrapped* row count (Shift-Enter / Ctrl-J add
    // lines; long lines wrap), capped so it never crowds out the transcript.
    let width = input_content_width(area);
    let max_rows = 8.min(area.height as usize / 2).max(1);
    let view = app.input_view();
    let (_, _, _, total_rows) = input_rows(&view.display, view.cursor_col, width);
    let input_height = total_rows.clamp(1, max_rows) as u16;
    let [body, rule, input, status] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(input_height),
        Constraint::Length(1),
    ])
    .areas(area);

    // The team sidebar splits the `body` band when there is a team and the
    // terminal is wide enough; otherwise the layout is byte-identical to before.
    let sidebar = !app.hide_sidebar() && !app.member_rows().is_empty() && area.width >= SIDEBAR_MIN_WIDTH;
    if sidebar {
        let [left, right] =
            Layout::horizontal([Constraint::Min(20), Constraint::Length(SIDEBAR_WIDTH)])
                .areas(body);
        draw_transcript(frame, left, app);
        draw_sidebar(frame, right, app);
    } else {
        draw_transcript(frame, body, app);
    }
    draw_rule(frame, rule);
    draw_input(frame, input, &view);
    draw_status(frame, status, app);
    draw_completion(frame, area, rule, app, sidebar);
    // The modal, if any, is drawn last — over the bands.
    draw_overlay(frame, area, app);
}

/// Draw the open modal, if any — the picker or the `F1` keymap.
fn draw_overlay(frame: &mut Frame, area: Rect, app: &App) {
    match app.overlay() {
        Some(Overlay::Pick(picker)) => draw_picker(frame, area, picker),
        Some(Overlay::Help) => draw_help(frame, area),
        None => {}
    }
}

/// Draw the `F1` keymap, centered like the picker.
fn draw_help(frame: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = KEYS
        .iter()
        .map(|(chord, what)| {
            Line::from(vec![
                Span::styled(format!(" {chord:<22}"), accent()),
                Span::styled((*what).to_string(), dim()),
            ])
        })
        .collect();
    lines.push(Line::default());
    lines.push(Line::from(Span::styled(" esc / F1 to close", dim())));

    let width = area.width.saturating_sub(4).clamp(1, 64);
    let height = (lines.len() as u16 + 2).min(area.height);
    let rect = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let block = WidgetBlock::default()
        .borders(Borders::ALL)
        .border_style(border())
        .title(Span::styled(" keys ", border()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Draw a picker modal, centered over everything else. `Clear` first so the
/// bands beneath do not bleed through (design §1: an overlay never reflows the
/// base layout).
fn draw_picker(frame: &mut Frame, area: Rect, picker: &Picker) {
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
        .border_style(border())
        .title(Span::styled(format!(" {} ", picker.title), border()));
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

/// Cap on completion rows shown at once.
const MAX_COMPLETION_ROWS: usize = 8;

/// Draw the inline `/`-command completion popup, floating just above the input
/// band (its bottom edge rests on the `rule`). Non-modal and non-reflowing: it
/// is `Clear`ed over the transcript and styled like the picker (design D7).
fn draw_completion(frame: &mut Frame, area: Rect, above: Rect, app: &App, sidebar: bool) {
    let rows = app.completion_rows();
    if rows.is_empty() {
        return;
    }
    let selected = app.completion_selected();
    // Leave two rows for the borders; cap so the list never dominates the screen.
    let max_items = MAX_COMPLETION_ROWS.min(above.y.saturating_sub(2) as usize);
    let visible = rows.len().min(max_items);
    if visible == 0 {
        return;
    }
    let start = if rows.len() <= visible {
        0
    } else {
        selected.saturating_sub(visible - 1).min(rows.len() - visible)
    };
    let height = visible as u16 + 2;
    // Keep clear of the team sidebar: when it is shown the popup stops at its
    // left edge (the same split predicate `draw` uses).
    let avail = if sidebar {
        area.width.saturating_sub(SIDEBAR_WIDTH)
    } else {
        area.width
    };
    let width = avail.saturating_sub(2).clamp(1, 64);
    let rect = Rect {
        x: area.x + 1,
        y: above.y - height,
        width,
        height,
    };

    frame.render_widget(Clear, rect);
    let block = WidgetBlock::default()
        .borders(Borders::ALL)
        .border_style(border())
        .title(Span::styled(" commands ", border()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(start).take(visible) {
        let marker = if i == selected {
            Span::styled("❯ ", accent())
        } else {
            Span::styled("  ", dim())
        };
        let mut spans = vec![marker];
        spans.extend(highlight(&row.label, row.range.as_ref()));
        if let Some(alias) = row.alias {
            spans.push(Span::styled(format!(" (/{alias})"), dim()));
        }
        if let Some(args) = row.args {
            spans.push(Span::styled(format!(" {args}"), dim()));
        }
        spans.push(Span::styled(format!("  {}", row.summary), dim()));
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
    // Record each committed block's line range so the selection bar can be drawn
    // in a second pass. A range starts *after* the separator, so it never spans
    // the blank line above the block.
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for (i, block) in app.transcript().iter().enumerate() {
        if i > 0 {
            lines.push(Line::default());
        }
        let start = lines.len();
        let block_lines = block_lines(block, width);
        let len = block_lines.len();
        lines.extend(block_lines);
        ranges.push(start..start + len);
    }
    // The in-flight message trails the committed transcript (it is transient, so
    // it is never a selection target).
    if let Some(message) = app.live() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        if let AgentMessage::Assistant { content, .. } = message {
            lines.extend(content_lines(content, width, true));
        }
    }
    let height = area.height as usize;
    let total = lines.len();
    let grew = total != app.total_lines();
    app.set_block_ranges(ranges);
    // Follow the tail unless the user has scrolled up; the renderer measures
    // the transcript and reconciles the scroll window.
    app.sync_scroll(total, height, area.width as usize);
    // While browsing, re-anchor the view across transcript growth so a run that
    // appends blocks never yanks the view to the tail; a deliberate wheel scroll
    // is left alone (this fires only when the content changed).
    if app.mode() == Mode::Browse && app.selected().is_some() && grew {
        app.reveal_selected();
    }
    let end = total.saturating_sub(app.scroll());
    let start = end.saturating_sub(height);
    let mut window: Vec<Line> = lines.drain(start..end).collect();
    // Second pass: replace column 0 of the selected block's visible rows with the
    // bar. Replacing (not prepending) keeps every glyph in its column.
    if let Some(range) = app.selected_range() {
        for (r, line) in window.iter_mut().enumerate() {
            if range.contains(&(start + r)) {
                paint_bar(line);
            }
        }
    }
    frame.render_widget(Paragraph::new(window), area);
}

/// Overwrite column 0 of a transcript row with the selection bar, keeping the
/// rest of the row's text and its styles (split, so the bar does not recolor
/// the gutter marker that follows). Leading *empty* spans render nothing, so the
/// first non-empty span owns column 0 — skipping them keeps the glyph columns
/// unshifted (a future constructor could emit one).
fn paint_bar(line: &mut Line<'static>) {
    let Some(idx) = line.spans.iter().position(|s| !s.content.is_empty()) else {
        // A blank row has nothing at column 0 to replace: it just gains the bar.
        line.spans.push(Span::styled("▌", accent()));
        return;
    };
    let (rest, style) = {
        let first = &mut line.spans[idx];
        let mut chars = first.content.chars();
        chars.next();
        let rest: String = chars.collect();
        let style = first.style;
        first.content = "▌".into();
        first.style = accent();
        (rest, style)
    };
    if !rest.is_empty() {
        line.spans.insert(idx + 1, Span::styled(rest, style));
    }
}

fn block_lines(block: &Block, width: usize) -> Vec<Line<'static>> {
    match block {
        Block::User(text) => wrap(text, width, " ❯ ", "   ", user()),
        Block::Assistant(content) => content_lines(content, width, false),
        Block::Tool(tool) => tool_lines(tool, width),
        Block::Notice(text) => wrap(text, width, "   ", "   ", dim()),
        Block::Error(text) => wrap(text, width, "   ", "   ", error_style()),
        Block::Diff { path, diff } => diff_block_lines(path, diff),
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
                lines.extend(wrap(text, width, THINK_FIRST, THINK_CONT, thinking()));
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

/// The changed file (dim), appended to a tool's `⚙` header when it touched one.
fn path_span(tool: &Tool) -> Option<Span<'static>> {
    tool.path
        .as_ref()
        .map(|path| Span::styled(format!("  {path}"), dim()))
}

fn tool_lines(tool: &Tool, width: usize) -> Vec<Line<'static>> {
    // A live tool: the header, then (collapsed) its last non-blank line or
    // (expanded) the tail of the live output, because it grows downward.
    if !tool.done {
        let mut header = vec![
            Span::styled("   ⚙ ", dim()),
            Span::styled(tool.name.clone(), tool_name()),
        ];
        header.extend(path_span(tool));
        let mut lines = vec![Line::from(header)];
        if tool.expanded {
            let body: Vec<&str> = tool.output.lines().collect();
            lines.extend(tool_body(&body, width, TOOL_EXPANDED_LINES, dim(), true));
            if let Some(hint) = more_hint(body.len().saturating_sub(TOOL_EXPANDED_LINES), false) {
                lines.push(hint);
            }
        } else if let Some(tail) = last_line(&tool.output) {
            lines.push(Line::from(vec![
                Span::styled(TOOL_INDENT, dim()),
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
        ("✓", success())
    };
    let expanded = tool.expanded || tool.is_error;
    let (note, wide) = summary_line(&tool.output);
    let mut spans = vec![
        Span::styled(format!("   {mark} "), style),
        Span::styled(tool.name.clone(), tool_name()),
    ];
    spans.extend(path_span(tool));
    if !expanded && !note.is_empty() {
        spans.push(Span::styled(format!(" · {note}"), dim()));
    }
    let mut lines = vec![Line::from(spans)];

    // Collapsed: the summary above plus a preview of the rest. Expanded: the
    // summary is dropped (a line is never shown twice) and the whole output is
    // drawn, wrapped char-exact — so no line, however long, is left unreachable.
    if expanded {
        let all: Vec<&str> = tool.output.lines().collect();
        lines.extend(tool_body(&all, width, all.len(), dim(), false));
    } else {
        let body = body_after_summary(&tool.output);
        lines.extend(tool_body(&body, width, TOOL_PREVIEW_LINES, dim(), false));
        let more_lines = body.len().saturating_sub(TOOL_PREVIEW_LINES);
        if let Some(hint) = more_hint(more_lines, wide) {
            lines.push(hint);
        }
    }
    lines
}

/// A re-shown change (`/changes`): the file, then its styled diff body.
fn diff_block_lines(path: &str, diff: &str) -> Vec<Line<'static>> {
    let (added, removed) = crate::app::diff_counts(diff);
    let mut lines = vec![Line::from(vec![
        Span::styled("   ", dim()),
        Span::styled(path.to_string(), dim()),
        Span::styled(format!(" · +{added} −{removed}"), dim()),
    ])];
    for raw in diff.lines() {
        let style = match raw.chars().next() {
            Some('+') => added_style(),
            Some('-') => removed_style(),
            _ => dim(),
        };
        lines.push(Line::from(vec![
            Span::styled("     ", dim()),
            Span::styled(raw.to_string(), style),
        ]));
    }
    lines
}

/// A done tool that changed a file: its edit shown as a diff under the `⚙`
/// line, with a `+a −r` summary. The diff is UI-only (`ToolOutput::diff`), so
/// it never reached the model.
fn diff_lines(tool: &Tool, diff: &str) -> Vec<Line<'static>> {
    let mut header = vec![
        Span::styled("   ⚙ ", dim()),
        Span::styled(tool.name.clone(), tool_name()),
    ];
    header.extend(path_span(tool));
    let mut lines = vec![Line::from(header)];

    // Collapsed shows a preview; expanded (or a failure) shows the whole diff.
    let all: Vec<&str> = diff.lines().collect();
    let limit = if tool.expanded || tool.is_error {
        TOOL_EXPANDED_LINES
    } else {
        TOOL_DIFF_PREVIEW_LINES
    };
    let take = all.len().min(limit);
    for raw in &all[..take] {
        let style = match raw.chars().next() {
            Some('+') => added_style(),
            Some('-') => removed_style(),
            _ => dim(),
        };
        lines.push(Line::from(vec![
            Span::styled(TOOL_INDENT, dim()),
            Span::styled((*raw).to_string(), style),
        ]));
    }
    if let Some(hint) = more_hint(all.len().saturating_sub(take), false) {
        lines.push(hint);
    }

    let (added, removed) = crate::app::diff_counts(diff);
    let (mark, style) = if tool.is_error {
        ("✗", error_style())
    } else {
        ("✓", success())
    };
    lines.push(Line::from(vec![
        Span::styled(format!("   {mark} "), style),
        Span::styled(tool.name.clone(), tool_name()),
        Span::styled(format!(" · +{added} −{removed}"), dim()),
    ]));
    lines
}

/// The tool body: `lines` sliced to `limit` (the head, or the tail when `tail`)
/// and wrapped char-exact under the gutter. The caller adds any hint.
fn tool_body(
    lines: &[&str],
    width: usize,
    limit: usize,
    style: Style,
    tail: bool,
) -> Vec<Line<'static>> {
    let total = lines.len();
    if total == 0 {
        return Vec::new();
    }
    let (start, end) = if total > limit {
        if tail {
            (total - limit, total)
        } else {
            (0, limit)
        }
    } else {
        (0, total)
    };
    body_rows(&lines[start..end], width, style)
}

/// Wrap a tool body to `width`, one physical row per wrapped segment, char-exact
/// (`wrap_input`: spaces kept, an over-long token hard-broken) so a long line is
/// reachable rather than clipped at the pane edge.
fn body_rows(lines: &[&str], width: usize, style: Style) -> Vec<Line<'static>> {
    if lines.is_empty() {
        return Vec::new();
    }
    let avail = width.saturating_sub(TOOL_INDENT.chars().count()).max(1);
    wrap_input(&lines.join("\n"), avail)
        .into_iter()
        .map(|row| prefixed(TOOL_INDENT, &row, style))
        .collect()
}

/// The dim hint under an elided tool body: `… +N more line(s) · Ctrl+O` when the
/// preview cuts whole lines (singular for one), or `… Ctrl+O to show the full
/// line` when only the summary is truncated (a single long line, empty body).
/// `None` when expansion would reveal nothing more.
fn more_hint(more_lines: usize, wide: bool) -> Option<Line<'static>> {
    let text = if more_lines > 0 {
        let line = if more_lines == 1 { "line" } else { "lines" };
        format!("{TOOL_INDENT}… +{more_lines} more {line} · Ctrl+O")
    } else if wide {
        format!("{TOOL_INDENT}… Ctrl+O to show the full line")
    } else {
        return None;
    };
    Some(Line::from(Span::styled(text, dim())))
}

/// The output lines after the summary line shown in the header. The summary is
/// the first non-blank line, so the body reads on from there — no duplication.
fn body_after_summary(text: &str) -> Vec<&str> {
    let lines: Vec<&str> = text.lines().collect();
    match lines.iter().position(|line| !line.trim().is_empty()) {
        Some(first) => lines[first + 1..].to_vec(),
        None => Vec::new(),
    }
}

/// A tool's header summary — its first non-blank line, truncated to 80 — plus
/// whether it was cut (so the caller knows expansion reveals more of it).
fn summary_line(text: &str) -> (String, bool) {
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    (truncate(line, 80), line.chars().count() > 80)
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

/// Width available to input text: the band minus the 3-column gutter.
fn input_content_width(area: Rect) -> usize {
    (area.width as usize).saturating_sub(3).max(1)
}

/// The input's visual rows, char-exact: wrap `text` to `width`, preserving every
/// space and hard-breaking an over-long token. At least one row per logical line.
fn wrap_input(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let chars: Vec<char> = line.chars().collect();
        rows.extend(wrap_line(&chars, width));
    }
    rows
}

/// Wrap one logical line into rows of at most `width` chars: break after the last
/// space that fits, else hard-break (no whitespace is collapsed).
fn wrap_line(chars: &[char], width: usize) -> Vec<String> {
    let width = width.max(1);
    if chars.is_empty() {
        return vec![String::new()];
    }
    let mut rows = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        if chars.len() - start <= width {
            rows.push(chars[start..].iter().collect());
            break;
        }
        let window = &chars[start..start + width];
        let end = match window.iter().rposition(|&c| c == ' ') {
            Some(pos) => start + pos + 1, // keep the space at the end of the row
            None => start + width,        // no space to break on: hard-break
        };
        rows.push(chars[start..end].iter().collect());
        start = end;
    }
    rows
}

/// The wrapped input and where the cursor sits in it:
/// `(rows, cursor_row, cursor_col, total_rows)`. `cursor` is a char index.
fn input_rows(input: &str, cursor: usize, width: usize) -> (Vec<String>, usize, usize, usize) {
    let width = width.max(1);
    let mut rows = wrap_input(input, width);

    // The cursor sits at the end of the wrapped text *before* it; a full last row
    // pushes it onto the next (wrapping) row.
    let (before, _) = split_at_char(input, cursor);
    let before_rows = wrap_input(&before, width);
    let mut cursor_row = before_rows.len().saturating_sub(1);
    let mut cursor_col = before_rows.last().map_or(0, |r| r.chars().count());
    if cursor_col >= width {
        cursor_row += 1;
        cursor_col = 0;
    }
    while rows.len() <= cursor_row {
        rows.push(String::new());
    }
    let total_rows = rows.len();
    (rows, cursor_row, cursor_col, total_rows)
}

fn draw_rule(frame: &mut Frame, area: Rect) {
    let rule = "─".repeat(area.width as usize);
    frame.render_widget(Paragraph::new(Line::from(Span::styled(rule, dim()))), area);
}

fn draw_input(frame: &mut Frame, area: Rect, view: &InputView) {
    let width = input_content_width(area);
    let (rows, cursor_row, cursor_col, _) = input_rows(&view.display, view.cursor_col, width);
    let height = area.height as usize;
    // Scroll so the cursor row stays visible; rows above `offset` are hidden.
    let offset = cursor_row.saturating_sub(height.saturating_sub(1));
    let offsets = row_offsets(&view.display, &rows);

    let mut lines: Vec<Line> = Vec::new();
    for (i, row) in rows.iter().enumerate().skip(offset).take(height) {
        // Only the very first row carries the prompt gutter, as before.
        let (prefix, prefix_style) = if i == 0 {
            (" ❯ ", accent())
        } else {
            ("   ", dim())
        };
        let cursor = (i == cursor_row).then_some(cursor_col);
        lines.push(styled_input_row(
            prefix,
            prefix_style,
            row,
            offsets[i],
            &view.chips,
            cursor,
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

/// Each row's starting char offset in `display`, so a chip range can be mapped
/// onto wrapped rows (a continuation row adds nothing; a new logical line eats
/// the `\n` separating them).
fn row_offsets(display: &str, rows: &[String]) -> Vec<usize> {
    let chars: Vec<char> = display.chars().collect();
    let mut offsets = Vec::with_capacity(rows.len());
    let mut pos = 0;
    for row in rows {
        offsets.push(pos);
        pos += row.chars().count();
        if pos < chars.len() && chars[pos] == '\n' {
            pos += 1;
        }
    }
    offsets
}

/// One input row as spans: chars inside a chip range are [`accent`]ed, the rest
/// raw, and the cursor `▌` is inserted at `cursor` (a char index within the row).
fn styled_input_row(
    prefix: &str,
    prefix_style: Style,
    row: &str,
    row_offset: usize,
    chips: &[Range<usize>],
    cursor: Option<usize>,
) -> Line<'static> {
    let mut spans = vec![Span::styled(prefix.to_string(), prefix_style)];
    let mut buf = String::new();
    let mut chip = false;
    for (j, c) in row.chars().enumerate() {
        if cursor == Some(j) {
            flush(&mut buf, chip, &mut spans);
            spans.push(Span::styled("▌", accent()));
        }
        let is_chip = chips.iter().any(|r| r.contains(&(row_offset + j)));
        if is_chip != chip {
            flush(&mut buf, chip, &mut spans);
            chip = is_chip;
        }
        buf.push(c);
    }
    if cursor == Some(row.chars().count()) {
        flush(&mut buf, chip, &mut spans);
        spans.push(Span::styled("▌", accent()));
    }
    flush(&mut buf, chip, &mut spans);
    Line::from(spans)
}

/// Push the pending run in `buf` as a span (chip-styled or raw) and clear it.
fn flush(buf: &mut String, chip: bool, spans: &mut Vec<Span<'static>>) {
    if buf.is_empty() {
        return;
    }
    let text = std::mem::take(buf);
    spans.push(if chip {
        Span::styled(text, accent())
    } else {
        Span::raw(text)
    });
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
        if app.mode() == Mode::Browse {
            spans.push(sep());
            spans.push(Span::styled("▤ browse", accent()));
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
            let bar_style = if ratio >= 0.85 {
                error_style()
            } else if ratio >= 0.6 {
                theme::theme().warn
            } else {
                success()
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

/// The workhorse: secondary chrome and quiet prose.
pub(crate) fn dim() -> Style {
    theme::theme().dim
}

/// A quieter grey than [`dim`] where color is available.
fn muted() -> Style {
    theme::theme().muted
}

/// The overlay / sidebar border and its title.
fn border() -> Style {
    theme::theme().border
}

/// A thinking block.
fn thinking() -> Style {
    theme::theme().thinking
}

/// A tool's name in its `⚙` / `✓` header.
fn tool_name() -> Style {
    theme::theme().tool_name
}

/// The user's own prompt block.
fn user() -> Style {
    theme::theme().user
}

/// A completed tool's `✓` mark.
fn success() -> Style {
    theme::theme().success
}

/// The style for a member's row, keyed by state (idle = dim, running = accent,
/// done = muted).
fn state_style(state: TeamState) -> Style {
    match state {
        TeamState::Idle => dim(),
        TeamState::Running => accent(),
        TeamState::Done => muted(),
    }
}

/// Clip a row to `width` display columns (char count; the sidebar is ASCII-ish,
/// and ratatui clips precisely anyway).
fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        text.chars().take(width).collect()
    }
}

/// Draw the team status sidebar: a bordered pane, one `name · model · state` row
/// per member, the row styled by state.
fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    let block = WidgetBlock::default()
        .borders(Borders::ALL)
        .border_style(border())
        .title(Span::styled(" team ", border()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let width = inner.width as usize;
    let lines: Vec<Line> = app
        .member_rows()
        .into_iter()
        .map(|(label, model, state, focused)| {
            let text = format!("{label} · {model} · {}", state.label());
            // The focused member is bolded — a style-only highlight, so rows stay
            // within the fixed sidebar width.
            let mut style = state_style(state);
            if focused {
                style = style.add_modifier(Modifier::BOLD);
            }
            Line::from(Span::styled(clip(&text, width), style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

pub(crate) fn accent() -> Style {
    theme::theme().accent
}

/// An added (`+`) diff line.
fn added_style() -> Style {
    theme::theme().diff_add
}

/// A removed (`-`) diff line.
fn removed_style() -> Style {
    theme::theme().diff_del
}

fn error_style() -> Style {
    theme::theme().error
}

/// Inline code and code blocks.
pub(crate) fn code_style() -> Style {
    theme::theme().code
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppEvent, Key};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use wcode_harness::protocol::SessionId;

    /// The default root surface's id (`App::new`'s single surface).
    fn root() -> SessionId {
        SessionId::agent("root")
    }

    /// Give `app` a root + one member surface (`label`/`model`).
    fn with_member(app: &mut App, label: &str, model: &str) {
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent(label),
                label: label.into(),
                model: model.into(),
                is_root: false,
            },
        ]);
    }

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
        app.handle(AppEvent::Agent(root(), wcode_harness::event::AgentEvent::TurnEnd {
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
        app.handle(AppEvent::Agent(root(), wcode_harness::event::AgentEvent::MessageStart {
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
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t1".into(),
                name: "edit".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
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

    #[test]
    fn the_run_summary_and_changes_picker_render() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t1".into(),
                name: "edit".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionEnd {
                call_id: "t1".into(),
                name: "edit".into(),
                output: "ok".into(),
                is_error: false,
                diff: Some("@@ -1 +1 @@\n-old\n+new".into()),
                path: Some("src/a.rs".into()),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::AgentEnd,
        ));

        let text = buffer_text(&render(&mut app, 60, 20));
        assert!(text.contains("1 file changed"), "summary missing: {text}");
        // The `⚙` line names the changed file.
        assert!(text.contains("src/a.rs"), "tool path missing: {text}");

        for c in "/changes".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 60, 20));
        assert!(text.contains("changes"), "picker title missing: {text}");
        assert!(text.contains("+1 −1"), "picker stats missing: {text}");

        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 60, 20));
        assert!(text.contains("-old"), "re-shown removal missing: {text}");
        assert!(text.contains("+new"), "re-shown addition missing: {text}");
    }

    #[test]
    fn the_resume_picker_lists_sessions() {
        let mut app = App::new();
        app.set_sessions(vec![crate::app::SessionItem {
            label: "a1b2c3 · 5m · add the picker".into(),
            path: std::path::PathBuf::from("/s/a1b2c3.jsonl"),
        }]);
        for c in "/resume".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 60, 12));
        assert!(text.contains("resume"), "picker title missing: {text}");
        assert!(text.contains("add the picker"), "label missing: {text}");
    }

    #[test]
    fn wrap_input_keeps_short_text_and_preserves_spaces() {
        assert_eq!(wrap_input("one  two", 20), vec!["one  two"]);
        assert_eq!(wrap_input("a  b", 10), vec!["a  b"]);
        assert_eq!(wrap_input("a\nb", 10), vec!["a", "b"]);
        assert_eq!(wrap_input("", 10), vec![""]);
    }

    #[test]
    fn wrap_input_breaks_on_the_last_space_keeping_it() {
        assert_eq!(wrap_input("a  b", 3), vec!["a  ", "b"]);
        assert_eq!(wrap_input("hello world", 8), vec!["hello ", "world"]);
    }

    #[test]
    fn wrap_input_hard_breaks_a_token_longer_than_width() {
        let token = "x".repeat(40);
        let rows = wrap_input(&token, 10);
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|r| r.chars().count() <= 10));
        assert_eq!(rows.concat(), token);
    }

    #[test]
    fn input_rows_place_the_cursor_in_wrapped_coordinates() {
        // A 16-char line at width 10 wraps; the cursor at the end is on row 1.
        let (rows, row, col, total) = input_rows("abcdefghijklmnop", 16, 10);
        assert_eq!(rows, vec!["abcdefghij", "klmnop"]);
        assert_eq!((row, col, total), (1, 6, 2));

        // A break on a space: the cursor follows the wrapped text.
        let (rows, row, col, _) = input_rows("hello world", 9, 8);
        assert_eq!(rows, vec!["hello ", "world"]);
        assert_eq!((row, col), (1, 3));
    }

    #[test]
    fn a_long_input_line_wraps_and_is_not_truncated() {
        let mut app = App::new();
        let line = "abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJKLMNOPQRSTUVWX";
        for c in line.chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        let text = buffer_text(&render(&mut app, 40, 12));
        // The tail lies beyond the 37-column content width — only wrapping shows it.
        assert!(text.contains("UVWX"), "tail truncated:\n{text}");
        assert!(text.contains("▌"), "cursor missing:\n{text}");
    }

    #[test]
    fn a_tall_input_scrolls_to_keep_the_cursor_row_visible() {
        let mut app = App::new();
        for i in 0..8 {
            for c in format!("line{i}").chars() {
                app.handle(AppEvent::Key(Key::Char(c)));
            }
            if i < 7 {
                app.handle(AppEvent::Key(Key::Newline));
            }
        }
        // Height 12 ⇒ at most 6 input rows, so 8 logical lines must scroll.
        let text = buffer_text(&render(&mut app, 40, 12));
        assert!(text.contains("line7"), "cursor row scrolled off:\n{text}");
        assert!(text.contains("▌"), "cursor missing:\n{text}");
        assert!(!text.contains("line0"), "the top should scroll off:\n{text}");
    }

    #[test]
    fn a_pasted_chip_renders_as_a_placeholder_not_raw_text() {
        let mut app = App::new();
        let blob = "aaa\nbbb\nccc\nddd"; // 4 lines ⇒ a chip
        app.handle(AppEvent::Paste(blob.into()));
        let text = buffer_text(&render(&mut app, 60, 8));
        assert!(text.contains("pasted"), "chip missing:\n{text}");
        assert!(text.contains("4 lines"), "line count missing:\n{text}");
        assert!(text.contains("15 chars"), "char count missing:\n{text}");
        assert!(!text.contains("bbb"), "raw paste leaked:\n{text}");
    }

    #[test]
    fn a_chip_at_a_narrow_width_still_renders_within_the_band() {
        let mut app = App::new();
        app.handle(AppEvent::Paste("x".repeat(500)));
        // Content width is 17 here, so the chip wraps across rows.
        let text = buffer_text(&render(&mut app, 20, 8));
        assert!(text.contains("pasted"), "chip missing:\n{text}");
        assert!(text.contains("❱"), "chip tail missing:\n{text}");
        assert!(text.contains("▌"), "cursor missing:\n{text}");
    }

    #[test]
    fn the_completion_popup_lists_matching_commands() {
        let mut app = App::new();
        for c in "/mo".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        let text = buffer_text(&render(&mut app, 60, 12));
        assert!(text.contains("commands"), "popup title missing:\n{text}");
        assert!(text.contains("/model"), "completion row missing:\n{text}");
    }

    #[test]
    fn the_completion_popup_shows_why_an_alias_matched() {
        let mut app = App::new();
        for c in "/s".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        let text = buffer_text(&render(&mut app, 60, 12));
        assert!(text.contains("/resume"), "row missing:\n{text}");
        assert!(text.contains("/sessions"), "alias hint missing:\n{text}");
    }

    #[test]
    fn draw_completion_is_safe_on_a_short_terminal() {
        // The popup floats above the rule; on a screen too short to seat it the
        // rect math must skip rather than underflow. `above.y.saturating_sub(2)`
        // caps the rows, and the drawn height keeps `above.y - height >= 0`.
        let mut app = App::new();
        for c in "/mo".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        assert!(!app.completion_rows().is_empty(), "the popup should be open");

        // A tiny area: the layout clamps the popup away entirely (no panic).
        render(&mut app, 20, 4);
        let text = buffer_text(&render(&mut app, 20, 5));
        assert!(!text.is_empty(), "a short screen still draws the bands");

        // Pin the rect math directly for degenerate `above` rects: below the
        // two-row border reserve, and exactly the popup height.
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
        terminal
            .draw(|frame| {
                let area = frame.area();
                for y in [0u16, 1, 2, 3, 5] {
                    let above = Rect {
                        x: area.x,
                        y,
                        width: area.width,
                        height: 1,
                    };
                    draw_completion(frame, area, above, &app, false);
                }
            })
            .unwrap();
    }

    #[test]
    fn an_empty_roster_draws_no_sidebar() {
        let mut app = App::new();
        let text = buffer_text(&render(&mut app, 80, 20));
        assert!(!text.contains("team"), "no sidebar without a team:\n{text}");
    }

    #[test]
    fn the_team_sidebar_lists_members_and_hides_when_narrow() {
        let mut app = App::new();
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("explorer"),
                label: "explorer".into(),
                model: "m1".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("reviewer"),
                label: "reviewer".into(),
                model: "m2".into(),
                is_root: false,
            },
        ]);
        app.handle(AppEvent::Agent(
            SessionId::agent("explorer"),
            wcode_harness::event::AgentEvent::AgentStart,
        ));

        // Wide enough: the sidebar shows the title, each member, and its state.
        let wide = buffer_text(&render(&mut app, 80, 20));
        assert!(wide.contains("team"), "sidebar title missing:\n{wide}");
        assert!(wide.contains("explorer"), "member missing:\n{wide}");
        assert!(wide.contains("m1"), "model missing:\n{wide}");
        assert!(wide.contains("running"), "state missing:\n{wide}");
        assert!(wide.contains("reviewer"), "member missing:\n{wide}");

        // Narrow (< 60 cols): the sidebar is hidden.
        let narrow = buffer_text(&render(&mut app, 50, 20));
        assert!(
            !narrow.contains("explorer"),
            "the sidebar should hide when narrow:\n{narrow}"
        );
    }

    #[test]
    fn the_sidebar_shows_only_at_the_width_threshold() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        // 59 cols: hidden. 60 (the threshold): shown.
        assert!(!buffer_text(&render(&mut app, 59, 12)).contains("explorer"));
        assert!(buffer_text(&render(&mut app, 60, 12)).contains("explorer"));
    }

    #[test]
    fn a_short_body_draws_the_sidebar_without_panicking() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        // Tiny heights leave a degenerate `body` (even 0–1 rows): no panic.
        for height in [3u16, 4, 5] {
            let _ = buffer_text(&render(&mut app, 80, height));
        }
    }

    #[test]
    fn the_completion_popup_does_not_overdraw_the_sidebar() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        for c in "/mo".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        assert!(!app.completion_rows().is_empty(), "the popup should be open");

        let width = 80u16;
        let terminal = render(&mut app, width, 16);
        let buf = terminal.backend().buffer();
        // The sidebar owns the rightmost `SIDEBAR_WIDTH` columns; the popup
        // (title "commands") must never paint there.
        let sidebar_left = width - SIDEBAR_WIDTH;
        let mut sidebar = String::new();
        for y in 0..buf.area.height {
            for x in sidebar_left..buf.area.width {
                sidebar.push_str(buf[(x, y)].symbol());
            }
            sidebar.push('\n');
        }
        assert!(
            !sidebar.contains("commands"),
            "the popup overdraws the sidebar:\n{sidebar}"
        );
        // Sanity: the popup is still drawn (to the left of the sidebar).
        assert!(buffer_text(&terminal).contains("commands"));
    }

    /// Push one tool invocation (start → end) into the focused surface.
    fn push_tool(app: &mut App, name: &str, output: &str, is_error: bool, diff: Option<&str>) {
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: name.into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionEnd {
                call_id: "t".into(),
                name: name.into(),
                output: output.into(),
                is_error,
                diff: diff.map(str::to_string),
                path: None,
            },
        ));
    }

    #[test]
    fn a_done_tool_collapses_to_a_preview_with_a_more_hint() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        let text = buffer_text(&render(&mut app, 70, 20));
        assert!(
            text.contains("… +15 more lines · Ctrl+O"),
            "collapsed hint missing:\n{text}"
        );
        assert!(text.contains("line-2"), "the head preview is missing:\n{text}");
        assert!(
            !text.contains("line-20"),
            "the tail must be elided when collapsed:\n{text}"
        );
    }

    #[test]
    fn ctrl_o_expands_the_last_tool_fully() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        app.handle(AppEvent::Key(Key::Ctrl('o')));
        let text = buffer_text(&render(&mut app, 70, 30));
        assert!(text.contains("line-20"), "the full output should show:\n{text}");
        assert!(
            !text.contains("more lines · Ctrl+O"),
            "the hint should be gone when expanded:\n{text}"
        );
    }

    #[test]
    fn an_errored_tool_renders_expanded() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("err-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, true, None);
        let text = buffer_text(&render(&mut app, 70, 30));
        assert!(
            text.contains("err-20"),
            "a failure must never be hidden:\n{text}"
        );
        assert!(
            !text.contains("more lines · Ctrl+O"),
            "an errored tool is expanded, so no hint:\n{text}"
        );
    }

    #[test]
    fn a_long_diff_is_capped_until_expanded() {
        let mut app = App::new();
        let diff = (1..=30)
            .map(|i| format!("+add-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "edit", "edited", false, Some(&diff));

        let collapsed = buffer_text(&render(&mut app, 70, 24));
        assert!(
            collapsed.contains("… +22 more lines · Ctrl+O"),
            "diff cap hint missing:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("+add-30"),
            "the diff tail must be elided:\n{collapsed}"
        );

        app.handle(AppEvent::Key(Key::Ctrl('o')));
        let expanded = buffer_text(&render(&mut app, 70, 40));
        assert!(expanded.contains("+add-30"), "the full diff should show:\n{expanded}");
        assert!(
            !expanded.contains("more lines · Ctrl+O"),
            "the hint should be gone when expanded:\n{expanded}"
        );
    }

    #[test]
    fn a_single_long_line_renders_its_tail_only_when_expanded() {
        let mut app = App::new();
        // A ~300-char line with no spaces: collapsed shows only the 80-char
        // summary, so the tail is unreachable until expanded (char-exact wrap).
        let line = format!("{}TAIL-REACHABLE", "x".repeat(285));
        push_tool(&mut app, "bash", &line, false, None);
        let collapsed = buffer_text(&render(&mut app, 70, 20));
        assert!(
            collapsed.contains("Ctrl+O"),
            "the truncation must be hinted:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("TAIL-REACHABLE"),
            "the tail must be hidden while collapsed:\n{collapsed}"
        );

        app.handle(AppEvent::Key(Key::Ctrl('o')));
        let expanded = buffer_text(&render(&mut app, 70, 20));
        assert!(
            expanded.contains("TAIL-REACHABLE"),
            "the whole line must be reachable when expanded:\n{expanded}"
        );
    }

    #[test]
    fn a_short_single_line_tool_output_is_shown_once() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "All 41 tests pass.", false, None);
        let text = buffer_text(&render(&mut app, 70, 10));
        assert_eq!(
            text.matches("All 41 tests pass.").count(),
            1,
            "the line is shown twice:\n{text}"
        );
    }

    #[test]
    fn a_multi_line_tool_output_previews_four_lines_then_expands_to_all() {
        let mut app = App::new();
        let output = (1..=10)
            .map(|i| format!("row {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);

        // Summary "row 1" + a four-line preview ("row 2".."row 5") + the hint.
        let collapsed = buffer_text(&render(&mut app, 70, 20));
        assert!(collapsed.contains("row 5"), "preview short:\n{collapsed}");
        assert!(
            !collapsed.contains("row 6"),
            "the fifth body line must be elided:\n{collapsed}"
        );
        assert!(collapsed.contains("… +5 more lines · Ctrl+O"), "{collapsed}");

        app.handle(AppEvent::Key(Key::Ctrl('o')));
        let expanded = buffer_text(&render(&mut app, 70, 20));
        assert!(expanded.contains("row 10"), "not all lines shown:\n{expanded}");
        assert!(
            !expanded.contains("more lines · Ctrl+O"),
            "the hint should be gone:\n{expanded}"
        );
    }

    #[test]
    fn the_more_hint_pluralizes_one_hidden_line() {
        // 6 output lines = summary + 5 body lines → 4 previewed, 1 hidden.
        let mut app = App::new();
        let six = (1..=6)
            .map(|i| format!("row {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &six, false, None);
        let text = buffer_text(&render(&mut app, 70, 20));
        assert!(text.contains("… +1 more line · Ctrl+O"), "{text}");
        assert!(!text.contains("+1 more lines"), "not pluralized:\n{text}");

        // 7 output lines = summary + 6 body lines → 2 hidden (plural).
        let mut app = App::new();
        let seven = (1..=7)
            .map(|i| format!("row {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &seven, false, None);
        let text = buffer_text(&render(&mut app, 70, 20));
        assert!(text.contains("… +2 more lines · Ctrl+O"), "{text}");
    }

    #[test]
    fn changes_re_shows_the_whole_diff_uncapped() {
        let mut app = App::new();
        // 20 diff lines: well past the inline collapsed cap of 8.
        let diff = (1..=20)
            .map(|i| format!("+line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: "edit".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionEnd {
                call_id: "t".into(),
                name: "edit".into(),
                output: "ok".into(),
                is_error: false,
                diff: Some(diff),
                path: Some("src/a.rs".into()),
            },
        ));

        for c in "/changes".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        app.handle(AppEvent::Key(Key::Enter)); // dispatch → the picker
        app.handle(AppEvent::Key(Key::Enter)); // select the row → re-show the diff

        // The re-shown change is uncapped: a line past the collapsed limit shows.
        let text = buffer_text(&render(&mut app, 80, 40));
        assert!(
            text.contains("+line-20"),
            "the re-shown diff must not be capped:\n{text}"
        );
    }

    #[test]
    fn ctrl_o_toggles_only_the_last_tool() {
        let mut app = App::new();
        let first = (1..=20)
            .map(|i| format!("FIRST-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let last = (1..=20)
            .map(|i| format!("LAST-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &first, false, None);
        push_tool(&mut app, "bash", &last, false, None);

        app.handle(AppEvent::Key(Key::Ctrl('o')));
        let text = buffer_text(&render(&mut app, 70, 45));
        // The last tool is fully expanded…
        assert!(text.contains("LAST-20"), "the last tool should expand:\n{text}");
        // …while the first stays collapsed (its preview stops at line 5).
        assert!(text.contains("FIRST-5"), "{text}");
        assert!(
            !text.contains("FIRST-6"),
            "only the last tool should toggle:\n{text}"
        );
    }

    #[test]
    fn a_resumed_errored_tool_renders_expanded() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("err-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        // The seeded path (a replayed session) is the one that gets forgotten.
        app.seed_history(
            &root(),
            &[AgentMessage::ToolResult {
                tool_call_id: "t".into(),
                name: "bash".into(),
                output,
                is_error: true,
            }],
        );

        let text = buffer_text(&render(&mut app, 70, 30));
        assert!(
            text.contains("err-20"),
            "a replayed failure must be expanded:\n{text}"
        );
        assert!(
            !text.contains("more lines · Ctrl+O"),
            "an errored tool is expanded, so no hint:\n{text}"
        );
    }

    #[test]
    fn a_wrapped_running_tool_body_counts_toward_the_scroll_height() {
        let mut app = App::new();
        // One long line, still streaming (not done), expanded with Ctrl-O.
        let line = format!("START{}END", "y".repeat(300));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: "bash".into(),
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionUpdate {
                call_id: "t".into(),
                name: "bash".into(),
                partial: line,
            },
        ));
        app.handle(AppEvent::Key(Key::Ctrl('o')));

        // A short terminal: the wrapped body overflows the transcript band.
        let _ = buffer_text(&render(&mut app, 40, 6));
        for _ in 0..50 {
            app.handle(AppEvent::Key(Key::PageUp));
        }
        assert!(
            app.scroll() > 0,
            "the wrapped rows must count toward the scroll height, not just be drawn"
        );
        // Scrolled to the top, the start of the wrapped body is reachable — it
        // would not be if the body were counted as a single line.
        let text = buffer_text(&render(&mut app, 40, 6));
        assert!(
            text.contains("START"),
            "the top of the wrapped body must be reachable:\n{text}"
        );
    }

    #[test]
    fn f1_renders_the_help_overlay_with_the_keymap() {
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::F(1)));
        let text = buffer_text(&render(&mut app, 64, 30));
        assert!(text.contains("keys"), "help title missing:\n{text}");
        // Every chord in `KEYS` must be rendered — so this can never go stale.
        for (chord, _) in KEYS {
            assert!(text.contains(chord), "chord {chord:?} missing:\n{text}");
        }
        assert!(text.contains("toggle this help"), "F1's description missing:\n{text}");
    }

    #[test]
    fn ctrl_b_hides_the_team_sidebar() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        assert!(buffer_text(&render(&mut app, 80, 16)).contains("explorer"));
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        assert!(
            !buffer_text(&render(&mut app, 80, 16)).contains("explorer"),
            "the sidebar should be hidden after Ctrl-B"
        );
    }

    #[test]
    fn ctrl_t_expands_all_tool_output_at_once() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        push_tool(&mut app, "bash", &output, false, None);
        // Collapsed, the tail is elided (a hint is drawn).
        assert!(buffer_text(&render(&mut app, 70, 40)).contains("more lines"));

        app.handle(AppEvent::Key(Key::Ctrl('t')));
        let text = buffer_text(&render(&mut app, 70, 40));
        assert!(!text.contains("more lines"), "all tools show their whole output:\n{text}");
        assert!(text.contains("line-20"), "the last line should show:\n{text}");
    }

    // --- transcript browse mode ---------------------------------------------

    /// An assistant message with a single text block.
    fn reply(text: &str) -> AgentMessage {
        AgentMessage::Assistant {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            stop_reason: wcode_harness::message::StopReason::Stop,
            usage: None,
            model: None,
        }
    }

    /// The transcript rows that begin with the selection bar.
    fn barred(text: &str) -> Vec<&str> {
        text.lines().filter(|l| l.starts_with('▌')).collect()
    }

    #[test]
    fn input_mode_renders_no_browse_affordance() {
        let mut app = App::new();
        app.seed_history(
            &root(),
            &[AgentMessage::user_text("hello world"), reply("a reply")],
        );
        let first = buffer_text(&render(&mut app, 60, 16));
        let second = buffer_text(&render(&mut app, 60, 16));
        assert_eq!(first, second, "rendering is not stable");
        // With the mode off the frame carries no bar and no mode token.
        assert!(!first.contains("▤ browse"), "the mode token leaked:\n{first}");
        assert!(
            barred(&first).is_empty(),
            "the selection bar leaked in input mode:\n{first}"
        );
    }

    #[test]
    fn the_bar_lands_on_the_selected_block_only() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("alpha\nbeta\ngamma")]);
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // select the last block
        assert_eq!(app.selected(), Some(1));

        let text = buffer_text(&render(&mut app, 40, 20));
        let bars = barred(&text);
        assert_eq!(bars.len(), 3, "one bar per block row:\n{text}");
        for (row, word) in bars.iter().zip(["alpha", "beta", "gamma"]) {
            assert!(row.contains(word), "row {row:?} is not {word}:\n{text}");
        }
        // The divider row above (index 0) is not barred.
        assert!(
            !text.lines().next().unwrap().starts_with('▌'),
            "the divider row is barred:\n{text}"
        );
    }

    #[test]
    fn a_resize_re_measures_the_selected_range() {
        let mut app = App::new();
        app.seed_history(
            &root(),
            &[AgentMessage::user_text("aaaa bbbb cccc dddd eeee ffff gggg")],
        );
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        let wide = buffer_text(&render(&mut app, 60, 20));
        assert_eq!(barred(&wide).len(), 1, "one row at 60 cols:\n{wide}");
        // A narrower width re-wraps: the ranges are re-measured, not stale.
        let narrow = buffer_text(&render(&mut app, 20, 20));
        assert_eq!(barred(&narrow).len(), 3, "re-wrapped at 20 cols:\n{narrow}");
        assert_eq!(app.selected(), Some(1), "the same block stays selected");
    }

    #[test]
    fn streaming_below_the_cursor_does_not_yank_or_move_the_selection() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("KEEP")]);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(1));
        let _ = render(&mut app, 40, 10); // measure ranges + viewport first

        // A run appends blocks below the selection.
        for i in 0..20 {
            app.handle(AppEvent::Agent(
                root(),
                wcode_harness::event::AgentEvent::MessageEnd {
                    message: reply(&format!("tail line {i}")),
                },
            ));
        }

        let text = buffer_text(&render(&mut app, 40, 10));
        assert_eq!(app.selected(), Some(1), "the selection names the same block");
        assert!(
            text.contains("KEEP"),
            "the selected block must stay in view:\n{text}"
        );
        assert!(
            !text.contains("tail line 19"),
            "the view must not be yanked to the tail:\n{text}"
        );
        assert!(app.scroll() > 0, "the view is anchored, not at the tail");
    }

    /// A rendered line's width, in glyphs.
    fn line_width(line: &Line) -> usize {
        line.spans.iter().map(|s| s.content.chars().count()).sum()
    }

    #[test]
    fn browse_injects_no_rows_so_the_measured_total_is_identical() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello"), reply("world")]);

        let input = buffer_text(&render(&mut app, 60, 20));
        let input_total = app.total_lines();
        assert!(!input.contains("▤ browse"), "the mode token leaked:\n{input}");
        assert!(barred(&input).is_empty(), "a bar leaked in input mode:\n{input}");

        // The same transcript in browse: the total is unchanged (the bar injects
        // no rows, so max_scroll stays sane) and the bar is drawn.
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        let browse = buffer_text(&render(&mut app, 60, 20));
        let browse_total = app.total_lines();
        assert_eq!(
            browse_total, input_total,
            "the bar must inject no rows (input {input_total} vs browse {browse_total})"
        );
        assert!(browse.contains("▤ browse"), "the mode token is missing:\n{browse}");
        assert!(!barred(&browse).is_empty(), "the bar is missing:\n{browse}");
    }

    #[test]
    fn leaving_browse_restores_the_input_frame() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello"), reply("world")]);
        let plain = buffer_text(&render(&mut app, 60, 20));
        let plain_total = app.total_lines();

        app.handle(AppEvent::Key(Key::Ctrl('g'))); // enter
        let _ = render(&mut app, 60, 20);
        app.handle(AppEvent::Key(Key::Esc)); // leave

        let after = buffer_text(&render(&mut app, 60, 20));
        assert_eq!(after, plain, "a browse round-trip must restore the frame");
        assert_eq!(app.total_lines(), plain_total);
    }

    #[test]
    fn paint_bar_replaces_the_first_visible_glyph_without_shifting() {
        // A leading empty span renders nothing, so column 0 is the *next* span's
        // first glyph; the bar must replace it, not push it right.
        let mut line = Line::from(vec![Span::raw(""), Span::raw("hello")]);
        paint_bar(&mut line);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "▌ello", "the bar must replace, not prepend: {text:?}");
    }

    #[test]
    fn paint_bar_never_shifts_a_text_row() {
        // Every block kind the transcript can draw.
        let assistant = Block::Assistant(vec![
            ContentBlock::Text {
                text: "# Heading\n\n- one\n- two\n\n```rust\nlet x = 1;\n```\n\n\
                       text `code` and [link](http://x) and **bold**"
                    .to_string(),
            },
            ContentBlock::Thinking {
                text: "a reasoning paragraph that wraps onto more than one line for sure"
                    .to_string(),
            },
        ]);
        let tool = |expanded, is_error, diff: Option<&str>| {
            Block::Tool(Tool {
                name: "bash".into(),
                output: "a\nb\nc\nd\ne\nf".into(),
                done: true,
                is_error,
                expanded,
                diff: diff.map(str::to_string),
                path: Some("f.rs".into()),
            })
        };
        let blocks = [
            Block::User("hi there".into()),
            assistant,
            tool(false, false, None),
            tool(true, false, None),
            tool(false, false, Some("@@ -1 +1 @@\n-old\n+new")),
            tool(true, true, None),
            Block::Notice("a note".into()),
            Block::Error("boom".into()),
            Block::Diff {
                path: "f.rs".into(),
                diff: "@@ -1 +1 @@\n-old\n+new".into(),
            },
        ];

        for block in &blocks {
            for line in block_lines(block, 60) {
                let before = line_width(&line);
                let mut painted = line.clone();
                paint_bar(&mut painted);
                let after = line_width(&painted);
                if before == 0 {
                    // A blank markdown row has nothing to replace: it gains the
                    // bar (0 → 1). Benign — no glyph moves.
                    assert_eq!(after, 1, "a blank row gains exactly the bar");
                } else {
                    assert_eq!(
                        after, before,
                        "a text row's width must not change: {line:?}"
                    );
                }
            }
        }
    }
}



