//! Markdown → transcript lines via `pulldown-cmark` (CommonMark + GFM tables,
//! strikethrough, task lists, footnotes). Fenced code is highlighted with
//! `syntect`; the wrap/table renderers are unchanged from the earlier
//! hand-rolled version — only tokenization moved to the event stream.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::theme::{self, ColorMode};
use crate::ui::{code_style, dim};

/// The 3-column gutter every assistant line shares.
const GUTTER: &str = "   ";

/// A markdown heading.
fn heading_style() -> Style {
    theme::theme().heading
}

/// A markdown link.
fn link_style() -> Style {
    theme::theme().link
}

/// Assistant prose — the uncolorized default.
fn body_style() -> Style {
    theme::theme().body
}

/// Render markdown `text` to transcript lines wrapped to `width`, under the
/// process color mode.
/// The pulldown-cmark options: CommonMark + GFM tables/strikethrough/task
/// lists/footnotes, with the source text preserved (no smart-punctuation) — D2.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_FOOTNOTES
}

/// One open list/quote level's gutter contribution (D4).
struct Gutter {
    /// Prefix on the level's first emitted line: `"• "` (bullet) or `"│ "`
    /// (quote). An ORDERED level RECOMPUTES it per item (`format!("{n}. ")`) —
    /// no literal number is stored (B2).
    first: String,
    /// Prefix on the level's other lines (marker-width pad): `"  "`, or a
    /// matching run of spaces for a recomputed ordered marker (B2).
    cont: String,
    /// The style this level imposes on its leaf content (quote → `quote_style()`).
    base: Style,
    /// Whether `first` has been consumed (its marker is already on a line).
    emitted: bool,
    /// For an ordered list: the NEXT item number (`Some`, seeded to `start - 1`
    /// by `list_level(Some(start))` so the first `Item`'s `+= 1` yields `start`);
    /// `None` for bullet/quote (B2).
    ordered_next: Option<u64>,
}

/// Which leaf block is being accumulated (D4).
#[derive(Clone, Copy)]
enum LeafKind {
    Paragraph,
    Heading,
    ItemText,
}

/// An open inline span (D5).
enum Inline {
    Emphasis,
    Strong,
    Link,
}

/// A fenced/indented code block in progress (D6).
struct CodeBlock {
    fence: Fenced,
    text: String,
}

impl CodeBlock {
    fn new(info: &str, mode: ColorMode) -> Self {
        Self {
            fence: Fenced::new(info, mode),
            text: String::new(),
        }
    }

    /// Render the collected code: one `Fenced::line` per source line (no `wrap`).
    fn render(self) -> Vec<Line<'static>> {
        let mut fence = self.fence;
        let body = self.text.strip_suffix('\n').unwrap_or(&self.text);
        body.split('\n').map(|line| fence.line(line)).collect()
    }
}

/// A table in progress (D7); `build()` yields the existing `Table`.
struct TableBuild {
    aligns: Vec<Align>,
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    cur_row: Vec<String>,
    cur_cell: String,
}

impl TableBuild {
    fn new(aligns: Vec<Alignment>) -> Self {
        Self {
            aligns: aligns
                .into_iter()
                .map(|a| match a {
                    Alignment::Center => Align::Center,
                    Alignment::Right => Align::Right,
                    // `Alignment::None` → `Align::Left` (pin #4).
                    Alignment::None | Alignment::Left => Align::Left,
                })
                .collect(),
            header: Vec::new(),
            rows: Vec::new(),
            cur_row: Vec::new(),
            cur_cell: String::new(),
        }
    }

    fn begin_row(&mut self) {
        self.cur_row.clear();
    }

    fn begin_cell(&mut self) {
        self.cur_cell.clear();
    }

    fn end_cell(&mut self) {
        self.cur_row.push(std::mem::take(&mut self.cur_cell));
    }

    /// `TableHead` is itself the header row (its cells are its direct children).
    fn end_head(&mut self) {
        self.header = std::mem::take(&mut self.cur_row);
    }

    fn end_row(&mut self) {
        self.rows.push(std::mem::take(&mut self.cur_row));
    }

    fn build(self) -> Table {
        Table {
            header: self.header,
            aligns: self.aligns,
            rows: self.rows,
        }
    }
}

/// The event walk (D4): accumulates the current leaf block's styled runs, a
/// gutter stack of open list/quote levels, and the completed output.
struct Blocks {
    /// Styled inline runs of the current leaf block (D4/D5).
    runs: Vec<(String, Style)>,
    /// Open inline spans, outermost first (D5).
    inline: Vec<Inline>,
    /// Open list/quote levels, outermost first (D4).
    stack: Vec<Gutter>,
    /// The current leaf's kind + base style, set on its `Start`.
    leaf: Option<(LeafKind, Style)>,
    /// A code block in progress (D6); `Some` swallows Text/SoftBreak.
    code: Option<CodeBlock>,
    /// A table in progress (D7); `Some` swallows Text into cells.
    table: Option<TableBuild>,
    /// Completed output.
    out: Vec<Line<'static>>,
    /// The wrap width — needed to FLUSH an open leaf on a block-level `Start`
    /// (B1), so the walk holds it rather than threading it through `on_start`.
    width: usize,
}

impl Blocks {
    fn new(width: usize) -> Self {
        Self {
            runs: Vec::new(),
            inline: Vec::new(),
            stack: Vec::new(),
            leaf: None,
            code: None,
            table: None,
            out: Vec::new(),
            width,
        }
    }

    /// `Event::Start` → FLUSH the open leaf (B1), then open a level
    /// (List/BlockQuote), begin a leaf (Paragraph/Heading), or enter code/table
    /// mode. `mode` is needed ONLY for the `Fenced` highlighter (D6).
    fn on_start(&mut self, tag: Tag, mode: ColorMode) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush_leaf();
                self.leaf = Some((LeafKind::Heading, heading_level_style(level as usize)));
            }
            Tag::Paragraph => {
                self.flush_leaf();
                let base = self.leaf_base();
                self.leaf = Some((LeafKind::Paragraph, base));
            }
            Tag::List(start) => {
                self.flush_leaf();
                self.stack.push(Self::list_level(start));
            }
            Tag::BlockQuote(_) => {
                self.flush_leaf();
                self.stack.push(Self::quote_level());
            }
            Tag::CodeBlock(kind) => {
                self.flush_leaf();
                let info = match &kind {
                    CodeBlockKind::Fenced(info) => info.as_ref(),
                    CodeBlockKind::Indented => "",
                };
                self.code = Some(CodeBlock::new(info, mode));
            }
            Tag::Table(aligns) => {
                self.flush_leaf();
                self.table = Some(TableBuild::new(aligns));
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.begin_row();
                }
            }
            Tag::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.begin_cell();
                }
            }
            Tag::Item => {
                // B2: an ordered level recomputes its marker per item.
                if let Some(top) = self.stack.last_mut() {
                    if let Some(next) = top.ordered_next {
                        let n = next + 1;
                        top.ordered_next = Some(n);
                        top.first = format!("{n}. ");
                        top.cont = " ".repeat(top.first.len());
                    }
                    top.emitted = false;
                }
            }
            Tag::Emphasis => self.inline.push(Inline::Emphasis),
            Tag::Strong => self.inline.push(Inline::Strong),
            Tag::Link { .. } | Tag::Image { .. } => self.inline.push(Inline::Link),
            _ => {}
        }
    }

    /// `Event::End` → close a level, FLUSH a leaf (`wrap` to the width), or emit
    /// a code block / table.
    fn on_end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item => self.flush_leaf(),
            TagEnd::List(_) | TagEnd::BlockQuote(_) => {
                self.stack.pop();
            }
            TagEnd::CodeBlock => {
                if let Some(code) = self.code.take() {
                    self.out.extend(code.render());
                }
            }
            TagEnd::TableCell => {
                if let Some(table) = self.table.as_mut() {
                    table.end_cell();
                }
            }
            TagEnd::TableHead => {
                if let Some(table) = self.table.as_mut() {
                    table.end_head();
                }
            }
            TagEnd::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.end_row();
                }
            }
            TagEnd::Table => {
                if let Some(table) = self.table.take() {
                    let width = self.width;
                    self.out.extend(table_lines(&table.build(), width));
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link | TagEnd::Image => {
                self.inline.pop();
            }
            _ => {}
        }
    }

    /// The remaining events: Text/Code/SoftBreak/HardBreak/Rule/
    /// TaskListMarker/FootnoteReference (D5). Code/table text is swallowed
    /// first (D6/D7).
    fn on_event(&mut self, ev: Event) {
        if self.code.is_some() || self.table.is_some() {
            let text = match &ev {
                Event::Text(t) | Event::Code(t) => t.as_ref(),
                Event::SoftBreak => " ",
                Event::HardBreak => "\n",
                _ => return,
            };
            if let Some(code) = self.code.as_mut() {
                code.text.push_str(text);
            } else if let Some(table) = self.table.as_mut() {
                table.cur_cell.push_str(text);
            }
            return;
        }
        match ev {
            Event::Text(t) => {
                let style = self.inline_style();
                self.push_run(&t, style);
            }
            Event::Code(t) => self.push_run(&t, code_style()),
            Event::SoftBreak => {
                let style = self.inline_style();
                self.push_run(" ", style);
            }
            Event::HardBreak => {
                let style = self.inline_style();
                self.push_run("\n", style);
            }
            Event::Rule => {
                let line = Self::rule_line();
                self.out.push(line);
            }
            Event::TaskListMarker(done) => {
                let style = self.inline_style();
                self.push_run(if done { "[x] " } else { "[ ] " }, style);
            }
            // Footnotes render their text only (Q1); HTML is a non-goal (§7).
            Event::FootnoteReference(_) | Event::Html(_) | Event::InlineHtml(_) => {}
            _ => {}
        }
    }

    /// Flush any trailing leaf and return the lines (defensive; the parser
    /// always closes its blocks).
    fn finish(mut self) -> Vec<Line<'static>> {
        self.flush_leaf();
        self.out
    }

    // ---- helpers ----

    /// Compose `(first, cont)` from the stack, CONSUMING each level's
    /// un-emitted `first` (so a marker appears once). `GUTTER` is prepended once.
    fn prefixes(&mut self) -> (String, String) {
        let mut first = String::new();
        let mut cont = String::new();
        for level in &mut self.stack {
            first.push_str(if level.emitted { &level.cont } else { &level.first });
            cont.push_str(&level.cont);
            level.emitted = true;
        }
        (format!("{GUTTER}{first}"), format!("{GUTTER}{cont}"))
    }

    /// The current leaf's base: its stored style, else the innermost level's
    /// `base`, else `body_style()`.
    fn leaf_base(&self) -> Style {
        if let Some((_, style)) = &self.leaf {
            return *style;
        }
        self.stack.last().map(|g| g.base).unwrap_or_else(body_style)
    }

    /// The style for the current inline context: body + Emphasis/Strong/Link.
    fn inline_style(&self) -> Style {
        let mut style = self.leaf_base();
        for span in &self.inline {
            match span {
                Inline::Emphasis => style = style.add_modifier(Modifier::ITALIC),
                Inline::Strong => style = style.add_modifier(Modifier::BOLD),
                Inline::Link => style = link_style(),
            }
        }
        style
    }

    /// Append a run, opening an implicit `ItemText` leaf if none is open
    /// (tight-list text arrives with no `Paragraph`).
    fn push_run(&mut self, text: &str, style: Style) {
        if self.leaf.is_none() {
            let base = self.leaf_base();
            self.leaf = Some((LeafKind::ItemText, base));
        }
        self.runs.push((text.to_string(), style));
    }

    /// Split `runs` on the `"\n"` hard-break sentinel and `wrap` each segment
    /// (first segment with `first`, the rest with `cont`) into `out`.
    fn flush_leaf(&mut self) {
        if self.runs.is_empty() {
            self.leaf = None;
            return;
        }
        let base = self.leaf_base();
        let (first, cont) = self.prefixes();
        let width = self.width;
        let mut segment: Vec<(String, Style)> = Vec::new();
        for (text, style) in std::mem::take(&mut self.runs) {
            if text == "\n" {
                if !segment.is_empty() {
                    self.out
                        .extend(wrap(std::mem::take(&mut segment), width, &first, &cont, base));
                }
            } else {
                segment.push((text, style));
            }
        }
        if !segment.is_empty() {
            self.out.extend(wrap(segment, width, &first, &cont, base));
        }
        self.leaf = None;
    }

    /// Build a list level. `Some(start)` seeds `ordered_next = start - 1`
    /// (so the first `Item` yields `start`); `None` is a bullet (B2/D8).
    fn list_level(start: Option<u64>) -> Gutter {
        match start {
            Some(n) => Gutter {
                first: format!("{n}. "),
                cont: " ".repeat(format!("{n}. ").len()),
                base: body_style(),
                emitted: false,
                ordered_next: Some(n.saturating_sub(1)),
            },
            None => Gutter {
                first: "• ".to_string(),
                cont: "  ".to_string(),
                base: body_style(),
                emitted: false,
                ordered_next: None,
            },
        }
    }

    /// Build a quote level (`first = "│ "`, `cont = "  "`, `base = quote_style()`).
    fn quote_level() -> Gutter {
        Gutter {
            first: "│ ".to_string(),
            cont: "  ".to_string(),
            base: quote_style(),
            emitted: false,
            ordered_next: None,
        }
    }

    /// `Event::Rule` → a dim thematic break.
    fn rule_line() -> Line<'static> {
        Line::from(Span::styled(format!("{GUTTER}{}", "─".repeat(24)), dim()))
    }
}

pub fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    render_mode(text, width, theme::color_mode())
}

/// The color-mode seam: `render` delegates here so tests can pin a mode (the
/// process-wide `color_mode()` is a `OnceLock`, not settable per test).
pub(crate) fn render_mode(text: &str, width: usize, mode: ColorMode) -> Vec<Line<'static>> {
    let mut blocks = Blocks::new(width);
    for ev in Parser::new_ext(text, options()) {
        match ev {
            Event::Start(tag) => blocks.on_start(tag, mode),
            Event::End(tag) => blocks.on_end(tag),
            other => blocks.on_event(other),
        }
    }
    blocks.finish()
}

/// The bundled syntax definitions, loaded once (match `theme.rs`'s install-once
/// style).
static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();

/// The bundled dark syntax theme, loaded once.
static SYNTAX_THEME: OnceLock<Theme> = OnceLock::new();

/// The bundled syntax set, loaded once — no-newline variants, since `render`
/// feeds one trimmed line at a time.
fn syntaxes() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_nonewlines)
}

/// The syntax theme, resolved once. B3: a TOTAL lookup — a missing/changed key
/// falls back to an empty theme, never panics in a render path.
fn syntax_theme() -> &'static Theme {
    SYNTAX_THEME.get_or_init(|| {
        ThemeSet::load_defaults()
            .themes
            .get("base16-ocean.dark")
            .cloned()
            .unwrap_or_default()
    })
}

/// A fence's state for one ```` ``` ```` block: always present inside a fence;
/// only the highlighter is optional (B1). A bare fence or a non-`Rgb` mode is
/// still a code block — just unhighlighted.
struct Fenced {
    hl: Option<HighlightLines<'static>>,
}

impl Fenced {
    /// Infallible: build the highlighter only under `Rgb` with a resolved syntax.
    fn new(info: &str, mode: ColorMode) -> Self {
        let hl = (mode == ColorMode::Rgb)
            .then(|| fence_syntax(info))
            .flatten()
            .map(|syn| HighlightLines::new(syn, syntax_theme()));
        Self { hl }
    }

    /// One fenced line: highlighted when possible, else the uniform `code_line`.
    fn line(&mut self, text: &str) -> Line<'static> {
        match self.hl.as_mut() {
            Some(hl) => highlight_line(hl, text),
            None => code_line(text),
        }
    }
}

/// B3: TOTAL. `find_syntax_by_token` then `find_syntax_by_extension`; `None` for a
/// bare or unknown info string (the caller renders uniformly).
/// bare or unknown info string (the caller renders uniformly).
fn fence_syntax(info: &str) -> Option<&'static SyntaxReference> {
    let ss = syntaxes();
    ss.find_syntax_by_token(info)
        .or_else(|| ss.find_syntax_by_extension(info))
}

/// syntect token style → ratatui style, honoring wcode's color-mode ladder. Under
/// `Rgb`: the exact `Color::Rgb` + font modifiers; under `Plain`/`Named`/`Indexed`:
/// the uniform `code_style()` (256/16-color quantization is a follow-up).
fn highlight_style(s: SynStyle, mode: ColorMode) -> Style {
    if mode != ColorMode::Rgb {
        return code_style();
    }
    let fg = s.foreground;
    let mut style = Style::default().fg(Color::Rgb(fg.r, fg.g, fg.b));
    if s.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if s.font_style.contains(FontStyle::UNDERLINE) {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

/// One highlighted fenced line: the dim `{GUTTER}│ ` gutter + token spans (never
/// wrapped). `hl` exists only under `ColorMode::Rgb` (see [`Fenced::new`]).
fn highlight_line(hl: &mut HighlightLines<'static>, text: &str) -> Line<'static> {
    let mut spans = vec![Span::styled(format!("{GUTTER}│ "), dim())];
    match hl.highlight_line(text, syntaxes()) {
        Ok(ranges) => {
            for (style, piece) in ranges {
                spans.push(Span::styled(
                    piece.to_string(),
                    highlight_style(style, ColorMode::Rgb),
                ));
            }
        }
        // A highlight failure must never panic in a render path — fall back.
        Err(_) => spans.push(Span::styled(text.to_string(), code_style())),
    }
    Line::from(spans)
}
fn code_line(line: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{GUTTER}│ "), dim()),
        Span::styled(line.to_string(), code_style()),
    ])
}

/// The style for a heading `level`: level 1 is the primary heading, deeper levels
/// step down to the quieter `heading_sub`.
/// step down to the quieter `heading_sub`.
fn heading_level_style(level: usize) -> Style {
    match level {
        1 => heading_style(),
        _ => theme::theme().heading_sub,
    }
}

/// A blockquote body is muted; its `│ ` gutter is `dim()` via `line_with`.
fn quote_style() -> Style {
    theme::theme().muted
}
// --- tables -----------------------------------------------------------------

/// A GFM table: a header row, an alignment row, and body rows.
struct Table {
    header: Vec<String>,
    aligns: Vec<Align>,
    rows: Vec<Vec<String>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Align {
    Left,
    Right,
    Center,
}

fn table_lines(table: &Table, width: usize) -> Vec<Line<'static>> {
    let cols = table
        .header
        .len()
        .max(table.rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut widths = vec![0usize; cols];
    for (c, cell) in table.header.iter().enumerate() {
        widths[c] = widths[c].max(disp(cell));
    }
    for row in &table.rows {
        for (c, cell) in row.iter().enumerate().take(cols) {
            widths[c] = widths[c].max(disp(cell));
        }
    }

    // Shrink the widest column until the row (with ` │ ` joins and the gutter)
    // fits, never below 3 cells; longer cells then wrap within the column.
    let overhead = disp(GUTTER) + 3 * cols.saturating_sub(1);
    let budget = width.saturating_sub(overhead);
    let mut total: usize = widths.iter().sum();
    while total > budget {
        let Some((widest, w)) = widths.iter().enumerate().max_by_key(|(_, w)| **w) else {
            break;
        };
        if *w <= 3 {
            break;
        }
        widths[widest] -= 1;
        total -= 1;
    }

    let mut out = vec![row_lines(
        &table.header,
        &widths,
        &table.aligns,
        heading_style(),
    )];
    let rule = widths
        .iter()
        .map(|w| "─".repeat(*w))
        .collect::<Vec<_>>()
        .join("─┼─");
    out.push(vec![Line::from(Span::styled(
        format!("{GUTTER}{rule}"),
        dim(),
    ))]);
    for row in &table.rows {
        out.push(row_lines(row, &widths, &table.aligns, body_style()));
    }
    out.into_iter().flatten().collect()
}

/// One table row, wrapping each cell to its column width and emitting as many
/// physical lines as the tallest cell needs.
fn row_lines(cells: &[String], widths: &[usize], aligns: &[Align], style: Style) -> Vec<Line<'static>> {
    let wrapped: Vec<Vec<String>> = widths
        .iter()
        .enumerate()
        .map(|(c, w)| wrap_cell(cells.get(c).map(String::as_str).unwrap_or(""), *w))
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);

    let mut out = Vec::with_capacity(height);
    for i in 0..height {
        let parts: Vec<String> = widths
            .iter()
            .enumerate()
            .map(|(c, w)| {
                let segment = wrapped[c].get(i).map(String::as_str).unwrap_or("");
                let align = aligns.get(c).copied().unwrap_or(Align::Left);
                pad(segment, *w, align)
            })
            .collect();
        out.push(Line::from(Span::styled(
            format!("{GUTTER}{}", parts.join(" │ ")),
            style,
        )));
    }
    out
}

/// Wrap a cell's text to `width`; a cell that fits stays on one line.
fn wrap_cell(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut atoms = Vec::new();
    for word in text.split_whitespace() {
        atoms.extend(hard_break(word, width));
    }
    if atoms.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut cur = String::new();
    for atom in atoms {
        if cur.is_empty() {
            cur = atom;
        } else if disp(&cur) + 1 + disp(&atom) <= width {
            cur.push(' ');
            cur.push_str(&atom);
        } else {
            out.push(std::mem::take(&mut cur));
            cur = atom;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn pad(text: &str, width: usize, align: Align) -> String {
    let gap = width.saturating_sub(disp(text));
    match align {
        Align::Left => format!("{text}{}", " ".repeat(gap)),
        Align::Right => format!("{}{text}", " ".repeat(gap)),
        Align::Center => {
            let left = gap / 2;
            format!("{}{text}{}", " ".repeat(left), " ".repeat(gap - left))
        }
    }
}

/// Split `word` into `width`-wide pieces so it can wrap instead of overflowing.
fn hard_break(word: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if disp(word) <= width {
        return vec![word.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = word;
    while disp(rest) > width {
        let chunk: String = rest.chars().take(width).collect();
        rest = &rest[chunk.len()..];
        out.push(chunk);
    }
    if !rest.is_empty() {
        out.push(rest.to_string());
    }
    out
}

/// Display width (characters; wide glyphs are treated as one cell for now).
fn disp(text: &str) -> usize {
    text.chars().count()
}

/// Greedy word-wrap styled runs, prefixing the first line with `first` and
/// continuations with `cont`. `base` styles the prefix.
/// continuations with `cont`. `base` styles the prefix.
fn wrap(
    runs: Vec<(String, Style)>,
    width: usize,
    first: &str,
    cont: &str,
    base: Style,
) -> Vec<Line<'static>> {
    // Model each atom as `(text, style, space_before)`. A style change splits the
    // atom (a word keeps ONE style); `space_before` records whether the SOURCE had
    // >=1 whitespace immediately before it, so a run boundary is not blindly a
    // space (`see `foo`.` stays `see foo.`, `a**b**c` stays `abc`).
    let mut words: Vec<(String, Style, bool)> = Vec::new();
    let mut cur = String::new();
    let mut cur_style = base;
    let mut space_pending = false; // whitespace seen since the last atom
    for (text, style) in runs {
        for ch in text.chars() {
            if ch.is_whitespace() {
                if !cur.is_empty() {
                    words.push((
                        std::mem::take(&mut cur),
                        cur_style,
                        !words.is_empty() && space_pending,
                    ));
                    // The whitespace below re-arms `space_pending`.
                }
                space_pending = true; // >=1 space collapses to one
            } else {
                if cur.is_empty() {
                    cur_style = style;
                }
                cur.push(ch);
            }
        }
        if !cur.is_empty() {
            // Flush the run's tail so a style change ends the atom.
            words.push((
                std::mem::take(&mut cur),
                cur_style,
                !words.is_empty() && space_pending,
            ));
            space_pending = false;
        }
    }

    let mut lines = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0usize;
    let mut prefix = first.to_string();
    let mut avail = width.saturating_sub(disp(&prefix)).max(1);
    // Break oversized words to the *narrowest* line so a piece always fits.
    let max_word = width
        .saturating_sub(disp(first).max(disp(cont)))
        .max(1);

    for (word, style, space_before) in words {
        for piece in hard_break(&word, max_word) {
            let width_of = disp(&piece);
            // Only a source-separated atom reserves a column for the separator.
            let gap = usize::from(!spans.is_empty() && space_before);
            if used + gap + width_of > avail && !spans.is_empty() {
                lines.push(line_with(&prefix, std::mem::take(&mut spans), base));
                prefix = cont.to_string();
                avail = width.saturating_sub(disp(&prefix)).max(1);
                used = 0;
            }
            if !spans.is_empty() && space_before {
                spans.push(Span::styled(" ", style));
                used += 1;
            }
            spans.push(Span::styled(piece, style));
            used += width_of;
        }
    }
    lines.push(line_with(&prefix, spans, base));
    lines
}

fn line_with(prefix: &str, spans: Vec<Span<'static>>, base: Style) -> Line<'static> {
    let mut all = Vec::with_capacity(spans.len() + 1);
    all.push(Span::styled(prefix.to_string(), dim()));
    all.extend(spans.into_iter().map(|s| Span {
        style: if s.style == Style::default() {
            base
        } else {
            s.style
        },
        ..s
    }));
    Line::from(all)
}

#[cfg(test)]
mod tests {
    use super::*;


    #[test]
    fn heading_levels_differ() {
        let lines = render("# h1\n## h2\n###### h6", 40);
        assert_eq!(text_of(&lines), ["   h1", "   h2", "   h6"]);
        let style_of = |line: &Line| line.spans.last().unwrap().style;
        assert_ne!(
            style_of(&lines[0]),
            style_of(&lines[1]),
            "a level-1 heading differs from a level-2"
        );
        assert_eq!(
            style_of(&lines[1]),
            style_of(&lines[2]),
            "levels 2..=6 share the `heading_sub` tier"
        );
    }

    #[test]
    fn ordered_lists_renumber_and_align_continuations() {
        // CommonMark renumbers from the list's start (1, 2, 3 …), not the literals.
        let text = text_of(&render("1. a\n2. b\n10. c", 40));
        assert_eq!(text, ["   1. a", "   2. b", "   3. c"]);

        // A wrapped item's continuation aligns under the text (real marker width).
        let lines = render("10. one two three four five six", 20);
        assert!(lines.len() >= 2, "the item wraps");
        assert_eq!(lines[0].spans[0].content.as_ref(), "   10. ");
        assert_eq!(
            lines[1].spans[0].content.as_ref(),
            format!("{GUTTER}{}", " ".repeat(4)),
            "the continuation pads to the marker width"
        );
    }

    #[test]
    fn blockquotes_get_a_gutter() {
        let lines = render("> quoted", 40);
        assert_eq!(text_of(&lines), ["   │ quoted"]);
        assert_eq!(lines[0].spans[0].content.as_ref(), "   │ ");
        assert_eq!(
            lines[0].spans[1].style,
            theme::theme().muted,
            "the quote body is muted"
        );

        // Nesting: one `│ ` per `>` level.
        assert_eq!(text_of(&render("> > deep", 40)), ["   │ │ deep"]);
    }

    #[test]
    fn nested_bullets_indent_under_the_parent() {
        let text = text_of(&render("- a\n  - b\n    - c", 40));
        assert_eq!(text, ["   • a", "     • b", "       • c"]);
    }

    #[test]
    fn italic_and_bold_disambiguate() {
        let lines = render("*i* and **b**", 40);
        let spans: Vec<&Span> = lines.iter().flat_map(|l| l.spans.iter()).collect();
        assert!(
            spans
                .iter()
                .any(|s| s.content.as_ref() == "i"
                    && s.style.add_modifier.contains(Modifier::ITALIC)),
            "`*i*` is italic: {spans:?}"
        );
        assert!(
            spans
                .iter()
                .any(|s| s.content.as_ref() == "b"
                    && s.style.add_modifier.contains(Modifier::BOLD)),
            "`**b**` is bold (matched before italic): {spans:?}"
        );
    }

    #[test]
    fn underscores_inside_identifiers_stay_plain() {
        // C2: the `_x_` word-boundary rule keeps identifiers plain.
        let lines = render("foo_bar_baz", 40);
        assert_eq!(text_of(&lines), ["   foo_bar_baz"]);
        assert!(
            lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)),
            "snake_case must not italicize: {lines:?}"
        );

        // A `__` run (a dunder) is STRONG (CommonMark), never italic.
        for (dunder, inner) in [("__init__", "init"), ("__dunder__", "dunder")] {
            let lines = render(dunder, 40);
            assert_eq!(text_of(&lines), [format!("   {inner}")]);
            let spans: Vec<&Span> = lines.iter().flat_map(|l| l.spans.iter()).collect();
            assert!(
                spans
                    .iter()
                    .all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)),
                "`{dunder}` must not italicize: {lines:?}"
            );
            assert!(
                spans
                    .iter()
                    .any(|s| s.style.add_modifier.contains(Modifier::BOLD)),
                "`{dunder}` is strong: {lines:?}"
            );
        }

        // A properly delimited `_x_` still italicizes.
        let delimited = render("a _word_ b", 40);
        assert!(
            delimited
                .iter()
                .flat_map(|l| l.spans.iter())
                .any(|s| s.content.as_ref() == "word"
                    && s.style.add_modifier.contains(Modifier::ITALIC)),
            "`_word_` italicizes: {delimited:?}"
        );
    }

    #[test]
    fn spaced_asterisks_stay_plain() {
        let lines = render("a * b * c", 40);
        assert_eq!(text_of(&lines), ["   a * b * c"]);
        assert!(
            lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)),
            "`a * b * c` must not italicize: {lines:?}"
        );
    }


    #[test]
    fn seven_hashes_are_not_a_heading() {
        let lines = render("####### x", 40);
        assert_eq!(text_of(&lines), ["   ####### x"]);
        assert_ne!(
            lines[0].spans.last().unwrap().style,
            heading_level_style(1),
            "7 hashes is a paragraph, not a heading"
        );
    }
    #[test]
    fn inline_run_boundary_keeps_no_space() {
        assert_eq!(text_of(&render("see `foo`.", 40)), ["   see foo."]);
    }

    #[test]
    fn glued_runs_stay_glued() {
        assert_eq!(text_of(&render("a**b**c", 40)), ["   abc"]);
    }

    #[test]
    fn punctuation_after_italic_keeps_no_space() {
        assert_eq!(text_of(&render("*i*,", 40)), ["   i,"]);
    }

    #[test]
    fn a_real_space_between_words_is_preserved() {
        assert_eq!(text_of(&render("one two", 40)), ["   one two"]);
        // A wrap point still inserts exactly one space.
        let lines = render("one two three four five six seven", 20);
        let rejoined = text_of(&lines)
            .iter()
            .map(|l| l.trim_start().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, "one two three four five six seven");
    }

    #[test]
    fn multiple_spaces_collapse_to_one() {
        assert_eq!(text_of(&render("one   two", 40)), ["   one two"]);
    }

    #[test]
    fn a_cross_run_space_is_kept_once_in_either_direction() {
        // A trailing space on run k, a leading space on run k+1 — exactly one space.
        assert_eq!(text_of(&render("a `b`", 40)), ["   a b"]);
        assert_eq!(text_of(&render("`x` y", 40)), ["   x y"]);
    }

    #[test]
    fn a_glued_oversized_word_breaks_mid_word_without_separators() {
        // `a**verylongbold**c` is three glued atoms; a narrow render may break
        // between them (accepted) but must insert no separator and stay in width.
        let lines = render("a**verylongbold**c", 12);
        for line in &lines {
            assert!(width_of(line) <= 12, "too long: {line:?}");
        }
        let joined: String = text_of(&lines)
            .iter()
            .map(|l| l.trim_start().to_string())
            .collect();
        assert_eq!(joined, "averylongboldc", "no separators between glued atoms");
    }

    /// The number of distinct styles in a sequence.
    fn distinct_styles(styles: impl IntoIterator<Item = Style>) -> usize {
        let mut seen: Vec<Style> = Vec::new();
        for s in styles {
            if !seen.contains(&s) {
                seen.push(s);
            }
        }
        seen.len()
    }

    #[test]
    fn a_rust_fence_has_multiple_distinct_styles() {
        let lines = render_mode("```rust\nlet x = 1; // c\n```", 60, ColorMode::Rgb);
        let count = distinct_styles(
            lines
                .iter()
                .flat_map(|l| l.spans.iter().skip(1))
                .map(|s| s.style),
        );
        assert!(
            count >= 2,
            "a Rust fence yields >= 2 distinct token styles: {lines:?}"
        );
    }

    #[test]
    fn unknown_or_absent_language_is_uniform_and_guttered() {
        for md in ["```\nplain text\n```", "```nope\nplain text\n```"] {
            let lines = render_mode(md, 60, ColorMode::Rgb);
            assert_eq!(lines.len(), 1, "{md:?}");
            assert_eq!(
                lines[0].spans[0].content.as_ref(),
                "   │ ",
                "the gutter proves it is a code block, not reparsed prose: {md:?}"
            );
            assert!(
                lines[0].spans.iter().skip(1).all(|s| s.style == code_style()),
                "an unknown/absent language is uniform: {md:?}"
            );
        }
    }

    #[test]
    fn a_non_rgb_mode_still_renders_a_code_block() {
        // B1 regression guard: a non-Rgb mode is still a fence, never prose.
        let lines = render_mode("```rust\nfn main() {}\n```", 60, ColorMode::Named);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].spans[0].content.as_ref(), "   │ ");
        assert!(
            lines[0].spans.iter().skip(1).all(|s| s.style == code_style()),
            "a non-Rgb fence falls back to the uniform code style"
        );
    }

    #[test]
    fn an_unclosed_fence_at_eof_renders_as_code() {
        let lines = render_mode("```rust\nlet x = 1;", 60, ColorMode::Rgb);
        assert_eq!(lines.len(), 1, "the unclosed body renders as one code line");
        assert_eq!(lines[0].spans[0].content.as_ref(), "   │ ");
        assert!(text_of(&lines)[0].contains("let x = 1;"));
    }

    #[test]
    fn highlight_style_follows_the_color_mode() {
        let s = syntect::highlighting::Style {
            foreground: syntect::highlighting::Color {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            },
            background: syntect::highlighting::Color {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            },
            font_style: FontStyle::BOLD,
        };
        assert_eq!(
            highlight_style(s, ColorMode::Rgb).fg,
            Some(Color::Rgb(1, 2, 3))
        );
        for mode in [ColorMode::Plain, ColorMode::Named, ColorMode::Indexed] {
            assert_eq!(
                highlight_style(s, mode),
                code_style(),
                "under {mode:?} the token falls back to the uniform code style"
            );
        }
    }
    fn text_of(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    fn width_of(line: &Line) -> usize {
        line.spans.iter().map(|s| s.content.chars().count()).sum()
    }

    #[test]
    fn headings_and_bullets_get_markers() {
        let text = text_of(&render("# Title\n- one\n- two", 40));
        assert_eq!(text[0], "   Title");
        assert_eq!(text[1], "   • one");
        assert_eq!(text[2], "   • two");
    }

    #[test]
    fn fenced_code_is_marked_and_not_wrapped_inline() {
        let text = text_of(&render("text\n```rust\nlet x = 1; // **not bold**\n```", 60));
        assert_eq!(text[0], "   text");
        assert_eq!(text[1], "   │ let x = 1; // **not bold**");
    }

    #[test]
    fn inline_code_and_bold_split_into_spans() {
        let lines = render("a `b` and **c**", 40);
        let styles: Vec<_> = lines[0].spans.iter().map(|s| s.style).collect();
        assert!(styles.iter().any(|s| s.add_modifier.contains(Modifier::BOLD)));
        assert!(styles.iter().any(|s| *s == code_style()));
    }

    #[test]
    fn tables_render_with_alignment() {
        let md = "| name | qty |\n|:-----|----:|\n| a | 1 |\n| bb | 22 |";
        let text = text_of(&render(md, 60));
        assert_eq!(text[0], "   name │ qty");
        assert!(text[1].starts_with("   ─") && text[1].contains('┼'));
        assert_eq!(text[2], "   a    │   1");
        assert_eq!(text[3], "   bb   │  22");
    }

    #[test]
    fn long_cells_wrap_instead_of_truncating() {
        let md = "| key | value |\n|---|---|\n| a | one two three four five |";
        let lines = render(md, 24);
        for line in &lines {
            assert!(width_of(line) <= 24, "too long: {line:?}");
        }
        // Every word survives across the row's wrapped lines.
        let row: Vec<String> = text_of(&lines)[2..]
            .iter()
            .flat_map(|s| s.split_whitespace().map(str::to_string))
            .collect();
        for word in ["a", "one", "two", "three", "four", "five"] {
            assert!(row.iter().any(|w| w == word), "lost {word} in {row:?}");
        }
    }

    #[test]
    fn a_word_longer_than_the_width_hard_breaks() {
        let long = "a".repeat(50);
        let lines = render(&long, 20);
        for line in &lines {
            assert!(width_of(line) <= 20, "too long: {}", width_of(line));
        }
        let joined: String = text_of(&lines)
            .iter()
            .map(|l| l.trim_start().to_string())
            .collect();
        assert_eq!(joined, long);
    }

    #[test]
    fn long_paragraphs_wrap_to_width() {
        let lines = render("one two three four five six seven", 20);
        for line in &lines {
            assert!(width_of(line) <= 20);
        }
        let rejoined: String = text_of(&lines)
            .iter()
            .map(|l| l.trim_start().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, "one two three four five six seven");
    }

    #[test]
    fn links_render_the_label_in_the_link_style() {
        let lines = render("see [docs](https://example.com/x) now", 60);
        let text = text_of(&lines).join(" ");
        assert!(text.contains("docs"), "label missing: {text}");
        assert!(!text.contains("https://"), "the url should not be shown: {text}");
        let link = link_style();
        assert!(
            lines
                .iter()
                .flat_map(|l| l.spans.iter())
                .any(|s| s.style == link && s.content.as_ref() == "docs"),
            "the label is not in the link style: {lines:?}"
        );
    }

    #[test]
    fn task_lists_render_a_checkbox() {
        let text = text_of(&render("- [ ] todo\n- [x] done", 40));
        assert_eq!(text.len(), 2, "{text:?}");
        assert!(text[0].contains("[ ] todo"), "{text:?}");
        assert!(text[1].contains("[x] done"), "{text:?}");
    }

    #[test]
    fn thematic_break_renders_a_rule() {
        let text = text_of(&render("a\n\n---\n\nb", 40));
        assert_eq!(text.len(), 3, "{text:?}");
        assert_eq!(text[0], "   a");
        assert!(text[1].starts_with("   ─"), "{text:?}");
        assert_eq!(text[2], "   b");
    }
}
