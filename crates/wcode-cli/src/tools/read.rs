use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// 1-based line number to start from.
    pub offset: Option<u64>,
    /// Maximum number of lines to return.
    pub limit: Option<u64>,
    /// `true` to suppress anchors and return raw `cat -n` lines; you only need
    /// anchors when the lines are targets for `edit`. Default `false`.
    pub plain: Option<bool>,
}

pub struct Read;

#[async_trait::async_trait]
impl TypedTool for Read {
    type Args = ReadArgs;
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read a text file. Every line is returned as ANCHOR│content; the 5-char anchor is the line's content address and the `edit` target. Anchors are stable: inserting/deleting lines elsewhere never changes a line's anchor, and reindenting (formatters) leaves anchors intact. No line numbers — use the anchor in `edit`. For raw text without anchors pass plain:true. Page with offset/limit. An empty file shows one insertion-point anchor."
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
        let start = args.offset.unwrap_or(1).max(1) as usize - 1;
        let limit = args.limit.unwrap_or(u64::MAX) as usize;
        let mut out = String::new();
        let plain = args.plain.unwrap_or(false);
        if lines.is_empty() && !plain {
            out.push_str(&format!(
                "{}  <empty file — insert at this anchor>\n",
                anchor::anchor("")
            ));
        } else if lines.is_empty() && plain {
            out.push('\n');
        }
        for (i, line) in lines.iter().skip(start).take(limit).enumerate() {
            let n = start + i + 1; // 1-based, for plain output
            if plain {
                out.push_str(&format!("{n:>6}\t{line}\n"));
            } else {
                out.push_str(&anchor::render(&anchor::anchor(line), line));
                out.push('\n');
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
                },
                &ctx,
            )
            .await;
        assert_eq!(out.output, "     1\tl1\n     2\tl2\n     3\tl3\n");
    }

    #[tokio::test]
    async fn empty_file_shows_insertion_anchor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: None,
                    limit: None,
                    plain: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("empty file"));
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
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
    }
}
