//! Immediate-mode rendering: compose the whole frame from [`App`] each draw.
//!
//! Bands (top → bottom): transcript · rule · [team strip] · input · status.
//! The team strip is a one-row band that appears only when there is a team.
//! See `docs/tui-design.md` for the visual spec.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
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

/// The team strip appears only when the terminal is at least this wide.
const TEAM_STRIP_MIN_WIDTH: u16 = 50;
/// … and at least this tall, so body + rule + strip + input + status leave
/// the transcript something to show.
const TEAM_STRIP_MIN_HEIGHT: u16 = 6;
/// Slots shown before overflow folds the rest into a right-aligned `+N`.
const MAX_TEAM_SLOTS: usize = 3;

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

    // The team strip is a one-row band between `rule` and `input`, present only
    // when there is a team and the terminal is big enough. With no team the
    // layout is byte-identical to the original four-band stack.
    let team = !app.member_rows().is_empty()
        && area.width >= TEAM_STRIP_MIN_WIDTH
        && area.height >= TEAM_STRIP_MIN_HEIGHT;
    let mut constraints = vec![Constraint::Min(1), Constraint::Length(1)];
    if team {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Length(input_height));
    constraints.push(Constraint::Length(1));
    let areas = Layout::vertical(constraints).split(area);
    let body = areas[0];
    let rule = areas[1];
    let (input, status) = if team {
        (areas[3], areas[4])
    } else {
        (areas[2], areas[3])
    };

    draw_transcript(frame, body, app);
    draw_rule(frame, rule);
    if team {
        draw_team_strip(frame, areas[2], app);
    }
    draw_input(frame, input, &view);
    draw_status(frame, status, app);
    draw_completion(frame, area, rule, app);
    draw_search_prompt(frame, area, rule, app);
    // The modal, if any, is drawn last — over the bands.
    draw_overlay(frame, area, app);
}

/// Draw the browse transcript-search prompt, floating just above the input
/// band like the command completion. Browse-owned and non-reflowing: it is
/// `Clear`ed over the transcript only while the prompt is open, so an
/// input-mode frame stays byte-identical.
fn draw_search_prompt(frame: &mut Frame, area: Rect, above: Rect, app: &App) {
    let Some(query) = app.search_query() else {
        return;
    };
    if app.mode() != Mode::Browse {
        return; // defensive: the prompt never survives exit_browse
    }
    // Two content rows + the borders; floats above the rule, so the team strip
    // (below the rule) can never collide.
    let height = 4;
    if above.y < height {
        return; // no room above the input band
    }
    let width = area.width.saturating_sub(2).clamp(1, 64);
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
        .title(Span::styled(" search ", border()));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let hits = app.search_hits();
    let count = if query.is_empty() {
        "type to search".to_string()
    } else {
        format!(
            "{hits} hit{}",
            if hits == 1 { "" } else { "s" }
        )
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("/{query}"), accent()),
            Span::styled(format!(" — {count}"), dim()),
        ]),
        Line::from(Span::styled("enter jump · esc close · n / N repeat", dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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
fn draw_completion(frame: &mut Frame, area: Rect, above: Rect, app: &App) {
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
    // The popup floats above the rule and is drawn over the transcript only; the
    // team strip sits below the rule, so the two can never overlap.
    let width = area.width.saturating_sub(2).clamp(1, 64);
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
        // A tool-call-only assistant block renders no lines. It keeps its slot in
        // `ranges` (the selection index aligns with the transcript), but it must
        // not contribute a separator — that would be a stray blank line.
        let block_lines = block_lines(block, width);
        if i > 0 && !block_lines.is_empty() && !lines.is_empty() {
            lines.push(Line::default());
        }
        let start = lines.len();
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

/// The dim hint under an elided tool body: `… +N more line(s)` when the preview
/// cuts whole lines (singular for one), or `… the full line is elided` when only
/// the summary is truncated (a single long line, empty body). `None` when
/// expansion would reveal nothing more. Informational only — the per-block
/// toggle is browse mode's `Enter`.
fn more_hint(more_lines: usize, wide: bool) -> Option<Line<'static>> {
    let text = if more_lines > 0 {
        let line = if more_lines == 1 { "line" } else { "lines" };
        format!("{TOOL_INDENT}… +{more_lines} more {line}")
    } else if wide {
        format!("{TOOL_INDENT}… the full line is elided")
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

/// The overlay / popup border and its title.
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

/// The style for a member's row, keyed by state: idle = dim, running = green
/// [`success`] (a clear "active" signal, distinct from the dim status line and
/// from idle/done), done = muted.
fn state_style(state: TeamState) -> Style {
    match state {
        TeamState::Idle => dim(),
        TeamState::Running => success(),
        TeamState::Done => muted(),
    }
}

/// The strip's slot layout for `n` members across `area_width` columns:
/// `(shown, slot_width, overflow)` — up to [`MAX_TEAM_SLOTS`] equal-width
/// slots, the members beyond them folded into a right-aligned `+N`. The width
/// budget is the band minus the `+N` segment and the inter-slot gaps, shared
/// evenly. `slot_width` is 0 when the band cannot seat even one slot; the
/// renderer draws nothing then (never a divide-by-zero).
fn slot_geometry(area_width: usize, n: usize) -> (usize, usize, usize) {
    let shown = n.min(MAX_TEAM_SLOTS);
    if area_width == 0 || shown == 0 {
        return (shown, 0, 0);
    }
    let overflow = n.saturating_sub(shown);
    // The right-aligned "+N": a separator space, the plus, and the digits.
    let overflow_w = if overflow > 0 {
        2 + overflow.to_string().chars().count()
    } else {
        0
    };
    let gaps = shown.saturating_sub(1);
    let inner = area_width
        .saturating_sub(overflow_w)
        .saturating_sub(gaps);
    (shown, inner / shown, overflow)
}

/// One slot's spans: a state-colored `glyph label` head plus a dim tail — the
/// live action, or ` —` for idle/done members with nothing running — clipped
/// to `width` with a single trailing `…` when cut. The clip takes from the
/// whole slot text up front (reserving the ellipsis), so a long label can
/// never push past the slot into a neighbor's share.
fn slot_span(
    label: &str,
    state: TeamState,
    action: Option<&str>,
    width: usize,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let head = format!("{} {label}", state.glyph());
    let tail = match (state, action) {
        (_, Some(action)) => format!(" {action}"),
        (TeamState::Idle | TeamState::Done, None) => " —".to_string(),
        (TeamState::Running, None) => String::new(),
    };
    let head_len = head.chars().count();
    let total = head_len + tail.chars().count();
    // Reserve the ellipsis slot *before* taking, so a cut slot never exceeds
    // `width` columns.
    let take = if total <= width { total } else { width - 1 };
    let head_take = take.min(head_len);
    let tail_take = take.saturating_sub(head_take);
    let cut = total > width;
    let mut spans = Vec::new();
    let mut head_text: String = head.chars().take(head_take).collect();
    if cut && tail_take == 0 {
        // The tail did not survive the clip; the ellipsis rides the head.
        head_text.push('…');
    }
    if !head_text.is_empty() {
        spans.push(Span::styled(head_text, state_style(state)));
    }
    if tail_take > 0 {
        let mut tail_text: String = tail.chars().take(tail_take).collect();
        if cut {
            tail_text.push('…');
        }
        spans.push(Span::styled(tail_text, dim()));
    }
    spans
}

/// Draw the active-team strip: one row between the rule and the input, up to
/// [`MAX_TEAM_SLOTS`] equal-share member slots (`glyph label` + a dim action)
/// with the rest folded into a right-aligned `+N`. Each share is filled out
/// with dim background and separated from the next by a dim `│`, so the band
/// reads edge-to-edge like the rule/status lines — a single member still spans
/// the whole width. Consumes [`App::member_rows`] — already ordered active-first
/// then by action recency. The model is not shown (§2).
fn draw_team_strip(frame: &mut Frame, area: Rect, app: &App) {
    if area.height == 0 {
        return;
    }
    let rows = app.member_rows();
    let n = rows.len();
    let width = area.width as usize;
    let (shown, share, overflow) = slot_geometry(width, n);
    if shown == 0 || share == 0 {
        return;
    }
    // The right-aligned "+N": a leading separator space, the plus, and the
    // digits (mirrors `slot_geometry`'s budget) — but only when members
    // overflow, so a no-overflow band reserves nothing on the right.
    let seg = if overflow > 0 {
        format!("+{overflow}")
    } else {
        String::new()
    };
    // The band minus the `+N` segment and the inter-share separators, shared
    // evenly (`slot_geometry`'s `inner`). The columns left over after the
    // equal shares fold into the last share, so shares + separators + `+N`
    // always sum to exactly the width.
    let inner = width
        .saturating_sub(seg.chars().count() + usize::from(overflow > 0))
        .saturating_sub(shown - 1);
    let slack = inner.saturating_sub(share.saturating_mul(shown));

    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (label, state, _, action)) in rows.into_iter().take(shown).enumerate() {
        let last = i == shown - 1;
        // Each share is `share` columns of content (the last stretches over
        // the `slack`); the dim `│` sits as a column BETWEEN shares, so shares
        // + separators sum to exactly `inner` and the band ends flush.
        let content_w = if last { share + slack } else { share };
        let slot = slot_span(label, state, action, content_w);
        let used: usize = slot.iter().map(|s| s.content.chars().count()).sum();
        spans.extend(slot);
        // Dim spaces fill the share's remainder: one continuous subtle band.
        if let Some(fill) = content_w.checked_sub(used).filter(|&f| f > 0) {
            spans.push(Span::styled(" ".repeat(fill), dim()));
        }
        if !last {
            spans.push(Span::styled("│", dim()));
        }
    }
    if overflow > 0 {
        let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
        let pad = width.saturating_sub(used).saturating_sub(seg.chars().count());
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), dim()));
        }
        spans.push(Span::styled(seg, dim()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
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
    use ratatui::style::Modifier;
    use wcode_harness::protocol::SessionId;

    /// The default root surface's id (`App::new`'s single surface).
    fn root() -> SessionId {
        SessionId::agent("root")
    }

    /// Feed one `Key::Char` event per character, as if typed.
    fn typed(app: &mut App, s: &str) {
        for c in s.chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
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

    /// The team strip's row: the first buffer row that names `label`.
    fn strip_row(terminal: &Terminal<TestBackend>, label: &str) -> u16 {
        buffer_text(terminal)
            .lines()
            .position(|line| line.contains(label))
            .expect("the strip row") as u16
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
                    draw_completion(frame, area, above, &app);
                }
            })
            .unwrap();
    }

    #[test]
    fn an_empty_roster_draws_no_strip() {
        let mut app = App::new();
        let text = buffer_text(&render(&mut app, 80, 20));
        assert!(!text.contains("team"), "no strip without a team:\n{text}");
    }

    #[test]
    fn the_team_strip_lists_members_and_hides_when_narrow() {
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

        // Wide enough: the strip shows each member with its status glyph — but
        // never the model (§2).
        let wide = buffer_text(&render(&mut app, 80, 20));
        assert!(
            wide.contains("● explorer"),
            "running member missing:\n{wide}"
        );
        assert!(wide.contains("○ reviewer"), "idle member missing:\n{wide}");
        assert!(
            !wide.contains("m1") && !wide.contains("m2"),
            "the model should be gone from the strip:\n{wide}"
        );

        // Narrow (< 50 cols): the strip is hidden.
        let narrow = buffer_text(&render(&mut app, 49, 20));
        assert!(
            !narrow.contains("explorer"),
            "the strip should hide when narrow:\n{narrow}"
        );
    }

    #[test]
    fn the_strip_shows_only_at_the_width_threshold() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        // 49 cols: hidden. 50 (the threshold): shown.
        assert!(!buffer_text(&render(&mut app, 49, 12)).contains("explorer"));
        assert!(buffer_text(&render(&mut app, 50, 12)).contains("explorer"));
    }

    #[test]
    fn a_short_terminal_draws_the_strip_without_panicking() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        // Tiny heights leave a degenerate `body` (even 0–1 rows): no panic. The
        // strip itself is hidden below its minimum height (6 rows).
        for height in [3u16, 4, 5] {
            let text = buffer_text(&render(&mut app, 80, height));
            assert!(
                !text.contains("explorer"),
                "the strip must hide below 6 rows ({height}):\n{text}"
            );
        }
        // From the minimum height up, body + rule + strip + input + status all
        // fit and the strip still renders on a short screen.
        for height in [6u16, 7, 8] {
            let text = buffer_text(&render(&mut app, 80, height));
            assert!(
                text.contains("explorer"),
                "the strip should render at {height} rows:\n{text}"
            );
        }
    }

    #[test]
    fn the_completion_popup_does_not_overdraw_the_strip() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m");
        for c in "/mo".chars() {
            app.handle(AppEvent::Key(Key::Char(c)));
        }
        assert!(!app.completion_rows().is_empty(), "the popup should be open");

        let terminal = render(&mut app, 80, 16);
        let rows: Vec<String> = buffer_text(&terminal).lines().map(str::to_string).collect();
        // The popup floats above the rule; the strip sits below it, so the two
        // must never share a row.
        let strip_row = rows
            .iter()
            .position(|row| row.contains("explorer"))
            .expect("the strip row");
        assert!(
            !rows[strip_row].contains("commands"),
            "the popup overdraws the strip:\n{}",
            rows[strip_row]
        );
        // Sanity: the popup is still drawn elsewhere.
        assert!(buffer_text(&terminal).contains("commands"));
    }

    #[test]
    fn strip_slots_are_equal_shares_after_overflow_and_gaps() {
        // 3 members, no overflow: three equal slots, two 1-char gaps.
        // 100 cols → (100 − 2) / 3 = 32 each.
        assert_eq!(slot_geometry(100, 3), (3, 32, 0));
        // 5 members: three slots + a right-aligned "+2" (2 chars + a space).
        // 100 → (100 − 3 − 2) / 3 = 31.
        assert_eq!(slot_geometry(100, 5), (3, 31, 2));
        // A band too narrow to seat a slot is guarded, never div-by-zero
        // (slot_width 0 means the renderer draws nothing).
        assert_eq!(slot_geometry(4, 5), (3, 0, 2));
        assert_eq!(slot_geometry(0, 4), (3, 0, 0));
        // No members: nothing to draw.
        assert_eq!(slot_geometry(80, 0), (0, 0, 0));
    }

    #[test]
    fn the_strip_band_sums_exactly_to_the_width() {
        // The band's shares + separators (+ `+N`) must total the width exactly
        // for every width and team size the strip can show — never a shortfall
        // at the right edge, never an overflow onto the next band.
        for width in [60usize, 80, 97, 120] {
            for n in 1..=10usize {
                let (shown, share, overflow) = slot_geometry(width, n);
                if shown == 0 || share == 0 {
                    continue; // guarded: the renderer draws nothing
                }
                // The `+N` segment, leading space included (as budgeted).
                let seg_w = if overflow > 0 {
                    overflow.to_string().chars().count() + 1
                } else {
                    0
                };
                let inner = width.saturating_sub(seg_w).saturating_sub(shown - 1);
                let slack = inner.saturating_sub(share.saturating_mul(shown));
                let total = share
                    .saturating_mul(shown)
                    .saturating_add(slack)
                    .saturating_add(shown - 1)
                    .saturating_add(seg_w);
                assert_eq!(total, width, "width {width}, n {n}");
            }
        }
    }

    #[test]
    fn the_strip_folds_members_beyond_three_into_a_plus_count() {
        let mut app = App::new();
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w1"),
                label: "w1".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w2"),
                label: "w2".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w3"),
                label: "w3".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w4"),
                label: "w4".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w5"),
                label: "w5".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        // Three equal slots share the band; the two remaining members fold into
        // a right-aligned "+2" instead of a fourth slot.
        let terminal = render(&mut app, 100, 12);
        let text = buffer_text(&terminal);
        assert!(text.contains("w1"), "slot w1 missing:\n{text}");
        assert!(text.contains("w2"), "slot w2 missing:\n{text}");
        assert!(text.contains("w3"), "slot w3 missing:\n{text}");
        assert!(text.contains("+2"), "overflow count missing:\n{text}");
        assert!(
            !text.contains("w4") && !text.contains("w5"),
            "members beyond the top three should fold into +N:\n{text}"
        );
        // "+2" is flush against the right edge, on the same dim band.
        let y = strip_row(&terminal, "w1");
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(99, y)].symbol(), "2", "+N not right-aligned:\n{text}");
        assert_eq!(buf[(98, y)].symbol(), "+");
        // The seam before it is dim fill, never slot content.
        assert_eq!(buf[(97, y)].symbol(), " ");
        assert_ne!(buf[(97, y)].style(), Style::default());
    }

    #[test]
    fn a_long_label_is_clipped_within_its_slot_without_hiding_neighbors() {
        let mut app = App::new();
        let long = "very-long-member-aaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("alpha"),
                label: "alpha".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent(long),
                label: long.into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("beta"),
                label: "beta".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        // Three slots at 80 cols → (80 − 2) / 3 = 26 each. The long label clips
        // inside its own slot (a single …) and must not hide "alpha" or "beta".
        let text = buffer_text(&render(&mut app, 80, 12));
        assert!(
            text.contains("alpha") && text.contains("beta"),
            "a long label hides its neighbors:\n{text}"
        );
        assert!(text.contains("…"), "the long label should clip:\n{text}");
        assert!(
            !text.contains(long),
            "the long label must be cut to its slot:\n{text}"
        );
        // The clip keeps the glyph + label prefix, only the tail is cut.
        assert!(
            text.contains("○ very-long-member"),
            "the clip must cut the label tail, not the head:\n{text}"
        );
    }

    #[test]
    fn the_strip_fills_the_band_edge_to_edge_for_any_team_size() {
        // 1, 2, or 3 members (no `+N`): the dim band reaches the right edge
        // exactly, so a lone or part-filled strip still spans the screen.
        for n in 1..=3 {
            let mut app = App::new();
            let mut surfaces = vec![crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            }];
            for i in 1..=n {
                surfaces.push(crate::SurfaceInfo {
                    id: SessionId::agent(format!("w{i}")),
                    label: format!("w{i}"),
                    model: "m".into(),
                    is_root: false,
                });
            }
            app.set_surfaces(surfaces);
            let terminal = render(&mut app, 80, 12);
            let y = strip_row(&terminal, "w1");
            let buf = terminal.backend().buffer();
            // Content starts at the left edge; the band's last column is
            // dim-filled, never empty chrome.
            assert_ne!(buf[(0, y)].symbol(), " ", "n={n}: content at the left edge");
            assert_eq!(buf[(79, y)].symbol(), " ", "n={n}: fill reaches the last col");
            assert_ne!(
                buf[(79, y)].style(),
                Style::default(),
                "n={n}: the fill is a dim band"
            );
            // `│` separators only between the n-1 shares, never at the edges.
            let bars: Vec<u16> = (0..80).filter(|&x| buf[(x, y)].symbol() == "│").collect();
            assert_eq!(bars.len(), n - 1, "n={n}: one separator per gap");
            assert_ne!(buf[(0, y)].symbol(), "│");
            assert_ne!(buf[(79, y)].symbol(), "│");
        }
    }

    #[test]
    fn the_running_member_reads_green() {
        let mut app = App::new();
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("runner"),
                label: "runner".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("idle"),
                label: "idle".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        app.handle(AppEvent::Agent(
            SessionId::agent("runner"),
            wcode_harness::event::AgentEvent::AgentStart,
        ));
        let terminal = render(&mut app, 80, 12);
        let text = buffer_text(&terminal);
        let y = strip_row(&terminal, "runner");
        let buf = terminal.backend().buffer();
        let row = text.lines().nth(y as usize).expect("the strip row");
        // The running head is a green signal — not faint/white like accent.
        let run_x = row[..row.find("runner").expect("running label")].chars().count() as u16;
        assert_eq!(buf[(run_x - 2, y)].symbol(), "●", "running glyph");
        assert_eq!(
            buf[(run_x - 2, y)].style().fg,
            success().fg,
            "running must read green, like the ✓ marks"
        );
        // …and stays distinct from an idle member's dim head.
        let idle_x = row[..row.find("idle").expect("idle label")].chars().count() as u16;
        assert_ne!(buf[(idle_x - 2, y)].style().fg, success().fg);
    }

    #[test]
    fn dim_bars_separate_the_shares() {
        let mut app = App::new();
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w1"),
                label: "w1".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w2"),
                label: "w2".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("w3"),
                label: "w3".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        let terminal = render(&mut app, 80, 12);
        let y = strip_row(&terminal, "w1");
        let buf = terminal.backend().buffer();
        // 80 cols, 3 shares: (80 − 2) / 3 = 26 each; the dim `│` is a
        // column BETWEEN shares (cols 26 and 53) — equal columns spanning the
        // screen, never at the outer edges, and the band ends flush at the edge.
        let bars: Vec<u16> = (0..80).filter(|&x| buf[(x, y)].symbol() == "│").collect();
        assert_eq!(bars, vec![26, 53], "bars between shares, not at the edges");
        assert!(buf[(26, y)].style().add_modifier.contains(Modifier::DIM), "the bar is dim chrome");
        assert!(buf[(53, y)].style().add_modifier.contains(Modifier::DIM));
        assert_eq!(buf[(79, y)].symbol(), " ", "3-member fill reaches the right edge");
        assert_ne!(buf[(79, y)].style(), Style::default());
    }

    #[test]
    fn a_long_label_never_eats_the_plus_count() {
        let mut app = App::new();
        let long = "very-long-member-aaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        app.set_surfaces(vec![
            crate::SurfaceInfo {
                id: root(),
                label: "root".into(),
                model: "rm".into(),
                is_root: true,
            },
            crate::SurfaceInfo {
                id: SessionId::agent(long),
                label: long.into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("alpha"),
                label: "alpha".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("beta"),
                label: "beta".into(),
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("gamma"),
                label: "gamma".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        // 80 cols, 4 members: three shares (25/25/25) + a right-aligned "+1".
        let terminal = render(&mut app, 80, 12);
        let text = buffer_text(&terminal);
        let y = strip_row(&terminal, "very-long-member");
        let buf = terminal.backend().buffer();
        // The "+1" survives at the far right…
        assert_eq!(buf[(78, y)].symbol(), "+");
        assert_eq!(buf[(79, y)].symbol(), "1", "+N hidden:\n{text}");
        // …while the long label clips inside its own share; its neighbors and
        // the seam before "+1" stay intact.
        assert!(text.contains("…"), "the long label should clip:\n{text}");
        assert!(
            text.contains("alpha") && text.contains("beta"),
            "a long label hides its neighbors:\n{text}"
        );
        for x in 75..78 {
            assert_eq!(buf[(x, y)].symbol(), " ", "dim seam before +N:\n{text}");
            assert_ne!(buf[(x, y)].style(), Style::default());
        }
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
            text.contains("… +15 more lines"),
            "collapsed hint missing:\n{text}"
        );
        assert!(text.contains("line-2"), "the head preview is missing:\n{text}");
        assert!(
            !text.contains("line-20"),
            "the tail must be elided when collapsed:\n{text}"
        );
    }

    #[test]
    fn enter_expands_the_selected_tool_fully() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        // Browse selects the last block (this tool); Enter expands it.
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 70, 30));
        assert!(text.contains("line-20"), "the full output should show:\n{text}");
        assert!(
            !text.contains("more lines"),
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
            !text.contains("more lines"),
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
            collapsed.contains("… +22 more lines"),
            "diff cap hint missing:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("+add-30"),
            "the diff tail must be elided:\n{collapsed}"
        );

        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));
        let expanded = buffer_text(&render(&mut app, 70, 40));
        assert!(expanded.contains("+add-30"), "the full diff should show:\n{expanded}");
        assert!(
            !expanded.contains("more lines"),
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
            collapsed.contains("the full line is elided"),
            "the truncation must be hinted:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("TAIL-REACHABLE"),
            "the tail must be hidden while collapsed:\n{collapsed}"
        );

        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));
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
        assert!(collapsed.contains("… +5 more lines"), "{collapsed}");

        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));
        let expanded = buffer_text(&render(&mut app, 70, 20));
        assert!(expanded.contains("row 10"), "not all lines shown:\n{expanded}");
        assert!(
            !expanded.contains("more lines"),
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
        assert!(text.contains("… +1 more line"), "{text}");
        assert!(!text.contains("+1 more lines"), "not pluralized:\n{text}");

        // 7 output lines = summary + 6 body lines → 2 hidden (plural).
        let mut app = App::new();
        let seven = (1..=7)
            .map(|i| format!("row {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &seven, false, None);
        let text = buffer_text(&render(&mut app, 70, 20));
        assert!(text.contains("… +2 more lines"), "{text}");
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
    fn enter_toggles_only_the_selected_tool() {
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

        // Browse selects the last block; Enter toggles just it.
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 70, 45));
        // The selected block is fully expanded…
        assert!(text.contains("LAST-20"), "the selected block should expand:\n{text}");
        // …while the first stays collapsed (its preview stops at line 5).
        assert!(text.contains("FIRST-5"), "{text}");
        assert!(
            !text.contains("FIRST-6"),
            "only the selected block should toggle:\n{text}"
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
            !text.contains("more lines"),
            "an errored tool is expanded, so no hint:\n{text}"
        );
    }

    #[test]
    fn a_wrapped_running_tool_body_counts_toward_the_scroll_height() {
        let mut app = App::new();
        // One long line, still streaming (not done), expanded in browse mode.
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
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));

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
    fn search_prompt_is_drawn_only_while_open_and_never_reflows() {
        let mut app = App::new();
        app.seed_history(
            &root(),
            &[
                AgentMessage::user_text("alpha one"),
                reply("beta two ALPHA three"),
                AgentMessage::user_text("gamma"),
            ],
        );
        let plain = buffer_text(&render(&mut app, 60, 16));
        let plain_total = app.total_lines();
        assert!(!plain.contains(" search "), "no prompt in input mode:\n{plain}");

        app.handle(AppEvent::Key(Key::Ctrl('g')));
        let browse = buffer_text(&render(&mut app, 60, 16));
        assert!(!browse.contains(" search "), "no prompt in plain browse:\n{browse}");

        // Type: the prompt appears with the query, the live hit count and a
        // hint, and it injects no rows into the transcript.
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alp");
        let open = buffer_text(&render(&mut app, 60, 16));
        assert!(open.contains("/alp"), "the query line is drawn:\n{open}");
        assert!(open.contains("2 hits"), "the live count is drawn:\n{open}");
        assert!(
            open.contains("enter jump · esc close"),
            "the hint line is drawn:\n{open}"
        );
        assert_eq!(app.total_lines(), plain_total, "the prompt adds no rows");

        // A second draw is byte-identical — the overlay is deterministic.
        let again = buffer_text(&render(&mut app, 60, 16));
        assert_eq!(open, again, "the prompt frame is stable");

        // Esc closes the prompt and restores the exact plain-browse frame.
        app.handle(AppEvent::Key(Key::Esc));
        let closed = buffer_text(&render(&mut app, 60, 16));
        assert_eq!(closed, browse, "closing the prompt restores the browse frame");
        assert!(!closed.contains(" search "), "the prompt is gone:\n{closed}");
    }

    #[test]
    fn enter_in_search_jumps_and_the_target_keeps_its_bar() {
        let mut app = App::new();
        app.seed_history(
            &root(),
            &[
                AgentMessage::user_text("alpha one"),
                reply("beta two"),
                AgentMessage::user_text("ALPHA three"),
            ],
        );
        app.handle(AppEvent::Key(Key::Ctrl('g'))); // selects the last block (ALPHA three)
        assert_eq!(app.selected(), Some(3));
        // transcript: [⋯ 3 earlier, alpha, beta, ALPHA] — "alpha" matches 0 and 3.
        app.handle(AppEvent::Key(Key::Char('k'))); // step up to "beta two" (block 2)
        assert_eq!(app.selected(), Some(2));
        app.handle(AppEvent::Key(Key::Char('/')));
        typed(&mut app, "alpha");
        app.handle(AppEvent::Key(Key::Enter));
        assert!(app.search_query().is_none(), "Enter closes the prompt");
        assert_eq!(app.selected(), Some(3), "first match at/after block 2 is 3");
        let text = buffer_text(&render(&mut app, 60, 16));
        assert!(!barred(&text).is_empty(), "the jumped-to block is drawn with its bar");
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

    #[test]
    fn enter_in_browse_expands_the_selected_tool_in_the_frame() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        app.handle(AppEvent::Key(Key::Ctrl('g')));

        let collapsed = buffer_text(&render(&mut app, 70, 30));
        assert!(!collapsed.contains("line-20"), "collapsed hides the tail:\n{collapsed}");
        // The bar covers the whole block: header + 4 preview lines + the hint.
        assert_eq!(barred(&collapsed).len(), 6, "{collapsed}");

        app.handle(AppEvent::Key(Key::Enter));
        let expanded = buffer_text(&render(&mut app, 70, 30));
        assert!(expanded.contains("line-20"), "Enter shows the full output:\n{expanded}");
        // Header + all 20 output lines.
        assert_eq!(barred(&expanded).len(), 21, "{expanded}");
    }

    #[test]
    fn a_tool_call_only_assistant_block_leaves_no_stray_blank() {
        let mut app = App::new();
        // An assistant turn carrying only a tool call renders zero lines: it must
        // not add a separator, or the transcript gains a stray blank line.
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "c1".into(),
                        name: "read".into(),
                        arguments: "{\"path\":\"a.rs\"}".parse().unwrap(),
                    }],
                    stop_reason: wcode_harness::message::StopReason::ToolUse,
                    usage: None,
                    model: None,
                },
            },
        ));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "c1".into(),
                name: "read".into(),
            },
        ));

        let text = buffer_text(&render(&mut app, 80, 12));
        let first = text.lines().next().unwrap_or_default();
        assert!(
            first.contains("read"),
            "the tool line is the first row, with no leading blank:\n{text}"
        );
    }
}



