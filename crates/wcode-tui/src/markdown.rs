//! A small markdown renderer for assistant messages: fenced code blocks,
//! headings, bullets, tables, and inline `code` / **bold**. Deliberately small —
//! the transcript needs readable prose and code, not a spec-complete parser.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::{code_style, dim};

/// The 3-column gutter every assistant line shares.
const GUTTER: &str = "   ";

/// Render markdown `text` to transcript lines wrapped to `width`.
pub fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    let raw: Vec<&str> = text.split('\n').collect();
    let mut lines = Vec::new();
    let mut in_fence = false;
    let mut i = 0;

    while i < raw.len() {
        let line = raw[i].trim_end();

        if in_fence {
            if line.trim_start().starts_with("```") {
                in_fence = false;
            } else {
                lines.push(code_line(line));
            }
            i += 1;
            continue;
        }
        if line.trim_start().starts_with("```") {
            in_fence = true;
            i += 1;
            continue;
        }
        if let Some((table, next)) = table_at(&raw, i) {
            lines.extend(table_lines(&table, width));
            i = next;
            continue;
        }
        if let Some(heading) = heading(line) {
            lines.extend(wrap(
                inline(heading),
                width,
                GUTTER,
                GUTTER,
                Style::new().add_modifier(Modifier::BOLD),
            ));
        } else if let Some(item) = bullet(line) {
            lines.extend(wrap(
                inline(item),
                width,
                &format!("{GUTTER}• "),
                &format!("{GUTTER}  "),
                Style::default(),
            ));
        } else if line.is_empty() {
            lines.push(Line::default());
        } else {
            lines.extend(wrap(
                inline(line),
                width,
                GUTTER,
                GUTTER,
                Style::default(),
            ));
        }
        i += 1;
    }
    lines
}

fn code_line(line: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{GUTTER}│ "), dim()),
        Span::styled(line.to_string(), code_style()),
    ])
}

/// `# Title` (one to six `#`) → `Title`.
fn heading(line: &str) -> Option<&str> {
    let text = line.trim_start();
    let hashes = text.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && text[hashes..].starts_with(' ') {
        Some(text[hashes..].trim_start())
    } else {
        None
    }
}

/// `- item` / `* item` / `+ item` → `item`.
fn bullet(line: &str) -> Option<&str> {
    let text = line.trim_start();
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|marker| text.strip_prefix(marker))
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
    // fits, never below 3 cells.
    let overhead = GUTTER.chars().count() + 3 * cols.saturating_sub(1);
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

    let mut out = vec![row_line(
        &table.header,
        &widths,
        &table.aligns,
        Style::new().add_modifier(Modifier::BOLD),
    )];
    let rule = widths
        .iter()
        .map(|w| "─".repeat(*w))
        .collect::<Vec<_>>()
        .join("─┼─");
    out.push(Line::from(Span::styled(format!("{GUTTER}{rule}"), dim())));
    for row in &table.rows {
        out.push(row_line(row, &widths, &table.aligns, Style::default()));
    }
    out
}

fn row_line(cells: &[String], widths: &[usize], aligns: &[Align], style: Style) -> Line<'static> {
    let mut parts = Vec::with_capacity(widths.len());
    for (c, w) in widths.iter().enumerate() {
        let cell = cells.get(c).map(String::as_str).unwrap_or("");
        let align = aligns.get(c).copied().unwrap_or(Align::Left);
        parts.push(pad(&truncate(cell, *w), *w, align));
    }
    Line::from(Span::styled(format!("{GUTTER}{}", parts.join(" │ ")), style))
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

fn truncate(text: &str, width: usize) -> String {
    if disp(text) <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Display width (characters; wide glyphs are treated as one cell for now).
fn disp(text: &str) -> usize {
    text.chars().count()
}

// --- inline spans -----------------------------------------------------------

/// Split a line into styled runs for inline `` `code` `` and `**bold**`.
fn inline(text: &str) -> Vec<(String, Style)> {
    let plain = Style::default();
    let mut runs = Vec::new();
    let mut buf = String::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
        {
            flush(&mut runs, &mut buf);
            runs.push((after[..end].to_string(), plain.add_modifier(Modifier::BOLD)));
            rest = &after[end + 2..];
            continue;
        }
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
        {
            flush(&mut runs, &mut buf);
            runs.push((after[..end].to_string(), code_style()));
            rest = &after[end + 1..];
            continue;
        }
        let ch = rest.chars().next().unwrap();
        buf.push(ch);
        rest = &rest[ch.len_utf8()..];
    }
    flush(&mut runs, &mut buf);
    runs
}

fn flush(runs: &mut Vec<(String, Style)>, buf: &mut String) {
    if !buf.is_empty() {
        runs.push((std::mem::take(buf), Style::default()));
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
    let mut words: Vec<(String, Style)> = Vec::new();
    for (text, style) in runs {
        for word in text.split_whitespace() {
            words.push((word.to_string(), style));
        }
    }

    let mut lines = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut used = 0usize;
    let mut prefix = first.to_string();
    let mut avail = width.saturating_sub(prefix.chars().count()).max(1);

    for (word, style) in words {
        let width_of = word.chars().count();
        let gap = usize::from(!spans.is_empty());
        if used + gap + width_of > avail && !spans.is_empty() {
            lines.push(line_with(prefix, spans, base));
            prefix = cont.to_string();
            avail = width.saturating_sub(prefix.chars().count()).max(1);
            spans = Vec::new();
            used = 0;
        }
        if !spans.is_empty() {
            spans.push(Span::styled(" ", style));
            used += 1;
        }
        spans.push(Span::styled(word, style));
        used += width_of;
    }
    lines.push(line_with(prefix, spans, base));
    lines
}

fn line_with(prefix: String, spans: Vec<Span<'static>>, base: Style) -> Line<'static> {
    let mut all = Vec::with_capacity(spans.len() + 1);
    all.push(Span::styled(prefix, dim()));
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
    fn wide_tables_shrink_to_fit() {
        let md = "| aaaaaaaa | bbbbbbbb |\n|---|---|\n| 1 | 2 |";
        let lines = render(md, 20);
        for line in &lines {
            assert!(width_of(line) <= 20, "too long: {}", width_of(line));
        }
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
}
