use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor::{self, ANCHOR_SEP};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EditArgs {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// Anchor (as shown by `read`) of the first line of the range to replace.
    pub from: String,
    /// Anchor (as shown by `read`) of the last line of the range to replace.
    /// Omit to address a single line.
    pub to: Option<String>,
    /// Replacement text for the range (may be multi-line).
    pub replacement: String,
    /// Optional verification: this text must appear ending at the range or in
    /// the line(s) immediately before it (its own line span bounds how far up
    /// it may reach). Use it to pin one of several identical lines.
    pub old_string: Option<String>,
    /// Replace every matching range instead of requiring exactly one.
    pub replace_all: Option<bool>,
    /// Whole-file digest from your last read of this file (the `# <path>
    /// digest <hex>` line `read` prints). Auto-filled by the harness; normally
    /// leave unset. A mismatch refuses the call (E_STALE_DIGEST) — re-read.
    #[serde(default)]
    pub expected_digest: Option<String>,
}

pub struct Edit {
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Edit {
    pub fn new(lock: Arc<tokio::sync::Mutex<()>>) -> Self {
        Self { lock }
    }
}

fn bad_anchor(ref_: &str, field: &str) -> ToolOutput {
    ToolOutput {
        output: format!(
            "[E_BAD_ANCHOR] `{field}` must be a bare 5-char anchor as shown by read (e.g. \"aB3x1\"), got \"{ref_}\". No line numbers, no `{ANCHOR_SEP}content`, no surrounding text."
        ),
        is_error: true,
        diff: None,
        path: None,
    }
}

#[async_trait::async_trait]
impl TypedTool for Edit {
    type Args = EditArgs;
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "Replace the line range covered by the from/to anchors with replacement — content-addressed, so edits above never shift the target. Ambiguous (identical lines share an anchor) or stale anchors are rejected with candidates, nothing written; old_string disambiguates, replace_all applies to every match. replacement is verbatim — preserve leading whitespace; re-read if the file changed before editing. Whole-file rewrite = write; literal string = replace."
    }
    /// Mutates the workspace — blocked in plan mode (`MUTATING_TOOLS`).
    fn mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let _guard = self.lock.lock().await;
        let path = super::resolve(&ctx.working_dir, &args.path);

        if !anchor::is_anchor(&args.from) {
            return bad_anchor(&args.from, "from");
        }
        if let Some(t) = &args.to
            && !anchor::is_anchor(t)
        {
            return bad_anchor(t, "to");
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("edit {}: {e}", args.path),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        };

        // Whole-file CAS (D1): refuse when the file moved since the digest was
        // captured. Runs before anchor resolution; nothing is written on refuse.
        if let Some(expected) = &args.expected_digest {
            let actual = anchor::file_digest(content.as_bytes());
            if &actual != expected {
                return ToolOutput {
                    output: crate::workspace::stale_digest(&args.path, expected, &actual),
                    is_error: true,
                    ..ToolOutput::default()
                };
            }
        }

        // A byte-empty or "\n"-only file is one anonymous empty insertion point, so
        // the agent can populate it with `edit { from: <emptyAnchor> }`.
        let empty = anchor::is_degenerate_empty(&content);
        let original = content.clone();
        let lines = if empty {
            vec![String::new()]
        } else {
            anchor::split_lines(&content)
        };
        let anchors = anchor::anchors_for(&lines);
        let ranges = anchor::find_ranges(
            &lines,
            &anchors,
            &args.from,
            args.to.as_deref(),
            args.old_string.as_deref(),
        );

        if ranges.is_empty() {
            return ToolOutput {
                output: stale_message(&args, &lines, &anchors),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        let replace_all = args.replace_all.unwrap_or(false);
        if ranges.len() > 1 && !replace_all {
            let candidates = ranges
                .iter()
                .map(|r| format!("  line {}: {}", r.start + 1, lines[r.start]))
                .collect::<Vec<_>>()
                .join("\n");
            return ToolOutput {
                output: format!(
                    "[E_AMBIGUOUS_ANCHOR] `{}` matches {} ranges in {} (identical lines share an anchor). Candidates:\n{}\nSet replace_all:true to apply to every match, or re-send with old_string (or a to-anchor) that is unique to the range you mean.",
                    args.from,
                    ranges.len(),
                    args.path,
                    candidates
                ),
                is_error: true,
                diff: None,
                path: None,
            };
        }

        // Overlapping ranges are a hard error: the apply loop runs bottom-up
        // but keeps using the *original* indices against the *accumulated*
        // content, which is only correct when ranges are disjoint. When a
        // multi-line range's `to` lands on or past the next `from` match, the
        // second range would replace text the first already wrote — silent
        // corruption. Fail closed instead of guessing which one was meant.
        for i in 1..ranges.len() {
            let prev = &ranges[i - 1];
            let cur = &ranges[i];
            if cur.start <= prev.end {
                return ToolOutput {
                    output: format!(
                        "[E_OVERLAPPING_RANGES] `replace_all` matches {} ranges for from=`{}` in {}, and match #{} (lines {}–{}) already contains match #{} (starting at line {}). Applying both would rewrite already-replaced text and corrupt the file — nothing was written. Pin ONE range with a unique old_string (or a closer to-anchor) and edit it alone, or drop `to` so matches are single lines.",
                        ranges.len(),
                        args.from,
                        args.path,
                        i,
                        prev.start + 1,
                        prev.end + 1,
                        i + 1,
                        cur.start + 1,
                    ),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        }

        // Apply bottom-up so earlier indices stay valid; each step re-anchors
        // against the accumulated content, and ranges below the current one
        // keep their original coordinates (edits above) or were already
        // applied (edits below).
        // A brand-new file (empty input) still ends with a newline.
        let ends_nl = empty || content.ends_with('\n');
        let mut updated = content;
        let mut spans = Vec::new();
        let mut applied = 0usize;
        for r in ranges.iter().rev() {
            spans.push(if r.start == r.end {
                format!("{}", r.start + 1)
            } else {
                format!("{}–{}", r.start + 1, r.end + 1)
            });
            let cur_lines = if updated.is_empty() {
                vec![String::new()]
            } else {
                anchor::split_lines(&updated)
            };
            let (next, changed) = anchor::apply_replace(&cur_lines, r, &args.replacement, ends_nl);
            if changed.is_some() {
                applied += 1;
            }
            updated = next;
        }
        if applied == 0 {
            return ToolOutput {
                output: format!(
                    "no-op: {} lines {} already match the replacement",
                    args.path,
                    spans.join(", ")
                ),
                is_error: false,
                diff: None,
                path: None,
            };
        }

        let tmp = super::temp_path(&path);
        match std::fs::write(&tmp, &updated).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => {
                // Echo the resulting region with fresh anchors so subsequent
                // edits chain without a re-read.
                let mut fresh = String::new();
                if let Some((first, last)) = anchor::changed_range(&original, &updated) {
                    for line in anchor::split_lines(&updated)
                        .iter()
                        .skip(first - 1)
                        .take(last - first + 1)
                    {
                        fresh.push_str(&anchor::render(&anchor::anchor(line), line));
                        fresh.push('\n');
                    }
                }
                let diff = super::diff::unified(&original, &updated);
                ToolOutput {
                    output: format!(
                        "edited {} (lines {}; {} range{}). Region now:\n{}anchors above are fresh — chain further edits without re-reading.",
                        args.path,
                        spans.join(", "),
                        applied,
                        if applied == 1 { "" } else { "s" },
                        fresh
                    ),
                    is_error: false,
                    diff,
                    path: Some(args.path.clone()),
                }
            }
            Err(e) => ToolOutput {
                output: format!("edit {}: {e}", args.path),
                is_error: true,
                diff: None,
                path: None,
            },
        }
    }
}

fn stale_message(args: &EditArgs, lines: &[String], anchors: &[String]) -> String {
    // `from` still present: the anchor is live but its range failed to resolve,
    // so pointing at that line is positional and actionable. `from` gone: there
    // is nothing to point at — an anchor is a content hash, so a missing one
    // says nothing about where the line used to be. Only a re-read yields fresh
    // anchors, so don't guess.
    let mut hint: Vec<String> = Vec::new();
    if let Some(i) = anchors.iter().position(|a| a == &args.from) {
        if let Some(t) = &args.to {
            if anchors[i..].iter().any(|a| a == t) {
                hint.push("`to` exists but old_string verification failed for that range.".into());
            } else {
                hint.push(format!(
                    "`to` anchor {t} not found after the `from` match on line {} (file changed since read?).",
                    i + 1
                ));
            }
        } else if args.old_string.is_some() {
            hint.push(
                "`from` was found but old_string does not match its line (file changed since read?)."
                    .into(),
            );
        }
        hint.push(format!(
            "    {}  {}",
            anchors[i],
            lines[i].chars().take(48).collect::<String>()
        ));
    }
    let detail = if hint.is_empty() {
        String::new()
    } else {
        format!("\n{}", hint.join("\n"))
    };
    format!(
        "[E_STALE_ANCHOR] no edit target in {} for from=`{}`{} — the file changed since read.{detail}\nRe-read the file (read {}) and retry with fresh anchors.",
        args.path,
        args.from,
        args.to
            .as_ref()
            .map(|t| format!(", to=`{t}`"))
            .unwrap_or_default(),
        args.path
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stale_digest_refuses_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a = 1\nb = 2\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a = 1");
        let mut a = args("f.txt", &ha, None, "a = 10", None, None);
        a.expected_digest = Some("000000000000".into());
        let out = tool().execute(a, &ctx).await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("E_STALE_DIGEST"), "{}", out.output);
        assert!(out.output.contains("re-read"), "{}", out.output);
        // Nothing written.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a = 1\nb = 2\n"
        );
    }

    #[tokio::test]
    async fn matching_digest_allows_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a = 1\nb = 2\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a = 1");
        let mut a = args("f.txt", &ha, None, "a = 10", None, None);
        a.expected_digest = Some(anchor::file_digest(b"a = 1\nb = 2\n"));
        let out = tool().execute(a, &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a = 10\nb = 2\n"
        );
    }

    fn tool() -> Edit {
        Edit::new(Arc::new(tokio::sync::Mutex::new(())))
    }

    fn args(
        path: &str,
        from: &str,
        to: Option<&str>,
        replacement: &str,
        old_string: Option<&str>,
        replace_all: Option<bool>,
    ) -> EditArgs {
        EditArgs {
            path: path.into(),
            from: from.into(),
            to: to.map(String::from),
            replacement: replacement.into(),
            old_string: old_string.map(String::from),
            replace_all,
            expected_digest: None,
        }
    }

    #[tokio::test]
    async fn single_line_edit_by_anchor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a = 1\nb = 2\nc = 3\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hb = anchor::anchor("b = 2");
        let out = tool()
            .execute(args("f.txt", &hb, None, "b = 20", None, None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("edited"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a = 1\nb = 20\nc = 3\n"
        );
        // Fresh anchors are echoed so the next edit chains without a re-read.
        // Fresh anchors are echoed so the next edit chains without a re-read.
        assert!(out.output.contains(&anchor::anchor("b = 20")));
        // The UI-only change record rides the result: the file it touched and a
        // unified diff of the edit (neither enters the model's context).
        assert_eq!(out.path.as_deref(), Some("f.txt"));
        assert!(out.diff.as_deref().is_some_and(|d| d.contains("+b = 20")));
    }

    #[tokio::test]
    async fn range_edit_across_adjacent_lines() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hc = anchor::anchor("c");
        let out = tool()
            .execute(args("f.txt", &ha, Some(&hc), "x\ny", None, None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\ny\nd\n"
        );
    }

    #[tokio::test]
    async fn deleting_the_last_line_echoes_a_real_region() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x\ny\nz\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hz = anchor::anchor("z");
        let out = tool()
            .execute(args("f.txt", &hz, None, "", None, None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\ny\n"
        );
        // The echo must show a real surviving line, not an empty "Region now".
        assert!(
            out.output
                .contains(&anchor::render(&anchor::anchor("y"), "y")),
            "echo should show the surviving line: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn ambiguous_duplicate_is_rejected_with_candidates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x\n}\ny\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let h = anchor::anchor("}");
        let out = tool()
            .execute(args("f.txt", &h, None, "z", None, None), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_AMBIGUOUS_ANCHOR"));
        assert!(out.output.contains("replace_all"));
        // Nothing was written.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\n}\ny\n}\n"
        );
    }

    #[tokio::test]
    async fn old_string_disambiguates_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x\n}\ny\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let h = anchor::anchor("}");
        let out = tool()
            .execute(args("f.txt", &h, None, "z", Some("y\n}"), None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x\n}\ny\nz\n"
        );
    }

    #[tokio::test]
    async fn old_string_matches_only_nearby_context() {
        // `beta` is the immediate context of the FIRST `alpha` only. The old
        // prefix search let it satisfy the second `alpha` too (ambiguous); the
        // bounded window keeps it local, so this pins the first line.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "beta\nalpha\nalpha\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let h = anchor::anchor("alpha");
        let out = tool()
            .execute(args("f.txt", &h, None, "ALPHA", Some("beta"), None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "beta\nALPHA\nalpha\n"
        );
    }

    #[tokio::test]
    async fn stale_anchor_is_rejected_with_reread_hint() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "only line here\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", "nope1", None, "x", None, None), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_STALE_ANCHOR"));
        assert!(out.output.contains("Re-read"));
        // The old hint listed 3 lines chosen by anchor-hash edit distance, which
        // is meaningless (a hash is not positional); it must be gone.
        assert!(
            !out.output.contains("Nearby"),
            "no hash-distance 'nearby' list: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn stale_from_present_shows_the_line_and_reason() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\nbeta\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        // `from` is live (alpha) but old_string doesn't match its line, so the
        // hint should name the reason and show that line (positional, useful).
        let ha = anchor::anchor("alpha");
        let out = tool()
            .execute(args("f.txt", &ha, None, "x", Some("nope"), None), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_STALE_ANCHOR"));
        assert!(
            out.output.contains("old_string does not match"),
            "{}",
            out.output
        );
        assert!(
            out.output.contains("alpha"),
            "shows the from line: {}",
            out.output
        );
    }

    #[tokio::test]
    async fn bad_anchor_format_is_rejected() {
        let (ctx, _rx) = super::super::test_ctx(std::path::Path::new("."));
        let out = tool()
            .execute(args("f.txt", "line 47", None, "x", None, None), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_BAD_ANCHOR"));
    }

    #[tokio::test]
    async fn anchors_are_stable_across_edits_above() {
        // The headline property: an insert above does not move the anchor below.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hc = anchor::anchor("c");
        // Insert two lines above c by editing the "a" line.
        let ha = anchor::anchor("a");
        let out = tool()
            .execute(args("f.txt", &ha, None, "a\nnew1\nnew2", None, None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        // Now c is at line 5; its anchor must be unchanged.
        let after = std::fs::read_to_string(dir.path().join("f.txt")).unwrap();
        let lines = anchor::split_lines(&after);
        assert_eq!(lines.len(), 5);
        assert_eq!(anchor::anchor(&lines[4]), hc);
        // And editing c by that old anchor still lands on exactly line 5.
        let out2 = tool()
            .execute(args("f.txt", &hc, None, "c100", None, None), &ctx)
            .await;
        assert!(!out2.is_error, "{}", out2.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nnew1\nnew2\nb\nc100\n"
        );
    }

    #[tokio::test]
    async fn replace_all_applies_to_every_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same\nsame\nsame\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let h = anchor::anchor("same");
        let out = tool()
            .execute(args("f.txt", &h, None, "diff", None, Some(true)), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "diff\ndiff\ndiff\n"
        );
    }

    #[tokio::test]
    async fn overlapping_replace_all_ranges_are_rejected() {
        // from=a to=b replace_all matches [0..=3] and [2..=3]: the second `a`
        // sits inside the first range, so applying both bottom-up with the
        // original indices would rewrite already-replaced text.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nx\na\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hb = anchor::anchor("b");
        let out = tool()
            .execute(args("f.txt", &ha, Some(&hb), "y", None, Some(true)), &ctx)
            .await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("E_OVERLAPPING_RANGES"));
        // Nothing was written — fail closed.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a\nx\na\nb\n"
        );
    }

    #[tokio::test]
    async fn disjoint_replace_all_ranges_still_apply() {
        // Non-overlapping multi-line ranges are the legitimate replace_all
        // shape and must keep working after the overlap guard.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nx\na\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hb = anchor::anchor("b");
        let out = tool()
            .execute(
                args("f.txt", &ha, Some(&hb), "A\nB", None, Some(true)),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "A\nB\nx\nA\nB\n"
        );
    }

    #[tokio::test]
    async fn populates_empty_file() {
        // Both degenerate empty states ("", "\n") populate identically from the
        // single insertion anchor.
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let he = anchor::anchor("");
        for content in ["", "\n"] {
            let f = dir.path().join("f.txt");
            std::fs::write(&f, content).unwrap();
            let out = tool()
                .execute(args("f.txt", &he, None, "l1\nl2", None, None), &ctx)
                .await;
            assert!(!out.is_error, "{}", out.output);
            assert_eq!(
                std::fs::read_to_string(&f).unwrap(),
                "l1\nl2\n",
                "for {content:?}"
            );
        }
    }

    #[tokio::test]
    async fn range_whose_half_is_deleted_returns_stale() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let ha = anchor::anchor("a");
        let hc = anchor::anchor("c"); // never in the file
        let out = tool()
            .execute(args("f.txt", &ha, Some(&hc), "x", None, None), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_STALE_ANCHOR"));
    }

    #[tokio::test]
    async fn replacement_keeps_indentation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "fn main() {\n    let a = 1;\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let h = anchor::anchor("    let a = 1;");
        let out = tool()
            .execute(
                args("f.txt", &h, None, "        let b = 2;", None, None),
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "fn main() {\n        let b = 2;\n}\n"
        );
    }
}

