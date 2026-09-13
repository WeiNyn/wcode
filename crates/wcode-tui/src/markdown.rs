//! A tiny markdown renderer for assistant messages: fenced code blocks,
//! headings, bullets, and inline `code` / **bold**. Deliberately small — the
//! transcript needs readable prose and code, not a spec-complete parser.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::{code_style, dim};

/// The 3-column gutter every assistant line shares.
const GUTTER: &str = "   ";

/// Render markdown `text` to transcript lines wrapped to `width`.
pub fn render(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut in_fence = false;

    for raw in text.split('\n') {
        let line = raw.trim_end();

        if in_fence {
            if line.trim_start().starts_with("```") {
                in_fence = false;
            } else {
                lines.push(Line::from(vec![
                    Span::styled(format!("{GUTTER}│ "), dim()),
                    Span::styled(line.to_string(), code_style()),
                ]));
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            in_fence = true;
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
    }
    lines
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

    #[test]
    fn headings_and_bullets_get_markers() {
        let lines = render("# Title\n- one\n- two", 40);
        let text = text_of(&lines);
        assert_eq!(text[0], "   Title");
        assert_eq!(text[1], "   • one");
        assert_eq!(text[2], "   • two");
    }

    #[test]
    fn fenced_code_is_marked_and_not_wrapped_inline() {
        let lines = render("text\n```rust\nlet x = 1; // **not bold**\n```", 60);
        let text = text_of(&lines);
        assert_eq!(text[0], "   text");
        assert_eq!(text[1], "   │ let x = 1; // **not bold**");
    }

    #[test]
    fn inline_code_and_bold_split_into_spans() {
        let lines = render("a `b` and **c**", 40);
        let styles: Vec<_> = lines[0].spans.iter().map(|s| s.style).collect();
        // gutter, "a", " ", code "b", " ", "and", " ", bold "c" — styles vary.
        assert!(styles.iter().any(|s| s.add_modifier.contains(Modifier::BOLD)));
        assert!(styles.iter().any(|s| *s == code_style()));
    }

    #[test]
    fn long_paragraphs_wrap_to_width() {
        let lines = render("one two three four five six seven", 20);
        for line in &lines {
            let len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
            assert!(len <= 20, "line too long: {len}");
        }
        let rejoined: String = text_of(&lines)
            .iter()
            .map(|l| l.trim_start().to_string())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(rejoined, "one two three four five six seven");
    }
}
