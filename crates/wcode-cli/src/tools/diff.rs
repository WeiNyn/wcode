//! A minimal unified diff, for *presentation only*.
//!
//! The edit tools attach this to their result's `diff` field — it rides the
//! `ToolExecutionEnd` event to the TUI but never enters the model's context
//! (the model already knows what it wrote). One hunk is enough: these tools
//! change a localized region, and a bounded single-hunk diff is both cheaper
//! and easier to read than a full Myers diff.

/// Context lines kept on each side of the change.
const CONTEXT: usize = 3;
/// Cap on emitted body lines — a whole-file `write` must not flood the pane.
const MAX_LINES: usize = 40;

/// A unified diff of `old` → `new`, or `None` when they are byte-identical.
///
/// The common prefix and suffix are trimmed; the changed middle is emitted as
/// `-`/`+` lines with up to [`CONTEXT`] context lines each side, under a single
/// `@@` header. The body is capped at [`MAX_LINES`] lines.
pub fn unified(old: &str, new: &str) -> Option<String> {
    if old == new {
        return None;
    }
    let old: Vec<&str> = old.lines().collect();
    let new: Vec<&str> = new.lines().collect();

    let mut start = 0;
    while start < old.len() && start < new.len() && old[start] == new[start] {
        start += 1;
    }
    let mut end = 0;
    while end < old.len() - start
        && end < new.len() - start
        && old[old.len() - 1 - end] == new[new.len() - 1 - end]
    {
        end += 1;
    }

    let pre = CONTEXT.min(start);
    let post = CONTEXT.min(end);
    let old_mid = &old[start..old.len() - end];
    let new_mid = &new[start..new.len() - end];

    let old_count = pre + old_mid.len() + post;
    let new_count = pre + new_mid.len() + post;
    let from = start - pre + 1;

    let mut body: Vec<String> = Vec::new();
    for line in &old[start - pre..start] {
        body.push(format!(" {line}"));
    }
    for line in old_mid {
        body.push(format!("-{line}"));
    }
    for line in new_mid {
        body.push(format!("+{line}"));
    }
    for line in &new[new.len() - post..] {
        body.push(format!(" {line}"));
    }

    let mut out = vec![format!("@@ -{from},{old_count} +{from},{new_count} @@")];
    if body.len() > MAX_LINES {
        let extra = body.len() - MAX_LINES;
        body.truncate(MAX_LINES);
        out.extend(body);
        out.push(format!("… (+{extra} more lines)"));
    } else {
        out.extend(body);
    }
    Some(out.join("\n"))
}

/// Split file content into logical lines. A trailing `\n` is a separator, not a
/// line, so `"a\n"` yields `["a"]`; a byte-empty file yields zero lines. A lone
/// `"\n"` file yields the single degenerate line `[""]`.
pub fn split_lines(content: &str) -> Vec<String> {
    if content.is_empty() {
        return vec![];
    }
    let mut v = content
        .split('\n')
        .map(str::to_string)
        .collect::<Vec<String>>();
    if content.ends_with('\n') {
        v.pop();
    }
    v
}

/// First/last 1-based line, in the NEW content, that the change touches, or
/// `None` when the two are byte-identical. Insertions/replacements report the new
/// lines; a pure deletion reports the surviving line that moved up into the gap
/// (or the last line, if the deletion was at EOF) — so the span is never one past
/// EOF. Callers use it to report *where* an edit landed.
pub fn changed_range(orig: &str, new: &str) -> Option<(usize, usize)> {
    if orig == new {
        return None;
    }
    let a = split_lines(orig);
    let b = split_lines(new);
    let mut first = 0;
    while first < a.len() && first < b.len() && a[first] == b[first] {
        first += 1;
    }
    let mut ia = a.len();
    let mut ib = b.len();
    while ia > first && ib > first && a[ia - 1] == b[ib - 1] {
        ia -= 1;
        ib -= 1;
    }
    // Region of the NEW content that differs, 0-based [first, ib). When
    // `ib > first` the changed lines are present (insertion or replacement).
    if ib > first {
        return Some((first + 1, ib));
    }
    // Pure deletion: the new side has no lines at the gap, so point at the line
    // that moved up into it — or the line just above — always a real line,
    // never one past EOF.
    if b.is_empty() {
        return Some((1, 1)); // nothing to echo; callers render an empty region
    }
    let keep = first.min(b.len() - 1);
    Some((keep + 1, keep + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_has_no_diff() {
        assert_eq!(unified("a\nb\n", "a\nb\n"), None);
        assert_eq!(unified("", ""), None);
    }

    #[test]
    fn a_changed_line_shows_context_minus_and_plus() {
        let d = unified("l1\nl2\nl3\nl4\nl5\n", "l1\nl2\nX\nl4\nl5\n").unwrap();
        assert!(d.starts_with("@@ -1,5 +1,5 @@"), "header: {d}");
        assert!(d.contains("\n-l3"), "missing removal: {d}");
        assert!(d.contains("\n+X"), "missing addition: {d}");
        assert!(d.contains("\n l2"), "missing context: {d}");
    }

    #[test]
    fn an_insertion_is_add_only() {
        let d = unified("a\nb\n", "a\nb\nc\n").unwrap();
        assert!(d.contains("\n+c"));
        assert!(!d.contains("\n-"), "unexpected removal: {d}");
    }

    #[test]
    fn a_new_file_is_all_additions() {
        let d = unified("", "a\nb\n").unwrap();
        assert!(d.starts_with("@@ -1,0 +1,2 @@"), "header: {d}");
        assert!(d.contains("+a") && d.contains("+b"));
    }

    #[test]
    fn the_body_is_capped() {
        let old: String = (0..100).map(|i| format!("l{i}\n")).collect();
        let new: String = (0..100).map(|i| format!("x{i}\n")).collect();
        let d = unified(&old, &new).unwrap();
        let lines = d.lines().count();
        assert!(lines <= 1 + MAX_LINES + 1, "diff too long: {lines}");
        assert!(d.contains("more lines"), "missing truncation marker: {d}");
    }
}
