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
