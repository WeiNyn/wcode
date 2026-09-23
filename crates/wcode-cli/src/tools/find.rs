use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::grep::{parse_globs, walker};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct FindArgs {
    /// Directory to search; defaults to the working directory.
    pub path: Option<String>,
    /// Comma-separated glob filters, e.g. "**/*.rs", "!tests/**", "Cargo.toml".
    pub glob: Option<String>,
    /// Only list dirs ("dir"), only files ("file"), or anything ("any").
    pub kind: Option<String>,
    /// Maximum number of entries to report.
    pub max: Option<u64>,
    /// List entries that `.gitignore` excludes too (default false).
    pub no_ignore: Option<bool>,
}

pub struct Find;

#[async_trait::async_trait]
impl TypedTool for Find {
    type Args = FindArgs;
    fn name(&self) -> &str {
        "find"
    }
    fn description(&self) -> &str {
        "List files and directories matching globs, one path per stdout line (relative to the working directory). Respects .gitignore; also skips .git/target/node_modules (pass no_ignore:true to list ignored entries). Use grep to search inside files; use find to locate them."
    }

    /// Read-only: safe to run alongside other calls in the same batch.
    fn parallel_safe(&self) -> bool {
        true
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let base = match &args.path {
            Some(p) => super::resolve(&ctx.working_dir, p),
            None => ctx.working_dir.clone(),
        };
        if !base.exists() {
            return ToolOutput {
                output: format!("find: {} does not exist", base.display()),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        let (includes, excludes) = parse_globs(args.glob.as_deref());
        let want_dir = matches!(args.kind.as_deref(), Some("dir"));
        let want_file = matches!(args.kind.as_deref(), Some("file"));
        let max = args.max.unwrap_or(500) as usize;

        let mut out = String::new();
        let mut shown = 0usize;
        let base_is_file = base.is_file();
        let mut entries: Vec<std::path::PathBuf> = Vec::new();
        if base_is_file {
            entries.push(base.clone());
        } else {
            // Check cancel per directory entry: a tree walk can be long, and
            // collecting it all up-front would make a mid-run cancel wait for
            // the whole walk to finish (the loop honors cancel only after the
            // tool returns).
            for entry in walker(&base, args.no_ignore.unwrap_or(false)) {
                // Check cancel per directory entry: a tree walk can be long, and
                // collecting it all up-front would make a mid-run cancel wait for
                // the whole walk to finish (the loop honors cancel only after the
                // tool returns).
                if ctx.cancel.is_cancelled() {
                    return ToolOutput {
                        output: "cancelled".to_string(),
                        is_error: true,
                        diff: None,
                        path: None,
                    };
                }
                if let Ok(entry) = entry {
                    entries.push(entry.into_path());
                }
            }
        }
        for p in entries {
            if want_dir && !p.is_dir() {
                continue;
            }
            if want_file && !p.is_file() {
                continue;
            }
            if shown >= max {
                out.push_str(&format!("[find: truncated at {max} entries]\n"));
                break;
            }
            let rel = p.strip_prefix(&ctx.working_dir).unwrap_or(&p);
            let rel_str = rel.to_string_lossy();
            if rel_str == "." {
                continue;
            }
            if !super::include_path(&rel_str, &includes, &excludes) {
                continue;
            }
            out.push_str(&rel_str);
            out.push('\n');
            shown += 1;
        }
        if out.is_empty() {
            out.push_str(&format!("find: nothing matched under {}\n", base.display()));
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

    #[tokio::test]
    async fn finds_by_glob_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "").unwrap();
        std::fs::write(dir.path().join("src/b.rs"), "").unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = Find
            .execute(
                FindArgs {
                    path: None,
                    glob: Some("**/*.rs".into()),
                    kind: Some("file".into()),
                    max: None,
                    no_ignore: None,
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error);
        assert!(out.output.contains("src/a.rs"));
        assert!(out.output.contains("src/b.rs"));
        assert!(!out.output.contains("Cargo.toml"));
        assert!(!out.output.contains("target"));
    }

    async fn list(dir: &std::path::Path, no_ignore: Option<bool>) -> String {
        let (ctx, _rx) = super::super::test_ctx(dir);
        Find
            .execute(
                FindArgs {
                    path: None,
                    glob: Some("**/*.txt".into()),
                    kind: Some("file".into()),
                    max: None,
                    no_ignore,
                },
                &ctx,
            )
            .await
            .output
    }

    #[tokio::test]
    async fn respects_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "hidden.txt\n").unwrap();
        std::fs::write(dir.path().join("hidden.txt"), "").unwrap();
        std::fs::write(dir.path().join("shown.txt"), "").unwrap();

        let out = list(dir.path(), None).await;
        assert!(out.contains("shown.txt"), "{out}");
        assert!(!out.contains("hidden.txt"), "gitignore not honored: {out}");

        let out = list(dir.path(), Some(true)).await;
        assert!(out.contains("hidden.txt"), "no_ignore did nothing: {out}");
    }
}
