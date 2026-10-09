use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::diff::split_lines;

/// A single line is capped at this many chars, appending `…(+N)` for the
/// truncated remainder, so one pathological line (minified JS, a base64 blob)
/// cannot flood the context window. The marker is a warning: a TRUNCATED line is
/// not the file's real text, so it cannot be used as an `old_string` — `grep`
/// gives the exact bytes. Tune here; no config surface.
const MAX_LINE_CHARS: usize = 300;
/// When `read` is called without a `limit`, the page is capped at this many
/// lines and a continuation note is appended. An explicit `limit` always wins
/// (even above the cap). Tune here; no config surface.
const MAX_READ_LINES: usize = 1000;

/// Display shape for a line that exceeds [`MAX_LINE_CHARS`]: the first
/// `MAX_LINE_CHARS` chars plus `…(+N)` for the truncated remainder.
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
    /// 1-based line number to start from (default 1).
    pub offset: Option<u64>,
    /// Maximum number of lines to return. Without it the page is capped and a
    /// continuation note is appended.
    pub limit: Option<u64>,
}

pub struct Read;

#[async_trait::async_trait]
impl TypedTool for Read {
    type Args = ReadArgs;
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read a text file. Each line is rendered as `<line number>\\t<content>`, so page with `offset`/`limit`. A line longer than 300 chars is truncated with `…(+N)` — a truncated line is NOT the real text, so never use it as an `old_string`; use `grep` for the exact bytes. Without `limit` the page is capped."
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
                    diff: None,
                    path: None,
                };
            }
        };

        let lines = split_lines(&content);
        let start = args.offset.unwrap_or(1).max(1) as usize - 1;
        let explicit_limit = args.limit.is_some();
        let mut limit = args.limit.unwrap_or(u64::MAX) as usize;

        // Page guard: without an explicit `limit`, cap the page to protect the
        // context window. An explicit `limit` always wins, even above the cap.
        let mut page_note = false;
        if !explicit_limit {
            let available = lines.len().saturating_sub(start);
            if available > MAX_READ_LINES {
                limit = limit.min(MAX_READ_LINES);
                page_note = true;
            }
        }

        let mut out = String::new();
        if lines.is_empty() {
            out.push_str("(empty file)\n");
        }
        for (i, line) in lines.iter().skip(start).take(limit).enumerate() {
            let shown = i + 1; // 1-based count actually shown
            let n = start + i + 1; // 1-based line number
            out.push_str(&format!("{n}\t{}\n", truncate_line(line)));
            if page_note && shown == limit {
                let remaining = lines.len().saturating_sub(start + shown);
                if remaining > 0 {
                    out.push_str(&format!(
                        "… ({remaining} more lines — re-read with offset/limit to continue)\n"
                    ));
                    break;
                }
            }
        }
        ToolOutput {
            output: out,
            is_error: false,
            diff: None,
            path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(path: &str, offset: Option<u64>, limit: Option<u64>) -> ReadArgs {
        ReadArgs {
            path: path.into(),
            offset,
            limit,
        }
    }

    #[tokio::test]
    async fn renders_one_numbered_line_per_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha\nbeta\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", None, None), &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(out.output, "1\talpha\n2\tbeta\n");
    }

    #[tokio::test]
    async fn offset_and_limit_page_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\nb\nc\nd\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", Some(2), Some(2)), &ctx).await;
        assert_eq!(out.output, "2\tb\n3\tc\n", "numbers stay absolute");
    }

    #[tokio::test]
    async fn a_trailing_newline_is_not_an_extra_line() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "only\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", None, None), &ctx).await;
        assert_eq!(out.output, "1\tonly\n");
    }

    #[tokio::test]
    async fn an_empty_file_says_so() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", None, None), &ctx).await;
        assert!(!out.is_error);
        assert_eq!(out.output, "(empty file)\n");
    }

    #[tokio::test]
    async fn a_very_long_line_is_truncated_with_a_marker() {
        let dir = tempfile::tempdir().unwrap();
        let long = "x".repeat(MAX_LINE_CHARS + 5);
        std::fs::write(dir.path().join("f.txt"), format!("{long}\n")).unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", None, None), &ctx).await;
        assert!(out.output.contains("…(+5)"), "{}", out.output);
        assert!(!out.output.contains(&long), "the full line must not appear");
    }

    #[tokio::test]
    async fn a_short_line_is_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "short\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", None, None), &ctx).await;
        assert_eq!(out.output, "1\tshort\n");
    }

    #[tokio::test]
    async fn an_oversized_page_is_capped_with_a_continuation_note() {
        let dir = tempfile::tempdir().unwrap();
        let body: String = (1..=MAX_READ_LINES + 10)
            .map(|i| format!("l{i}\n"))
            .collect();
        std::fs::write(dir.path().join("f.txt"), body).unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("f.txt", None, None), &ctx).await;
        assert!(out.output.contains("10 more lines"), "{}", &out.output[out.output.len().saturating_sub(120)..]);
        // An explicit limit above the cap is honoured.
        let out = Read.execute(args("f.txt", None, Some((MAX_READ_LINES + 10) as u64)), &ctx).await;
        assert!(!out.output.contains("more lines"), "explicit limit wins");
    }

    #[tokio::test]
    async fn a_missing_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read.execute(args("nope.txt", None, None), &ctx).await;
        assert!(out.is_error);
        assert!(out.output.starts_with("read nope.txt:"), "{}", out.output);
    }
}
