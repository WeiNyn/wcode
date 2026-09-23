//! A small markdown renderer for assistant messages: fenced code blocks,
//! headings, bullets, tables, and inline `code` / **bold**. Deliberately small —
//! the transcript needs readable prose and code, not a spec-complete parser.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{FontStyle, Style as SynStyle, Theme, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

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
pub fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    render_mode(text, width, theme::color_mode())
}

/// The color-mode seam: `render` delegates here so tests can pin a mode (the
/// process-wide `color_mode()` is a `OnceLock`, not settable per test).
pub(crate) fn render_mode(text: &str, width: usize, mode: ColorMode) -> Vec<Line<'static>> {
    let raw: Vec<&str> = text.split('\n').collect();
    let mut lines = Vec::new();
    // The stateful highlighter lives for exactly one fence (B1: always `Some`
    // inside a fence; only its highlighter is optional).
    let mut fence: Option<Fenced> = None;
    let mut i = 0;

    while i < raw.len() {
        let line = raw[i].trim_end();

        if let Some(f) = fence.as_mut() {
            if is_fence(line) {
                fence = None; // close: drop the highlighter
            } else {
                lines.push(f.line(line)); // stateful: feeds THIS fenced line
            }
            i += 1;
            continue;
        }
        if is_fence(line) {
            // B2: a bare ``` still opens a fence; `Fenced::new` is infallible (B1).
            fence = Some(Fenced::new(fence_info(line).unwrap_or(""), mode));
            i += 1;
            continue;
        }
        if let Some((table, next)) = table_at(&raw, i) {
            lines.extend(table_lines(&table, width));
            i = next;
            continue;
        }
        // Order: indentation first (so `  - x` is a nested bullet), then quote
        // (wins over heading/bullet), heading, ordered, bullet, blank, paragraph.
        let (level, rest) = leading_indent(line);
        if let Some((body, depth)) = quote(rest) {
            let (first, cont) = quote_gutter(level, depth);
            lines.extend(wrap(inline(body), width, &first, &cont, quote_style()));
        } else if let Some((lvl, text)) = heading(rest) {
            let gutter = indent_prefix(level);
            lines.extend(wrap(
                inline(text),
                width,
                &gutter,
                &gutter,
                heading_level_style(lvl),
            ));
        } else if let Some((marker, body)) = ordered(rest) {
            let pad = " ".repeat(marker.chars().count());
            let first = format!("{}{marker}", indent_prefix(level));
            let cont = format!("{}{pad}", indent_prefix(level));
            lines.extend(wrap(inline(body), width, &first, &cont, body_style()));
        } else if let Some(item) = bullet(rest) {
            let first = format!("{}• ", indent_prefix(level));
            let cont = format!("{}  ", indent_prefix(level));
            lines.extend(wrap(inline(item), width, &first, &cont, body_style()));
        } else if rest.is_empty() {
            lines.push(Line::default());
        } else {
            let gutter = indent_prefix(level);
            lines.extend(wrap(inline(rest), width, &gutter, &gutter, body_style()));
        }
        i += 1;
    }
    lines
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

/// True for any ```` ``` ```` line (open or close), bare or tagged.
fn is_fence(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

/// The token after the backticks, trimmed: ```` ```rust ```` → `Some("rust")`,
/// ```` ``` ```` → `None`. Takes the FIRST whitespace/comma-delimited word, so
/// ```` ```rust,no_run ```` / ```` ```rust ignore ```` still resolve.
fn fence_info(line: &str) -> Option<&str> {
    let after = line.trim_start().strip_prefix("```")?;
    let token = after
        .trim_start()
        .split([' ', '\t', ','])
        .next()
        .unwrap_or("");
    (!token.is_empty()).then_some(token)
}

/// B3: TOTAL. `find_syntax_by_token` then `find_syntax_by_extension`; `None` for a
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

/// `- item` / `* item` / `+ item` → `item`.
fn bullet(line: &str) -> Option<&str> {
    let text = line.trim_start();
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| text.strip_prefix(marker))
}

/// Split leading indentation: `(level, remainder)`. Each 2 columns — a space, or a
/// tab counted as two — is one level; an odd trailing space floors. `remainder`
/// carries no leading whitespace.
fn leading_indent(line: &str) -> (usize, &str) {
    let rest = line.trim_start_matches([' ', '\t']);
    let cols = line[..line.len() - rest.len()]
        .chars()
        .map(|c| if c == '\t' { 2 } else { 1 })
        .sum::<usize>();
    (cols / 2, rest)
}

/// `# Title` (one to six `#`) → `(level, "Title")`; the `#` marker is dropped.
/// Seven-or-more hashes are not a heading.
fn heading(line: &str) -> Option<(usize, &str)> {
    let text = line.trim_start();
    let hashes = text.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && text[hashes..].starts_with(' ') {
        Some((hashes, text[hashes..].trim_start()))
    } else {
        None
    }
}

/// The style for a heading `level`: level 1 is the primary heading, deeper levels
/// step down to the quieter `heading_sub`.
fn heading_level_style(level: usize) -> Style {
    match level {
        1 => heading_style(),
        _ => theme::theme().heading_sub,
    }
}

/// `1. ` / `2) ` / `10. ` (1+ digits, then `. ` or `) `) → `(marker, body)`. The
/// `marker` keeps its number and trailing space so a wrapped continuation aligns
/// under the item text.
fn ordered(line: &str) -> Option<(&str, &str)> {
    let text = line.trim_start();
    let digits = text.chars().take_while(|c| c.is_ascii_digit()).count();
    let after = &text[digits..];
    if digits == 0 || !(after.starts_with(". ") || after.starts_with(") ")) {
        return None;
    }
    Some((&text[..digits + 2], &text[digits + 2..]))
}

/// `> text` → `(body, depth)`; each leading `>` (with an optional single space) adds
/// a level, so `> > x` is depth 2.
fn quote(line: &str) -> Option<(&str, usize)> {
    let mut rest = line;
    let mut depth = 0;
    while let Some(after) = rest.strip_prefix('>') {
        depth += 1;
        rest = after.strip_prefix(' ').unwrap_or(after);
    }
    (depth > 0).then_some((rest, depth))
}

/// `{GUTTER}` + 2·`level` spaces — the prefix/continuation base for indented content.
fn indent_prefix(level: usize) -> String {
    format!("{GUTTER}{}", " ".repeat(2 * level))
}

/// `(prefix, continuation)` for a blockquote at `level`/`depth`: one `│ ` per depth,
/// with a matching run of spaces on wrapped continuations.
fn quote_gutter(level: usize, depth: usize) -> (String, String) {
    let base = indent_prefix(level);
    (
        format!("{base}{}", "│ ".repeat(depth)),
        format!("{base}{}", "  ".repeat(depth)),
    )
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

/// A table starting at `lines[start]` (`header` + `|---|` delimiter), returning
/// it and the index just past its last row.
fn table_at(lines: &[&str], start: usize) -> Option<(Table, usize)> {
    let header = *lines.get(start)?;
    let delimiter = *lines.get(start + 1)?;
    if !header.contains('|') || !is_delimiter(delimiter) {
        return None;
    }
    let (header, aligns) = (split_row(header), parse_aligns(delimiter));

    let mut rows = Vec::new();
    let mut i = start + 2;
    while let Some(row) = lines.get(i) {
        if !row.contains('|') || row.trim().is_empty() {
            break;
        }
        rows.push(split_row(row));
        i += 1;
    }
    Some((Table { header, aligns, rows }, i))
}

/// A delimiter row is made only of `-`, `:`, `|`, and spaces, with a dash.
fn is_delimiter(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty()
        && t.contains('-')
        && t.contains('|')
        && t.chars().all(|c| matches!(c, '-' | ':' | '|' | ' ' | '\t'))
}

/// Split `| a | b |` into `["a", "b"]` (leading/trailing pipes ignored).
fn split_row(line: &str) -> Vec<String> {
    let t = line.trim().trim_matches('|');
    t.split('|').map(|cell| cell.trim().to_string()).collect()
}

fn parse_aligns(delimiter: &str) -> Vec<Align> {
    split_row(delimiter)
        .iter()
        .map(|cell| match (cell.starts_with(':'), cell.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        })
        .collect()
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

// --- inline spans -----------------------------------------------------------

/// Split a line into styled runs for inline `` `code` `` and `**bold**`.
fn inline(text: &str) -> Vec<(String, Style)> {
    let plain = body_style();
    let mut runs = Vec::new();
    let mut buf = String::new();
    let mut rest = text;
    // The last character consumed — the left neighbour of `rest`, for the `_x_`
    // word-boundary rule (`prev: Option<char>` avoids peeking an empty buffer).
    let mut prev: Option<char> = None;

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            flush(&mut runs, &mut buf);
            runs.push((after[..end].to_string(), plain.add_modifier(Modifier::BOLD)));
            prev = Some('*');
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('[')
            && let Some(close) = after.find(']')
            && let Some(url) = after[close + 1..].strip_prefix('(')
            && let Some(end) = url.find(')')
        {
            // `[label](url)` renders the label alone, in the link style (the TUI
            // cannot follow a link, so the URL would only add noise).
            flush(&mut runs, &mut buf);
            runs.push((after[..close].to_string(), link_style()));
            prev = Some(')');
            rest = &url[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            flush(&mut runs, &mut buf);
            runs.push((after[..end].to_string(), code_style()));
            prev = Some('`');
            rest = &after[end + 1..];
            continue;
        }
        // Italic is LAST, so bold/link/code above already claimed their markers.
        // `*x*` needs non-space content edges; `_x_` also needs word boundaries so
        // identifiers (`foo_bar_baz`) stay plain.
        if let Some(after) = rest.strip_prefix('*')
            && let Some(end) = after.find('*')
        {
            let content = &after[..end];
            if has_italic_edges(content) {
                flush(&mut runs, &mut buf);
                runs.push((content.to_string(), plain.add_modifier(Modifier::ITALIC)));
                prev = Some('*');
                rest = &after[end + 1..];
                continue;
            }
        }
        if let Some(after) = rest.strip_prefix('_')
            && let Some(end) = after.find('_')
            // A `prev`/closing guard keeps a `__` run (a dunder like `__init__`)
            // from italicizing its inner word.
            && prev.is_none_or(|c| !c.is_alphanumeric() && c != '_')
            && after[end + 1..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        {
            let content = &after[..end];
            if has_italic_edges(content) {
                flush(&mut runs, &mut buf);
                runs.push((content.to_string(), plain.add_modifier(Modifier::ITALIC)));
                prev = Some('_');
                rest = &after[end + 1..];
                continue;
            }
        }
        let ch = rest.chars().next().unwrap();
        buf.push(ch);
        prev = Some(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush(&mut runs, &mut buf);
    runs
}

/// Italic content must be non-empty with non-whitespace first/last characters, so
/// `a * b * c` stays plain.
fn has_italic_edges(content: &str) -> bool {
    match (content.chars().next(), content.chars().last()) {
        (Some(first), Some(last)) => !first.is_whitespace() && !last.is_whitespace(),
        _ => false,
    }
}

fn flush(runs: &mut Vec<(String, Style)>, buf: &mut String) {
    if !buf.is_empty() {
        runs.push((std::mem::take(buf), body_style()));
    }
}

/// Greedy word-wrap styled runs, prefixing the first line with `first` and
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
    fn ordered_lists_keep_their_numbers() {
        let text = text_of(&render("1. a\n2. b\n10. c", 40));
        assert_eq!(text, ["   1. a", "   2. b", "   10. c"]);

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

        // A `__` run (a dunder) must not italicize its inner word either.
        for dunder in ["__init__", "__dunder__"] {
            let lines = render(dunder, 40);
            assert_eq!(text_of(&lines), [format!("   {dunder}")]);
            assert!(
                lines
                    .iter()
                    .flat_map(|l| l.spans.iter())
                    .all(|s| !s.style.add_modifier.contains(Modifier::ITALIC)),
                "`{dunder}` must not italicize: {lines:?}"
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
    fn a_quoted_bullet_is_not_reparsed() {
        // The quoted body is emitted literally — a bullet-looking body stays text.
        assert_eq!(text_of(&render("> - x", 40)), ["   │ - x"]);
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
}
