use std::sync::Arc;

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

#[derive(Deserialize, schemars::JsonSchema)]
pub struct EditArgs {
    /// File path (relative to the working directory unless absolute).
    pub path: String,
    /// The exact text to replace — **byte-exact**, so whitespace counts. Must
    /// occur exactly once unless `replace_all`; include the surrounding lines to
    /// make it unique.
    pub old_string: String,
    /// The text to put in its place, verbatim — leading whitespace is preserved.
    /// Empty deletes the match.
    pub new_string: String,
    /// Replace every occurrence instead of requiring a unique match.
    pub replace_all: Option<bool>,
}

pub struct Edit {
    lock: Arc<tokio::sync::Mutex<()>>,
}

impl Edit {
    pub fn new(lock: Arc<tokio::sync::Mutex<()>>) -> Self {
        Self { lock }
    }
}

#[async_trait::async_trait]
impl TypedTool for Edit {
    type Args = EditArgs;
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "Replace an exact literal string in a file — content-addressed, so edits elsewhere never shift the target. `old_string` is byte-exact (whitespace counts) and must occur exactly once unless `replace_all`; include surrounding lines to make it unique. `new_string` is verbatim. Whole-file rewrite = `write`."
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

        if args.old_string.is_empty() {
            return ToolOutput {
                output: format!(
                    "[E_EMPTY_OLD_STRING] `old_string` is empty, so there is nothing to match in {}. Give the exact text to replace (or use `write` for a whole-file rewrite).",
                    args.path
                ),
                is_error: true,
                diff: None,
                path: None,
            };
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


        let replace_all = args.replace_all.unwrap_or(false);
        let matches = content.matches(&args.old_string).count();
        if matches == 0 {
            return ToolOutput {
                output: format!(
                    "[E_NO_MATCH] `old_string` does not occur in {}. The match is byte-exact — re-read the file and copy the text (whitespace counts).",
                    args.path
                ),
                is_error: true,
                diff: None,
                path: None,
            };
        }
        if matches > 1 && !replace_all {
            return ToolOutput {
                output: format!(
                    "[E_AMBIGUOUS_MATCH] `old_string` occurs {matches} times in {}. Include more surrounding lines to pin one, or set `replace_all: true` to replace every match.",
                    args.path
                ),
                is_error: true,
                diff: None,
                path: None,
            };
        }

        let updated = if replace_all {
            content.replace(&args.old_string, &args.new_string)
        } else {
            content.replacen(&args.old_string, &args.new_string, 1)
        };
        // Nothing to do: the replacement is what is already there. Report it and
        // write nothing, so the file's mtime is untouched.
        if updated == content {
            return ToolOutput {
                output: format!(
                    "no-op: {} already matches the replacement — nothing written",
                    args.path
                ),
                is_error: false,
                diff: None,
                path: None,
            };
        }
        let diff = super::diff::unified(&content, &updated);
        // tmp+rename so a crash mid-write can't truncate the original (same-fs rename).
        let tmp = super::temp_path(&path);
        match std::fs::write(&tmp, &updated).and_then(|_| std::fs::rename(&tmp, &path)) {
            Ok(_) => {
                let n = if replace_all { matches } else { 1 };
                ToolOutput {
                    output: format!(
                        "edited {} ({n} replacement{})",
                        args.path,
                        if n == 1 { "" } else { "s" }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> Edit {
        Edit::new(Arc::new(tokio::sync::Mutex::new(())))
    }

    fn args(path: &str, old: &str, new: &str, replace_all: Option<bool>) -> EditArgs {
        EditArgs {
            path: path.into(),
            old_string: old.into(),
            new_string: new.into(),
            replace_all,
        }
    }

    #[tokio::test]
    async fn replaces_a_unique_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a = 1\nb = 2\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", "a = 1", "a = 10", None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a = 10\nb = 2\n"
        );
        assert_eq!(out.path.as_deref(), Some("f.txt"));
    }

    #[tokio::test]
    async fn a_missing_old_string_refuses_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a = 1\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", "nope", "x", None), &ctx).await;
        assert!(out.is_error);
        assert!(out.output.contains("E_NO_MATCH"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "a = 1\n",
            "a refused edit must not touch the file"
        );
    }

    #[tokio::test]
    async fn an_ambiguous_match_refuses_unless_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "x = 1\nx = 1\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", "x = 1", "y", None), &ctx).await;
        assert!(out.is_error);
        assert!(out.output.contains("E_AMBIGUOUS_MATCH"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "x = 1\nx = 1\n"
        );

        let out = tool()
            .execute(args("f.txt", "x = 1", "y", Some(true)), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "y\ny\n"
        );
        assert!(out.output.contains("2 replacements"), "{}", out.output);
    }

    #[tokio::test]
    async fn an_empty_old_string_refuses() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "a\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", "", "x", None), &ctx).await;
        assert!(out.is_error);
        assert!(out.output.contains("E_EMPTY_OLD_STRING"), "{}", out.output);
    }

    #[tokio::test]
    async fn a_multiline_match_is_replaced_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "fn f() {\n    a();\n}\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool()
            .execute(args("f.txt", "    a();", "    b();\n    c();", None), &ctx)
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "fn f() {\n    b();\n    c();\n}\n"
        );
    }

    #[tokio::test]
    async fn an_empty_new_string_deletes_the_match() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("f.txt"), "keep\ndrop\n").unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());
        let out = tool().execute(args("f.txt", "drop\n", "", None), &ctx).await;
        assert!(!out.is_error, "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("f.txt")).unwrap(),
            "keep\n"
        );
    }
}
