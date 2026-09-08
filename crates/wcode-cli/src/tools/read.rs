use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor;

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
        "Read a text file. Every line is returned as ANCHOR│content; the 5-char anchor is the line's content address and the `edit` target. Anchors are stable: inserting/deleting lines elsewhere never changes a line's anchor, and reindenting (formatters) leaves anchors intact. No line numbers — use the anchor in `edit`. For raw text without anchors pass plain:true. Page with offset/limit, or pass `from` (an anchor from grep or a previous edit echo) plus optional `context` to read the region around a hit without knowing its line number. An empty file shows one insertion-point anchor."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let path = super::resolve(&ctx.working_dir, &args.path);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("read {}: {e}", args.path),
                    is_error: true,
                    details: None,
                };
            }
        };
        let lines = anchor::split_lines(&content);
        let mut ambiguity_note: Option<String> = None;
        let (start, limit) = match &args.from {
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
                        details: None,
                    };
                }
                if args.offset.is_some() {
                    return ToolOutput {
                        output: format!(
                            "[E_CONFLICT] `from` and `offset` are mutually exclusive: `from` addresses a line by content anchor, `offset` by line number. Use one or the other. (context={:?}, plain={:?})",
                            args.context, args.plain
                        ),
                        is_error: true,
                        details: None,
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
                        details: None,
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
                let n = start + i + 1; // 1-based, for plain output
                if plain {
                    out.push_str(&format!("{n:>6}\t{line}\n"));
                } else {
                    out.push_str(&anchor::render(&anchor::anchor(line), line));
                    out.push('\n');
                }
            }
        }
        ToolOutput {
            output: out,
            is_error: false,
            details: None,
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
}
