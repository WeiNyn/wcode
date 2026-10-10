//! Immediate-mode rendering: compose the whole frame from [`App`] each draw.
//!
//! Bands (top → bottom): session · transcript · [working-team region] · composer
//! band. The team region grows to at most three rows (running teammates only);
//! the composer is a **running-head block** (a head line + a `─` rule) over an
//! open writing line and a foot line — no box, no corners.
//! See `docs/tui-design.md` for the visual spec.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block as WidgetBlock, BorderType, Borders, Clear, Paragraph};
use wcode_harness::event::{TodoItem, TodoStatus};
use wcode_harness::message::{AgentMessage, ContentBlock};

use crate::TeamState;
use crate::app::{AffordanceHit, App, Block, InputView, KEYS, Mode, Overlay, Picker, SessionHead, Tool, Turn};
use crate::app::{ChangeRow, change_totals, changes_header};
use crate::markdown;
use crate::theme;

/// First/continuation prefixes for a thinking block (`···` then an aligned
/// continuation column).
const THINK_FIRST: &str = "   ··· ";
const THINK_CONT: &str = "       ";

/// The **single** content column shared by the user block, the `WCODE` head, the
/// assistant body (markdown), the thinking body, and the inline tool tree (D010).
/// One column, so a prompt and its reply line up; the longest speaker label is
/// `WCODE` (5), plus the 1-col margin and a 1-col gap.
const CONTENT_COL: usize = 7;
/// The tool tree's branch glyph + its trailing space on a tool's head row.
const TREE_BRANCH: &str = "├ ";
/// The tool tree's continuation glyph + the 3-space gap the body/params/hint ride.
const TREE_PIPE: &str = "│   ";
/// The tool tree's terminator glyph — one row, drawn only when the tool has a body.
const TREE_END: &str = "└";
/// Output lines a collapsed tool shows before a `… +N more` hint.
const TOOL_PREVIEW_LINES: usize = 4;
/// Output lines a fully expanded tool shows before the hint returns.
const TOOL_EXPANDED_LINES: usize = 200;
/// Diff lines a collapsed tool shows before the hint.
const TOOL_DIFF_PREVIEW_LINES: usize = 8;

/// The working-team region appears only when the terminal is at least this wide.
const TEAM_MIN_WIDTH: u16 = 50;
/// … and at least this tall, so the transcript, the team region, and the
/// composer band all leave the transcript something to show.
const TEAM_MIN_HEIGHT: u16 = 8;
/// The docked left sidebar's fixed width in columns. Its content is clipped
/// to fit; the bands to its right are NOT reflowed to compensate.
const SIDEBAR_WIDTH: u16 = 30;
/// The sidebar docks only when the WHOLE terminal is at least this wide —
/// below it there is not enough room for a 30-col panel AND a usable bands
/// column, so `draw` leaves the layout completely untouched.
const SIDEBAR_MIN_WIDTH: u16 = 80;
/// Wide, centered in its band (the "printed page"), and **runtime-adjustable**
/// (`App::measure`, set by `/width` and `Alt-[` / `Alt-]`). It governs the
/// transcript only — the composer band stays edge-to-edge.
/// The band width at (or above) which the measure centers, leaving a real margin
/// on each side. Below it the content uses the full band width (so an 80-col
/// terminal keeps its `❯` at the gutter).
const MEASURE_MIN_BAND: usize = 84;
/// The pinned book header band's height in rows — the head row over its `─` rule
/// (D010). A fixed top band; it never scrolls with the transcript.
const HEADER_H: u16 = 2;

/// Draw the full frame. Stateless: everything comes from `app`.
///
/// Bands (top → bottom), in the bands column to the right of the optional
/// sidebar: the transcript (flex), the working-team region (0..=3 rows — a
/// rider, only while a member runs), and the composer band. The composer's head
/// and foot lines carry the chrome the old status band used to. The team region
/// collapses to nothing when no
/// teammate is running.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let full = frame.area();
    app.set_terminal_width(full.width);
    // Start every frame from an empty hit map: a closed sidebar / a resize must
    // never leave stale geometry behind (the app holds no layout constants).
    app.clear_hit_map();
    // PHASE 2 — the docked left SIDEBAR. When Ctrl-B has it open AND the
    // terminal is wide enough, split the WHOLE area horizontally into
    // [sidebar | bands] and render the existing band stack in the right
    // column. When closed (or too narrow) `area` is the full frame untouched,
    // so every geometry below — and every popup/overlay anchor — is
    // byte-identical.
    // D5: a fresh, empty transcript carries a dim `type a message …` hint.
    app.seed_empty_hint();
    // The PINNED book header (D010): a fixed top band, chrome above the bands.
    let head = SessionHead::of(app);
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

    // The working-team region sits ABOVE the composer band: one row per RUNNING
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

    // The pinned header (head row + `─` rule) is a fixed top band [D010]. Below
    // it: at least one transcript row is reserved; the composer band (the rule, the
    // input, and the foot line) then has priority over the team region, so a short
    // terminal degrades gracefully instead of overflowing.
    let header_h = if area.height > HEADER_H + 1 { HEADER_H } else { 0 };
    let spare = area.height.saturating_sub(header_h + 1);
    let box_h = (input_height + 2).min(spare).max(1);
    team_h = team_h.min(spare.saturating_sub(box_h));

    let areas = Layout::vertical([
        Constraint::Length(header_h), // pinned book header (head row + rule)
        Constraint::Min(1),           // transcript (flexes)
        Constraint::Length(team_h),   // working-team region (a rider, 0..=3)
        Constraint::Length(box_h),    // composer band (rule · input · foot)
    ])
    .split(area);
    let header = areas[0];
    let body = areas[1];
    let team = areas[2];
    let editor = areas[3];

    draw_header(frame, header, &head);
    draw_transcript(frame, body, app);
    if team_h > 0 {
        let working = app.working_team_rows();
        draw_working_team(frame, team, &working);
    }
    draw_input_box(frame, editor, app, &view);
    // The completion/search popups anchor on the composer band's TOP row — the
    // full-width rule — so their bottom edge rests on the rule and they float over
    // the transcript interior, never over the composer.
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
/// The popup anchors on a 1-row `above` rect at the composer band's top row
/// (the head line), so it floats over the transcript interior and never covers
/// the composer.
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
        .border_type(BorderType::Rounded)
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
        .border_type(BorderType::Rounded)
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
        .border_type(BorderType::Rounded)
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
        .border_type(BorderType::Rounded)
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
/// plus the in-flight message. `area` is the plain transcript band; the content
/// renders in a centered **measure** (at most `App::measure()` cols, when the band
/// is at least [`MEASURE_MIN_BAND`] wide), so the wrap width is the measure and
/// the viewport height is `area.height`, feeding `sync_scroll`; the selection bar
/// paints the measure's column 0 in a second pass.
///
/// `Surface::cache` keys on width, so a reflow (e.g. toggling the sidebar,
/// which changes the bands width by `SIDEBAR_WIDTH`) invalidates every cached
/// block.
/// The pinned book header (D010): the head row over a full-width `─` rule, drawn
/// as a fixed top band (byte-identical to the old transcript block's two rows,
/// but it never scrolls). Chrome — no affordances, never a browse/click target.
fn draw_header(frame: &mut Frame, area: Rect, head: &SessionHead) {
    if area.height == 0 {
        return;
    }
    let width = area.width as usize;
    frame.render_widget(
        Paragraph::new(session_head_row(head, width)),
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: 1,
        },
    );
    if area.height >= 2 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("─".repeat(width), dim()))),
            Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: 1,
            },
        );
    }
}

fn draw_transcript(frame: &mut Frame, area: Rect, app: &mut App) {
    // The measure: a centered content column when the band leaves a real margin,
    // else the full band width. The composer band is NOT centered.
    let band = area.width as usize;
    // The band's own width caps the measure; below the centering threshold the
    // band is used whole. The measure itself is runtime-adjustable (D011).
    let measure = if band >= MEASURE_MIN_BAND {
        app.measure().min(band)
    } else {
        band
    };
    let pad = (band - measure) / 2;
    let width = measure;
    // The centered measure column: the content renders here, and hit-testing
    // targets this rect (not the full band).
    let col = Rect {
        x: area.x + pad as u16,
        y: area.y,
        width: measure as u16,
        height: area.height,
    };
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
    // it is never a selection target). `append_live_lines` owns the separator and
    // the `(live_rev, width)` cache; the returned range is dropped.
    let _ = app.focused_mut().append_live_lines(width, &mut lines);
    let height = area.height as usize;
    let total = lines.len();
    let grew = total != app.total_lines();
    app.set_block_ranges(ranges.clone());
    // Follow the tail unless the user has scrolled up; the renderer measures
    // the transcript and reconciles the scroll window.
    app.sync_scroll(total, height, measure);
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
    // Third pass: restyle the selected chars with `Modifier::REVERSED`. It only
    // RESTYLES (splits spans, changes no content), so no row's width changes and no
    // row is injected — the same invariant `paint_bar` protects. `start` is the
    // global line of the window's top row (the value the bar pass used).
    if let Some((a, b)) = app.text_sel() {
        for (r, line) in window.iter_mut().enumerate() {
            let g = start + r; // the global line of this row
            if g < a.line || g > b.line {
                continue;
            }
            let lo = if g == a.line { a.col } else { 0 };
            // INCLUSIVE end: highlight through `b.col`, i.e. `[lo, b.col + 1)`.
            let hi = if g == b.line { b.col + 1 } else { usize::MAX };
            recolor_range(line, lo, hi, selection_style());
        }
    }
    // Publish the transcript viewport for hit-testing: `start` is the global line
    // at the band's top row; `rows` the plain text of each visible row.
    let rows_text: Vec<String> = window
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    // D32 — publish the affordance cells. A thinking block's collapsed `▸`/`▣`
    // row is the block's FIRST line; a `Block::Turn` publishes one hit PER INLINE
    // TOOL (each tool's head row is its toggle row). The rects are pure functions
    // of (kind, width, range) — recomputed every frame, never cached (the
    // (rev, width) cache stores `Line`s, not geometry).
    let (_, note_copy) = note_affordance_cols(measure);
    let affordances: Vec<AffordanceHit> = ranges
        .iter()
        .enumerate()
        .filter_map(|(i, range)| {
            let row0 = range.start.checked_sub(start)?;
            match &app.transcript()[i] {
                Block::Turn(turn) => {
                    // A `Turn` is its body (no head, D012): the prose and the tools
                    // interleaved INLINE in call order. Publish the thinking
                    // affordance (on the block's first row) and one hit per inline
                    // tool's head row.
                    let mut hits = Vec::new();
                    if matches!(turn.content.first(), Some(ContentBlock::Thinking { .. })) {
                        let r = row0;
                        if r < window.len() {
                            let y = area.y + r as u16;
                            hits.push(AffordanceHit {
                                block: i,
                                item: None,
                                toggle: Rect::new(col.x, y, measure.saturating_sub(2) as u16, 1),
                                copy: Rect::new(col.x + note_copy as u16, y, 1, 1),
                            });
                        }
                    }
                    // The inline tool offsets MIRROR the render walk exactly (both
                    // come from `turn_inline`), so a hit can never drift.
                    for (k, off) in inline_tool_offsets(turn, measure) {
                        let r = row0 + off;
                        if r < window.len() {
                            let y = area.y + r as u16;
                            hits.push(AffordanceHit {
                                block: i,
                                item: Some(k),
                                // The whole head row through the `▸`; the `▣` cell
                                // stays disjoint (`kind_at` checks toggle first).
                                toggle: Rect::new(col.x, y, measure.saturating_sub(2) as u16, 1),
                                copy: Rect::new(col.x + note_copy as u16, y, 1, 1),
                            });
                        }
                    }
                    Some(hits)
                }
                Block::Tool(tool) => {
                    // A standalone tool (a live/running tool, before its answer
                    // commits) is a single inline panel; with NO `── notes ──` rule
                    // its head row is block-relative line 0.
                    let _ = tool;
                    let r = row0;
                    if r >= window.len() {
                        return None;
                    }
                    let y = area.y + r as u16;
                    Some(vec![AffordanceHit {
                        block: i,
                        item: Some(0),
                        toggle: Rect::new(col.x, y, measure.saturating_sub(2) as u16, 1),
                        copy: Rect::new(col.x + note_copy as u16, y, 1, 1),
                    }])
                }
                _ => None,
            }
        })
        .flatten()
        .collect();
    app.set_transcript_hit(col, start, rows_text, affordances);
    frame.render_widget(Paragraph::new(window), col);
}

/// Restyle the char range `[lo, hi)` of `line` with `style` (the caller passes
/// `selection_style()`, i.e. `Modifier::REVERSED`). Splits spans at `lo`/`hi`
/// exactly as `paint_bar` splits at column 0; NEVER changes `line`'s text, so its
/// glyph width is identical before and after. `lo`/`hi` are char indices, clamped
/// to the row's char count. The selection is INCLUSIVE on both ends, so a caller
/// highlighting through `b.col` passes `hi = b.col + 1`.
fn recolor_range(line: &mut Line<'static>, lo: usize, hi: usize, style: Style) {
    let total: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
    let lo = lo.min(total);
    let hi = hi.min(total);
    if lo >= hi {
        return;
    }
    let mut out: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 2);
    let mut pos = 0;
    for span in line.spans.drain(..) {
        let n = span.content.chars().count();
        let (start, end) = (pos, pos + n);
        pos = end;
        // No overlap with `[lo, hi)`: keep the span whole.
        if end <= lo || start >= hi {
            out.push(span);
            continue;
        }
        let base = span.style;
        let chars: Vec<char> = span.content.chars().collect();
        let a = lo.saturating_sub(start).min(n);
        let b = (hi - start).min(n);
        if a > 0 {
            out.push(Span::styled(chars[..a].iter().collect::<String>(), base));
        }
        out.push(Span::styled(chars[a..b].iter().collect::<String>(), base.patch(style)));
        if b < n {
            out.push(Span::styled(chars[b..].iter().collect::<String>(), base));
        }
    }
    line.spans = out;
}

/// The selection highlight style: `Modifier::REVERSED` over the default style.
fn selection_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
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
        // The transcript's user block: a `YOU` speaker head + the wrapped prompt
        // (the `❯` glyph stays the composer's, not the transcript's).
        Block::User(text) => speaker_lines(text, width),
        // One exchange: the `WCODE` head, the run's prose (with `¹` marks), and
        // the ledger (its foot).
        Block::Turn(turn) => turn_lines(turn, width),
        // A standalone tool (a live/running tool before its answer commits, or an
        // orphan with no turn): a single inline tool panel (no `── notes ──` rule).
        Block::Tool(tool) => tool_inline_lines(1, tool, width),
        Block::Notice(text) => wrap(text, width, "   ", "   ", dim()),
        Block::Btw(text) => wrap(text, width, " btw ", "     ", thinking()),
        Block::Error(text) => wrap(text, width, "   ", "   ", error_style()),
        Block::Diff { path, diff } => diff_block_lines(path, diff),
        Block::Todos(todos) => todos_lines(todos, width),
    }
}

/// The user block's head row: a reverse-video ` YOU` run in the gutter, padded so
/// the prompt lands at [`CONTENT_COL`], then the first wrapped line of `text`. The
/// assistant carries **no** head — it is the page itself (D012).
fn user_head(text: &str) -> Line<'static> {
    let filled = 1 + "YOU".chars().count(); // the leading space + the name
    let pad = CONTENT_COL.saturating_sub(filled);
    Line::from(vec![
        Span::styled(" YOU".to_string(), selection_style()),
        Span::styled(" ".repeat(pad), dim()),
        Span::styled(text.to_string(), user()),
    ])
}

/// The user block's continuation gutter, aligning the prompt under [`CONTENT_COL`].
fn user_cont(text: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(" ".repeat(CONTENT_COL), dim()),
        Span::styled(text.to_string(), user()),
    ])
}

/// The user block: line 0 is the ` YOU` head + the first wrapped line, the rest
/// the continuation gutter. The prompt reads in the accent role (D011).
fn speaker_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let mut first_line = true;
    for raw in text.split('\n') {
        let avail = width.saturating_sub(CONTENT_COL).max(1);
        let segments = greedy_wrap(raw, avail);
        let mut iter = segments.into_iter();
        let head = iter.next().unwrap_or_default();
        if first_line {
            out.push(user_head(&head));
            first_line = false;
        } else {
            out.push(user_cont(&head));
        }
        for segment in iter {
            out.push(user_cont(&segment));
        }
    }
    out
}

/// Row 0 of the pinned book header (D010): `padded_row(left, right, "─", dim(),
/// width)`, where `left` = `[ WCODE reversed ] · session <short_id>` and `right`
/// = `"project · ⎇ branch   model · effort"` — the **3-space gap sits INSIDE the
/// right run** (D009 §3 option A), and `padded_row` supplies the ONE `─` fill
/// between `left` and `right`. The ladder (fed to `pick_titles`): full → drop
/// `session` → drop the branch → drop `effort` → truncate with `…`. The raw
/// `head.session` is shortened via `short_id`.
fn session_head_row(head: &SessionHead, width: usize) -> Line<'static> {
    let session = head
        .session
        .as_deref()
        .map(|id| format!(" · session {}", short_id(id)));
    let branch = head.branch.as_deref().map(|b| format!("⎇ {b}"));
    let effort = head.effort.as_deref();

    let left = |with_session: bool| -> Vec<Span<'static>> {
        let mut spans = vec![Span::styled("WCODE", selection_style())];
        if let (true, Some(s)) = (with_session, &session) {
            spans.push(Span::styled(s.clone(), dim()));
        }
        spans
    };
    let right = |with_branch: bool, with_effort: bool| -> Vec<Span<'static>> {
        // `project · ⎇ branch` — the project is `muted`, the joined rest `dim`.
        let project_branch = if with_branch {
            join_title(&head.project, branch.as_deref())
        } else {
            head.project.clone()
        };
        let (head_part, rest) = split_at_char(&project_branch, head.project.chars().count());
        let mut spans = vec![Span::styled(head_part, muted())];
        if !rest.is_empty() {
            spans.push(Span::styled(rest, dim()));
        }
        // The 3-space gap before the model group, INSIDE the right run.
        spans.push(Span::styled("   ".to_string(), dim()));
        spans.push(Span::styled(head.model.clone(), dim()));
        if let (true, Some(e)) = (with_effort, &effort) {
            spans.push(Span::styled(format!(" · {e}"), dim()));
        }
        spans
    };

    let levels = vec![
        (left(true), right(true, true)),
        (left(false), right(true, true)),
        (left(false), right(false, true)),
        (left(false), right(false, false)),
    ];
    // A 1-col gap so the `─` fill always has room; `padded_row` fills it.
    let (l, r) = pick_titles(&levels, width.saturating_sub(1));
    padded_row(&l, &r, "─", dim(), width as u16)
}

/// One exchange (`Block::Turn`): the run's body (prose + tools, inline). There is
/// no `WCODE` head — the assistant is the page itself (D012).
fn turn_lines(turn: &Turn, width: usize) -> Vec<Line<'static>> {
    turn_inline_lines(turn, width)
}

/// The turn's body: the prose/thinking interleaved with each tool's panel, in
/// call order. # Contracts: exactly one panel per tool (a `ToolCall` whose result
/// is not yet in `turn.tools` emits the mark but no panel); the `¹` is SUPPRESSED
/// when no prose landed since the previous tool (a text-less round, D009 §4).
fn turn_inline_lines(turn: &Turn, width: usize) -> Vec<Line<'static>> {
    turn_inline(turn, width).0
}

/// The turn-relative row offset (from the turn's first body line) of each inline tool's
/// head row, in call order — the D32 mirror of `turn_inline_lines`. # Contracts:
/// one `(k, off)` per tool ACTUALLY emitted (a `ToolCall` with no result is
/// skipped); `off` accounts for every prose/thinking/tool row above it. A drift
/// from the render misaligns EVERY hit below it.
fn inline_tool_offsets(turn: &Turn, width: usize) -> Vec<(usize, usize)> {
    turn_inline(turn, width).1
}

/// The single walk behind [`turn_inline_lines`] and [`inline_tool_offsets`], so
/// the render and the D32 hit map can never drift: it returns the turn's body
/// lines and the turn-relative row offset of each emitted tool's head row.
fn turn_inline(turn: &Turn, width: usize) -> (Vec<Line<'static>>, Vec<(usize, usize)>) {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut offsets: Vec<(usize, usize)> = Vec::new();
    // The body starts at line 0 (there is no `WCODE` head).
    let mut off = 0usize;
    // The `¹` ordinal: the count of `ToolCall`s seen (one per-turn series).
    let mut refn = 0usize;
    // Whether the line just emitted is prose (a `¹` may attach) or a tool row —
    // the structural "suppressed on a text-less round" rule (D009 §4).
    let mut tail_is_prose = false;
    for (i, block) in turn.content.iter().enumerate() {
        match block {
            ContentBlock::Text { text } => {
                let lines = markdown::render(text, width);
                tail_is_prose |= !lines.is_empty();
                off += lines.len();
                out.extend(lines);
            }
            ContentBlock::Thinking { text } => {
                // The affordance belongs to the leading `Thinking` only.
                let lines = thinking_block_lines(text, width, turn.thinking_open, i == 0);
                tail_is_prose = true;
                off += lines.len();
                out.extend(lines);
            }
            ContentBlock::ToolCall { .. } => {
                refn += 1;
                if tail_is_prose && let Some(last) = out.last_mut() {
                    last.spans.push(Span::styled(footnote_mark(refn), link()));
                }
                // Mid-run: the assistant message commits BEFORE
                // `ToolExecutionStart`, so the result may not be in `tools` yet —
                // emit the mark, but no panel.
                if let Some(tool) = turn.tools.get(refn - 1) {
                    offsets.push((refn - 1, off));
                    let lines = tool_inline_lines(refn, tool, width);
                    off += lines.len();
                    out.extend(lines);
                }
                tail_is_prose = false;
            }
        }
    }
    (out, offsets)
}

/// The tool tree's base indent (D010): three columns, the same marker column the
/// thinking row's `···` sits in, so every block's lead glyph lines up.
fn tree_indent() -> String {
    "   ".to_string()
}

/// The prefix a tool's body/params/hint rows ride: the tree indent + `│   `.
fn tree_body_prefix() -> String {
    format!("{}{TREE_PIPE}", tree_indent())
}

/// The tool tree's terminator row: the indent + `└`, drawn when a tool has a body.
fn tree_end_row() -> Line<'static> {
    Line::from(Span::styled(format!("{}{TREE_END}", tree_indent()), dim()))
}

/// One tool's inline tree (D010/T1): a single head row — `├ name  target` with
/// the `note · ms` (or `+a −r · ms`) stats and the `▸`/`▣` affordances
/// right-aligned — then the body rows under `│   ` (params + output/diff/preview
/// plus the `… +N more` hint), closed by a `└` terminator when there is a body.
///
/// The name prints ONCE; there is no `N` ordinal and no separate `✓ name · note`
/// row. `n` is the tool's 1-based call order (kept for the render walk); row 0 is
/// the D32 toggle row, and `Block::Tool` also uses it with `n = 1`.
fn tool_inline_lines(_n: usize, tool: &Tool, width: usize) -> Vec<Line<'static>> {
    let expanded = tool.expanded || tool.is_error;
    let mut body: Vec<Line<'static>> = Vec::new();
    if !tool.done || expanded {
        body.extend(panel_param_lines(tool, width));
    }
    body.extend(tool_panel_body(tool, width, expanded));
    let mut out = vec![tool_head_row(tool, width)];
    if !body.is_empty() {
        out.extend(body);
        out.push(tree_end_row());
    }
    out
}

/// Render the live (streaming) assistant message for `width` — the same body
/// `draw_transcript` used inline, with the streaming cursor (`live = true`).
/// Empty for a non-assistant message.
pub(crate) fn live_lines(message: &AgentMessage, width: usize) -> Vec<Line<'static>> {
    match message {
        AgentMessage::Assistant { content, .. } => content_lines(content, width, true, false),
        _ => Vec::new(),
    }
}

/// Render an assistant message's blocks in order, dropping tool calls (their
/// own line carries them). `live` renders thinking inline (in-flight) and
/// appends a cursor to the last line; a committed block draws thinking as the
/// collapsed row unless `thinking_open` (D33).
fn content_lines(
    content: &[ContentBlock],
    width: usize,
    live: bool,
    thinking_open: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // The `¹` reference ordinal, PER MESSAGE (reset each `content_lines` call).
    // The foot's note number is per TURN, so the two coincide whenever the
    // non-final rounds carry no prose (the usual `A(tc) → T → … → A(answer)`
    // shape, where the text-less rounds emit no reference at all).
    let mut refn = 0usize;
    for (i, block) in content.iter().enumerate() {
        match block {
            ContentBlock::Text { text } => {
                lines.extend(markdown::render(text, width));
            }
            ContentBlock::Thinking { text } => {
                if live {
                    // In-flight: stream inline, expanded — as before D33.
                    lines.extend(wrap(text, width, THINK_FIRST, THINK_CONT, thinking()));
                } else {
                    // The affordance belongs to the block's FIRST line only (the
                    // publish requires `content.first()` to be `Thinking`), so
                    // only the leading thinking row draws `▸`/`▾`/`▣`.
                    lines.extend(thinking_block_lines(text, width, thinking_open, i == 0));
                }
            }
            ContentBlock::ToolCall { .. } => {
                // A printed superscript reference to the turn's foot (the note
                // list). SUPPRESSED when the block has emitted no prose yet: a
                // text-less tool-call round has no sentence to mark, and a
                // replayed/orphan tool is still numbered in the foot, unmarked.
                refn += 1;
                if let Some(last) = lines.last_mut() {
                    last.spans.push(Span::styled(footnote_mark(refn), link()));
                }
            }
        }
    }
    if live {
        match lines.last_mut() {
            Some(last) => last.spans.push(Span::styled("▌", accent())),
            None => lines.push(Line::from(Span::styled(
                format!("{}▌", " ".repeat(CONTENT_COL)),
                accent(),
            ))),
        }
    }
    lines
}

/// The superscript footnote mark for the n-th note of a turn: `¹²³…` for 1..=9,
/// the ASCII-clean `[n]` for n >= 10.
fn footnote_mark(n: usize) -> String {
    const MARKS: [char; 9] = ['¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    match n.checked_sub(1).and_then(|i| MARKS.get(i)) {
        Some(c) => c.to_string(),
        None => format!("[{n}]"),
    }
}

/// One tool's head row (D010/T1): `├ name  target`, the `note · ms` stats and
/// the `▸`/`▣` affordances right-aligned at the D32 columns — the name prints
/// ONCE and this row is the tool's toggle/copy row. The target is inlined on the
/// head only when the params block is empty (otherwise it rides the params rows).
fn tool_head_row(tool: &Tool, width: usize) -> Line<'static> {
    let glyph = if tool.expanded || tool.is_error {
        "▾"
    } else {
        "▸"
    };
    let stats = tool_stats(tool);
    // An errored tool's head carries the `✗` mark (red) in place of the tree
    // branch, so a failure reads at a glance; a success stays quiet.
    let (mark, mark_style, name_style) = if tool.is_error {
        ("✗ ", error_style(), error_style())
    } else {
        (TREE_BRANCH, dim(), tool_name())
    };
    let prefix = format!("{}{mark}", tree_indent());
    let label = match tool.target.as_ref().or(tool.path.as_ref()) {
        Some(target) if tool.params.is_empty() => format!("{}  {target}", tool.name),
        _ => tool.name.clone(),
    };
    let aff = format!("{glyph} ▣");
    // The fill sits between the left run and the right run (`stats   ▸ ▣`), so
    // the `▸`/`▣` stay pinned at their published D32 columns.
    let left_len = prefix.chars().count() + label.chars().count();
    let tail_len = if stats.is_empty() {
        aff.chars().count()
    } else {
        stats.chars().count() + 3 + aff.chars().count()
    };
    let pad = width.saturating_sub(left_len + tail_len).max(1);
    let mut spans = vec![
        Span::styled(prefix, mark_style),
        Span::styled(label, name_style),
        Span::styled(" ".repeat(pad), dim()),
    ];
    if !stats.is_empty() {
        spans.push(Span::styled(stats, dim()));
        spans.push(Span::styled("   ", dim()));
    }
    spans.push(Span::styled(aff, dim()));
    Line::from(spans)
}

/// A tool's head-row stats: the summary line (or the diff's `+a −r`) joined with
/// the wall-clock ms by ` · `. Empty while the tool is still running (no note, no
/// timing yet), so the head row carries no stats.
fn tool_stats(tool: &Tool) -> String {
    if !tool.done {
        return String::new();
    }
    let note = match &tool.diff {
        Some(diff) => {
            let (added, removed) = crate::app::diff_counts(diff);
            format!("+{added} −{removed}")
        }
        None => summary_line(&tool.output).0,
    };
    let mut parts: Vec<String> = Vec::new();
    if !note.is_empty() {
        parts.push(note);
    }
    if let Some(ms) = tool.duration_ms {
        parts.push(format_ms(ms));
    }
    parts.join(" · ")
}

/// The note head's affordance cells (0-based from the row's left edge): the
/// `▸`/`▾` toggle glyph at `width - 3` and the `▣` copy at `width - 1` (the
/// `▸ ▣` run flush right).
fn note_affordance_cols(width: usize) -> (usize, usize) {
    (width.saturating_sub(3), width.saturating_sub(1))
}

/// A committed thinking block (D33): the collapsed `··· thinking ▸ ▣` row,
/// plus the `thinking`-styled body when expanded. No panel frame —
/// thinking is lighter than a tool, so a bare affordance row keeps it cheap.
fn thinking_block_lines(
    text: &str,
    width: usize,
    open: bool,
    affordance: bool,
) -> Vec<Line<'static>> {
    let mut out = vec![thinking_header_line(width, open, affordance)];
    if open {
        out.extend(wrap(text, width, THINK_CONT, THINK_CONT, thinking()));
    }
    out
}

/// The thinking block's header row: `··· thinking` (dim) with the
/// `▸`/`▾`/`▣` affordance run right-aligned at the SAME columns a note head uses
/// (`note_affordance_cols`), so one publish geometry serves both.
/// Without a published region the glyphs are SUPPRESSED — a drawn affordance
/// must always route a click (the D32 invariant).
fn thinking_header_line(
    width: usize,
    open: bool,
    affordance: bool,
) -> Line<'static> {
    let head = "thinking";
    if !affordance {
        return Line::from(vec![
            Span::styled(THINK_FIRST.to_string(), dim()),
            Span::styled(head, dim()),
        ]);
    }
    let (toggle_col, _) = note_affordance_cols(width);
    let used = THINK_FIRST.chars().count() + head.chars().count();
    // The `▸` sits at the SAME screen column as a note's (`note_affordance_cols`
    // is row-local and absolute; THINK_FIRST already carries the indent).
    let fill = toggle_col.saturating_sub(used);
    let glyph = if open { "▾" } else { "▸" };
    Line::from(vec![
        Span::styled(THINK_FIRST.to_string(), dim()),
        Span::styled(head, dim()),
        Span::styled(" ".repeat(fill), dim()),
        Span::styled(glyph.to_string(), dim()),
        Span::styled(" ".to_string(), dim()),
        Span::styled("▣".to_string(), dim()),
    ])
}

/// The fence info for a param VALUE — the file extension for a file tool, `sh`
/// for a `bash` command, else `None` (the value renders in `body`).
fn param_syntax(tool: &Tool, key: &str) -> Option<String> {
    match tool.name.as_str() {
        "bash" if key == "command" => Some("sh".to_string()),
        "read" | "write" | "replace" | "edit" | "edits" => {
            code_extension(tool.path.as_deref().or(tool.target.as_deref()))
        }
        _ => None,
    }
}

/// One `key  value` row: the key in `dim()` padded to `key_col`, the value in
/// body strength (or syntax-highlighted when `info` resolves), continuations
/// aligned under the value column. Returns ≥1 line.
fn param_row(
    key: &str,
    value: &str,
    key_col: usize,
    width: usize,
    info: Option<&str>,
) -> Vec<Line<'static>> {
    let prefix = tree_body_prefix();
    let indent = prefix.chars().count();
    let value_col = indent + key_col + 2;
    let avail = width.saturating_sub(value_col).max(1);
    let lead0 = {
        let pad = value_col.saturating_sub(indent + key.chars().count());
        format!("{prefix}{key}{}", " ".repeat(pad))
    };
    let cont = " ".repeat(value_col);
    let mut hl = info.map(|i| markdown::CodeHighlight::new(i, theme::color_mode()));
    let rows = match hl.as_mut() {
        Some(h) if h.is_active() => wrap_styled(&h.spans(value), avail),
        _ => wrap_styled(&[Span::styled(value.to_string(), theme::theme().body)], avail),
    };
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let lead = if i == 0 { lead0.clone() } else { cont.clone() };
            let mut spans = vec![Span::styled(lead, dim())];
            spans.extend(row.iter().cloned());
            Line::from(spans)
        })
        .collect()
}

/// The params block: one [`param_row`] per `(key, value)`, keys aligned to the
/// longest; the value is syntax-highlighted when its key names code. Empty
/// `params` → no rows.
fn panel_param_lines(tool: &Tool, width: usize) -> Vec<Line<'static>> {
    if tool.params.is_empty() {
        return Vec::new();
    }
    let key_col = tool
        .params
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap_or(0);
    tool.params
        .iter()
        .flat_map(|(key, value)| {
            let info = param_syntax(tool, key);
            param_row(key, value, key_col, width, info.as_deref())
        })
        .collect()
}

/// The tool's body rows under the tree's `│`: the params block precedes it, and
/// the `note · ms` (or `+a −r · ms`) stats ride the head row ([`tool_stats`]). No
/// `── notes ──` rule, no `✓` summary row.
fn tool_panel_body(tool: &Tool, width: usize, expanded: bool) -> Vec<Line<'static>> {
    let info_owned = tool_body_syntax(tool);
    let info = info_owned.as_deref();
    let mut out = Vec::new();

    // 1. LIVE — the growing tail; no stats yet.
    if !tool.done {
        if expanded {
            let all: Vec<&str> = tool.output.lines().collect();
            out.extend(tool_body(&all, width, TOOL_EXPANDED_LINES, info, true));
            if let Some(hint) = more_hint(all.len().saturating_sub(TOOL_EXPANDED_LINES), false) {
                out.push(hint);
            }
        } else if let Some(tail) = last_line(&tool.output) {
            out.push(prefixed(&tree_body_prefix(), &tail, dim()));
        }
        return out;
    }

    // 2. DIFF — the change's body (the `+a −r` stats ride the head row).
    if let Some(diff) = &tool.diff {
        let all: Vec<&str> = diff.lines().collect();
        let limit = if expanded {
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
            out.push(prefixed(&tree_body_prefix(), raw, style));
        }
        if let Some(hint) = more_hint(all.len().saturating_sub(take), false) {
            out.push(hint);
        }
        return out;
    }

    // 3. DONE — the full body (expanded) or a dim preview + `… +N more` (collapsed).
    let (_, wide) = summary_line(&tool.output);
    if expanded {
        let all: Vec<&str> = tool.output.lines().collect();
        out.extend(tool_body(&all, width, all.len(), info, false));
    } else {
        let body = body_after_summary(&tool.output);
        out.extend(tool_body(&body, width, TOOL_PREVIEW_LINES, info, false));
        let more = body.len().saturating_sub(TOOL_PREVIEW_LINES);
        if let Some(hint) = more_hint(more, wide) {
            out.push(hint);
        }
    }
    out
}

/// A committed todos block (D35): a `── todos  d/t ──` header, then one `☑`/`☐`
/// row per item. Completed text is `muted`, everything else `body`; the glyphs and
/// header are `dim`. Rendered live in the transcript (the `Todo` feed updates it).
fn todos_lines(todos: &[TodoItem], width: usize) -> Vec<Line<'static>> {
    let done = todos
        .iter()
        .filter(|t| t.status == TodoStatus::Completed)
        .count();
    let head = format!("   ── todos  {done}/{} ", todos.len());
    let fill = width.saturating_sub(head.chars().count());
    let mut out = vec![Line::from(vec![
        Span::styled(head, dim()),
        Span::styled("─".repeat(fill), dim()),
    ])];
    for item in todos {
        let (mark, text_style) = match item.status {
            TodoStatus::Completed => ("☑", muted()),
            _ => ("☐", theme::theme().body),
        };
        out.push(Line::from(vec![
            Span::styled(format!("   {mark} "), dim()),
            Span::styled(item.content.clone(), text_style),
        ]));
    }
    out
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

/// The fence info string for a tool's body — the file extension for a file tool
/// (`read`/`write`/`replace`/`edit`), so its content colorizes; `None` otherwise.
fn tool_body_syntax(tool: &Tool) -> Option<String> {
    match tool.name.as_str() {
        "read" | "write" | "replace" | "edit" | "edits" => {
            code_extension(tool.path.as_deref().or(tool.target.as_deref()))
        }
        _ => None,
    }
}

/// The extension of a path's file name (`a/b/foo.rs` → `rs`), if any.
fn code_extension(path: Option<&str>) -> Option<String> {
    let ext = path?.rsplit('/').next()?.rsplit_once('.')?.1;
    (!ext.is_empty()).then(|| ext.to_string())
}

/// Wrap a styled line into rows of at most `width` columns, coalescing runs of one
/// style and hard-breaking an over-long token (char-exact; spaces kept).
fn wrap_styled(spans: &[Span<'static>], width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let chars: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|s| s.content.chars().map(move |c| (c, s.style)))
        .collect();
    if chars.is_empty() {
        return vec![Vec::new()];
    }
    let mut rows = Vec::new();
    for chunk in chars.chunks(width) {
        let mut row: Vec<Span<'static>> = Vec::new();
        for (c, style) in chunk {
            match row.last_mut() {
                Some(last) if last.style == *style => last.content.to_mut().push(*c),
                _ => row.push(Span::styled(c.to_string(), *style)),
            }
        }
        rows.push(row);
    }
    rows
}

/// The tool body: `lines` sliced to `limit` (the head, or the tail when `tail`)
/// and wrapped char-exact under the gutter. The caller adds any hint.
fn tool_body(
    lines: &[&str],
    width: usize,
    limit: usize,
    info: Option<&str>,
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
    body_rows(&lines[start..end], width, info)
}

/// Wrap a tool body to `width`, one physical row per wrapped segment, char-exact
/// (`wrap_input`: spaces kept, an over-long token hard-broken) so a long line is
/// reachable rather than clipped at the pane edge.
fn body_rows(lines: &[&str], width: usize, info: Option<&str>) -> Vec<Line<'static>> {
    if lines.is_empty() {
        return Vec::new();
    }
    let prefix = tree_body_prefix();
    let avail = width.saturating_sub(prefix.chars().count()).max(1);
    let mut hl = info.map(|i| markdown::CodeHighlight::new(i, theme::color_mode()));
    let mut out = Vec::new();
    for line in lines {
        let spans = match hl.as_mut() {
            Some(h) if h.is_active() => h.spans(line),
            _ => vec![Span::styled((*line).to_string(), dim())],
        };
        for row in wrap_styled(&spans, avail) {
            let mut spans = vec![Span::styled(prefix.clone(), dim())];
            spans.extend(row);
            out.push(Line::from(spans));
        }
    }
    out
}

/// The dim hint under an elided tool body: `… +N more line(s)` when the preview
/// cuts whole lines (singular for one), or `… the full line is elided` when only
/// the summary is truncated (a single long line, empty body). `None` when
/// expansion would reveal nothing more. Informational only — the per-block
/// toggle is browse mode's `Enter`.
fn more_hint(more_lines: usize, wide: bool) -> Option<Line<'static>> {
    let prefix = tree_body_prefix();
    let text = if more_lines > 0 {
        let line = if more_lines == 1 { "line" } else { "lines" };
        format!("{prefix}… +{more_lines} more {line}")
    } else if wide {
        format!("{prefix}… the full line is elided")
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

/// Draw the composer: a **running-head block** (a head line over a full-width
/// `─` rule) above an **open writing line** (the input) and a **foot line**.
/// There is no rounded box and no corners — the chrome the box's four corners
/// used to carry now rides four plain rows (head: project/branch/session left,
/// model/effort right; foot: the gauge left, plan/browse/state/scroll right).
///
/// The input is drawn at the band's **full width**, so the composer wraps at
/// `area.width - 3` (the `❯ ` gutter), not the box's narrower inner width.
///
/// On a band too short to seat all three rows the **rule drops first** (it is
/// pure decoration), then the foot line — so a 1-row input always keeps its
/// writing line. The composer's **head row is dropped** (D009): the chrome it
/// carried now rides the transcript's session-head block.
fn draw_input_box(frame: &mut Frame, area: Rect, app: &App, view: &InputView) {
    let (bl, br) = corner_titles(app, area);
    let w = area.width;
    let h = area.height;
    let mut row = area.y;
    // The full-width rule — only when the band seats rule · input · foot.
    if h >= 3 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("─".repeat(w as usize), dim()))),
            Rect {
                x: area.x,
                y: row,
                width: w,
                height: 1,
            },
        );
        row += 1;
    }
    // Foot line — pinned to the last row when it leaves the input a row.
    let foot = (h >= 2).then(|| area.y + h - 1);
    // The writing line fills the gap between the head/rule and the foot line.
    let input_end = foot.unwrap_or(area.y + h);
    if input_end > row {
        draw_input(
            frame,
            Rect {
                x: area.x,
                y: row,
                width: w,
                height: input_end - row,
            },
            view,
        );
    }
    if let Some(y) = foot {
        frame.render_widget(
            Paragraph::new(padded_row(&bl, &br, "─", dim(), w)),
            Rect {
                x: area.x,
                y,
                width: w,
                height: 1,
            },
        );
    }
}

/// One full-width composer row: `left` at column 0 and `right` right-aligned to
/// the last column, the gap between filled with `fill` in `style`. A head line
/// fills with spaces (a running head has no rule); a foot line fills with `─`
/// (the gauge-to-state leader). [`corner_titles`] has already budgeted the pair
/// to `width - 1`, so the gap is ≥ 1; a wider pair is clipped by the `Paragraph`.
fn padded_row(
    left: &Line<'static>,
    right: &Line<'static>,
    fill: &str,
    style: Style,
    width: u16,
) -> Line<'static> {
    let width = width as usize;
    let lw: usize = left.spans.iter().map(|s| s.content.chars().count()).sum();
    let rw: usize = right.spans.iter().map(|s| s.content.chars().count()).sum();
    let mut spans = left.spans.clone();
    let gap = width.saturating_sub(lw + rw);
    if gap > 0 {
        // One space of breathing room on each side of the fill, so the leader
        // never abuts the left or right run (`session <id> ─── … wcode`).
        spans.push(Span::styled(" ".to_string(), style));
        if gap > 2 {
            spans.push(Span::styled(fill.repeat(gap - 2), style));
        }
        if gap > 1 {
            spans.push(Span::styled(" ".to_string(), style));
        }
    }
    spans.extend(right.spans.iter().cloned());
    Line::from(spans)
}

/// Assemble the composer's foot-line pair (left: the context gauge; right:
/// `[⏻ plan] · [▤ browse] · {state} · [↑ N] <N>/<M>`), width-budgeted to `area` so
/// the two titles never overdraw each other. The composer's **head-row pair is
/// gone** (D009) — the chrome it carried now rides the transcript's session-head
/// block.
///
/// Each title shares the row: `area.width` minus a 1-col gap so they never touch
/// (there are no border columns — the composer is frameless). Fields are dropped
/// least-important-first (the `↑ N` scroll, then the gauge, then
/// `⏻ plan`/`▤ browse`), and a still-too-long title is truncated with a trailing
/// `…`. The surviving minimum is the mode/state (foot-right).
fn corner_titles(app: &App, area: Rect) -> (Line<'static>, Line<'static>) {
    // Title columns available (a 1-col gap between the two titles; the composer
    // is frameless, so there are no border columns to reserve).
    let avail = (area.width as usize).saturating_sub(1);

    // --- foot row: the gauge   ·   `⏻ plan · ▤ browse · {state} · ↑ N` --------
    let (state, state_style) = match (app.running(), app.run_elapsed()) {
        (true, Some(d)) => (
            format!("⠹ running {}", format_ms(d.as_millis() as u64)),
            accent(),
        ),
        (true, None) => ("⠹ running".to_string(), accent()),
        // A pending `/btw` is the other in-flight state — accent, like running.
        (false, _) if app.asking() => ("⠹ btw…".to_string(), accent()),
        (false, _) => ("⏸ idle".to_string(), dim()),
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
        spans.push(Span::styled(state.clone(), state_style));
        if show_scroll && app.scroll() > 0 {
            spans.push(sep());
            spans.push(Span::styled(format!("↑ {}", app.scroll()), dim()));
        }
        // The folio: `<N>/<M>` = the surface's turn count (N == M — the same
        // series as the turn order); a first turn reads `1/1`. Space-joined (the
        // design's folio convention), not a second `·`, so the foot line keeps
        // ≤1 `·` per metadata group.
        if app.turns() > 0 {
            spans.push(Span::styled(" ".to_string(), dim()));
            spans.push(Span::styled(format!("{n}/{n}", n = app.turns()), dim()));
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
    pick_titles(&bottom_levels, avail)
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

/// The user's own prompt — the accent role, so a prompt stands apart from a reply.
fn user() -> Style {
    theme::theme().user
}

/// A quieter grey than [`dim`] where color is available.
fn muted() -> Style {
    theme::theme().muted
}

/// A markdown link — the turn's `¹` footnote reference shares it.
fn link() -> Style {
    theme::theme().link
}

/// The overlay / popup border and its title.
fn border() -> Style {
    theme::theme().border
}

/// A thinking block.
fn thinking() -> Style {
    theme::theme().thinking
}

/// A tool's name in its `»` / `✓` header.
fn tool_name() -> Style {
    theme::theme().tool_name
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

/// The D36 changes tree as sidebar rows: a `Dir` header (dim), then its `File`
/// leaves as `  ├─ {name}` with the `+a −r` stats RIGHT-aligned. The name is
/// clipped first, so the stats always survive a narrow sidebar.
fn change_tree_lines(rows: &[ChangeRow], width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for row in rows {
        match row {
            ChangeRow::Dir(dir) => {
                out.push(clipped_row(vec![(format!("  {dir}"), dim())], width));
            }
            ChangeRow::File {
                branch,
                name,
                added,
                removed,
                path: _,
            } => {
                let prefix = format!("  {branch} ");
                let plus = format!("+{added}");
                let minus = format!("−{removed}");
                let stats_w = plus.chars().count() + 1 + minus.chars().count();
                let budget = width.saturating_sub(prefix.chars().count() + stats_w + 1);
                let (name, more) = split_at_char(name, budget);
                let name = if more.is_empty() {
                    name
                } else {
                    format!("{name}…")
                };
                let left = format!("{prefix}{name}");
                let pad = width.saturating_sub(left.chars().count() + stats_w);
                out.push(clipped_row(
                    vec![
                        (left, theme::theme().body),
                        (" ".repeat(pad), dim()),
                        (plus, added_style()),
                        (" ".to_string(), dim()),
                        (minus, removed_style()),
                    ],
                    width,
                ));
            }
        }
    }
    out
}

/// Draw the docked left sidebar: one fixed-width column stacking the “more
/// info” the bands cannot all show at once — **agents**, **changes**.
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
/// - agents  → [`App::member_rows`] (`label`, [`crate::TeamState`], focused,
///   live action); glyph `state.glyph()` styled by `state_style(state)`.
///   `member_rows` carries no per-member elapsed, so the elapsed is not
///   reachable; only the focused surface's [`App::run_elapsed`] rides its row.
/// - changes → [`App::change_tree`] (a directory tree, D36).
fn draw_sidebar(frame: &mut Frame, area: Rect, app: &mut App) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    // ---- agents ---------------------------------------------------------
    lines.push(section_rule("agents", None, w));
    // Publish each drawn member row for hit-testing. `member_rows_indexed`
    // borrows `app` immutably; that borrow ends before `set_sidebar_hit` needs
    // `&mut app`, so iterate the owned rows BY VALUE and collect `(index, y)`.
    let indexed = app.member_rows_indexed();
    let mut members: Vec<(usize, u16)> = Vec::new();
    if indexed.is_empty() {
        lines.push(Line::from(Span::styled("  —", dim())));
    }
    let bottom = area.y.saturating_add(area.height);
    // The "agents" header owns area.y.
    for (n, (idx, label, state, focused, action)) in indexed.into_iter().enumerate() {
        let y = area.y + 1 + n as u16;
        if y < bottom {
            members.push((idx, y));
        }
        // `  1 ● explorer *  read a.rs` — the number badge (muted, so `Alt-N` is
        // visible), the glyph colored by state, the live action dim, `*` marks the
        // focused surface. A per-member elapsed is not exposed, so the focused row
        // carries the run elapsed instead.
        let number = format!("  {}", n + 1);
        let mut head = format!(
            " {} {}{}",
            state.glyph(),
            label,
            if focused { " *" } else { "" }
        );
        if focused && let Some(elapsed) = app.run_elapsed() {
            head.push_str(&format!("  {}", format_ms(elapsed.as_millis() as u64)));
        }
        let tail = action.map(|a| format!("  {a}")).unwrap_or_default();
        lines.push(clipped_row(
            vec![(number, muted()), (head, state_style(state)), (tail, dim())],
            w,
        ));
    }

    app.set_sidebar_hit(area, members);

    // ---- changes --------------------------------------------------------
    // A blank line sets the changes section off from the roster (an outline).
    lines.push(Line::from(""));
    let tree = app.change_tree();
    if tree.is_empty() {
        lines.push(section_rule("changes", None, w));
        lines.push(Line::from(Span::styled("  —", dim())));
    } else {
        let (files, added, removed) = change_totals(&tree);
        lines.push(section_rule(&changes_header(files, added, removed), None, w));
        lines.extend(change_tree_lines(&tree, w));
    }

    // Borderless by default so a 30-col panel is 30 cols of content; the
    // bands' own left padding separates the two columns. `Paragraph` clips
    // the row list to `area` — a short panel just loses the bottom sections.
    frame.render_widget(Paragraph::new(lines), area);
}

/// A sidebar section header in the book-outline style: `── label  right ────…`,
/// the `─` fill reaching the panel's edge (D011). An empty `right` is dropped.
fn section_rule(label: &str, right: Option<&str>, width: usize) -> Line<'static> {
    let mut head = format!("── {label}");
    if let Some(r) = right.filter(|r| !r.is_empty()) {
        head.push_str(&format!("  {r}"));
    }
    head.push(' ');
    let used = head.chars().count();
    Line::from(vec![
        Span::styled(head, dim()),
        Span::styled("─".repeat(width.saturating_sub(used)), dim()),
    ])
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
/// The working-team region ABOVE the composer band: one row per RUNNING teammate,
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
    use crate::app::{Action, App, AppEvent, Key, MouseEvent, MouseKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use wcode_harness::protocol::SessionId;

/// A committed `Turn` block for render tests.
fn a_turn(content: Vec<ContentBlock>) -> Block {
    Block::Turn(crate::app::Turn {
        n: 1,
        content,
        tools: Vec::new(),
        thinking_open: false,
        open: false,
    })
}

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

    /// The `(x, y)` cells whose style carries `Modifier::REVERSED` — the selection
    /// highlight. `buffer_text` reads only symbols, so it cannot see the highlight;
    /// this is the style-aware probe the highlight tests assert against.
    fn reversed_cells(terminal: &Terminal<TestBackend>) -> Vec<(u16, u16)> {
        let buffer = terminal.backend().buffer();
        let mut cells = Vec::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer[(x, y)].modifier.contains(Modifier::REVERSED) {
                    cells.push((x, y));
                }
            }
        }
        cells
    }

    /// The screen `(row, x0)` of the row carrying `needle`, from the last-published
    /// transcript hit map (publish one with a `render` first).
    fn row_of(app: &App, needle: &str) -> (u16, u16) {
        let hit = app.hit.transcript.as_ref().expect("a transcript hit");
        let idx = hit.rows.iter().position(|r| r.contains(needle)).expect("the row");
        (hit.rect.y + idx as u16, hit.rect.x)
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
        let text = buffer_text(&render(&mut app, 40, 8));
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
        assert!(text.contains("YOU"), "the user block renders a `YOU` head: {text}");
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
        app.handle(AppEvent::Agent(root(), wcode_harness::event::AgentEvent::MessageEnd {
            message: AgentMessage::Assistant {
                content: vec![ContentBlock::ToolCall {
                    id: "t1".into(), name: "read".into(), arguments: serde_json::json!({}),
                }],
                stop_reason: wcode_harness::message::StopReason::ToolUse,
                usage: None, model: None,
            },
        }));
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
    fn a_tool_diff_renders_when_expanded() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "t1".into(),
                        name: "edit".into(),
                        arguments: serde_json::json!({ "path": "f.rs" }),
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
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand the note
        let text = buffer_text(&render(&mut app, 60, 20));
        assert!(text.contains("-old"), "removal missing: {text}");
        assert!(text.contains("+new"), "addition missing: {text}");
        assert!(text.contains("+1 −1"), "summary missing: {text}");
    }

    #[test]
    fn the_run_summary_and_changes_picker_render() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "t1".into(),
                        name: "edit".into(),
                        arguments: serde_json::json!({ "path": "src/a.rs" }),
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
        // The `»` row names the tool; the changed path rides its params.
        assert!(text.contains("├ edit"), "tool head missing: {text}");
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand to reveal the path
        let text = buffer_text(&render(&mut app, 60, 20));
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

    /// Push one tool invocation (its call, then start → end) into the focused
    /// surface. The assistant's `ToolCall` commits FIRST (the inline render emits
    /// a tool panel only at its `ToolCall` site), then the lifecycle events.
    fn push_tool(app: &mut App, name: &str, output: &str, is_error: bool, diff: Option<&str>) {
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "t".into(),
                        name: name.into(),
                        arguments: serde_json::json!({}),
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
    fn a_done_tool_shows_its_head_rows_and_a_preview() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        let text = buffer_text(&render(&mut app, 70, 20));
        // One tree row carries the head: the name ONCE, the `line-1` stats, and
        // the `▸`/`▣` affordances; a collapsed tool shows a dim preview + hint.
        assert!(!text.contains("── notes"), "no foot rule:\n{text}");
        assert!(text.contains("├ bash"), "the inline tool head:\n{text}");
        assert_eq!(
            text.matches("bash").count(),
            1,
            "the name prints once — no separate `✓` row:\n{text}"
        );
        assert!(text.contains("line-1"), "the stats summary:\n{text}");
        assert!(text.contains("line-2"), "the preview:\n{text}");
        assert!(text.contains("… +15 more lines"), "the hint:\n{text}");
    }

    #[test]
    fn enter_expands_the_selected_tool_fully() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);
        // `Ctrl-T` expands every inline tool's output.
        app.handle(AppEvent::Key(Key::Ctrl('t')));
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

        // Collapsed: the head row (with the `+a −r` stats) + a capped preview.
        let collapsed = buffer_text(&render(&mut app, 70, 24));
        assert!(collapsed.contains("├ edit"), "{collapsed}");
        assert!(collapsed.contains("+30 −0"), "the diff stats ride the head:\n{collapsed}");
        assert!(
            !collapsed.contains("+add-30"),
            "the diff is collapsed:\n{collapsed}"
        );

        app.handle(AppEvent::Key(Key::Ctrl('t')));
        let expanded = buffer_text(&render(&mut app, 70, 40));
        assert!(expanded.contains("+add-30"), "the full diff should show:\n{expanded}");
    }

    #[test]
    fn a_single_long_line_renders_its_tail_only_when_expanded() {
        let mut app = App::new();
        // A ~300-char line with no spaces: collapsed shows only the 80-char
        // summary, so the tail is unreachable until expanded (char-exact wrap).
        let line = format!("{}TAIL-REACHABLE", "x".repeat(285));
        push_tool(&mut app, "bash", &line, false, None);
        let collapsed = buffer_text(&render(&mut app, 80, 20));
        assert!(
            !collapsed.contains("TAIL-REACHABLE"),
            "the tail must be hidden while collapsed:\n{collapsed}"
        );

        app.handle(AppEvent::Key(Key::Ctrl('t')));
        let expanded = buffer_text(&render(&mut app, 80, 20));
        // Char-exact wrap may split the tail across rows, so join the body
        // (drop whitespace and the tree `│`) before checking reachability.
        let joined: String = expanded
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '│')
            .collect();
        assert!(
            joined.contains("TAIL-REACHABLE"),
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
    fn a_multi_line_tool_output_expands_to_all() {
        let mut app = App::new();
        let output = (1..=10)
            .map(|i| format!("row {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);

        // Collapsed: the summary ("row 1") on the head + a preview that stops
        // short of the body — the rest is behind the disclosure.
        let collapsed = buffer_text(&render(&mut app, 70, 20));
        assert!(collapsed.contains("row 1"), "the summary:\n{collapsed}");
        assert!(!collapsed.contains("row 6"), "the body is collapsed:\n{collapsed}");

        app.handle(AppEvent::Key(Key::Ctrl('t'))); // `Ctrl-T` expands every tool
        let expanded = buffer_text(&render(&mut app, 70, 20));
        assert!(expanded.contains("row 10"), "not all lines shown:\n{expanded}");
    }

    #[test]
    fn the_more_hint_pluralizes_one_hidden_line() {
        // The hint is a unit of the (expanded) body: 1 is singular, 2+ plural.
        let one = more_hint(1, false).expect("a hint");
        assert_eq!(one.spans[0].content.as_ref(), "   │   … +1 more line");
        let two = more_hint(2, false).expect("a hint");
        assert_eq!(two.spans[0].content.as_ref(), "   │   … +2 more lines");
        // `wide` adds the single-line-elision hint; 0 with no `wide` → none.
        assert!(more_hint(0, false).is_none());
        assert!(more_hint(0, true).is_some());
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
    fn a_tool_row_toggles_independently() {
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

        // A click on the LAST tool row's toggle expands only it.
        let _ = render(&mut app, 70, 50); // publish the hit map
        let toggle = {
            let hit = app.hit.transcript.as_ref().expect("a hit");
            hit.affordances.last().expect("a tool hit").toggle
        };
        for kind in [MouseKind::Down, MouseKind::Up] {
            app.handle(AppEvent::Mouse(MouseEvent {
                kind,
                col: toggle.x + 3,
                row: toggle.y,
            }));
        }
        let text = buffer_text(&render(&mut app, 70, 50));
        assert!(text.contains("LAST-20"), "the clicked tool expands:\n{text}");
        assert!(!text.contains("FIRST-20"), "only the clicked tool expands:\n{text}");
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
    fn an_errored_tool_head_shows_the_error_mark() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "boom: command not found", true, None);
        let text = buffer_text(&render(&mut app, 80, 20));
        assert!(
            text.contains("✗ bash"),
            "an errored tool head carries the `✗` mark:\n{text}"
        );
    }

    #[test]
    fn param_syntax_picks_shell_or_the_file_extension() {
        let base = || Tool {
            name: "bash".into(),
            target: None,
            output: String::new(),
            done: true,
            is_error: false,
            expanded: false,
            diff: None,
            path: None,
            duration_ms: None,
            params: Vec::new(),
        };
        let mut bash = base();
        bash.name = "bash".into();
        assert_eq!(param_syntax(&bash, "command").as_deref(), Some("sh"));
        assert_eq!(param_syntax(&bash, "timeout"), None);
        let mut read = base();
        read.name = "read".into();
        read.path = Some("src/ui.rs".into());
        assert_eq!(param_syntax(&read, "path").as_deref(), Some("rs"));
    }

    #[test]
    fn code_extension_reads_the_suffix() {
        assert_eq!(code_extension(Some("src/ui.rs")).as_deref(), Some("rs"));
        assert_eq!(code_extension(Some("a/b/foo.py")).as_deref(), Some("py"));
        assert_eq!(code_extension(Some("Makefile")), None);
        assert_eq!(code_extension(Some("dir.d/file")), None);
        assert_eq!(code_extension(None), None);
    }

    #[test]
    fn wrap_styled_breaks_a_long_span_into_rows() {
        let spans = vec![Span::styled("abcdefghij".to_string(), Style::default())];
        let rows = wrap_styled(&spans, 4);
        assert_eq!(rows.len(), 3, "10 chars at width 4 → three rows");
        let text: Vec<String> = rows
            .iter()
            .map(|r| r.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        assert_eq!(text, ["abcd", "efgh", "ij"]);
    }

    #[test]
    fn a_wrapped_running_tool_body_counts_toward_the_scroll_height() {
        let mut app = App::new();
        // One long line, still streaming (not done), expanded in browse mode.
        let line = format!("START{}END", "y".repeat(300));
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "t".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "command": "yes" }),
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
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand the running tool

        // A short terminal: the wrapped body overflows the transcript band.
        let _ = buffer_text(&render(&mut app, 40, 8));
        for _ in 0..50 {
            app.handle(AppEvent::Key(Key::PageUp));
        }
        assert!(
            app.scroll() > 0,
            "the wrapped rows must count toward the scroll height (total={}, scroll={}):\n{}",
            app.total_lines(),
            app.scroll(),
            buffer_text(&render(&mut app, 40, 8))
        );
        // Scrolled to the top, the start of the wrapped body is reachable — it
        // would not be if the body were counted as a single line. A taller band
        // so the body clears the head + `WCODE` + the tool head.
        let text = buffer_text(&render(&mut app, 40, 16));
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
        // Collapsed, neither tool's tail shows (only the dim previews).
        assert!(!buffer_text(&render(&mut app, 70, 60)).contains("line-20"));

        app.handle(AppEvent::Key(Key::Ctrl('t')));
        let text = buffer_text(&render(&mut app, 70, 60));
        assert!(text.contains("line-20"), "all notes show their whole output:\n{text}");
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
    fn click_a_block_frame_shows_the_bar_on_the_clicked_block() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello"), reply("world")]);
        let _ = render(&mut app, 60, 20); // publish the hit map
        // Read the band geometry back rather than hardcoding a row: click the
        // first visible row of the transcript band.
        // Click a real block row (row 0 is the session head — chrome, not a target).
        let (row, _x0) = row_of(&app, "hello");
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: 2, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Up, col: 2, row }));
        let text = buffer_text(&render(&mut app, 60, 20));
        assert!(!barred(&text).is_empty(), "the clicked block carries the bar:\n{text}");
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
        assert_eq!(barred(&narrow).len(), 4, "re-wrapped at 20 cols:\n{narrow}");
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
    fn a_click_on_a_non_block_row_changes_nothing() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        let before = buffer_text(&render(&mut app, 60, 20));

        // Click a blank separator row — it lies between blocks, inside no range.
        let (row, x0) = {
            let hit = app.hit.transcript.as_ref().expect("a transcript hit");
            let idx = hit.rows.iter().position(|r| r.is_empty()).expect("a blank row");
            (hit.rect.y + idx as u16, hit.rect.x)
        };
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: x0, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Up, col: x0, row }));

        let after = buffer_text(&render(&mut app, 60, 20));
        assert_eq!(after, before, "a non-block click leaves the frame byte-identical");
        assert_eq!(app.mode(), Mode::Input, "no browse was entered");
        assert_eq!(app.selected(), None, "nothing was selected");
    }

    #[test]
    fn a_drag_highlight_does_not_inject_rows() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello"), reply("world")]);
        let plain = buffer_text(&render(&mut app, 60, 20));
        let plain_total = app.total_lines();

        // A drag over the transcript restyles the covered cells; it adds no row and
        // (being a restyle) changes no glyph.
        let hit = app.hit.transcript.as_ref().expect("a transcript hit");
        let row = hit.rect.y;
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: 0, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Drag, col: 5, row }));
        let dragged = buffer_text(&render(&mut app, 60, 20));
        assert_eq!(app.total_lines(), plain_total, "a highlight injects no rows");
        assert_eq!(dragged, plain, "a highlight restyles, never reflows");
    }

    #[test]
    fn a_drag_highlight_reverses_exactly_the_selected_cells() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        let first = render(&mut app, 60, 20);
        let (row, _x0) = row_of(&app, "hello");
        // Locate the 'h' of "hello" (the `YOU` head fills the gutter before it).
        let start = {
            let buffer = first.backend().buffer();
            (0..buffer.area.width)
                .find(|&x| buffer[(x, row)].symbol() == "h")
                .expect("the 'h' of hello")
        };

        // Drag over "hel".
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: start, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Drag, col: start + 2, row }));

        let terminal = render(&mut app, 60, 20);
        // The `YOU` head run is reversed by design; filter to the text region.
        let mut cells: Vec<(u16, u16)> = reversed_cells(&terminal)
            .into_iter()
            .filter(|&(x, y)| y == row && x >= start)
            .collect();
        cells.sort_unstable();
        assert_eq!(
            cells,
            vec![(start, row), (start + 1, row), (start + 2, row)],
            "exactly the selected cells are reversed"
        );
    }

    #[test]
    fn a_past_end_anchor_copies_and_highlights_nothing() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        let _ = render(&mut app, 60, 20);
        let (row, x0) = row_of(&app, "hello");
        let len = app
            .hit
            .transcript
            .as_ref()
            .unwrap()
            .rows
            .iter()
            .find(|r| r.contains("hello"))
            .unwrap()
            .chars()
            .count() as u16;
        let past = x0 + len; // one cell past the row's end

        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: past, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Drag, col: past + 2, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Up, col: past + 2, row }));

        assert!(
            app.take_actions().iter().all(|a| !matches!(a, Action::Copy(_))),
            "a past-end anchor copies nothing"
        );
        let terminal = render(&mut app, 60, 20);
        // The `YOU` head is reversed by design; the drag must add no text cell.
        let start = {
            let buffer = terminal.backend().buffer();
            (0..buffer.area.width)
                .find(|&x| buffer[(x, row)].symbol() == "h")
                .expect("the 'h' of hello")
        };
        let stray: Vec<(u16, u16)> = reversed_cells(&terminal)
            .into_iter()
            .filter(|&(x, y)| y == row && x >= start)
            .collect();
        assert!(stray.is_empty(), "a past-end anchor highlights no text: {stray:?}");
    }

    #[test]
    fn the_highlight_persists_after_copy_and_a_wheel_scroll() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        let _ = render(&mut app, 60, 20);
        let (row, x0) = row_of(&app, "hello");

        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: x0, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Drag, col: x0 + 4, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Up, col: x0 + 4, row }));
        assert!(
            matches!(app.take_actions().as_slice(), [Action::Copy(_)]),
            "the drag copied the selection"
        );

        let selection = app.text_sel();
        assert!(selection.is_some(), "the highlight survives the copy");
        // A wheel notch scrolls the view; it must not clear the selection.
        app.handle(AppEvent::Key(Key::ScrollUp));
        assert_eq!(app.text_sel(), selection, "a wheel scroll keeps the selection");
    }

    #[test]
    fn esc_does_not_clear_a_live_selection() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        let _ = render(&mut app, 60, 20);
        let (row, x0) = row_of(&app, "hello");

        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: x0, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Drag, col: x0 + 3, row }));
        let selection = app.text_sel();
        assert!(selection.is_some(), "a live selection");

        app.handle(AppEvent::Key(Key::Esc));
        assert_eq!(
            app.text_sel(),
            selection,
            "Esc no longer quits at idle, so it leaves a live selection in place"
        );
    }

    #[test]
    fn a_mouse_click_under_an_overlay_is_a_no_op() {
        let mut app = App::new();
        app.seed_history(&root(), &[AgentMessage::user_text("hello")]);
        let _ = render(&mut app, 60, 20);
        let (row, x0) = row_of(&app, "hello");

        app.handle(AppEvent::Key(Key::F(1))); // open the help overlay
        assert!(app.overlay().is_some(), "the overlay is up");

        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Down, col: x0 + 2, row }));
        app.handle(AppEvent::Mouse(MouseEvent { kind: MouseKind::Up, col: x0 + 2, row }));
        assert_eq!(app.mode(), Mode::Input, "the click did not enter browse");
        assert_eq!(app.selected(), None, "the click did not select a block");
        assert!(app.text_sel().is_none(), "the click did not start a selection");
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
        // transcript: [⋯ earlier, alpha, beta-turn, ALPHA] — "alpha" matches 1 and 3.
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
        let assistant = a_turn(vec![
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
                params: Vec::new(),
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
    fn a_drag_highlight_changes_no_row_width() {
        // Mirror `paint_bar_never_shifts_a_text_row`: a restyle must not move a glyph.
        for block in [
            Block::User("hi there".into()),
            a_turn(vec![ContentBlock::Text {
                text: "# Head\n\ntext `code` and **bold**".into(),
            }]),
            Block::Notice("a note".into()),
        ] {
            for line in block_lines(&block, 40) {
                let before = line_width(&line);
                let mut painted = line.clone();
                recolor_range(&mut painted, 0, usize::MAX, selection_style());
                assert_eq!(line_width(&painted), before, "a highlight must not reflow");
            }
        }
    }

    #[test]
    fn ctrl_t_expands_the_inline_tool_in_the_frame() {
        let mut app = App::new();
        let output = (1..=20)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        push_tool(&mut app, "bash", &output, false, None);

        let collapsed = buffer_text(&render(&mut app, 70, 30));
        assert!(!collapsed.contains("line-20"), "collapsed hides the tail:\n{collapsed}");

        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand the inline tool
        let expanded = buffer_text(&render(&mut app, 70, 30));
        assert!(expanded.contains("line-20"), "`Ctrl-T` shows the full output:\n{expanded}");
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
        // A tool-call-only round renders the tool INLINE — no foot rule, and no
        // stray blank between the `WCODE` head and the tool row.
        assert!(text.contains("├ read"), "the inline tool:\n{text}");
        assert!(!text.contains("── notes"), "no foot rule:\n{text}");
    }

    #[test]
    fn the_sidebar_heads_its_sections_with_a_rule() {
        use wcode_harness::protocol::SessionId;
        let mut app = App::new();
        let surf = |id: &str, label: &str, is_root: bool| crate::SurfaceInfo {
            id: SessionId::agent(id),
            label: label.into(),
            model: "m".into(),
            is_root,
        };
        app.set_surfaces(vec![
            surf("root", "wcode", true),
            surf("explorer", "explorer", false),
            surf("developer", "developer", false),
        ]);
        app.handle(AppEvent::Key(Key::Ctrl('b'))); // open the sidebar
        let text = buffer_text(&render(&mut app, 120, 24));
        assert!(text.contains("── agents"), "the agents section rule:\n{text}");
        assert!(text.contains("explorer"), "a member row:\n{text}");
        assert!(text.contains("── changes"), "the changes section rule:\n{text}");
    }

    #[test]
    fn a_turn_opens_on_its_body_with_no_head() {
        let mut app = App::new();
        push_assistant(
            &mut app,
            "# the anchor is the text\n\nAn anchor is the text you quote.",
        );
        let text = buffer_text(&render(&mut app, 80, 12));
        // Only the pinned header carries `WCODE`; the turn itself has no head
        // (D012 — the assistant is the page).
        assert_eq!(
            text.matches("WCODE").count(),
            1,
            "no `WCODE` speaker head:\n{text}"
        );
        assert!(
            text.contains("An anchor is the text you quote."),
            "the body:\n{text}"
        );
    }

    #[test]
    fn a_headingless_turn_renders_its_body() {
        let mut app = App::new();
        push_assistant(&mut app, "no heading");
        let text = buffer_text(&render(&mut app, 80, 12));
        assert!(text.contains("no heading"), "the body:\n{text}");
        assert_eq!(text.matches("WCODE").count(), 1, "no speaker head:\n{text}");
    }

    #[test]
    fn the_user_prompt_reads_in_the_user_role() {
        let you = speaker_lines("a prompt", 80);
        assert_eq!(
            you[0].spans.last().unwrap().style,
            theme::theme().user,
            "the prompt reads in the accent role"
        );
        assert_ne!(
            theme::theme().user,
            theme::theme().body,
            "the accent must differ from the assistant's default body"
        );
    }

    #[test]
    fn the_session_head_is_not_a_browse_selection_target() {
        // The pinned header is chrome in a band, not a block: Ctrl-G selects the
        // turn (block 0), and there is nothing above it to skip to.
        let mut app = App::new();
        push_assistant(&mut app, "hello");
        let _ = render(&mut app, 80, 12);
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        assert_eq!(app.selected(), Some(0), "the turn");
        app.handle(AppEvent::Key(Key::Char('k'))); // nothing above the turn
        assert_eq!(app.selected(), Some(0), "the header is not a target");
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
        // Two sections only (D35): the member list and the change list. Todos
        // moved to the transcript, so its section header is gone.
        for header in ["agents", "changes"] {
            assert!(frame.contains(header), "missing {header} header:\n{frame}");
        }
        assert!(
            !frame.contains("Todos"),
            "the Todos sidebar section must be gone:\n{frame}"
        );
        assert!(frame.contains("explorer"), "member row missing:\n{frame}");
        // The member row is NUMBERED (D34), so `Alt-N` is visible not guessed.
        assert!(
            frame.contains("1 ○ explorer"),
            "the numbered member row is missing:\n{frame}"
        );
    }

    /// Finish one `edit` tool that changed `path` with `diff`.
    fn push_change(app: &mut App, path: &str, diff: &str) {
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
                diff: Some(diff.into()),
                path: Some(path.into()),
                duration_ms: None,
            },
        ));
    }

    #[test]
    fn the_sidebar_renders_the_changes_tree() {
        let mut app = App::new();
        push_change(&mut app, "crates/wcode-tui/src/ui.rs", "@@ -1 +1 @@\n+a\n+b\n");
        push_change(&mut app, "crates/wcode-tui/src/app.rs", "@@ -1 +1 @@\n-a\n+b\n");
        app.handle(AppEvent::Key(Key::Ctrl('b')));
        let frame = buffer_text(&render(&mut app, 100, 30));
        assert!(
            frame.contains("changes  2 files  +3 −1"),
            "the tree header:\n{frame}"
        );
        assert!(
            frame.contains("crates/wcode-tui/src/"),
            "the directory line:\n{frame}"
        );
        assert!(frame.contains("├─ app.rs"), "a non-last leaf:\n{frame}");
        assert!(frame.contains("└─ ui.rs"), "the last leaf:\n{frame}");
    }

    #[test]
    fn the_changes_overlay_right_aligns_the_stats() {
        let mut app = App::new();
        push_change(&mut app, "crates/wcode-tui/src/ui.rs", "@@ -1 +1 @@\n+a\n+b\n");
        push_change(&mut app, "crates/wcode-tui/src/app.rs", "@@ -1 +1 @@\n-a\n+b\n");
        let _ = render(&mut app, 100, 30); // record the terminal width for the modal
        typed(&mut app, "/changes");
        app.handle(AppEvent::Key(Key::Enter));
        let text = buffer_text(&render(&mut app, 100, 30));
        // Both leaves' `+a −r` stats end at the same column (D36).
        // Both leaves' `+a −r` groups start at the same column: the row after the
        // modal's `│` + the `❯ ` marker is the item, so the `+` column tracks the
        // padding. (Measuring the row's end would just find the modal's `│`.)
        let plus: Vec<usize> = text
            .lines()
            .filter(|l| l.contains("├─ ") || l.contains("└─ "))
            .map(|l| l.chars().position(|c| c == '+').expect("a stats row"))
            .collect();
        assert_eq!(plus.len(), 2, "two leaves in the overlay:\n{text}");
        assert_eq!(plus[0], plus[1], "the stats are right-aligned:\n{text}");
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
            !narrow.contains("agents"),
            "no panel under the width threshold:\n{narrow}"
        );
    }

    #[test]
    fn the_composer_carries_the_foot_line_only() {
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
        // The composer is rule + open writing line — NO box, NO corners.
        assert!(
            !text.contains('╭') && !text.contains('╮') && !text.contains('╰') && !text.contains('╯'),
            "the composer must be frameless:\n{text}"
        );
        assert!(!text.contains('│'), "no side border columns:\n{text}");
        let lines: Vec<&str> = text.lines().collect();
        // No composer head row (D009): the LAST full-width rule leads the composer
        // band, the writing line below it, the foot line below that.
        let rule = lines
            .iter()
            .rposition(|l| *l == "─".repeat(80))
            .expect("a composer rule");
        assert!(lines[rule + 1].contains('❯'), "the writing line:\n{text}");
        assert!(
            lines[rule + 2].contains('─') && lines[rule + 2].ends_with("⏸ idle"),
            "the foot line carries the leader + state:\n{text}"
        );
    }

    #[test]
    fn the_composer_foot_line_shows_the_folio() {
        let mut app = App::new();
        // No turn yet → no folio.
        let text = buffer_text(&render(&mut app, 80, 14));
        assert!(!text.contains("0/0"), "no folio before a turn:\n{text}");
        // A first turn reads `1/1`; a second `2/2` (an `AgentEnd` seals the turn,
        // so the next reply opens a NEW exchange).
        // Each run is `AgentStart` … `AgentEnd`; one run = one turn (the folio).
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::AgentStart,
        ));
        push_assistant(&mut app, "one");
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::AgentEnd,
        ));
        let text = buffer_text(&render(&mut app, 80, 14));
        assert!(text.contains("1/1"), "the folio:\n{text}");

        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::AgentStart,
        ));
        push_assistant(&mut app, "two");
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::AgentEnd,
        ));
        let text = buffer_text(&render(&mut app, 80, 14));
        assert!(text.contains("2/2"), "the folio updates:\n{text}");
    }

    #[test]
    fn the_head_renders_row_zero_with_the_session_and_branch_groups() {
        let mut app = App::new();
        app.set_cwd(Some("wcode".into()));
        app.set_git(Some("main*".into()));
        app.set_status(crate::app::Status {
            session: Some("abcdef0123456789".into()),
            model: "zephyr-9".into(),
            effort: Some("high".into()),
            ..Default::default()
        });
        let text = buffer_text(&render(&mut app, 80, 14));
        let first = text.lines().next().unwrap_or_default();
        // Row 0: the reverse-video `WCODE`, the session, the project · branch, and
        // the model · effort far right.
        assert!(first.contains("WCODE"), "the speaker head:\n{text}");
        assert!(first.contains("session abcdef01"), "the session group:\n{text}");
        assert!(first.contains("wcode"), "the project group:\n{text}");
        assert!(first.contains("⎇ main*"), "the branch:\n{text}");
        assert!(first.contains("zephyr-9 · high"), "model · effort:\n{text}");
        // The rule follows the head, on the measure.
        assert_eq!(
            text.lines().nth(1),
            Some("─".repeat(80).as_str()),
            "the rule:\n{text}"
        );
    }

    #[test]
    fn the_composer_draws_no_head_row() {
        let mut app = App::new();
        app.set_cwd(Some("wcode".into()));
        app.set_git(Some("main*".into()));
        let text = buffer_text(&render(&mut app, 80, 14));
        // The chrome rides the head block; the composer has no head row above its
        // rule (the row directly above the composer rule carries no `wcode`/`⎇`).
        let lines: Vec<&str> = text.lines().collect();
        let rule = lines
            .iter()
            .rposition(|l| *l == "─".repeat(80))
            .expect("the composer rule");
        assert!(
            !lines[rule - 1].contains("wcode") && !lines[rule - 1].contains('⎇'),
            "no composer head row above the rule:\n{text}"
        );
    }

    #[test]
    fn tools_render_inline_in_call_order_not_at_the_foot() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "first", false, None);
        push_assistant(&mut app, "between");
        push_tool(&mut app, "bash", "second", false, None);
        let text = buffer_text(&render(&mut app, 70, 30));
        assert!(!text.contains("── notes"), "no foot ledger rule:\n{text}");
        // The tools sit at their call sites, in call order, around the prose.
        let one = text.find("├ bash").expect("the first tool");
        let mid = text.find("between").expect("the prose");
        let two = text[one + "├ bash".len()..]
            .find("├ bash")
            .map(|i| i + one + "├ bash".len())
            .expect("the second tool");
        assert!(one < mid && mid < two, "tools inline in call order:\n{text}");
    }

    #[test]
    fn the_foot_notes_rule_is_gone() {
        let mut app = App::new();
        push_tool(&mut app, "bash", "ok", false, None);
        let text = buffer_text(&render(&mut app, 70, 20));
        assert!(!text.contains("── notes"), "the `── notes ──` rule is gone:\n{text}");
        assert!(text.contains("├ bash"), "the tool is inline:\n{text}");
    }

    #[test]
    fn the_committed_turn_marks_the_reference_at_the_call_site() {
        let mut app = App::new();
        push_assistant(&mut app, "Some prose.");
        push_tool(&mut app, "bash", "out", false, None);
        let text = buffer_text(&render(&mut app, 70, 30));
        // The `¹` rides the prose, at the call site, above its inline tool.
        let mark = text.find('¹').expect("the reference mark");
        let tool = text.find("├ bash").expect("the inline tool");
        assert!(mark < tool, "the mark precedes its tool:\n{text}");
    }

    #[test]
    fn a_text_less_round_marks_no_prose_but_still_renders_its_tool() {
        let mut app = App::new();
        // Two tool calls with no prose between them: no `¹`, but both render.
        push_tool(&mut app, "bash", "one", false, None);
        push_tool(&mut app, "bash", "two", false, None);
        let text = buffer_text(&render(&mut app, 70, 30));
        assert!(!text.contains('¹'), "no mark on a text-less round:\n{text}");
        assert!(
            text.matches("├ bash").count() == 2,
            "both tools still render (once each):\n{text}"
        );
    }

    #[test]
    fn the_foot_line_run_state_is_dim_idle_and_accent_running() {
        let mut app = App::new();
        app.set_cwd(Some("wcode".into()));
        let area = Rect::new(0, 0, 100, 3);

        // Idle: the state stays `dim` (D4b). (The project's `muted` style now rides
        // the session-head block — see the header tests.)
        let (_bl, br) = corner_titles(&app, area);
        let state = br
            .spans
            .iter()
            .find(|s| s.content.contains("idle"))
            .expect("the state span");
        assert_eq!(state.style, dim(), "idle state → dim");

        // Running: the state takes the accent budget.
        let id = app.focused_id().clone();
        app.handle(AppEvent::Agent(
            id,
            wcode_harness::event::AgentEvent::AgentStart,
        ));
        let (_bl, br) = corner_titles(&app, area);
        let state = br
            .spans
            .iter()
            .find(|s| s.content.contains("running"))
            .expect("the state span");
        assert_eq!(state.style, accent(), "running state → accent");
    }

    #[test]
    fn an_in_flight_btw_shows_in_the_foot_line() {
        let mut app = App::new();
        // Submit `/btw` — the side ask stays in flight until the reply arrives.
        typed(&mut app, "/btw why?");
        app.handle(AppEvent::Key(Key::Enter));
        let _ = app.take_actions();

        let text = buffer_text(&render(&mut app, 80, 14));
        assert!(text.contains("btw…"), "the foot line shows the pending btw:\n{text}");
        assert!(
            !text.contains("⏸ idle"),
            "the idle state is replaced while asking:\n{text}"
        );
    }

    #[test]
    fn the_session_id_rides_the_session_head() {
        let mut app = App::new();
        app.set_status(crate::app::Status {
            session: Some("abcdef0123456789".into()),
            ..Default::default()
        });
        push_assistant(&mut app, "hello there");
        let text = buffer_text(&render(&mut app, 80, 12));
        // The session id rides the session-head block (the composer has no head row).
        let first = text.lines().next().unwrap_or_default();
        assert!(first.contains("WCODE"), "the head leads at row 0:\n{text}");
        assert!(text.contains("hello there"), "the reply body:\n{text}");
        assert!(
            text.contains("session abcdef01"),
            "the session id is not on the session head:\n{text}"
        );
    }

    #[test]
    fn a_frame_with_no_session_reserves_no_session_row() {
        let mut app = App::new();
        push_assistant(&mut app, "hello there");
        let text = buffer_text(&render(&mut app, 80, 12));
        assert!(
            !text.contains(" session "),
            "no session row is reserved without an id:\n{text}"
        );
    }

    #[test]
    fn a_fresh_surface_shows_the_empty_state_hint() {
        let mut app = App::new();
        let text = buffer_text(&render(&mut app, 80, 12));
        assert!(text.contains("type a message"), "the fresh hint:\n{text}");
        assert!(text.contains("/help for commands"), "the fresh hint:\n{text}");

        // A resumed session seeds real turns, which retire the hint (D5).
        app.seed_history(&root(), &[AgentMessage::user_text("old question")]);
        let text = buffer_text(&render(&mut app, 80, 12));
        assert!(
            !text.contains("type a message"),
            "the hint must not linger after history:\n{text}"
        );
    }

    #[test]
    fn the_overlays_use_rounded_borders() {
        // The F1 help.
        let mut app = App::new();
        app.handle(AppEvent::Key(Key::F(1)));
        let help = buffer_text(&render(&mut app, 80, 24));
        assert!(help.contains('╭'), "the help border is rounded:\n{help}");
        assert!(!help.contains('┌'), "a square corner remains in the help:\n{help}");

        // A picker modal.
        let mut app = App::new();
        app.set_models(vec!["m1".into()]);
        typed(&mut app, "/model");
        app.handle(AppEvent::Key(Key::Enter));
        let pick = buffer_text(&render(&mut app, 80, 24));
        assert!(pick.contains('╭'), "the picker border is rounded:\n{pick}");
        assert!(!pick.contains('┌'), "a square corner remains in the picker:\n{pick}");

        // The `/`-command completion popup.
        let mut app = App::new();
        typed(&mut app, "/");
        let comp = buffer_text(&render(&mut app, 80, 24));
        assert!(comp.contains("commands"), "the completion popup is drawn:\n{comp}");
        assert!(comp.contains('╭'), "the completion border is rounded:\n{comp}");
        assert!(!comp.contains('┌'), "a square corner remains in the completion:\n{comp}");
    }

    #[test]
    fn the_model_appears_exactly_once_in_the_composer() {
        // Regression carried over from the old status line: the model must be
        // named once, on the composer head line's right.
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
    fn a_short_terminal_draws_the_composer_without_panicking() {
        let mut app = App::new();
        // Every band height must draw without panicking (the rule/foot degrade).
        for height in [2u16, 3, 4, 5, 6, 8] {
            let text = buffer_text(&render(&mut app, 80, height));
            assert!(
                !text.contains('╭') && !text.contains('╰'),
                "the composer is frameless at height {height}:\n{text}"
            );
        }
        // At five rows the composer band seats rule · input · foot (no head row).
        let text = buffer_text(&render(&mut app, 80, 5));
        assert!(text.lines().any(|l| l == "─".repeat(80)), "a rule:\n{text}");
        assert!(text.contains('❯'), "the input survives:\n{text}");
        assert!(text.contains("⏸ idle"), "the foot line survives:\n{text}");
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
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand the note
        let text = buffer_text(&render(&mut app, 80, 14));
        assert!(text.contains("bash"), "tool name missing:\n{text}");
        assert!(
            text.contains("cargo test -p wcode-cli"),
            "the tool's input (command) must render:\n{text}"
        );
    }
    #[test]
    fn a_completed_tool_shows_a_start_line_and_an_end_line() {
        // Regression: the done block must carry its head row (name + stats) and
        // must not repeat the name on a second `✓` row.
        let mut app = App::new();
        push_tool(&mut app, "bash", "hello", false, None);
        let text = buffer_text(&render(&mut app, 80, 12));
        assert!(text.contains("├ bash"), "the tree head row is missing:\n{text}");
        assert!(text.contains("hello"), "the stats summary is missing:\n{text}");
        assert_eq!(
            text.matches("bash").count(),
            1,
            "the name prints once — no `✓ bash` repeat:\n{text}"
        );
    }

    // ---- slice 3 (D31): the tool panel — params, the mid rule, affordances ----

    /// Commit the assistant block carrying a `bash` call's `cmd`/`cwd` args, so
    /// the `Tool` block gets a populated `params` (D31). One call id: `"t"`.
    fn push_bash_call(app: &mut App, cmd: &str, cwd: &str) {
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::ToolCall {
                        id: "t".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({ "cmd": cmd, "cwd": cwd }),
                    }],
                    stop_reason: wcode_harness::message::StopReason::ToolUse,
                    usage: None,
                    model: None,
                },
            },
        ));
    }

    /// Push a running `bash` tool (start only): params present, body empty.
    fn push_bash_live(app: &mut App, cmd: &str, cwd: &str) {
        push_bash_call(app, cmd, cwd);
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionStart {
                call_id: "t".into(),
                name: "bash".into(),
            },
        ));
    }

    /// Push a finished `bash` tool (start → end) with its call's `cmd`/`cwd`.
    fn push_bash_panel(app: &mut App, cmd: &str, cwd: &str, output: &str) {
        push_bash_live(app, cmd, cwd);
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::ToolExecutionEnd {
                call_id: "t".into(),
                name: "bash".into(),
                output: output.into(),
                is_error: false,
                diff: None,
                path: None,
                duration_ms: None,
            },
        ));
    }

    #[test]
    fn a_note_shows_the_full_command_unclipped() {
        let mut app = App::new();
        let cmd = "cargo test -p wcode-cli --all-targets -- --nocapture --color=always";
        assert!(
            cmd.chars().count() > 60,
            "the fixture must exceed the old 60-char clip"
        );
        push_bash_panel(&mut app, cmd, "/Users/wei/Workspace/wcode", "ok");
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand: the params carry the command
        // At 80 cols the band is below the centering threshold, so the tool gets
        // the full width and the whole command is reachable (no 60-char clip) —
        // the params may wrap across rows, but its tail is present.
        let text = buffer_text(&render(&mut app, 80, 20));
        assert!(
            text.contains("--color=always"),
            "the tail of the command must render (no 60-char clip):\n{text}"
        );
    }

    #[test]
    fn a_note_shows_its_params_when_expanded() {
        let mut app = App::new();
        push_bash_panel(&mut app, "cargo test", "/Users/wei/Workspace/wcode", "ok");
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand the note
        let text = buffer_text(&render(&mut app, 100, 20));
        let cmd_row = text
            .lines()
            .find(|l| l.contains("cmd"))
            .expect("a cmd key row");
        assert!(
            cmd_row.contains("cargo test"),
            "the cmd value rides its key row:\n{text}"
        );
        let cwd_row = text
            .lines()
            .find(|l| l.contains("cwd"))
            .expect("a cwd key row");
        assert!(
            cwd_row.contains("/Users/wei/Workspace/wcode"),
            "the cwd value rides its key row:\n{text}"
        );
    }

    #[test]
    fn the_tool_rows_share_the_content_column() {
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "ok");
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand: params show
        let text = buffer_text(&render(&mut app, 80, 20));
        let row = |needle: &str| {
            text.lines()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("no {needle} row:\n{text}"))
        };
        // The `├` head and the `│` body share the gutter column (3); their
        // content lands at the shared content column (7), like the prose.
        let col = |row: &str, needle: char| row.chars().position(|c| c == needle);
        assert_eq!(col(row("├ bash"), '├'), Some(3), "the head");
        assert_eq!(col(row("│   cmd"), '│'), Some(3), "the body shares the gutter");
    }

    #[test]
    fn a_note_is_frameless() {
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "line one\nline two");
        app.handle(AppEvent::Key(Key::Ctrl('t'))); // expand: params + body show
        let text = buffer_text(&render(&mut app, 80, 20));
        // The tree glyphs (`├ │ └`) are declared; only BOX frame glyphs are banned.
        for bad in ['╭', '╮', '╰', '╯', '┤'] {
            assert!(!text.contains(bad), "a frame glyph {bad} survives:\n{text}");
        }
        // The tool renders inline; there is no `── notes ──` foot rule.
        assert!(text.contains("├ bash"), "the inline tool:\n{text}");
        assert!(!text.contains("── notes"), "no foot rule:\n{text}");
    }

    #[test]
    fn footnote_mark_maps_1_to_9_then_falls_back_to_ascii() {
        assert_eq!(footnote_mark(1), "¹");
        assert_eq!(footnote_mark(3), "³");
        assert_eq!(footnote_mark(9), "⁹");
        assert_eq!(footnote_mark(10), "[10]");
        assert_eq!(footnote_mark(12), "[12]");
    }

    #[test]
    fn a_rendered_turn_shows_its_tool_inline() {
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "ok");
        push_assistant(&mut app, "# the head\n\nSome prose.");
        let text = buffer_text(&render(&mut app, 80, 16));
        assert!(text.contains("├ bash"), "the inline tool:\n{text}");
        assert!(!text.contains("── notes"), "no foot rule:\n{text}");
        for bad in ['╭', '╮', '╰', '╯'] {
            assert!(!text.contains(bad), "frameless — no {bad}:\n{text}");
        }
    }

    #[test]
    fn the_superscript_is_present_with_prose_and_absent_when_textless() {
        // A reply with prose + a tool call emits a `¹` at the call position.
        let with = content_lines(
            &[
                ContentBlock::Text {
                    text: "Some prose here".into(),
                },
                ContentBlock::ToolCall {
                    id: "c".into(),
                    name: "read".into(),
                    arguments: Default::default(),
                },
            ],
            40,
            false,
            false,
        );
        let text: String = with
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains('¹'), "the reference is present: {text:?}");

        // A text-less tool-call round emits no reference (and no line at all).
        let without = content_lines(
            &[ContentBlock::ToolCall {
                id: "c".into(),
                name: "read".into(),
                arguments: Default::default(),
            }],
            40,
            false,
            false,
        );
        assert!(without.is_empty(), "a text-less round renders nothing: {without:?}");
    }

    #[test]
    fn a_note_toggle_and_copy_target_the_note() {
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "the output");
        let _ = render(&mut app, 80, 20); // publish the hit map
        let (block, toggle) = {
            let hit = app.hit.transcript.as_ref().expect("a hit");
            let a = &hit.affordances[0];
            assert_eq!(a.item, Some(0), "the hit names note 0");
            (a.block, a.toggle)
        };
        // A click on the toggle region expands the note.
        for kind in [MouseKind::Down, MouseKind::Up] {
            app.handle(AppEvent::Mouse(MouseEvent {
                kind,
                col: toggle.x + 3,
                row: toggle.y,
            }));
        }
        assert!(
            matches!(&app.transcript()[block], Block::Turn(t) if t.tools[0].expanded),
            "the click expands the tool"
        );

        // The `▣` cell on the tool row copies that tool's output.
        let _ = render(&mut app, 80, 20); // republish the hit map
        let copy = {
            let hit = app.hit.transcript.as_ref().expect("a hit");
            hit.affordances[0].copy
        };
        for kind in [MouseKind::Down, MouseKind::Up] {
            app.handle(AppEvent::Mouse(MouseEvent {
                kind,
                col: copy.x,
                row: copy.y,
            }));
        }
        assert!(
            app.take_actions()
                .iter()
                .any(|a| matches!(a, Action::Copy(t) if t == "the output")),
            "the copy yields the tool's output"
        );
    }

    #[test]
    fn a_multi_note_foot_publishes_a_hit_per_note() {
        let mut app = App::new();
        // A two-round turn: two tool calls, then the answer folds both into ONE foot.
        push_bash_panel(&mut app, "ls", "/w", "first output");
        push_bash_panel(&mut app, "pwd", "/w", "second output");
        push_assistant(&mut app, "the answer");
        {
            let Block::Turn(t) = &app.transcript()[0] else {
                panic!("a turn, got {:?}", app.transcript());
            };
            assert_eq!(t.tools.len(), 2, "both tools in the ledger");
        }

        let _ = render(&mut app, 80, 24);
        let (hits, rows) = {
            let hit = app.hit.transcript.as_ref().expect("a hit");
            assert_eq!(hit.affordances.len(), 2, "one hit per row");
            let hits: Vec<_> = hit.affordances.iter().map(|a| (a.block, a.item)).collect();
            let rows: Vec<_> = hit.affordances.iter().map(|a| a.toggle.y).collect();
            (hits, rows)
        };
        // The pinned header is a band, not a block, so the turn is block 0.
        assert_eq!(
            hits,
            [(0, Some(0)), (0, Some(1))],
            "same block, one hit per row"
        );
        assert_ne!(rows[0], rows[1], "the two hits sit on distinct rows");

        // Toggling the SECOND row expands only it.
        let toggle2 = app.hit.transcript.as_ref().unwrap().affordances[1].toggle;
        for kind in [MouseKind::Down, MouseKind::Up] {
            app.handle(AppEvent::Mouse(MouseEvent {
                kind,
                col: toggle2.x + 3,
                row: toggle2.y,
            }));
        }
        {
            // The pinned header is a band, not a block, so the turn is block 0.
            let Block::Turn(t) = &app.transcript()[0] else {
                panic!("a turn, got {:?}", app.transcript());
            };
            assert!(t.tools[1].expanded, "the second row expands");
            assert!(!t.tools[0].expanded, "the first row stays collapsed");
        }

        // Copying the SECOND note yields its output (re-read the freshly drawn cells).
        let _ = render(&mut app, 80, 24);
        let copy2 = app.hit.transcript.as_ref().unwrap().affordances[1].copy;
        for kind in [MouseKind::Down, MouseKind::Up] {
            app.handle(AppEvent::Mouse(MouseEvent {
                kind,
                col: copy2.x,
                row: copy2.y,
            }));
        }
        assert!(
            app.take_actions()
                .iter()
                .any(|a| matches!(a, Action::Copy(t) if t == "second output")),
            "the copy yields the second note's output"
        );
    }

    #[test]
    fn note_affordances_are_right_aligned() {
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "ok");
        let width = 80usize;
        let text = buffer_text(&render(&mut app, width as u16, 20));
        let header = text
            .lines()
            .find(|l| l.contains("├ bash"))
            .expect("the tool head row");
        let col = header
            .chars()
            .position(|c| c == '▸')
            .expect("the collapse affordance");
        assert_eq!(
            col,
            width - 3,
            "the `▸` is flush right at width - 3:\n{header}"
        );
    }

    #[test]
    fn a_rendered_note_publishes_cells_on_the_drawn_glyphs() {
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "ok");
        let mut terminal = render(&mut app, 80, 20);

        // The published cells line up with the drawn glyphs.
        let (block, item, toggle, copy) = {
            let hit = app.hit.transcript.as_ref().expect("a transcript hit");
            assert_eq!(hit.affordances.len(), 1, "one note → one affordance record");
            let a = &hit.affordances[0];
            (a.block, a.item, a.toggle, a.copy)
        };
        assert_eq!(item, Some(0), "the hit names note 0");
        assert!(
            matches!(&app.transcript()[block], Block::Turn(t) if !t.tools.is_empty()),
            "the affordance names the turn block"
        );

        let buf = terminal.backend().buffer();
        // The wide toggle ENDS on the `▸`; `copy` is exactly the `▣` glyph.
        let glyph_col = toggle.x + toggle.width - 1;
        assert_eq!(buf[(glyph_col, toggle.y)].symbol(), "▸", "the ▸ glyph");
        assert_eq!(buf[(copy.x, copy.y)].symbol(), "▣", "the copy cell");
        assert_eq!(copy.y, toggle.y, "both affordances share the head row");
        assert!(
            !toggle.contains((copy.x, copy.y).into()),
            "the wide toggle and the copy cell are disjoint"
        );

        // End to end: a click on the note's head (the toggle region) expands it.
        let click_col = toggle.x + 3;
        app.handle(AppEvent::Mouse(MouseEvent {
            kind: MouseKind::Down,
            col: click_col,
            row: toggle.y,
        }));
        app.handle(AppEvent::Mouse(MouseEvent {
            kind: MouseKind::Up,
            col: click_col,
            row: toggle.y,
        }));
        let expanded = matches!(&app.transcript()[block], Block::Turn(t) if t.tools[0].expanded);
        assert!(expanded, "clicking the tool head expands it");

        // The freshly toggled frame draws `▾` in the same cell.
        terminal = render(&mut app, 80, 20);
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(glyph_col, toggle.y)].symbol(), "▾", "the toggle flips");
    }

    #[test]
    fn the_transcript_centers_its_measure_on_a_wide_band() {
        // 120 cols: the measure (68) is centered — a 26-col margin each side.
        let mut app = App::new();
        let text = buffer_text(&render(&mut app, 120, 40));
        let hit = app.hit.transcript.as_ref().expect("a transcript hit");
        assert_eq!((hit.rect.x, hit.rect.width), (26, 68), "a centered measure");
        // The transcript content is indented by the pad; the margin to its left is
        // blank. (The pinned header above is full-bleed, so read a transcript row.)
        let body = text
            .lines()
            .nth(hit.rect.y as usize)
            .unwrap_or_default();
        assert_eq!(
            body.chars().take(26).collect::<String>().trim(),
            "",
            "the margin is blank:\n{text}"
        );
        assert!(
            body.contains("type a message"),
            "the transcript body rides the measure:\n{text}"
        );
        // The pinned header itself is full-bleed: `WCODE` starts at column 0.
        assert_eq!(
            text.lines().next().unwrap_or_default().chars().next(),
            Some('W'),
            "the pinned header is full-bleed:\n{text}"
        );

        // 80 cols: below the threshold — the content rides the gutter.
        let mut app = App::new();
        let text = buffer_text(&render(&mut app, 80, 24));
        let hit = app.hit.transcript.as_ref().expect("a transcript hit");
        assert_eq!(
            (hit.rect.x, hit.rect.width),
            (0, 80),
            "full width below the threshold"
        );
        let first = text.lines().next().unwrap_or_default();
        assert_eq!(
            first.chars().next(),
            Some('W'),
            "the head rides the gutter at 80 cols:\n{text}"
        );
    }

    #[test]
    fn the_measure_threshold_flips_at_84() {
        // 83 cols → the full band width; 84 → the 68-col measure centers (pad 8).
        let mut app = App::new();
        let _ = render(&mut app, 83, 24);
        let hit = app.hit.transcript.as_ref().expect("a transcript hit");
        assert_eq!((hit.rect.x, hit.rect.width), (0, 83), "83 → full width");

        let mut app = App::new();
        let _ = render(&mut app, 84, 24);
        let hit = app.hit.transcript.as_ref().expect("a transcript hit");
        assert_eq!((hit.rect.x, hit.rect.width), (8, 68), "84 → centered");
    }

    #[test]
    fn a_wide_band_publishes_the_note_cells_on_the_centered_column() {
        // The affordance rects must follow the measure column (`col.x`), not the
        // band's left edge — else clicks land off by `pad`.
        let mut app = App::new();
        push_bash_panel(&mut app, "ls", "/w", "ok");
        let terminal = render(&mut app, 120, 20);
        let (toggle, copy) = {
            let hit = app.hit.transcript.as_ref().expect("a transcript hit");
            assert_eq!(hit.rect.x, 26, "the measure is centered");
            let a = &hit.affordances[0];
            (a.toggle, a.copy)
        };
        let buf = terminal.backend().buffer();
        let glyph_col = toggle.x + toggle.width - 1;
        assert_eq!(
            buf[(glyph_col, toggle.y)].symbol(),
            "▸",
            "the ▸ glyph lands on the published cell"
        );
        assert_eq!(
            buf[(copy.x, copy.y)].symbol(),
            "▣",
            "the ▣ cell lands on the published cell"
        );
    }

    // ---- slice 5 (D33): thinking collapses on commit, expands on toggle ----

    /// Commit an assistant block whose content is a single thinking block.
    fn push_thinking(app: &mut App, text: &str) {
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::Thinking { text: text.into() }],
                    stop_reason: wcode_harness::message::StopReason::Stop,
                    usage: None,
                    model: None,
                },
            },
        ));
    }

    #[test]
    fn thinking_renders_one_line_until_expanded() {
        let mut app = App::new();
        let reasoning = "a reasoning paragraph that is long enough to wrap \
                         across more than one line when it finally expands";
        push_thinking(&mut app, reasoning);

        let collapsed = buffer_text(&render(&mut app, 80, 24));
        let rows: Vec<&str> = collapsed
            .lines()
            .filter(|l| l.contains("··· thinking"))
            .collect();
        assert_eq!(rows.len(), 1, "exactly one thinking row:\n{collapsed}");
        assert!(
            !rows[0].contains("chars"),
            "the row is quiet — no char count:\n{collapsed}"
        );
        assert!(
            !collapsed.contains("long enough to wrap"),
            "the body is hidden while collapsed:\n{collapsed}"
        );

        // Browse-select the block and expand via `Enter`.
        app.handle(AppEvent::Key(Key::Ctrl('g')));
        app.handle(AppEvent::Key(Key::Enter));
        let expanded = buffer_text(&render(&mut app, 80, 24));
        assert!(
            expanded.contains("long enough to wrap"),
            "the body shows when expanded:\n{expanded}"
        );
        assert!(expanded.contains('▾'), "the toggle glyph flips:\n{expanded}");
    }

    #[test]
    fn only_the_first_thinking_row_draws_the_affordance() {
        let mut app = App::new();
        // Two thinking contents in ONE committed block. Only the first is the
        // block's first line, so only it has a published hit region (the publish
        // requires `content.first()` to be `Thinking`); the second must not draw a
        // dead `▸ ▣`.
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageEnd {
                message: AgentMessage::Assistant {
                    content: vec![
                        ContentBlock::Thinking {
                            text: "first thought".into(),
                        },
                        ContentBlock::Thinking {
                            text: "second thought".into(),
                        },
                    ],
                    stop_reason: wcode_harness::message::StopReason::Stop,
                    usage: None,
                    model: None,
                },
            },
        ));
        let text = buffer_text(&render(&mut app, 80, 24));
        let rows: Vec<&str> = text.lines().filter(|l| l.contains("··· thinking")).collect();
        assert_eq!(rows.len(), 2, "two thinking rows:\n{text}");
        assert_eq!(text.matches('▸').count(), 1, "exactly one toggle glyph:\n{text}");
        assert_eq!(text.matches('▣').count(), 1, "exactly one copy glyph:\n{text}");
        assert!(rows[0].contains('▸'), "the first row draws it:\n{text}");
        assert!(!rows[1].contains('▸'), "the second row does not:\n{text}");
    }

    #[test]
    fn thinking_auto_collapses_on_turn_end() {
        let mut app = App::new();
        let reasoning = "reasoning that is visible only while the turn streams";
        // In-flight: a `MessageStart` (uncommitted) renders thinking inline.
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::MessageStart {
                message: AgentMessage::Assistant {
                    content: vec![ContentBlock::Thinking {
                        text: reasoning.into(),
                    }],
                    stop_reason: wcode_harness::message::StopReason::Stop,
                    usage: None,
                    model: None,
                },
            },
        ));
        let live = buffer_text(&render(&mut app, 80, 24));
        assert!(
            live.contains("visible only while the turn streams"),
            "in-flight thinking streams inline:\n{live}"
        );
        assert!(
            !live.contains("··· thinking"),
            "no collapsed row while in-flight:\n{live}"
        );

        // The turn ends: the block commits collapsed to the one-liner.
        push_thinking(&mut app, reasoning);
        let done = buffer_text(&render(&mut app, 80, 24));
        assert!(
            !done.contains("visible only while the turn streams"),
            "the body hides once committed:\n{done}"
        );
        assert!(
            done.contains("··· thinking"),
            "the committed block shows the one-liner:\n{done}"
        );
    }

    #[test]
    fn a_rendered_thinking_row_publishes_a_clickable_toggle() {
        let mut app = App::new();
        let reasoning = "reasoning that should appear once the row is clicked";
        push_thinking(&mut app, reasoning);
        let _ = render(&mut app, 80, 24);

        // The committed thinking row publishes exactly one affordance record.
        let (block, toggle) = {
            let hit = app.hit.transcript.as_ref().expect("a transcript hit");
            assert_eq!(
                hit.affordances.len(),
                1,
                "the thinking row publishes its affordance"
            );
            let a = &hit.affordances[0];
            (a.block, a.toggle)
        };
        assert!(matches!(&app.transcript()[block], Block::Turn(_)));

        // A click on the published toggle cell expands the row.
        app.handle(AppEvent::Mouse(MouseEvent {
            kind: MouseKind::Down,
            col: toggle.x + 1,
            row: toggle.y,
        }));
        app.handle(AppEvent::Mouse(MouseEvent {
            kind: MouseKind::Up,
            col: toggle.x + 1,
            row: toggle.y,
        }));
        let text = buffer_text(&render(&mut app, 80, 24));
        assert!(
            text.contains("should appear once the row is clicked"),
            "the click expands the thinking:\n{text}"
        );
    }

    // ---- slice 7 (D35): the todo checklist lives in the transcript ----

    #[test]
    fn todos_render_in_the_transcript() {
        let mut app = App::new();
        app.handle(AppEvent::Agent(
            root(),
            wcode_harness::event::AgentEvent::Todo {
                todos: vec![
                    TodoItem {
                        content: "map the seam".into(),
                        status: TodoStatus::Pending,
                    },
                    TodoItem {
                        content: "write the test".into(),
                        status: TodoStatus::Completed,
                    },
                    TodoItem {
                        content: "run the suite".into(),
                        status: TodoStatus::Pending,
                    },
                ],
            },
        ));
        let frame = buffer_text(&render(&mut app, 80, 24));
        assert!(
            frame.contains("── todos  1/3 ──"),
            "the todos header:\n{frame}"
        );
        assert!(
            frame.contains("☑ write the test"),
            "the completed row:\n{frame}"
        );
        assert!(frame.contains("☐ map the seam"), "a pending row:\n{frame}");
        assert!(frame.contains("☐ run the suite"), "a pending row:\n{frame}");
    }
}

