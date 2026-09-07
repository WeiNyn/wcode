use std::sync::{Arc, Mutex};

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReplaceArgs {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// Exact text to replace; must occur exactly once unless replace_all.
    pub old_string: String,
    /// Replacement text.
    pub new_string: String,
    /// Replace every occurrence instead of requiring a unique match.
    pub replace_all: Option<bool>,
}

pub struct Replace {
    lock: Arc<Mutex<()>>,
}

impl Replace {
    pub fn new(lock: Arc<Mutex<()>>) -> Self {
        Self { lock }
    }
}

#[async_trait::async_trait]
impl TypedTool for Replace {
    type Args = ReplaceArgs;
    fn name(&self) -> &str {
        "replace"
    }
    fn description(&self) -> &str {
        "Replace an exact literal string in a file without reading it first. `old_string` must occur exactly once unless `replace_all` is set. Prefer `edit` (anchors) for targeted line edits — `replace` is for quick, unique, whole-text substitutions."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        // Sync fs + std lock: guard never crosses an await, edits stay serialized.
        let _guard = self.lock.lock().unwrap();
        let path = super::resolve(&ctx.working_dir, &args.path);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("replace {}: {e}", args.path),
                    is_error: true,
                    details: None,
                };
            }
        };
        let matches = content.matches(&args.old_string).count();
        if matches == 0 {
            return ToolOutput {
                output: format!("old_string not found in {}", args.path),
                is_error: true,
                details: None,
            };
        }
        if matches > 1 && !args.replace_all.unwrap_or(false) {
            return ToolOutput {
                output: format!(
                    "old_string matches {matches} times in {}; set replace_all: true or include more surrounding context",
                    args.path
                ),
                is_error: true,
                details: None,
            };
        }
        let updated = if args.replace_all.unwrap_or(false) {
            content.replace(&args.old_string, &args.new_string)
        } else {
            content.replacen(&args.old_string, &args.new_string, 1)
        };
        // ponytail: tmp+rename so a crash mid-write can't truncate the original (same-fs rename).
        let tmp = path.with_extension("tmp-wcode");
        match std::fs::write(&tmp, updated).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => ToolOutput {
                output: format!("replaced in {}", args.path),
                is_error: false,
                details: None,
            },
            Err(e) => ToolOutput {
                output: format!("replace {}: {e}", args.path),
                is_error: true,
                details: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> Replace {
        Replace::new(Arc::new(Mutex::new(())))
    }

    fn args(path: &str, old: &str, new: &str, replace_all: Option<bool>) -> ReplaceArgs {
        ReplaceArgs {
            path: path.into(),
            old_string: old.into(),
            new_string: new.into(),
            replace_all,
        }
    }

    #[tokio::test]
    async fn unique_match_replaces_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha beta gamma").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", "beta", "BETA", None), &ctx)
            .await;
        assert!(!out.is_error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "alpha BETA gamma"
        );
    }

    #[tokio::test]
    async fn zero_matches_is_not_found_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", "nope", "x", None), &ctx).await;
        assert!(out.is_error);
        assert!(out.output.contains("not found"));
    }

    #[tokio::test]
    async fn multiple_matches_fail_without_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same same same").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", "same", "diff", None), &ctx)
            .await;
        assert!(out.is_error);
        assert!(out.output.contains("matches 3 times"));
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "same same same"
        );
    }

    #[tokio::test]
    async fn replace_all_replaces_every_occurrence() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same same same").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", "same", "diff", Some(true)), &ctx)
            .await;
        assert!(!out.is_error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "diff diff diff"
        );
    }
}
