use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor;

/// Anchored display caps a single line at this many chars, appending `…(+N)`
/// for the truncated remainder. Raw (`plain:true`) mode is untouched. The
/// rendered anchor still hashes the FULL line, so a truncated line remains a
/// valid, unambiguous `edit` target. Tune here; no config surface.
const MAX_LINE_CHARS: usize = 300;
/// When `read` is called without a `limit`, the page is capped at this many
/// lines and a continuation note is appended. An explicit `limit` always wins
/// (even above the cap). Tune here; no config surface.
const MAX_READ_LINES: usize = 1000;

/// Anchored display shape for a line that exceeds `MAX_LINE_CHARS`: first
/// `MAX_LINE_CHARS` chars plus `…(+N)` for the truncated remainder. Plain mode
/// never calls this (raw text means raw).
fn truncate_line(line: &str) -> String {
    let len = line.chars().count();
    if len <= MAX_LINE_CHARS {
        return line.to_string();
    }
    let head: String = line.chars().take(MAX_LINE_CHARS).collect();
    format!("{head}…(+{})", len - MAX_LINE_CHARS)
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// 1-based line number to start from. Mutually exclusive with `from`.
    pub offset: Option<u64>,
    /// Maximum number of lines to return.
    pub limit: Option<u64>,
    /// `true` to suppress anchors and return raw `cat -n` lines; you only need
    /// anchors when the lines are targets for `edit`. Default `false`.
    pub plain: Option<bool>,
    /// Anchor (as shown by `read`, `grep`, or a previous `edit` echo) to read
    /// from instead of a line number — so a grep hit or a fresh edit region is
    /// immediately pageable. Reading starts at the first line bearing this
    /// anchor and continues to end of file (or `limit` lines); `context`
    /// includes that many lines above it. Mutually exclusive with `offset`.
    pub from: Option<String>,
    /// Lines of context to include *above* the `from` anchor. Only valid with
    /// `from`. Default 0.
    pub context: Option<u64>,
}

pub struct Read;

#[async_trait::async_trait]
impl TypedTool for Read {
    type Args = ReadArgs;
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read a text file. Each line renders as ANCHOR│content — the 5-char anchor is its content address and the target for edit. Anchors survive edits elsewhere and hash raw content, so indentation matters; re-read after a formatter reindents. Page with offset/limit, or pass from (an anchor from grep or a prior edit echo) plus context. plain:true prints cat -n style."
    }

    /// Read-only: safe to run alongside other calls in the same batch.
    fn parallel_safe(&self) -> bool {
        true
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let path = super::resolve(&ctx.working_dir, &args.path);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("read {}: {e}", args.path),
                    is_error: true,
                };
            }
        };
        let lines = anchor::split_lines(&content);
        let mut ambiguity_note: Option<String> = None;
        let (start, mut limit) = match &args.from {
            None => (
                args.offset.unwrap_or(1).max(1) as usize - 1,
                args.limit.unwrap_or(u64::MAX) as usize,
            ),
            Some(an) => {
                if !anchor::is_anchor(an) {
                    return ToolOutput {
                        output: format!(
                            "[E_BAD_ANCHOR] `from` must be a bare 5-char anchor as shown by read (e.g. \"aB3x1\"), got \"{an}\". No line numbers, no `{}content, no surrounding text.",
                            anchor::ANCHOR_SEP
                        ),
                        is_error: true,
                    };
                }
                if args.offset.is_some() {
                    return ToolOutput {
                        output: format!(
                            "[E_CONFLICT] `from` and `offset` are mutually exclusive: `from` addresses a line by content anchor, `offset` by line number. Use one or the other. (context={:?}, plain={:?})",
                            args.context, args.plain
                        ),
                        is_error: true,
                    };
                }
                let anchors = anchor::anchors_for(&lines);
                let matches = anchor::find_ranges(&lines, &anchors, an, None, None);
                if matches.is_empty() {
                    return ToolOutput {
                        output: format!(
                            "[E_STALE_ANCHOR] no line in {} carries the anchor `{an}` — the file changed since you last read it. Re-read (read {}) and retry with a fresh anchor.",
                            args.path, args.path
                        ),
                        is_error: true,
                    };
                }
                if matches.len() > 1 {
                    ambiguity_note = Some(format!(
                        "{an} matches {} lines in {} (identical lines share an anchor): {}.\n",
                        matches.len(),
                        args.path,
                        matches
                            .iter()
                            .map(|r| (r.start + 1).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                let context = args.context.unwrap_or(0) as usize;
                (
                    matches[0].start.saturating_sub(context),
                    args.limit.unwrap_or(u64::MAX) as usize,
                )
            }
        };
        // Page guard: without an explicit `limit` (raw and anchored modes alike),
        // cap the default page to protect the context window. An explicit
        // `limit` always wins, even above the cap.
        let explicit_limit = args.limit.is_some();
        let mut page_note = false;
        if !explicit_limit {
            let available = lines.len().saturating_sub(start);
            if available > MAX_READ_LINES {
                limit = limit.min(MAX_READ_LINES);
                page_note = true;
            }
        }

        let mut out = String::new();
        if let Some(note) = ambiguity_note {
            out.push_str(&note);
        }
        let plain = args.plain.unwrap_or(false);
        if anchor::is_degenerate_empty(&content) {
            // Byte-empty and "\n"-only files are one anonymous insertion point;
            // a "\n"-only file is not a real blank line.
            if plain {
                out.push('\n');
            } else {
                out.push_str(&format!(
                    "{}  <empty file — insert at this anchor>\n",
                    anchor::anchor("")
                ));
            }
        } else {
            for (i, line) in lines.iter().skip(start).take(limit).enumerate() {
                let rendered = i + 1; // 1-based count actually shown
                let n = start + i + 1; // 1-based line number, for plain output
                if plain {
                    out.push_str(&format!("{n:>6}\t{line}\n"));
                } else {
                    // Anchor hashes the FULL line; only the display is capped,
                    // so the truncated line stays a valid `edit` target.
                    out.push_str(&anchor::render(&anchor::anchor(line), &truncate_line(line)));
                    out.push('\n');
                }
                if page_note && rendered == limit {
                    let remaining = lines.len().saturating_sub(start + rendered);
                    if remaining > 0 {
                        out.push_str(&format!(
                            "… ({remaining} more lines — re-read with offset/limit to continue)\n"
                        ));
                        break;
                    }
                }
            }
        }
        ToolOutput {
            output: out,
            is_error: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reads_with_anchors_and_paging() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("f.txt"),
            (1..=10)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: Some(2),
                    limit: Some(2),
                    plain: None,
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        let rendered: Vec<&str> = out.output.lines().collect();
        assert_eq!(rendered.len(), 2);
        // Every line is ANCHOR│content with a parseable 5-char anchor.
        for line in rendered {
            let (hash, content) = line.split_once(anchor::ANCHOR_SEP).unwrap();
            assert!(anchor::is_anchor(hash), "bad anchor: {line}");
            assert!(content.contains("line "));
        }
    }

    #[tokio::test]
    async fn plain_mode_keeps_old_cat_n_shape() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "l1\nl2\nl3").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: Some(true),
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert_eq!(out.output, "     1\tl1\n     2\tl2\n     3\tl3\n");
    }

    #[tokio::test]
    async fn empty_file_shows_insertion_anchor() {
        // A byte-empty file and a "\n"-only file are the same degenerate state:
        // one anonymous insertion point, never a phantom blank line.
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let mut outputs = Vec::new();
        for content in ["", "\n"] {
            let f = dir.path().join("f.txt");
            std::fs::write(&f, content).unwrap();
            let out = Read
                .execute(
                    ReadArgs {
                        path: "f.txt".into(),
                        offset: None,
                        limit: None,
                        plain: None,
                        from: None,
                        context: None,
                    },
                    &ctx,
                )
                .await;
            assert!(!out.is_error);
            assert!(out.output.contains("empty file"), "for {content:?}");
            outputs.push(out.output);
        }
        assert_eq!(
            outputs[0], outputs[1],
            "empty & newline-only must render identically"
        );
    }

    #[tokio::test]
    async fn from_anchor_reads_to_end_of_file() {
        // The continuation seam: an anchor from grep or a previous edit echo
        // pages to that line without the model needing a line number.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\ne\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hc = anchor::anchor("c");
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: Some(hc),
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        let rendered: Vec<&str> = out.output.lines().collect();
        assert_eq!(rendered.len(), 3); // c, d, e
        let first = rendered[0].split_once(anchor::ANCHOR_SEP).unwrap();
        assert_eq!(first.0, anchor::anchor("c"));
        assert!(rendered[2].contains("e"));
    }

    #[tokio::test]
    async fn from_anchor_with_context_shows_lines_above() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\ne\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hd = anchor::anchor("d");
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: Some(3),
                    plain: None,
                    from: Some(hd),
                    context: Some(2),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        let rendered: Vec<&str> = out.output.lines().collect();
        assert_eq!(rendered.len(), 3); // b, c, d
        assert!(rendered[0].contains("b"));
        assert!(rendered[1].contains("c"));
        assert_eq!(
            rendered[2].split_once(anchor::ANCHOR_SEP).unwrap().0,
            anchor::anchor("d")
        );
    }

    #[tokio::test]
    async fn from_anchor_context_clamps_at_first_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hb = anchor::anchor("b");
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: Some(hb),
                    context: Some(10),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(
            out.output.lines().next().unwrap().contains("a"),
            "{}",
            out.output
        );
    }

    #[tokio::test]
    async fn from_anchor_notes_ambiguity_for_duplicates() {
        // Duplicated lines share an anchor; reading is non-destructive, so we
        // read from the FIRST occurrence and say so, listing all of them.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x\n}\ny\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let h = anchor::anchor("}");
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: Some(1),
                    plain: None,
                    from: Some(h),
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("matches 2 lines"));
        assert!(
            out.output.contains("lines in"),
            "no ambiguity note\n{}",
            out.output
        );
    }

    #[tokio::test]
    async fn from_anchor_not_found_is_stale_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: Some("nope1".to_string()),
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_STALE_ANCHOR"));
    }

    #[tokio::test]
    async fn from_and_offset_conflict_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let hb = anchor::anchor("b");
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: Some(2),
                    limit: None,
                    plain: None,
                    from: Some(hb),
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("E_CONFLICT"));
    }

    #[tokio::test]
    async fn missing_file_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "nope.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
    }

    #[test]
    fn truncate_line_caps_long_lines_and_passes_short() {
        assert_eq!(truncate_line("short"), "short");
        assert_eq!(truncate_line(""), "");
        let long = "x".repeat(MAX_LINE_CHARS + 7);
        let t = truncate_line(&long);
        assert!(t.ends_with("…(+7)"), "{t}");
        assert_eq!(t.chars().count(), MAX_LINE_CHARS + 5); // head + '…' + "(+7)"
        let exact = "y".repeat(MAX_LINE_CHARS);
        assert_eq!(
            truncate_line(&exact),
            exact,
            "exact boundary must not truncate"
        );
    }

    #[tokio::test]
    async fn anchored_read_truncates_long_lines_and_keeps_full_line_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let long = "fn ".to_string() + &"x".repeat(MAX_LINE_CHARS + 50) + "()";
        std::fs::write(dir.path().join("f.txt"), format!("short\n{long}\n")).unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        let lines: Vec<&str> = out.output.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("short"), "{}", lines[0]);
        // Display is truncated, but the leading anchor hashes the FULL line.
        let (hash, display) = lines[1].split_once(anchor::ANCHOR_SEP).unwrap();
        assert_eq!(hash, anchor::anchor(&long));
        assert!(display.contains("…(+"), "{}", display);
    }

    #[tokio::test]
    async fn no_limit_caps_page_and_notes_continuation() {
        let dir = tempfile::tempdir().unwrap();
        let n = MAX_READ_LINES + 10;
        let content = (1..=n)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.path().join("f.txt"), &content).unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());

        // No limit: capped at MAX_READ_LINES with a continuation note.
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        let lines: Vec<&str> = out.output.lines().collect();
        assert_eq!(lines.len(), MAX_READ_LINES + 1); // page + note
        assert!(lines[MAX_READ_LINES].contains("10 more lines"));

        // An explicit limit above the cap is honored (no note).
        let out2 = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: Some((MAX_READ_LINES + 10) as u64),
                    plain: None,
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out2.is_error, "{}", out2.output);
        assert_eq!(out2.output.lines().count(), MAX_READ_LINES + 10);

        // Plain mode is capped too.
        let out3 = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: Some(true),
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out3.is_error, "{}", out3.output);
        assert_eq!(out3.output.lines().count(), MAX_READ_LINES + 1);
    }
}

#[cfg(test)]
mod indentation_regression {
    use super::*;

    #[tokio::test]
    async fn anchored_read_keeps_indentation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "fn main() {\n    let a = 1;\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                    from: None,
                    context: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(
            out.output.contains("    let a = 1;\n"),
            "indent was stripped:\n{:?}",
            out.output
        );
    }
}
