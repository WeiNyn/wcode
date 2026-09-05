use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// 1-based line number to start from.
    pub offset: Option<u64>,
    /// Maximum number of lines to return.
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
        "Read lines from a file, `cat -n` style: 6-wide right-aligned line number, tab, line content."
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
        let start = args.offset.unwrap_or(1).max(1) as usize - 1;
        let limit = args.limit.unwrap_or(u64::MAX) as usize;
        let mut out = String::new();
        for (i, line) in content.lines().skip(start).take(limit).enumerate() {
            out.push_str(&format!("{:>6}\t{}\n", start + i + 1, line));
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
    async fn reads_with_offset_and_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("f.txt"),
            (1..=100)
                .map(|i| format!("l{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Read
            .execute(
                ReadArgs {
                    path: "f.txt".into(),
                    offset: Some(10),
                    limit: Some(3),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert_eq!(out.output, "    10\tl10\n    11\tl11\n    12\tl12\n");
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
                },
                &ctx,
            )
            .await;
        assert!(out.is_error);
    }
}
