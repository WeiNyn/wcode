use std::sync::Arc;

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
    /// Whole-file digest from your last read of this file (the `# <path>
    /// digest <hex>` line `read` prints). Auto-filled by the harness; normally
    /// leave unset. A mismatch refuses the call (E_STALE_DIGEST) — re-read.
    #[serde(default)]
    pub expected_digest: Option<String>,
}

pub struct Replace {
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Replace {
    pub fn new(lock: Arc<tokio::sync::Mutex<()>>) -> Self {
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
        "Replace an exact literal string in a file: old_string must occur exactly once unless replace_all is set. Byte-exact match (whitespace counts). Prefer edit for line-targeted changes."
    }
    /// Mutates the workspace — blocked in plan mode (`MUTATING_TOOLS`).
    fn mutating(&self) -> bool {
        true
    }

    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        // Async lock + sync fs: the guard may cross awaits, but the fs work stays
        // serialized and synchronous.
        let _guard = self.lock.lock().await;
        let path = super::resolve(&ctx.working_dir, &args.path);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("replace {}: {e}", args.path),
                    is_error: true,
                    diff: None,
                    path: None,
                };
            }
        };
        // Whole-file CAS (D1): refuse when the file moved since the digest was
        // captured, before touching the match count. Nothing is written.
        if let Some(out) =
            super::stale_digest_guard(&args.path, &content, args.expected_digest.as_deref())
        {
            return out;
        }
        let matches = content.matches(&args.old_string).count();
        if matches == 0 {
            return ToolOutput {
                output: format!("old_string not found in {}", args.path),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        if matches > 1 && !args.replace_all.unwrap_or(false) {
            return ToolOutput {
                output: format!(
                    "old_string matches {matches} times in {}; set replace_all: true or include more surrounding context",
                    args.path
                ),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        let updated = if args.replace_all.unwrap_or(false) {
            content.replace(&args.old_string, &args.new_string)
        } else {
            content.replacen(&args.old_string, &args.new_string, 1)
        };
        let diff = super::diff::unified(&content, &updated);
        // ponytail: tmp+rename so a crash mid-write can't truncate the original (same-fs rename).
        let tmp = super::temp_path(&path);
        match std::fs::write(&tmp, updated).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => ToolOutput {
                output: format!("replaced in {}", args.path),
                is_error: false,
                diff,
                path: Some(args.path.clone()),
            },
            Err(e) => ToolOutput {
                output: format!("replace {}: {e}", args.path),
                is_error: true,
                diff: None,
                path: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::anchor;

    #[tokio::test]
    async fn stale_digest_refuses_the_replace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha beta").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let mut a = args("f.txt", "beta", "BETA", None);
        a.expected_digest = Some("000000000000".into());
        let out = tool().execute(a, &ctx).await;
        assert!(out.is_error, "{}", out.output);
        assert!(out.output.contains("E_STALE_DIGEST"), "{}", out.output);
        assert!(out.output.contains("re-read"), "{}", out.output);
        // Nothing written.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "alpha beta"
        );
    }

    #[tokio::test]
    async fn matching_digest_allows_the_replace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "alpha beta").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let mut a = args("f.txt", "beta", "BETA", None);
        a.expected_digest = Some(anchor::file_digest(b"alpha beta"));
        let out = tool().execute(a, &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "alpha BETA"
        );
    }

    fn tool() -> Replace {
        Replace::new(Arc::new(tokio::sync::Mutex::new(())))
    }

    fn args(path: &str, old: &str, new: &str, replace_all: Option<bool>) -> ReplaceArgs {
        ReplaceArgs {
            path: path.into(),
            old_string: old.into(),
            new_string: new.into(),
            replace_all,
            expected_digest: None,
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

