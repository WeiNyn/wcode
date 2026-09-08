//! Optional AST-structural rewrite backed by ast-grep (`ast-grep`/`sg`).
//!
//! Companion to `ast_search`: same "auto if binary on PATH" registration. One
//! FILE per call, applied atomically (rewrite into a same-directory temp copy
//! that keeps the file's extension, then rename over the original — the same
//! crash-safe pattern as `edit`/`write`). Structural rewrites bypass line
//! anchors by design (whole-AST transforms), but the echoed diff carries line
//! numbers for anchor-based follow-ups.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use wcode_harness::tool::{ToolContext, ToolOutput, TypedTool};

use super::anchor;

#[derive(Deserialize, schemars::JsonSchema)]
pub struct AstEditArgs {
    /// File to rewrite (a single file; applied atomically).
    pub path: String,
    /// AST pattern; `$UPPERCASE` are wildcards (e.g. `println!($A, $B)`).
    pub pattern: String,
    /// Replacement template referencing the pattern's wildcards.
    pub rewrite: String,
    /// Force the language (e.g. "rust", "ts", "py"). Auto-detected when omitted.
    pub lang: Option<String>,
    /// `false` = dry-run: print the diff and write nothing. Default `true` =
    /// apply the rewrite and echo the diff.
    pub commit: Option<bool>,
}

pub struct AstEdit {
    lock: Arc<Mutex<()>>,
}

impl AstEdit {
    pub fn new(lock: Arc<Mutex<()>>) -> Self {
        Self { lock }
    }
}

/// The modern `ast-grep` binary is preferred (its `sg` alias prints a
/// deprecation banner on stderr); fall back to `sg`.
fn find_bin() -> Option<&'static str> {
    let paths = std::env::var_os("PATH")?;
    let has = |name: &str| {
        std::env::split_paths(&paths)
            .filter(|p| !p.as_os_str().is_empty())
            .any(|p| p.join(name).is_file())
    };
    if has("ast-grep") {
        Some("ast-grep")
    } else if has("sg") {
        Some("sg")
    } else {
        None
    }
}

/// Whether either ast-grep binary is on PATH. If false, the tool is simply not
/// registered (see tools::default_tools).
pub fn available() -> bool {
    find_bin().is_some()
}

/// The `sg` alias's deprecation banner pollutes stderr; drop it (and its
/// separators) so real errors are unambiguous.
fn clean_stderr(s: &str) -> String {
    s.lines()
        .filter(|l| !(l.starts_with("=====") || l.starts_with("WARNING:")))
        .collect::<Vec<_>>()
        .join("\n")
}

#[async_trait::async_trait]
impl TypedTool for AstEdit {
    type Args = AstEditArgs;
    fn name(&self) -> &str {
        "ast_edit"
    }
    fn description(&self) -> &str {
        "Structural code rewrite via ast-grep. `pattern` is ordinary code with `$UPPERCASE` wildcards (e.g. `println!($A, $B)`), `rewrite` re-emits those wildcards (e.g. `dbg!($B)`). Operates on ONE file per call, applied atomically. Default `commit:true` applies and echoes the diff; `commit:false` dry-runs (diff only, nothing written). Structural rewrites bypass line anchors, so re-read the file afterwards before anchoring further edits."
    }
    async fn execute(&self, args: Self::Args, ctx: &ToolContext) -> ToolOutput {
        let _guard = self.lock.lock().unwrap();
        let Some(bin) = find_bin() else {
            return ToolOutput {
                output:
                    "ast_edit: no ast-grep binary on PATH (tool should not have been registered)"
                        .into(),
                is_error: true,
                details: None,
            };
        };
        let path = super::resolve(&ctx.working_dir, &args.path);
        let original = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return ToolOutput {
                    output: format!("ast_edit {}: {e}", args.path),
                    is_error: true,
                    details: None,
                };
            }
        };

        // Dry-run: ast-grep with `-r` and no `-U` prints a diff and exits 0;
        // no match exits 1 with empty stdout. This is also the diff we echo.
        let mut base = std::process::Command::new(bin);
        base.arg("run")
            .arg("-p")
            .arg(&args.pattern)
            .arg("-r")
            .arg(&args.rewrite);
        if let Some(lang) = &args.lang {
            base.arg("--lang").arg(lang);
        }
        base.env("NO_COLOR", "1").arg(&path);

        let dry = match base.output() {
            Ok(o) => o,
            Err(e) => {
                return ToolOutput {
                    output: format!("ast_edit: failed to run `{bin}`: {e}"),
                    is_error: true,
                    details: None,
                };
            }
        };
        let diff = String::from_utf8_lossy(&dry.stdout).trim().to_string();
        let err = clean_stderr(&String::from_utf8_lossy(&dry.stderr));
        if diff.is_empty() && err.is_empty() {
            return ToolOutput {
                output: format!(
                    "ast_edit {}: no structural matches for `{}` — nothing to rewrite.",
                    args.path, args.pattern
                ),
                is_error: false,
                details: None,
            };
        }
        if !dry.status.success() {
            let body = if err.is_empty() {
                diff.clone()
            } else {
                err.clone()
            };
            return ToolOutput {
                output: format!(
                    "ast_edit {}: `{bin}` exited {}:\n{body}",
                    args.path, dry.status
                ),
                is_error: true,
                details: None,
            };
        }

        let commit = args.commit.unwrap_or(true);
        if !commit {
            return ToolOutput {
                output: format!(
                    "ast_edit {} (dry-run — nothing written). Diff:\n{diff}\nRe-run with commit:true to apply.",
                    args.path
                ),
                is_error: false,
                details: None,
            };
        }

        // Atomic apply: rewrite a same-directory temp copy that keeps the
        // file's extension (so ast-grep still detects the language), then
        // rename over the original.
        let suffix = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let tmp = path.with_extension(format!("wcode-ast{suffix}"));
        if let Err(e) = std::fs::write(&tmp, &original) {
            return ToolOutput {
                output: format!("ast_edit {}: {e}", args.path),
                is_error: true,
                details: None,
            };
        }
        let applied = {
            let mut c = std::process::Command::new(bin);
            c.arg("run")
                .arg("-p")
                .arg(&args.pattern)
                .arg("-r")
                .arg(&args.rewrite);
            if let Some(lang) = &args.lang {
                c.arg("--lang").arg(lang);
            }
            c.env("NO_COLOR", "1").arg("-U").arg(&tmp).output()
        };
        let updated = match applied {
            Ok(o) if o.status.success() => match std::fs::read_to_string(&tmp) {
                Ok(u) => u,
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp);
                    return ToolOutput {
                        output: format!("ast_edit {}: {e}", args.path),
                        is_error: true,
                        details: None,
                    };
                }
            },
            Ok(o) => {
                let body_stdout = String::from_utf8_lossy(&o.stdout).trim().to_string();
                let body_stderr = clean_stderr(&String::from_utf8_lossy(&o.stderr));
                let body = if body_stderr.is_empty() {
                    body_stdout
                } else {
                    body_stderr
                };
                let _ = std::fs::remove_file(&tmp);
                return ToolOutput {
                    output: format!(
                        "ast_edit {}: apply failed ({}):\n{body}",
                        args.path, o.status
                    ),
                    is_error: true,
                    details: None,
                };
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                return ToolOutput {
                    output: format!("ast_edit {}: failed to run `{bin}`: {e}", args.path),
                    is_error: true,
                    details: None,
                };
            }
        };
        // Install atomically and clean the temp copy.
        let _ = std::fs::remove_file(&tmp);
        let install = std::fs::write(path.with_extension("tmp-wcode"), &updated)
            .and_then(|_| std::fs::rename(path.with_extension("tmp-wcode"), &path));
        if let Err(e) = install {
            return ToolOutput {
                output: format!("ast_edit {}: {e}", args.path),
                is_error: true,
                details: None,
            };
        }

        let summary = match anchor::changed_range(&original, &updated) {
            Some((a, b)) if a == b => format!("line {a} changed"),
            Some((a, b)) => format!("lines {a}–{b} changed"),
            None => "no byte change (rewrite was idempotent)".to_string(),
        };
        ToolOutput {
            output: format!(
                "ast_edit {}: {summary}. Diff:\n{diff}\n(re-read {} for fresh anchors before further edits.)",
                args.path, args.path
            ),
            is_error: false,
            details: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_detects_binary_on_path() {
        let _ = available();
    }

    #[tokio::test]
    async fn dry_run_differs_from_commit() {
        if !available() {
            return; // environment lacks ast-grep; nothing to exercise
        }
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.rs");
        std::fs::write(
            &f,
            "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n",
        )
        .unwrap();
        let (ctx, _rx) = super::super::test_ctx(dir.path());

        // Dry run: diff echoed, file untouched.
        let out = AstEdit::new(Arc::new(Mutex::new(())))
            .execute(
                AstEditArgs {
                    path: "a.rs".into(),
                    pattern: "println!($A, $B)".into(),
                    rewrite: "dbg!($B)".into(),
                    lang: Some("rust".into()),
                    commit: Some(false),
                },
                &ctx,
            )
            .await;
        assert!(!out.is_error, "{}", out.output);
        assert!(out.output.contains("dbg!(x)"), "{}", out.output);
        assert!(out.output.contains("nothing written"), "{}", out.output);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "fn main() {\n    let x = 1;\n    println!(\"{}\", x);\n}\n"
        );

        // Commit: file rewritten, diff echoed.
        let out2 = AstEdit::new(Arc::new(Mutex::new(())))
            .execute(
                AstEditArgs {
                    path: "a.rs".into(),
                    pattern: "println!($A, $B)".into(),
                    rewrite: "dbg!($B)".into(),
                    lang: Some("rust".into()),
                    commit: Some(true),
                },
                &ctx,
            )
            .await;
        assert!(!out2.is_error, "{}", out2.output);
        assert!(out2.output.contains("changed"), "{}", out2.output);
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "fn main() {\n    let x = 1;\n    dbg!(x);\n}\n"
        );
    }
}
