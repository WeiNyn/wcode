//! Immediate-mode rendering: compose the whole frame from [`App`] each draw.
//!
//! Bands (top → bottom): session · transcript · [working-team region] · input
//! box. The team region grows to at most three rows (running teammates only);
//! the rounded input box carries the chrome in its four corners.
//! See `docs/tui-design.md` for the visual spec.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, BorderType, Borders, Clear, Paragraph};
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

/// The working-team region appears only when the terminal is at least this wide.
const TEAM_MIN_WIDTH: u16 = 50;
/// … and at least this tall, so the session line, the transcript, the team
/// region, and the input box all leave the transcript something to show.
const TEAM_MIN_HEIGHT: u16 = 8;
/// The docked left sidebar's fixed width in columns. Its content is clipped
/// to fit; the bands to its right are NOT reflowed to compensate.
const SIDEBAR_WIDTH: u16 = 30;
/// The sidebar docks only when the WHOLE terminal is at least this wide —
/// below it there is not enough room for a 30-col panel AND a usable bands
/// column, so `draw` leaves the layout completely untouched.
const SIDEBAR_MIN_WIDTH: u16 = 80;

/// Draw the full frame. Stateless: everything comes from `app`.
///
/// Bands (top → bottom), in the bands column to the right of the optional
/// sidebar: a dim `session` line, the transcript, the working-team region
/// (0..=3 rows), and the rounded input box whose four corners carry the chrome
/// the old status band used to. The team region collapses to nothing when no
/// teammate is running.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let full = frame.area();
    // PHASE 2 — the docked left SIDEBAR. When Ctrl-B has it open AND the
    // terminal is wide enough, split the WHOLE area horizontally into
    // [sidebar | bands] and render the existing band stack in the right
    // column. When closed (or too narrow) `area` is the full frame untouched,
    // so every geometry below — and every popup/overlay anchor — is
    // byte-identical.
    let (sidebar, area) = if app.sidebar() && full.width >= SIDEBAR_MIN_WIDTH {
        let [sb, bands] = Layout::horizontal([
            Constraint::Length(SIDEBAR_WIDTH),
            Constraint::Min(1),
        ])
        .areas(full);
        (Some(sb), bands)
    } else {
        (None, full)
    };
    if let Some(sb) = sidebar {
        // The docked panel floats nothing: it is the left column itself. The
        // modal is drawn over `full` (see the tail), so it covers the sidebar.
        draw_sidebar(frame, sb, app);
    }
    // The input grows with its *wrapped* row count (Shift-Enter / Ctrl-J add
    // lines; long lines wrap), capped so it never crowds out the transcript.
    let width = input_content_width(area);
    let max_rows = 8.min(area.height as usize / 2).max(1);
    let view = app.input_view();
    let (_, _, _, total_rows) = input_rows(&view.display, view.cursor_col, width);
    let input_height = total_rows.clamp(1, max_rows) as u16;

    // The working-team region sits ABOVE the input box: one row per RUNNING
    // teammate (the accessor orders oldest→newest and caps at three). It
    // collapses to nothing when none are running or the terminal cannot seat it
    // without starving the transcript.
    // Only the COUNT is needed for the layout; the rows themselves are fetched
    // again below, after the transcript's `&mut` borrow ends.
    let working_count = app.working_team_rows().len();
    let mut team_h = if working_count == 0
        || area.width < TEAM_MIN_WIDTH
        || area.height < TEAM_MIN_HEIGHT
    {
        0
    } else {
        working_count as u16
    };

    // The session line (1 row) and at least one transcript row are reserved; the
    // input box (two borders + the composer) then has priority over the team
    // region, so a short terminal degrades gracefully instead of overflowing.
    let session_h = u16::from(app.status().session.is_some());
    let spare = area.height.saturating_sub(session_h + 1);
    let box_h = (input_height + 2).min(spare).max(1);
    team_h = team_h.min(spare.saturating_sub(box_h));

    let areas = Layout::vertical([
        Constraint::Length(session_h), // session id
        Constraint::Min(1),         // transcript
        Constraint::Length(team_h), // working-team region
        Constraint::Length(box_h),  // input box
    ])
    .split(area);
    let session = areas[0];
    let body = areas[1];
    let team = areas[2];
    let editor = areas[3];

    draw_session_line(frame, session, app);
    draw_transcript(frame, body, app);
    if team_h > 0 {
        let working = app.working_team_rows();
        draw_working_team(frame, team, &working);
    }
    draw_input_box(frame, editor, app, &view);
    // The completion/search popups float just above the input box's TOP border,
    // over the transcript interior — never over the composer.
    let above = Rect {
        y: editor.y,
        height: 1,
        ..area
    };
    draw_completion(frame, area, above, app);
    draw_search_prompt(frame, area, above, app);
    // The modal, if any, is drawn last — over the WHOLE terminal (`full`), so
    // it covers the docked sidebar too; the popups above float over the bands.
    draw_overlay(frame, full, app);
}

/// Draw the browse transcript-search prompt, floating just above the input
/// band like the command completion. Browse-owned and non-reflowing: it is
/// `Clear`ed over the transcript only while the prompt is open, so an
/// input-mode frame stays byte-identical.
///
/// The popup anchors on a 1-row `above` rect at the input box's TOP border, so
/// it floats over the transcript interior and never covers the composer.
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

/// Draw the transcript into its band: the committed blocks (cached per width)
/// plus the in-flight message. `area` is the plain transcript band — the wrap
/// width is `area.width` and the viewport height is `area.height`, feeding
/// `sync_scroll`; the selection bar paints column 0 in a second pass.
///
/// `Surface::cache` keys on width, so a reflow (e.g. toggling the sidebar,
/// which changes the bands width by `SIDEBAR_WIDTH`) invalidates every cached
/// block.
fn draw_transcript(frame: &mut Frame, area: Rect, app: &mut App) {
    let width = area.width as usize;
    let mut lines: Vec<Line> = Vec::new();
    // Record each committed block's line range so the selection bar can be drawn
    // in a second pass. A range starts *after* the separator, so it never spans
    // the blank line above the block.
    let mut ranges: Vec<Range<usize>> = Vec::new();
    // Length FIRST (immutable), then each index appended through the `&mut` cache:
    // `append_block_lines` owns the transcript/cache split borrow, so this loop
    // never holds `transcript().iter()` across a mutable cache borrow.
    let n = app.transcript().len();
    for i in 0..n {
        ranges.push(app.focused_mut().append_block_lines(i, width, &mut lines));
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

pub(crate) fn block_lines(block: &Block, width: usize) -> Vec<Line<'static>> {
    match block {
        Block::User(text) => wrap(text, width, " ❯ ", "   ", user()),
        Block::Assistant(content) => content_lines(content, width, false),
        Block::Tool(tool) => tool_lines(tool, width),
        Block::Notice(text) => wrap(text, width, "   ", "   ", dim()),
        Block::Btw(text) => wrap(text, width, " btw ", "     ", thinking()),
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
/// The tool's INPUT target (`Tool::target`), falling back to the changed path —
/// the `command`/`path`/`pattern` the call named, clipped to one line so a long
/// command never floods the header.
fn target_span(tool: &Tool) -> Option<Span<'static>> {
    let target = tool.target.as_ref().or(tool.path.as_ref())?;
    let first = target.lines().next().unwrap_or(target.as_str());
    let clipped = if first.chars().count() > 60 {
        let (cut, _) = split_at_char(first, 59);
        format!("{cut}…")
    } else {
        first.to_string()
    };
    Some(Span::styled(format!("  {clipped}"), dim()))
}

fn tool_lines(tool: &Tool, width: usize) -> Vec<Line<'static>> {
    // A live tool: the header, then (collapsed) its last non-blank line or
    // (expanded) the tail of the live output, because it grows downward.
    if !tool.done {
        let mut header = vec![
            Span::styled("   ⚙ ", dim()),
            Span::styled(tool.name.clone(), tool_name()),
        ];
        header.extend(target_span(tool));
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
    spans.extend(target_span(tool));
    if !expanded && !note.is_empty() {
        spans.push(Span::styled(format!(" · {note}"), dim()));
    }
    if let Some(ms) = tool.duration_ms {
        spans.push(Span::styled(format!(" · {}", format_ms(ms)), dim()));
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
    header.extend(target_span(tool));
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

/// The dim `session <short id>` line above the transcript. Blank when no session
/// id is known (an attached/remote session may not have one).
fn draw_session_line(frame: &mut Frame, area: Rect, app: &App) {
    let Some(session) = &app.status().session else {
        return;
    };
    let line = Line::from(Span::styled(format!(" session {}", short_id(session)), dim()));
    frame.render_widget(Paragraph::new(line), area);
}

/// Draw the input box: a ROUNDED bordered `Block` wrapping the composer, whose
/// four corners carry the chrome the old status band used to (project/branch,
/// model/effort, the context gauge, and the mode/state). The composer renders in
/// the block's inner rect, so its wrap width is `area.width - 2`.
fn draw_input_box(frame: &mut Frame, area: Rect, app: &App, view: &InputView) {
    let (tl, tr, bl, br) = corner_titles(app, area);
    let block = WidgetBlock::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border())
        .title_top(tl.left_aligned())
        .title_top(tr.right_aligned())
        .title_bottom(bl.left_aligned())
        .title_bottom(br.right_aligned());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    draw_input(frame, inner, view);
}

/// Assemble the input box's four corner titles, width-budgeted to `area` so the
/// two titles on a row never overdraw the border.
///
/// Each row's two titles share the row: `area.width` minus the two border
/// columns and a 1-col gap so they never touch. Fields are dropped
/// least-important-first (top: ` · ⎇ branch` then ` · effort`; bottom: `↑ N`
/// scroll, then the gauge, then `⏻ plan`/`▤ browse`), and a still-too-long title
/// is truncated with a trailing `…`. The surviving minimum is the project
/// (top-left) and the mode/state (bottom-right).
fn corner_titles(
    app: &App,
    area: Rect,
) -> (Line<'static>, Line<'static>, Line<'static>, Line<'static>) {
    // Title columns available per row (the two border columns + a 1-col gap).
    let avail = (area.width as usize).saturating_sub(3);

    // --- top row: `{project} · ⎇ {branch}`   ·   `{model} · {effort}` --------
    let project = app.cwd().unwrap_or("wcode").to_string();
    let model = app.status().model.clone();
    let branch = app.git().map(|git| format!("⎇ {git}"));
    let effort = app.status().effort.clone();
    let tl = |text: &str| -> Vec<Span<'static>> {
        let (head, rest) = split_at_char(text, project.chars().count());
        let mut spans = vec![Span::styled(head, accent())];
        if !rest.is_empty() {
            spans.push(Span::styled(rest, dim()));
        }
        spans
    };
    let tr = |text: String| -> Vec<Span<'static>> { vec![Span::styled(text, dim())] };
    let top_levels = vec![
        (
            tl(&join_title(&project, branch.as_deref())),
            tr(join_title(&model, effort.as_deref())),
        ),
        (tl(&project), tr(join_title(&model, effort.as_deref()))),
        (tl(&project), tr(model.clone())),
    ];
    let (top_left, top_right) = pick_titles(&top_levels, avail);

    // --- bottom row: the gauge   ·   `⏻ plan · ▤ browse · {state} · ↑ N` -----
    let state = match (app.running(), app.run_elapsed()) {
        (true, Some(d)) => format!("⠹ running {}", format_ms(d.as_millis() as u64)),
        (true, None) => "⠹ running".to_string(),
        (false, _) => "⏸ idle".to_string(),
    };
    let br = |show_plan: bool, show_browse: bool, show_scroll: bool| -> Vec<Span<'static>> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if show_plan && app.status().plan {
            spans.push(Span::styled("⏻ plan", accent()));
        }
        if show_browse && app.mode() == Mode::Browse {
            if !spans.is_empty() {
                spans.push(sep());
            }
            spans.push(Span::styled("▤ browse", accent()));
        }
        if !spans.is_empty() {
            spans.push(sep());
        }
        spans.push(Span::styled(state.clone(), dim()));
        if show_scroll && app.scroll() > 0 {
            spans.push(sep());
            spans.push(Span::styled(format!("↑ {}", app.scroll()), dim()));
        }
        spans
    };
    let gauge = token_spans(app).unwrap_or_default();
    let bottom_levels = vec![
        (gauge.clone(), br(true, true, true)),
        (gauge.clone(), br(true, true, false)),
        (Vec::new(), br(true, true, false)),
        (Vec::new(), br(false, false, false)),
    ];
    let (bottom_left, bottom_right) = pick_titles(&bottom_levels, avail);

    (top_left, top_right, bottom_left, bottom_right)
}

/// Join a title's base with an optional extra using the ` · ` separator.
fn join_title(base: &str, extra: Option<&str>) -> String {
    match extra {
        Some(extra) => format!("{base} · {extra}"),
        None => base.to_string(),
    }
}

/// The first of `levels` (fullest → most-dropped) whose two titles fit `avail`
/// columns; else the last level with each side truncated to fit. Titles are
/// [`Span`] runs, so styling rides through the budget.
fn pick_titles(
    levels: &[(Vec<Span<'static>>, Vec<Span<'static>>)],
    avail: usize,
) -> (Line<'static>, Line<'static>) {
    let width = |spans: &[Span<'static>]| -> usize {
        spans.iter().map(|s| s.content.chars().count()).sum()
    };
    let (mut left, mut right) = levels.last().cloned().unwrap_or_default();
    for (l, r) in levels {
        if width(l) + width(r) <= avail {
            left = l.clone();
            right = r.clone();
            break;
        }
    }
    if width(&left) + width(&right) > avail {
        // Last resort: split the row and truncate each side.
        let left_room = (avail / 2).max(1);
        left = truncate_spans(&left, left_room);
        right = truncate_spans(&right, avail.saturating_sub(width(&left)));
    }
    (Line::from(left), Line::from(right))
}

/// Truncate a span run to at most `max` columns, keeping the first span's style.
fn truncate_spans(spans: &[Span<'static>], max: usize) -> Vec<Span<'static>> {
    let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
    if text.chars().count() <= max {
        return spans.to_vec();
    }
    let style = spans.first().map_or_else(Style::default, |s| s.style);
    vec![Span::styled(truncate(&text, max), style)]
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

/// A gauge of `cells` parallelograms: filled `▰`, empty `▱` (the caller colors
/// it by fill: green → yellow → red). e.g. `▰▰▰▰▰▰▱▱`.
fn bar(ratio: f64, cells: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0) * cells as f64).round() as usize).min(cells);
    let mut out = "▰".repeat(filled);
    out.push_str(&"▱".repeat(cells - filled));
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

/// `12ms`, `1.2s`, `1m04s` — a duration for the tool header and the running
/// state glyph, mirroring `format_tokens` / `format_scaled`.
fn format_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        format!("{}m{:02}s", ms / 60_000, (ms % 60_000) / 1000)
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
/// from idle/done), done = muted, failed = red [`error_style`].
fn state_style(state: TeamState) -> Style {
    match state {
        TeamState::Idle => dim(),
        TeamState::Running => success(),
        TeamState::Done => muted(),
        TeamState::Failed => error_style(),
    }
}

/// Draw the docked left sidebar: one fixed-width column stacking the “more
/// info” the bands cannot all show at once — **Team**, **Todos**, **Changes**.
/// Each section is a dim header line followed by its rows; an
/// empty section keeps its header with a dim `—` placeholder, so the stack
/// does not jump as data arrives.
///
/// Never panics and never reflows the bands: it is a fixed column and every
/// row is clipped to `area` (a `Paragraph` drops rows past the bottom and
/// clips long text at the right edge — no wrap). A zero height/width
/// draws nothing.
///
/// Data, per section:
/// - Team    → [`App::member_rows`] (`label`, [`crate::TeamState`], focused,
///   live action); glyph `state.glyph()` styled by `state_style(state)`.
///   `member_rows` drops `last_action_at`, so a per-member elapsed is not
///   reachable; only the focused surface's [`App::run_elapsed`] rides its row.
/// - Todos   → [`App::last_todos`] (`☑`/`☐` + `content`, header `done/total`).
/// - Changes → [`App::changes`] (`path · +added −removed`).
fn draw_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ---- Team -----------------------------------------------------------
    lines.push(Line::from(Span::styled("Team", dim())));
    let rows = app.member_rows();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled("  —", dim())));
    }
    for (label, state, focused, action) in rows {
        // `  ● explorer *  read a.rs` — glyph colored by state, the live
        // action dim, `*` marks the focused surface. A per-member elapsed is
        // not exposed, so the focused row carries the run elapsed instead.
        let mut head = format!(
            "  {} {}{}",
            state.glyph(),
            label,
            if focused { " *" } else { "" }
        );
        if focused && let Some(elapsed) = app.run_elapsed() {
            head.push_str(&format!("  {}", format_ms(elapsed.as_millis() as u64)));
        }
        let tail = action.map(|a| format!("  {a}")).unwrap_or_default();
        lines.push(clipped_row(vec![(head, state_style(state)), (tail, dim())], w));
    }

    // ---- Todos ----------------------------------------------------------
    match app.last_todos() {
        Some(todos) if !todos.is_empty() => {
            let done = todos
                .iter()
                .filter(|t| t.status == wcode_harness::event::TodoStatus::Completed)
                .count();
            lines.push(Line::from(Span::styled(
                format!("Todos  ☑ {done}/{}", todos.len()),
                dim(),
            )));
            for t in todos {
                let mark = if t.status == wcode_harness::event::TodoStatus::Completed {
                    "☑"
                } else {
                    "☐"
                };
                lines.push(clipped_row(vec![(format!("  {mark} {}", t.content), dim())], w));
            }
        }
        _ => {
            lines.push(Line::from(Span::styled("Todos", dim())));
            lines.push(Line::from(Span::styled("  —", dim())));
        }
    }

    // ---- Changes --------------------------------------------------------
    let changes = app.changes();
    if changes.is_empty() {
        lines.push(Line::from(Span::styled("Changes", dim())));
        lines.push(Line::from(Span::styled("  —", dim())));
    } else {
        lines.push(Line::from(Span::styled(
            format!("Changes  {}", changes.len()),
            dim(),
        )));
        for c in changes {
            // `  src/a.rs · +3 −1` — path dim, `+a` added, `−r` removed. Clip
            // the path first (reserving the counts) so the numbers stay visible.
            let sep = " · ";
            let added = format!("+{}", c.added);
            let removed = format!("−{}", c.removed);
            let fixed = 2 + sep.chars().count() + added.chars().count() + 1 + removed.chars().count();
            let budget = w.saturating_sub(fixed);
            let (cut, more) = split_at_char(&c.path, budget);
            let path = if more.is_empty() || budget == 0 {
                cut
            } else {
                let (shorter, _) = split_at_char(&c.path, budget - 1);
                format!("{shorter}…")
            };
            lines.push(clipped_row(
                vec![
                    (format!("  {path}"), dim()),
                    (sep.to_string(), dim()),
                    (added, added_style()),
                    (" ".to_string(), dim()),
                    (removed, removed_style()),
                ],
                w,
            ));
        }
    }

    // Borderless by default so a 30-col panel is 30 cols of content; the
    // bands' own left padding separates the two columns. `Paragraph` clips
    // the row list to `area` — a short panel just loses the bottom sections.
    frame.render_widget(Paragraph::new(lines), area);
}

/// One clipped sidebar row: concatenate the styled segments, and when the row
/// overflows `width` columns drop the tail and ride a `…` on the last
/// surviving span. Char-safe; reserves the ellipsis column before taking.
fn clipped_row(segs: Vec<(String, Style)>, width: usize) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if width == 0 {
        return Line::from(spans);
    }
    let total: usize = segs.iter().map(|(t, _)| t.chars().count()).sum();
    // Reserve the ellipsis column *before* taking, so a cut row never exceeds
    // `width` columns.
    let cut = total > width;
    let mut budget = if cut { width - 1 } else { width };
    for (text, style) in segs {
        if budget == 0 {
            break;
        }
        let n = text.chars().count();
        if n <= budget {
            budget -= n;
            if !text.is_empty() {
                spans.push(Span::styled(text, style));
            }
        } else {
            let (head, _) = split_at_char(&text, budget);
            budget = 0;
            if !head.is_empty() {
                spans.push(Span::styled(head, style));
            }
        }
    }
    if cut && let Some(last) = spans.pop() {
        let mut text = last.content.into_owned();
        text.push('…');
        spans.push(Span::styled(text, last.style));
    }
    Line::from(spans)
}
/// The working-team region ABOVE the input box: one row per RUNNING teammate,
/// `glyph label  action`. `rows` is already filtered to running members, ordered
/// oldest→newest (the LATEST event is the BOTTOM row), and capped to three by
/// [`App::working_team_rows`]. The root is excluded there; idle/done/failed
/// members never appear. No `+N` overflow and no equal-share `│` separators — a
/// plain left-aligned row, clipped by the `Paragraph` at the band's right edge.
fn draw_working_team(frame: &mut Frame, area: Rect, rows: &[(&str, TeamState, Option<&str>)]) {
    for (i, (label, state, action)) in rows.iter().enumerate() {
        let row = Rect {
            y: area.y + i as u16,
            height: 1,
            ..area
        };
        let mut spans = vec![
            Span::styled(format!(" {} ", state.glyph()), state_style(*state)),
            Span::styled((*label).to_string(), state_style(*state)),
        ];
        if let Some(action) = action {
            spans.push(Span::styled(format!("  {action}"), dim()));
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), row);
    }
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

    /// Commit a text `Assistant` block (the cache's most common entry).
    fn push_assistant(app: &mut App, text: &str) {
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::Text { text: text.into() }],
                    stop_reason: wcode_harness::message::StopReason::Stop,
                    usage: None,
                    model: None,
                },
            },
        ));
    }

    #[test]
    fn cache_hit_keeps_the_same_frame() {
        let mut app = App::new();
        push_assistant(&mut app, "hello there");
        push_tool(&mut app, "bash", "one\ntwo", false, None);

        let first = buffer_text(&render(&mut app, 60, 20));
        let before = app.focused().cache_misses();
        let second = buffer_text(&render(&mut app, 60, 20));
        assert_eq!(first, second, "an unchanged frame is byte-identical");
        assert_eq!(
            app.focused().cache_misses(),
            before,
            "the second frame hits every block (zero renders)"
        );
    }

    #[test]
    fn cache_misses_only_for_the_mutated_tool_block() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "one\ntwo", false, None);
        push_tool(&mut app, "read", "three", false, None);
        let _ = render(&mut app, 60, 20); // warm both entries
        let before = app.focused().cache_misses();

        // A live tool update mutates the LAST block only.
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionUpdate {
                call_id: "t".into(),
                name: "bash".into(),
                partial: " more".into(),
            },
        ));
        let _ = render(&mut app, 60, 20);
        assert_eq!(
            app.focused().cache_misses() - before,
            1,
            "only the mutated tool block re-renders; the rest hit"
        );
    }

    #[test]
    fn cache_misses_every_block_after_a_width_change_then_hits() {
        let mut app = App::new();
        push_assistant(&mut app, "hello there");
        push_tool(&mut app, "bash", "one\ntwo", false, None);
        let _ = render(&mut app, 60, 20);
        let n = app.transcript().len();

        let before = app.focused().cache_misses();
        let _ = render(&mut app, 61, 20);
        assert_eq!(
            app.focused().cache_misses() - before,
            n,
            "a resize re-renders every block once (the width key)"
        );
        let after = app.focused().cache_misses();
        let _ = render(&mut app, 61, 20);
        assert_eq!(app.focused().cache_misses(), after, "the re-cache then hits");
    }

    #[test]
    fn the_live_message_is_never_cached() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "one\ntwo", false, None);
        let _ = render(&mut app, 60, 20); // warm the committed block
        let before = app.focused().cache_misses();

        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageStart {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::Text {
                        text: "streaming".into(),
                    }],
                    stop_reason: wcode_harness::message::StopReason::Stop,
                    usage: None,
                    model: None,
                },
            },
        ));
        let text = buffer_text(&render(&mut app, 60, 20));
        assert!(text.contains("streaming"), "the live message renders:\n{text}");
        assert_eq!(
            app.focused().cache_misses(),
            before,
            "the committed block stayed a hit while the live message rendered"
        );
    }

    /// The subtlest risk: `seed_history` inserts its divider MID-VEC, shifting
    /// every later block. All three parallel vecs must shift together.
    #[test]
    fn a_seed_mid_vec_insert_keeps_the_cache_aligned() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "one\ntwo", false, None);
        let _ = render(&mut app, 60, 20);

        app.seed_history(
            &root(),
            &[
                AgentMessage::user_text("earlier-1"),
                AgentMessage::user_text("earlier-2"),
            ],
        );
        let first = buffer_text(&render(&mut app, 60, 20));
        assert!(first.contains("earlier-1"), "seeded block missing:\n{first}");
        assert!(first.contains("earlier-2"), "second seeded block missing:\n{first}");
        assert!(
            first.contains("2 earlier message"),
            "the mid-vec divider notice is missing:\n{first}"
        );

        let before = app.focused().cache_misses();
        let second = buffer_text(&render(&mut app, 60, 20));
        assert_eq!(first, second, "the frame is stable after the mid-vec insert");
        assert_eq!(
            app.focused().cache_misses(),
            before,
            "the shifted blocks re-cached, then hit"
        );
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
        assert_eq!(bar(0.0, 8), "▱▱▱▱▱▱▱▱");
        assert_eq!(bar(1.0, 8), "▰▰▰▰▰▰▰▰");
        assert_eq!(bar(0.5, 8), "▰▰▰▰▱▱▱▱");
        assert_eq!(bar(2.0, 8), "▰▰▰▰▰▰▰▰"); // clamped
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
    fn a_completed_tool_shows_its_duration() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(root(), wcode_harness::event::AgentEvent::ToolExecutionStart {
            call_id: "t1".into(), name: "read".into(),
        }));
        app.handle(AppEvent::Agent(root(), wcode_harness::event::AgentEvent::ToolExecutionEnd {
            call_id: "t1".into(), name: "read".into(),
            output: "128 lines".into(), is_error: false,
            diff: None, path: None, duration_ms: Some(12),
        }));
        let text = buffer_text(&render(&mut app, 60, 12));
        assert!(text.contains("read"), "name missing: {text}");
        assert!(text.contains("12ms"), "duration missing: {text}");
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
                duration_ms: None,
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
                duration_ms: None,
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
        // Content width is ~18 here, so the chip wraps across rows and the
        // input scrolls to keep the cursor (after the chip) visible.
        let text = buffer_text(&render(&mut app, 20, 8));
        assert!(text.contains("chars"), "chip missing:\n{text}");
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
                duration_ms: None,
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
                duration_ms: None,
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
                target: None,
                output: "a\nb\nc\nd\ne\nf".into(),
                done: true,
                is_error,
                expanded,
                diff: diff.map(str::to_string),
                path: Some("f.rs".into()),
                duration_ms: None,
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

    #[test]
    fn ctrl_b_toggles_the_sidebar_flag() {
        let mut app = App::new();
        assert!(!app.sidebar(), "the sidebar is OFF by default");
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        assert!(app.sidebar(), "Ctrl-B opens it");
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        assert!(!app.sidebar(), "Ctrl-B closes it again");
    }

    #[test]
    fn the_open_sidebar_shows_its_section_headers_and_a_member_row() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m1");
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        let frame = buffer_text(&render(&mut app, 100, 24));
        // Section headers are sidebar-only (the strip/status never spell these).
        for header in ["Team", "Todos", "Changes"] {
            assert!(frame.contains(header), "missing {header} header:\n{frame}");
        }
        assert!(frame.contains("explorer"), "member row missing:\n{frame}");
    }

    #[test]
    fn a_closed_sidebar_is_byte_identical_to_the_base_layout() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m1");
        let base = buffer_text(&render(&mut app, 100, 24)); // sidebar OFF
        assert_eq!(base, buffer_text(&render(&mut app, 100, 24)));
        // Open, then close: the frame returns to the exact base bytes.
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        let open = buffer_text(&render(&mut app, 100, 24));
        assert_ne!(open, base, "the open frame actually differs");
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        assert_eq!(
            buffer_text(&render(&mut app, 100, 24)),
            base,
            "closing restores the byte-identical base frame"
        );
    }

    #[test]
    fn a_narrow_terminal_never_docks_the_sidebar() {
        let mut app = App::new();
        with_member(&mut app, "explorer", "m1");
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        let narrow = buffer_text(&render(&mut app, 60, 20)); // < SIDEBAR_MIN_WIDTH
        assert!(
            !narrow.contains("Changes"),
            "no panel under the width threshold:\n{narrow}"
        );
    }

    #[test]
    fn the_input_box_carries_the_corner_titles() {
        let mut app = App::new();
        app.set_cwd(Some("myrepo".into()));
        app.set_git(Some("main*".into()));
        app.set_status(crate::app::Status {
            model: "zephyr-9".into(),
            effort: Some("high".into()),
            session: Some("abcdef0123456789".into()),
            context_limit: Some(1_000_000),
            plan: false,
        });
        let text = buffer_text(&render(&mut app, 80, 14));
        // The input box is rounded and carries the chrome in its four corners.
        assert!(
            text.contains('╭') && text.contains('╮'),
            "no rounded box:\n{text}"
        );
        assert!(text.contains("myrepo"), "project (cwd) top-left:\n{text}");
        assert!(text.contains("⎇ main*"), "branch missing:\n{text}");
        assert!(text.contains("zephyr-9"), "model top-right:\n{text}");
        assert!(text.contains("high"), "effort missing:\n{text}");
        assert!(text.contains("⏸ idle"), "state bottom-right:\n{text}");
    }

    #[test]
    fn the_session_line_is_the_top_row_above_the_transcript() {
        let mut app = App::new();
        app.set_status(crate::app::Status {
            session: Some("abcdef0123456789".into()),
            ..Default::default()
        });
        let text = buffer_text(&render(&mut app, 80, 12));
        let first = text.lines().next().unwrap_or_default();
        assert!(
            first.contains("abcdef01"),
            "session id is not the top row:\n{text}"
        );
    }

    #[test]
    fn the_model_appears_exactly_once_in_the_box() {
        // Regression carried over from the old status line: the model must be
        // named once, on the box's top-right.
        let mut app = App::new();
        app.set_status(crate::app::Status {
            model: "zephyr-9".into(),
            ..Default::default()
        });
        let text = buffer_text(&render(&mut app, 80, 12));
        assert_eq!(text.matches("zephyr-9").count(), 1, "model once:\n{text}");
    }

    #[test]
    fn the_team_region_lists_only_running_members() {
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
                model: "m".into(),
                is_root: false,
            },
            crate::SurfaceInfo {
                id: SessionId::agent("reviewer"),
                label: "reviewer".into(),
                model: "m".into(),
                is_root: false,
            },
        ]);
        // Only explorer runs; reviewer stays idle and must not appear.
        app.handle(AppEvent::Agent(
            SessionId::agent("explorer"),
            wcode_harness::event::AgentEvent::AgentStart,
        ));
        let text = buffer_text(&render(&mut app, 80, 16));
        assert!(text.contains("explorer"), "running member missing:\n{text}");
        assert!(
            !text.contains("reviewer"),
            "an idle member must not show in the team region:\n{text}"
        );
    }

    #[test]
    fn a_short_terminal_draws_the_input_box_without_panicking() {
        let mut app = App::new();
        for height in [2u16, 3, 4, 5, 6, 8] {
            let _ = render(&mut app, 80, height); // must not panic
        }
    }

    #[test]
    fn a_tool_call_renders_its_input_target() {
        // The tool's INPUT rides the committed assistant block's `ToolCall`
        // arguments; a `bash` call must show its command on the tool line.
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "c1".into(),
                        name: "bash".into(),
                        arguments: "{\"command\":\"cargo test -p wcode-cli\"}"
                            .parse()
                            .unwrap(),
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
                name: "bash".into(),
            },
        ));
        let text = buffer_text(&render(&mut app, 80, 14));
        assert!(text.contains("bash"), "tool name missing:\n{text}");
        assert!(
            text.contains("cargo test -p wcode-cli"),
            "the tool's input (command) must render:\n{text}"
        );
    }
}
